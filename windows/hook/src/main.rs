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
//! * Only `PermissionRequest` waits for an answer, because approving from the
//!   island is the whole point. No answer means empty stdout, and Claude Code
//!   asks in the terminal exactly as if Coucou were not installed.
//!
//! Usage: `coucou-hook [--agent cursor|kiro] <EventName>` (the name is also
//! read from the JSON). Without `--agent` the caller is Claude Code.

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

/// Set by Coucou on the agent CLIs it runs itself (its chat, `/usage`).
const QUIET_ENV: &str = "COUCOU_QUIET";

fn main() {
    if std::env::var_os(QUIET_ENV).is_some() {
        std::process::exit(0);
    }
    let Some((payload, event, agent)) = read_event() else { std::process::exit(0) };

    let waits_for_answer = event == "PermissionRequest";
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
        if let Some((stdout, stderr, code)) = answer(&agent, &decision) {
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

/// Reads stdin and returns the payload to forward, the event name and the agent.
fn read_event() -> Option<(String, String, String)> {
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
    {
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            if arg == "--agent" {
                agent = it.next().unwrap_or_default();
            } else if arg_event.is_empty() {
                arg_event = arg;
            }
        }
    }
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
    let event = normalize(&agent, &event, map);
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
    Some((line, event, agent))
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
    fn long_strings_are_cut_on_a_char_boundary() {
        let mut v = serde_json::json!({ "tool_input": { "content": "é".repeat(4000) } });
        truncate_strings(&mut v);
        let s = v["tool_input"]["content"].as_str().unwrap();
        assert!(s.len() <= MAX_FIELD_LEN + 4);
        assert!(s.ends_with('…'));
    }
}
