//! coucou-hook — the relay Claude Code runs on every hook event.
//!
//! Reads the hook JSON on stdin, adds a little terminal context, and hands it to
//! Coucou over the named pipe `\\.\pipe\coucou-<sid>` (Windows) or the Unix
//! socket `$XDG_RUNTIME_DIR/coucou.sock` (Linux).
//!
//! Hard rule (docs/CLAUDE.md): **never block Claude Code.**
//! * If the pipe does not exist — Coucou is closed — we exit 0 immediately with
//!   nothing on stdout, and the session carries on untouched.
//! * Every step runs under a deadline enforced by the main thread, so a pipe that
//!   accepts the connection and then stops reading cannot wedge the session
//!   either: we abandon the worker and exit.
//! * Only `PermissionRequest` and a question (`--ask`) wait for an answer,
//!   because answering from the island is the whole point. No answer means
//!   empty stdout, and Claude Code asks in the terminal exactly as if Coucou
//!   were not installed.
//!
//! Usage: `coucou-hook [--agent cursor|kiro] [--ask] <EventName>` (the name is
//! also read from the JSON). Without `--agent` the caller is Claude Code.
//! `--ask` is the PreToolUse hook matched on `AskUserQuestion`: anything else
//! reaching it exits at once.
//! `coucou-hook --statusline [--wrap <hex>]` is Claude Code's status line.

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::Duration;

/// Budget for getting a pipe connection. Beyond this Claude Code wins, always.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
/// Whole-run budget for an event nobody waits on: connect and write, no more.
const FIRE_AND_FORGET_BUDGET: Duration = Duration::from_secs(2);
/// How long a permission prompt may stay on screen before the terminal takes over.
const DECISION_BUDGET: Duration = Duration::from_secs(110);

/// Fields that are pointless to forward and can be enormous (a whole file read,
/// a full command output). The island never shows them.
const DROPPED_FIELDS: &[&str] = &["tool_response", "tool_output", "transcript_path", "user_email"];
/// Longest string forwarded for any single field; the island truncates to far
/// less than this anyway.
const MAX_FIELD_LEN: usize = 2_000;

mod diff;

#[cfg(windows)]
mod win;
#[cfg(windows)]
use win::connect;

#[cfg(target_os = "linux")]
mod unix;
#[cfg(target_os = "linux")]
use unix::connect;

/// The event name the app knows a question by.
const QUESTION_EVENT: &str = "AskUserQuestion";

/// Set by Coucou on the agent CLIs it runs itself (its chat, `/usage`).
const QUIET_ENV: &str = "COUCOU_QUIET";

fn main() {
    if std::env::var_os(QUIET_ENV).is_some() {
        std::process::exit(0);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--statusline") {
        let wrapped = args
            .iter()
            .position(|a| a == "--wrap")
            .and_then(|i| args.get(i + 1))
            .and_then(|h| unhex(h));
        statusline(wrapped);
    }
    let Some(Event { payload, name: event, agent, questions }) = read_event() else { std::process::exit(0) };

    let waits_for_answer = event == "PermissionRequest" || event == QUESTION_EVENT;
    let budget = if waits_for_answer { DECISION_BUDGET } else { FIRE_AND_FORGET_BUDGET };

    // The worker owns every blocking call. If it overruns the budget we simply
    // stop listening and exit: the process dying takes the pipe handle with it.
    // (No catch_unwind here — the release profile is panic = "abort", so it would
    // be dead code. `talk` is written to have nothing to panic on instead.)
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let _ = tx.send(talk(&payload, waits_for_answer));
    });

    if let Ok(Some(decision)) = rx.recv_timeout(budget) {
        let reply = match &questions {
            Some(q) => question_answer(q, &decision).map(|out| (out, "", 0)),
            None => answer(&agent, &decision),
        };
        if let Some((stdout, stderr, code)) = reply {
            if !stdout.is_empty() {
                let mut out = std::io::stdout();
                let _ = writeln!(out, "{stdout}");
                let _ = out.flush();
            }
            if !stderr.is_empty() {
                let _ = writeln!(std::io::stderr(), "{stderr}");
            }
            std::process::exit(code);
        }
    }
    // Nothing printed: the agent asks or carries on as if we were not here.
    std::process::exit(0);
}

/// Claude Code's status line: hands the plan limits to the app, then runs the
/// status line the user had before (kept hex-encoded in `--wrap`) so it shows
/// exactly as it did. The app is told in parallel and never delays the line.
fn statusline(wrapped: Option<String>) -> ! {
    let mut raw = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut raw);

    let (tx, rx) = mpsc::channel::<()>();
    if let Some(line) = statusline_payload(&raw) {
        std::thread::spawn(move || {
            talk(&line, false);
            let _ = tx.send(());
        });
    } else {
        drop(tx);
    }

    let code = wrapped.map_or(0, |cmd| run_wrapped(&cmd, &raw));
    let _ = rx.recv_timeout(STATUSLINE_BUDGET);
    std::process::exit(code);
}

