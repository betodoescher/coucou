// Chat through the Cursor CLI (`agent`), on the user's own Cursor plan.
//
// Cursor has no public chat API, and calling its private endpoints breaks its
// terms. The CLI is the supported way: `agent login` once in a browser, then
// headless `agent -p` runs. It always runs Cursor's agent harness, so it is
// slower than a raw model call; `--mode ask` keeps it read-only.
//
// Runs in a private, empty folder so the agent has nothing of the user's to
// read, and the same folder every time because `--resume` is scoped to it.

use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::claude::{ChatContext, ChatReply, SYSTEM_PROMPT};
use crate::{platform, secrets};

pub const DEFAULT_MODEL: &str = "auto";
const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_INLINE_TEXT: u64 = 200_000;

/// The CLI session the next turn resumes; None until the first answer.
#[derive(Default)]
pub struct CursorChat {
    session: Mutex<Option<String>>,
}

impl CursorChat {
    pub fn reset(&self) {
        *self.session.lock().unwrap() = None;
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorStatus {
    /// Path of the CLI, or None when it is not installed.
    pub cli: Option<String>,
    /// What `agent status` says, e.g. "Logged in as …".
    pub status: String,
    pub logged_in: bool,
}

#[derive(Deserialize)]
struct CliResult {
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    result: String,
    session_id: Option<String>,
}

fn cli_path() -> Option<std::path::PathBuf> {
    find_cli(&["agent", "cursor-agent"])
}

/// CLI installers put their binary in ~/.local/bin, which a session started
/// from the desktop launcher often does not have on its PATH.
pub(crate) fn find_cli(names: &[&str]) -> Option<std::path::PathBuf> {
    names.iter().find_map(|n| platform::find_on_path(n)).or_else(|| {
        let bin = platform::home_dir().join(".local").join("bin");
        names.iter().map(|n| bin.join(n)).find(|p| p.is_file())
    })
}

fn workdir() -> Result<std::path::PathBuf, String> {
    let dir = crate::settings::local_dir().join("cursor-chat");
    platform::ensure_private_dir(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn command(cli: &std::path::Path) -> Command {
    let mut cmd = Command::new(cli);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    // Through the environment, never argv: other users can read argv in `ps`.
    // Without a key the CLI uses the `agent login` session, i.e. the plan.
    if let Some(key) = secrets::get("cursor-api-key") {
        cmd.env("CURSOR_API_KEY", key);
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

async fn run(mut cmd: Command) -> Result<std::process::Output, String> {
    let child = cmd.spawn().map_err(|e| format!("Could not start the Cursor CLI: {e}"))?;
    match tokio::time::timeout(TIMEOUT, child.wait_with_output()).await {
        Ok(out) => out.map_err(|e| e.to_string()),
        Err(_) => Err("Cursor took too long to answer.".into()),
    }
}

/// One chat turn.
pub async fn send(
    chat: &CursorChat,
    model: &str,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let cli = cli_path().ok_or("Cursor CLI not found. Install it, then run `agent login`.")?;
    let session = chat.session.lock().unwrap().clone();
    let prompt = build_prompt(session.is_none(), context, &query, "Cursor")?;

    let mut cmd = command(&cli);
    cmd.current_dir(workdir()?)
        .args(["-p", "--output-format", "json", "--mode", "ask", "--trust", "--model", model]);
    if let Some(id) = &session {
        cmd.args(["--resume", id]);
    }
    // `--` so a message starting with "-" is never read as an option.
    cmd.arg("--").arg(prompt);

    let out = run(cmd).await?;
    let parsed = parse_result(&out.stdout, &out.stderr)?;
    if let Some(id) = parsed.1 {
        *chat.session.lock().unwrap() = Some(id);
    }
    Ok(ChatReply { text: parsed.0 })
}

/// The message for a CLI that keeps the conversation itself: the system prompt
/// and the context go in the first turn only.
pub(crate) fn build_prompt(
    first: bool,
    context: Option<ChatContext>,
    query: &str,
    who: &str,
) -> Result<String, String> {
    let mut prompt = String::new();
    if first {
        prompt.push_str(SYSTEM_PROMPT);
        prompt.push_str("\n\n");
        match &context {
            Some(ChatContext::File { name, path }) => {
                let text = inline_text(path)
                    .ok_or(format!("Only text files can be sent to {who} for now."))?;
                prompt.push_str(&format!("File: {name}\nFile contents:\n{text}\n\n"));
            }
            Some(ChatContext::Window { app_name, title, url }) => {
                prompt.push_str(&format!("Context — App: {app_name}, Window: {title}"));
                if let Some(url) = url {
                    prompt.push_str(&format!(", URL: {url}"));
                }
                prompt.push_str("\n\n");
            }
            None => {}
        }
    }
    prompt.push_str(query);
    Ok(prompt)
}

/// (answer, session id) from the CLI's JSON, or the CLI's own error message.
fn parse_result(stdout: &[u8], stderr: &[u8]) -> Result<(String, Option<String>), String> {
    let stdout = String::from_utf8_lossy(stdout);
    let last = stdout.lines().rev().find(|l| l.trim_start().starts_with('{'));
    if let Some(r) = last.and_then(|l| serde_json::from_str::<CliResult>(l).ok()) {
        let text = r.result.trim().to_string();
        if r.is_error {
            return Err(if text.is_empty() { "Cursor returned an error.".into() } else { text });
        }
        if text.is_empty() {
            return Err("No response text.".into());
        }
        return Ok((text, r.session_id));
    }
    let stderr = String::from_utf8_lossy(stderr);
    let msg = [stderr.trim(), stdout.trim()].into_iter().find(|s| !s.is_empty()).unwrap_or("Cursor CLI failed.");
    Err(msg.chars().take(300).collect())
}

fn inline_text(path: &str) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > MAX_INLINE_TEXT {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Whether the CLI is installed and signed in, for the settings window.
pub async fn status() -> CursorStatus {
    let Some(cli) = cli_path() else {
        return CursorStatus { cli: None, status: String::new(), logged_in: false };
    };
    let mut cmd = command(&cli);
    cmd.arg("status");
    let text = match run(cmd).await {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(err) => err,
    };
    let text = strip_ansi(&text);
    CursorStatus {
        logged_in: text.to_lowercase().contains("logged in as"),
        status: text,
        cli: Some(cli.to_string_lossy().to_string()),
    }
}

/// Opens the browser sign-in. Returns at once; the settings window polls status.
pub fn login() -> Result<(), String> {
    let cli = cli_path().ok_or("Cursor CLI not found.")?;
    let mut cmd = std::process::Command::new(cli);
    cmd.arg("login").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    platform::no_console(&mut cmd).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// (id, label) pairs from `agent models`.
pub async fn models() -> Vec<(String, String)> {
    let Some(cli) = cli_path() else { return Vec::new() };
    let mut cmd = command(&cli);
    cmd.arg("models");
    match run(cmd).await {
        Ok(out) => parse_models(&strip_ansi(&String::from_utf8_lossy(&out.stdout))),
        Err(_) => Vec::new(),
    }
}

fn parse_models(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.trim().split_once(" - "))
        .filter(|(id, _)| !id.is_empty() && !id.contains(' '))
        .map(|(id, label)| (id.to_string(), label.trim().to_string()))
        .collect()
}

#[derive(Serialize, Debug, PartialEq)]
pub struct CursorPlan {
    pub plan: String,
    pub percent: f64,
    pub auto: Option<f64>,
    pub api: Option<f64>,
    pub resets: String,
}

/// What `/usage` shows. It only exists in the interactive CLI, so the CLI runs
/// in a pseudo-terminal (`script`), gets the command typed in, and its table is read.
#[cfg(unix)]
pub async fn plan_usage() -> Result<CursorPlan, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let cli = cli_path().ok_or("Cursor CLI not found.")?;
    let dir = crate::settings::local_dir().join("cursor-usage");
    platform::ensure_private_dir(&dir).map_err(|e| e.to_string())?;
    let quoted = format!("'{}'", cli.to_string_lossy().replace('\'', r"'\''"));
    let mut cmd = Command::new("script");
    cmd.args(["-qfc", &format!("stty cols 120 rows 40; exec {quoted}"), "/dev/null"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(key) = secrets::get("cursor-api-key") {
        cmd.env("CURSOR_API_KEY", key);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Could not start the Cursor CLI: {e}"))?;
    let (Some(mut stdin), Some(mut stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err("Could not start the Cursor CLI.".into());
    };

    let read = async {
        let (mut buf, mut chunk) = (Vec::new(), [0u8; 8192]);
        // Where the output stood at each try; None until the first one.
        let (mut typed_at, mut tries): (Option<usize>, u32) = (None, 0);
        loop {
            match tokio::time::timeout(Duration::from_millis(1500), stdout.read(&mut chunk)).await {
                Ok(Ok(0)) | Ok(Err(_)) => return Err("The Cursor CLI closed.".to_string()),
                Ok(Ok(n)) => {
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf);
                    if typed_at.is_some() && text.contains("Esc to close") {
                        return Ok(strip_ansi(&text));
                    }
                }
                // The screen has settled. Typed key by key: a pasted burst is
                // not read as a command. Retyped if the keys went unseen
                // because the CLI was still starting.
                Err(_) if !buf.is_empty() && tries < 3 => {
                    let unseen = typed_at.is_none_or(|at| !String::from_utf8_lossy(&buf[at..]).contains("usage"));
                    if unseen {
                        typed_at = Some(buf.len());
                        tries += 1;
                        for key in ["/", "u", "s", "a", "g", "e", "\r"] {
                            stdin.write_all(key.as_bytes()).await.map_err(|e| e.to_string())?;
                            tokio::time::sleep(Duration::from_millis(150)).await;
                        }
                    }
                }
                Err(_) => {}
            }
        }
    };
    let text = tokio::time::timeout(Duration::from_secs(45), read)
        .await
        .map_err(|_| "Cursor took too long to show its usage.".to_string())??;
    parse_plan(&text).ok_or_else(|| "Could not read Cursor's usage.".into())
}

// ponytail: no `script` on Windows; reading `/usage` there needs a ConPTY.
#[cfg(not(unix))]
pub async fn plan_usage() -> Result<CursorPlan, String> {
    Err("Not available on Windows yet.".into())
}

/// " Usage • Pro            Resets 15 de out."
/// " Included        69% used     ████"
fn parse_plan(text: &str) -> Option<CursorPlan> {
    let table = &text[text.rfind("Usage •")?..];
    let head = table.lines().next()?.trim_start_matches("Usage •").trim();
    let (plan, resets) = head.split_once("Resets ").unwrap_or((head, ""));
    let percent = |name: &str| {
        table.lines().find_map(|l| {
            let rest = l.trim().strip_prefix(name)?;
            rest.split_whitespace().next()?.strip_suffix('%')?.parse().ok()
        })
    };
    Some(CursorPlan {
        plan: plan.trim().into(),
        percent: percent("Included")?,
        auto: percent("Auto"),
        api: percent("API"),
        resets: resets.trim().into(),
    })
}

pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_answer_and_session_from_the_cli_json() {
        let ok = br#"{"type":"result","subtype":"success","is_error":false,"result":"oi","session_id":"abc"}"#;
        assert_eq!(parse_result(ok, b""), Ok(("oi".into(), Some("abc".into()))));

        let failed = br#"{"type":"result","is_error":true,"result":"quota exceeded"}"#;
        assert_eq!(parse_result(failed, b""), Err("quota exceeded".into()));

        // Not JSON at all: the CLI's own message is what the user needs.
        assert_eq!(
            parse_result(b"Cannot use this model: x.\n", b""),
            Err("Cannot use this model: x.".into())
        );
        assert_eq!(parse_result(b"", b"Not authenticated\n"), Err("Not authenticated".into()));
    }

    #[test]
    fn lists_models_and_skips_headers() {
        let text = "Available models\n\nauto - Auto (current, default)\ncomposer-2.5 - Composer 2.5\n";
        assert_eq!(
            parse_models(text),
            vec![
                ("auto".into(), "Auto (current, default)".into()),
                ("composer-2.5".into(), "Composer 2.5".into()),
            ]
        );
        assert_eq!(strip_ansi("\u{1b}[32m✓\u{1b}[0m Logged in"), "✓ Logged in");
    }

    #[test]
    fn reads_the_usage_table() {
        let text = "  → /usage   Show plan and on-demand usage\r\n  Auto\r\n\
            \u{1b}[1m Usage • Pro\u{1b}[22m          Resets 15 de out.\r\n\
            \x20Monthly plan and on-demand usage\r\n\r\n Category   Current   Usage\r\n\
            \x20Included   69% used   ████░░\r\n   Auto   68% used   ███░░\r\n\
            \x20  API   100% used   █████\r\n On-Demand   Disabled   ———\r\n Esc to close\r\n";
        assert_eq!(
            parse_plan(&strip_ansi(text)),
            Some(CursorPlan {
                plan: "Pro".into(),
                percent: 69.0,
                auto: Some(68.0),
                api: Some(100.0),
                resets: "15 de out.".into(),
            })
        );
        assert_eq!(parse_plan("Not logged in"), None);
    }
}