/// How long the relay may outlive the user's own status line.
const STATUSLINE_BUDGET: Duration = Duration::from_millis(500);

/// Only the limits travel: the rest of the status line JSON stays on this machine.
fn statusline_payload(raw: &[u8]) -> Option<String> {
    let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
    let v = serde_json::from_slice::<serde_json::Value>(raw).ok()?;
    let limits = v.get("rate_limits").filter(|l| l.is_object())?;
    let mut line = serde_json::json!({ "hook_event_name": "StatusLine", "rate_limits": limits }).to_string();
    line.push('\n');
    Some(line)
}

/// Runs the user's previous status line with the same input; its output and
/// exit code are the status line's.
fn run_wrapped(cmd: &str, input: &[u8]) -> i32 {
    use std::process::{Command, Stdio};
    let spawn = |program: &str, flag: &str| {
        Command::new(program)
            .args([flag, cmd])
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
    };
    // Claude Code runs status lines through `sh` (Git Bash on Windows).
    let child = spawn("sh", "-c");
    #[cfg(windows)]
    let child = child.or_else(|_| spawn("cmd", "/C"));
    let Ok(mut child) = child else { return 0 };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    child.wait().ok().and_then(|s| s.code()).unwrap_or(0)
}

fn unhex(s: &str) -> Option<String> {
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

/// The island's answer in each agent's own words: stdout, stderr, exit code.
/// Cursor reads `permission` from stdout; Kiro blocks a tool on exit code 2.
fn answer(agent: &str, decision: &str) -> Option<(String, &'static str, i32)> {
    let allow = match decision.trim() {
        "allow" | "always" => true,
        "deny" => false,
        _ => return None,
    };
    Some(match (agent, allow) {
        ("cursor", true) => (r#"{"permission":"allow"}"#.into(), "", 0),
        ("cursor", false) => (
            r#"{"permission":"deny","user_message":"Denied from Coucou","agent_message":"The user denied this command from Coucou."}"#.into(),
            "",
            0,
        ),
        ("kiro", true) => (String::new(), "", 0),
        ("kiro", false) => (String::new(), "Denied from Coucou", 2),
        _ => (decision_json(decision)?, "", 0),
    })
}

/// Cursor and Kiro name their events their own way; the app speaks Claude
/// Code. Shell commands become a PermissionRequest so the island can gate them
/// (the app declines at once unless approvals for agents are turned on).
fn normalize(agent: &str, event: &str, map: &mut serde_json::Map<String, serde_json::Value>) -> String {
    use serde_json::{json, Value};
    let tool = map.get("tool_name").and_then(Value::as_str).unwrap_or("");
    match (agent, event) {
        ("cursor", "beforeShellExecution") => {
            let command = map.get("command").cloned().unwrap_or(Value::Null);
            map.insert("tool_name".into(), json!("Shell"));
            map.insert("tool_input".into(), json!({ "command": command }));
            "PermissionRequest".into()
        }
        ("cursor", _) => match event {
            "beforeSubmitPrompt" => "UserPromptSubmit".into(),
            _ => {
                let mut c = event.chars();
                c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
            }
        },
        ("kiro", "PreToolUse") if tool == "execute_bash" || tool == "shell" => "PermissionRequest".into(),
        _ => event.into(),
    }
}

/// The documented PermissionRequest output. Anything we do not recognise prints
/// nothing at all rather than guessing — silence is the safe answer.
/// See https://code.claude.com/docs/en/hooks
fn decision_json(decision: &str) -> Option<String> {
    let behavior = match decision.trim() {
        // "always" still answers a plain allow; remembering it is the island's
        // business, not Claude Code's.
        "allow" | "always" => r#"{"behavior":"allow"}"#.to_string(),
        "deny" => r#"{"behavior":"deny","message":"Denied from Coucou"}"#.to_string(),
        _ => return None,
    };
    Some(format!(
        r#"{{"hookSpecificOutput":{{"hookEventName":"PermissionRequest","decision":{behavior}}}}}"#
    ))
}

/// The island answers a question with `{"answers":{"<question>":"<label>"}}`
/// (an array of labels for a multi-select). Claude Code takes them back as the
/// tool's input, next to the untouched questions.
fn question_answer(questions: &serde_json::Value, decision: &str) -> Option<String> {
    let reply = serde_json::from_str::<serde_json::Value>(decision.trim()).ok()?;
    let answers = reply.get("answers")?.as_object()?;
    if answers.is_empty() {
        return None;
    }
    Some(
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": { "questions": questions, "answers": answers },
            }
        })
        .to_string(),
    )
}

struct Event {
    /// The line to forward to the app.
    payload: String,
    name: String,
    agent: String,
    /// A question's `tool_input.questions` as Claude Code sent it, before any
    /// truncation: the answer must hand them back unchanged.
    questions: Option<serde_json::Value>,
}

/// Reads stdin and returns what to forward and how to answer.
fn read_event() -> Option<Event> {
    let mut raw = Vec::new();
    if std::io::stdin().read_to_end(&mut raw).is_err() || raw.is_empty() {
        return None;
    }
    // Some shells hand us a UTF-8 BOM; serde_json would choke on it.
    if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        raw.drain(..3);
    }

    let mut payload = serde_json::from_slice::<serde_json::Value>(&raw).ok()?;
    let map = payload.as_object_mut()?;

    // Parse argv: "coucou-hook.exe [--agent <name>] [<EventName>]"
    // --agent tags the payload with coucou_agent so the app routes to the right pill.
    // Absent or invalid names are validated and discarded by the app, not here.
    let mut agent = String::new();
    let mut arg_event = String::new();
    let mut ask = false;
    {
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            if arg == "--agent" {
                agent = it.next().unwrap_or_default();
            } else if arg == "--ask" {
                ask = true;
            } else if arg_event.is_empty() {
                arg_event = arg;
            }
        }
    }
    let questions = if ask {
        if map.get("tool_name").and_then(|v| v.as_str()) != Some(QUESTION_EVENT) {
            return None;
        }
        Some(map.get("tool_input")?.get("questions")?.clone())
    } else {
        None
    };
    // Which agent this hook was installed for. Absent means Claude Code,
    // so existing hook commands keep working unchanged.
    if !agent.is_empty() {
        map.insert("coucou_agent".into(), serde_json::Value::String(agent.clone()));
    }
    let event = map
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or(arg_event);
    diff::attach(&agent, &event, map);
    let event = if questions.is_some() { QUESTION_EVENT.to_string() } else { normalize(&agent, &event, map) };
    map.insert("hook_event_name".into(), serde_json::Value::String(event.clone()));

    for field in DROPPED_FIELDS {
        map.remove(*field);
    }

    // Cursor sends an empty cwd; its first workspace root is the project.
    let root = map
        .get("workspace_roots")
        .and_then(|v| v.get(0))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if let Some(root) = root {
        if map.get("cwd").and_then(|v| v.as_str()).map(str::is_empty).unwrap_or(true) {
            map.insert("cwd".into(), serde_json::Value::String(root));
        }
    }

    let cwd_missing = map
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(str::is_empty)
        .unwrap_or(true);
    if cwd_missing {
        if let Ok(cwd) = std::env::current_dir() {
            map.insert(
                "cwd".into(),
                serde_json::Value::String(cwd.to_string_lossy().to_string()),
            );
        }
    }

    // Which terminal the session runs in. Unlike macOS, Coucou here accepts
    // events from every terminal, so this is context only — never a filter.
    for (key, var) in [
        ("term_program", "TERM_PROGRAM"),
        ("wt_session", "WT_SESSION"),
        ("term_session_id", "TERM_SESSION_ID"),
        ("vscode_pid", "VSCODE_PID"),
        ("session_pid", "CLAUDE_CODE_SSE_PORT"),
    ] {
        if !map.contains_key(key) {
            let value = std::env::var(var).unwrap_or_default();
            map.insert(key.into(), serde_json::Value::String(value));
        }
    }

    truncate_strings(&mut payload);

    let mut line = payload.to_string();
    line.push('\n');
    Some(Event { payload: line, name: event, agent, questions })
}

/// Caps every string in the payload. A single Write can carry a whole file.
fn truncate_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            if s.len() > MAX_FIELD_LEN {
                // Cut on a char boundary; a lone byte index can split UTF-8.
                let mut end = MAX_FIELD_LEN;
                while end > 0 && !s.is_char_boundary(end) {
                    end -= 1;
                }
                s.truncate(end);
                s.push('…');
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(truncate_strings),
        serde_json::Value::Object(map) => map.values_mut().for_each(truncate_strings),
        _ => {}
    }
}

/// Connect, send, and — for a permission request — wait for the island's word.
fn talk(payload: &str, waits_for_answer: bool) -> Option<String> {
    let mut pipe = connect()?;

    if pipe.write_all(payload.as_bytes()).is_err() {
        return None;
    }
    let _ = pipe.flush();

    if !waits_for_answer {
        return None;
    }

    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let answer = String::from_utf8_lossy(&buf).trim().to_string();
    (!answer.is_empty()).then_some(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_json_matches_the_documented_shape() {
        assert_eq!(
            decision_json("allow").unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#
        );
        assert_eq!(
            decision_json("deny").unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied from Coucou"}}}"#
        );
        // "always" is an island concept; Claude Code just gets an allow.
        assert!(decision_json("always").unwrap().contains(r#""behavior":"allow""#));
    }

    #[test]
    fn anything_unrecognised_prints_nothing() {
        assert!(decision_json("").is_none());
        assert!(decision_json("maybe").is_none());
        // The shape the app used to send must not be mistaken for a decision.
        assert!(decision_json(r#"{"permissionDecision":"allow"}"#).is_none());
    }

    #[test]
    fn cursor_and_kiro_answers() {
        assert_eq!(answer("cursor", "allow").unwrap().0, r#"{"permission":"allow"}"#);
        assert!(answer("cursor", "deny").unwrap().0.contains(r#""permission":"deny""#));
        assert_eq!(answer("kiro", "deny").unwrap().2, 2);
        assert_eq!(answer("kiro", "allow").unwrap(), (String::new(), "", 0));
        assert!(answer("", "allow").unwrap().0.contains("hookSpecificOutput"));
        assert!(answer("cursor", "maybe").is_none());
    }

    #[test]
    fn cursor_and_kiro_events_are_normalized() {
        let mut m = serde_json::Map::new();
        assert_eq!(normalize("cursor", "sessionStart", &mut m), "SessionStart");
        assert_eq!(normalize("cursor", "postToolUseFailure", &mut m), "PostToolUseFailure");
        assert_eq!(normalize("cursor", "beforeSubmitPrompt", &mut m), "UserPromptSubmit");
        m.insert("command".into(), serde_json::json!("ls"));
        assert_eq!(normalize("cursor", "beforeShellExecution", &mut m), "PermissionRequest");
        assert_eq!(m["tool_input"]["command"], "ls");
        let mut k = serde_json::Map::new();
        k.insert("tool_name".into(), serde_json::json!("execute_bash"));
        assert_eq!(normalize("kiro", "PreToolUse", &mut k), "PermissionRequest");
        k.insert("tool_name".into(), serde_json::json!("fs_read"));
        assert_eq!(normalize("kiro", "PreToolUse", &mut k), "PreToolUse");
        assert_eq!(normalize("", "preToolUse", &mut k), "preToolUse");
    }

    #[test]
    fn question_answers_go_back_with_the_untouched_questions() {
        let questions = serde_json::json!([{ "question": "Which DB?", "options": [{ "label": "Postgres" }, { "label": "SQLite" }] }]);
        let out = question_answer(&questions, r#"{"answers":{"Which DB?":"Postgres"}}"#).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let o = &v["hookSpecificOutput"];
        assert_eq!(o["hookEventName"], "PreToolUse");
        assert_eq!(o["permissionDecision"], "allow");
        assert_eq!(o["updatedInput"]["questions"], questions);
        assert_eq!(o["updatedInput"]["answers"]["Which DB?"], "Postgres");
        // Multi-select answers are arrays and pass through as such.
        let multi = question_answer(&questions, r#"{"answers":{"Which DB?":["Postgres","SQLite"]}}"#).unwrap();
        assert!(multi.contains(r#"["Postgres","SQLite"]"#));
        // Anything else, including the permission words, prints nothing.
        for bad in ["allow", "deny", "", r#"{"answers":{}}"#, r#"{"other":1}"#] {
            assert!(question_answer(&questions, bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_status_line_relays_only_the_limits() {
        let raw = br#"{"model":{"id":"opus"},"cwd":"/secret","rate_limits":{"five_hour":{"used_percentage":42,"resets_at":1791050000}}}"#;
        let line = statusline_payload(raw).unwrap();
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["hook_event_name"], "StatusLine");
        assert_eq!(v["rate_limits"]["five_hour"]["used_percentage"], 42);
        assert!(!line.contains("secret"));
        assert!(statusline_payload(br#"{"model":{}}"#).is_none());
        assert!(statusline_payload(b"garbage").is_none());
    }

    #[test]
    fn the_wrapped_command_round_trips() {
        assert_eq!(unhex("6563686f2027c3a92720247e").as_deref(), Some("echo 'é' $~"));
        assert!(unhex("abc").is_none());
        assert!(unhex("zz").is_none());
    }

    #[test]
    fn long_strings_are_cut_on_a_char_boundary() {
        let mut v = serde_json::json!({ "tool_input": { "content": "é".repeat(4000) } });
        truncate_strings(&mut v);
        let s = v["tool_input"]["content"].as_str().unwrap();
        assert!(s.len() <= MAX_FIELD_LEN + 4);
        assert!(s.ends_with('…'));
    }
}
