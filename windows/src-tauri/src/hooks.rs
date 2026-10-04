// Claude Code hook installation.
//
// The rule from CLAUDE.md is strict and is followed to the letter:
// read %USERPROFILE%\.claude\settings.json, take a dated backup, merge without
// touching anybody else's hooks, show the diff, and write only after an explicit
// click. Uninstall removes Coucou's entries and nothing else.
//
// The command is only the quoted exe path in forward slashes plus the event name:
// on Windows Claude Code runs hook commands through Git Bash, and anything with
// PowerShell or cmd in it breaks.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager};
use crate::{platform, settings};

/// Which agent's hooks: Claude Code (`~/.claude/settings.json`), Cursor
/// (`~/.cursor/hooks.json`, editor and `agent` CLI) or Kiro CLI 3
/// (`~/.kiro/hooks/coucou.json`, a file that is Coucou's alone).
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Claude,
    Cursor,
    Kiro,
}

/// Every event the island reacts to, with the hook timeout written to settings.json.
/// PermissionRequest waits for a human, so it gets the decision timeout + 10 s.
pub const HOOK_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("SessionEnd", 10),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 10),
    ("PostToolUse", 10),
    ("PostToolUseFailure", 10),
    ("PermissionRequest", 120),
    ("Notification", 10),
    ("Stop", 10),
    ("StopFailure", 10),
    ("SubagentStart", 10),
    ("SubagentStop", 10),
];

/// Cursor's names. `beforeShellExecution` is the one that can wait for a click
/// (the relay turns it into a PermissionRequest).
const CURSOR_EVENTS: &[(&str, u64)] = &[
    ("sessionStart", 10),
    ("sessionEnd", 10),
    ("beforeSubmitPrompt", 10),
    ("preToolUse", 10),
    ("postToolUse", 10),
    ("postToolUseFailure", 10),
    ("beforeShellExecution", 120),
    ("afterFileEdit", 10),
    ("afterAgentResponse", 10),
    ("stop", 10),
    ("subagentStart", 10),
    ("subagentStop", 10),
];

/// Kiro CLI 3 triggers. PreToolUse of a shell command can wait for a click.
const KIRO_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("SessionEnd", 10),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 120),
    ("PostToolUse", 10),
    ("Stop", 10),
];

fn events(t: Target) -> &'static [(&'static str, u64)] {
    match t {
        Target::Claude => HOOK_EVENTS,
        Target::Cursor => CURSOR_EVENTS,
        Target::Kiro => KIRO_EVENTS,
    }
}

/// Marker that identifies a Coucou entry inside settings.json.
const MARKER: &str = "coucou-hook";

/// Claude Code's question tool. Its own PreToolUse entry waits for the answer
/// picked on the island (Claude Code 2.1.85+).
const QUESTION_TOOL: &str = "AskUserQuestion";
const QUESTION_TIMEOUT: u64 = 120;

/// Set on the agent CLIs Coucou runs itself (chat, `/usage`): their hooks
/// still fire, and coucou-hook stays silent rather than report them as sessions.
pub const QUIET_ENV: &str = "COUCOU_QUIET";

/// Coucou's own working folders for those CLIs, for agents that do not pass
/// the environment on to their hooks.
pub fn is_own_workdir(cwd: &str) -> bool {
    let cwd = Path::new(cwd);
    ["cursor-chat", "cursor-usage", "kiro-chat", "kiro-usage"]
        .iter()
        .any(|d| cwd.starts_with(settings::local_dir().join(d)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub installed: bool,
    /// Installed, but missing an entry this version adds: reinstalling fixes it.
    pub outdated: bool,
    pub settings_path: String,
    pub hook_path: String,
    pub hook_ready: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPreview {
    pub diff: String,
    pub backup: String,
    pub settings_path: String,
    /// Identifies the bytes this diff was computed from; handed back to `write`
    /// so we only ever apply what the user actually looked at.
    pub fingerprint: String,
}

pub fn settings_path(t: Target) -> PathBuf {
    let home = platform::home_dir();
    match t {
        Target::Claude => home.join(".claude").join("settings.json"),
        Target::Cursor => home.join(".cursor").join("hooks.json"),
        Target::Kiro => home.join(".kiro").join("hooks").join("coucou.json"),
    }
}

/// Reads the target's settings file.
///
/// The only error that means "start from nothing" is the file not being there.
/// Everything else — a lock held by another process, a permission problem, JSON
/// we cannot parse — is reported, because the alternative is treating somebody's
/// unreadable settings as an empty object and then writing that back over them.
fn read_settings(t: Target) -> Result<Value, String> {
    let path = settings_path(t);
    match std::fs::read(&path) {
        Ok(bytes) => parse_settings(&bytes, &path.display().to_string()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        // A lock, a permission problem, a bad drive: all of them mean we do not
        // know what is in there, and not knowing is not the same as empty.
        Err(err) => Err(format!("Can't read {}: {err}", path.display())),
    }
}

/// The parsing half of `read_settings`, split out so it can be tested without a
/// home directory.
fn parse_settings(bytes: &[u8], path: &str) -> Result<Value, String> {
    // PowerShell writes a UTF-8 BOM with `Set-Content -Encoding utf8`, and
    // serde_json refuses it. Stripping it is safe and well defined; guessing at
    // anything else is not.
    let text = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    match serde_json::from_slice::<Value>(text) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(format!("{path} isn't a JSON object — Coucou won't touch it.")),
        Err(err) => Err(format!(
            "{path} isn't valid JSON ({err}). Fix or move it, then try again — Coucou won't overwrite it."
        )),
    }
}

/// The settings as they are, or an empty object when we cannot tell. Only for
/// read-only paths like `status()`, which must never fail loudly; anything that
/// writes uses `read_settings()` and surfaces the error instead.
fn read_settings_lossy(t: Target) -> Value {
    read_settings(t).unwrap_or_else(|_| json!({}))
}

fn agent_flag(t: Target) -> &'static str {
    match t {
        Target::Claude => "",
        Target::Cursor => "--agent cursor ",
        Target::Kiro => "--agent kiro ",
    }
}

fn hook_command(t: Target, event: &str) -> String {
    format!("{} {}{event}", quoted_exe(), agent_flag(t))
}

#[cfg(windows)]
fn quoted_exe() -> String {
    format!("\"{}\"", settings::hook_exe_path().to_string_lossy().replace('\\', "/"))
}

/// Claude Code runs the command through `sh`, which still reads `$`, `` ` ``
/// and `\` inside double quotes. Single quotes keep the path a path, whatever
/// the home directory is called.
#[cfg(unix)]
fn quoted_exe() -> String {
    sh_quote(&settings::hook_exe_path().to_string_lossy())
}

/// Claude Code's PreToolUse entry for its question tool.
fn question_entry() -> Value {
    json!({
        "matcher": QUESTION_TOOL,
        "hooks": [{
            "type": "command",
            "command": format!("{} --ask PreToolUse", quoted_exe()),
            "timeout": QUESTION_TIMEOUT,
        }]
    })
}

fn is_question_entry(entry: &Value) -> bool {
    entry.get("matcher").and_then(Value::as_str) == Some(QUESTION_TOOL) && entry_is_ours(entry)
}

/// Claude Code only hands the plan limits to its status line. Ours relays
/// them and then runs the user's own status line, carried hex-encoded in the
/// command so uninstalling can put it back exactly.
fn statusline_command(previous: Option<&str>) -> String {
    match previous {
        Some(cmd) => format!("{} --statusline --wrap {}", quoted_exe(), hex(cmd)),
        None => format!("{} --statusline", quoted_exe()),
    }
}

fn statusline_ours(line: &Value) -> Option<&str> {
    line.get("command").and_then(Value::as_str).filter(|c| c.contains(MARKER))
}

/// The user's own status line command wrapped inside ours, if any.
fn statusline_wrapped(command: &str) -> Option<String> {
    let (_, rest) = command.split_once("--wrap ")?;
    unhex(rest.split_whitespace().next()?)
}

/// Whether installing would add our status line (a line Coucou can't wrap doesn't count).
fn statusline_missing(settings: &Value) -> bool {
    let line = &settings["statusLine"];
    line.is_null() || line["command"].as_str().is_some() && statusline_ours(line).is_none()
}

fn with_our_statusline(root: &mut Map<String, Value>) {
    let line = match root.get("statusLine") {
        None => json!({ "type": "command", "command": statusline_command(None) }),
        Some(Value::Object(current)) => {
            let Some(command) = current.get("command").and_then(Value::as_str) else { return };
            let previous = match statusline_ours(&Value::Object(current.clone())) {
                Some(ours) => statusline_wrapped(ours),
                None => Some(command.to_string()),
            };
            let mut line = current.clone();
            line.insert("command".into(), json!(statusline_command(previous.as_deref())));
            Value::Object(line)
        }
        Some(_) => return,
    };
    root.insert("statusLine".into(), line);
}

fn without_our_statusline(root: &mut Map<String, Value>) {
    let Some(line) = root.get("statusLine") else { return };
    let Some(ours) = statusline_ours(line) else { return };
    match statusline_wrapped(ours) {
        Some(previous) => {
            let mut line = line.as_object().cloned().unwrap_or_default();
            line.insert("command".into(), json!(previous));
            root.insert("statusLine".into(), Value::Object(line));
        }
        None => {
            root.remove("statusLine");
        }
    }
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
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

/// `s` as one single-quoted shell word: `'` becomes `'\''`, nothing else is
/// special inside single quotes.
#[cfg(unix)]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Claude nests commands in `{"hooks":[{"command"}]}`; Cursor puts `command`
/// on the entry itself.
fn entry_is_ours(entry: &Value) -> bool {
    let is_ours = |h: &Value| {
        h.get("command")
            .and_then(Value::as_str)
            .map(|c| c.contains(MARKER))
            .unwrap_or(false)
    };
    is_ours(entry)
        || entry
            .get("hooks")
            .and_then(Value::as_array)
            .map(|hooks| hooks.iter().any(is_ours))
            .unwrap_or(false)
}

/// Settings with Coucou's hooks added; everything else is left untouched.
/// Kiro's file is Coucou's alone, so it is simply the whole thing.
fn merged(t: Target, existing: &Value) -> Value {
    if t == Target::Kiro {
        let hooks: Vec<Value> = KIRO_EVENTS
            .iter()
            .map(|(event, timeout)| {
                json!({
                    "name": format!("coucou-{event}"),
                    "trigger": event,
                    "action": { "type": "command", "command": hook_command(t, event) },
                    "timeout": timeout,
                    "enabled": true,
                })
            })
            .collect();
        return json!({ "version": "v1", "hooks": hooks });
    }

    let mut root = existing.as_object().cloned().unwrap_or_default();
    if t == Target::Cursor && !root.contains_key("version") {
        root.insert("version".into(), json!(1));
    }
    let mut hooks = root
        .get("hooks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new);

    for (event, timeout) in events(t) {
        let mut list = hooks
            .get(*event)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        list.retain(|entry| !entry_is_ours(entry));
        list.push(match t {
            Target::Cursor => json!({ "command": hook_command(t, event), "timeout": timeout }),
            _ => json!({
                "hooks": [{
                    "type": "command",
                    "command": hook_command(t, event),
                    "timeout": timeout,
                }]
            }),
        });
        if t == Target::Claude && *event == "PreToolUse" {
            list.push(question_entry());
        }
        hooks.insert((*event).to_string(), Value::Array(list));
    }

    root.insert("hooks".into(), Value::Object(hooks));
    if t == Target::Claude {
        with_our_statusline(&mut root);
    }
    Value::Object(root)
}

/// Settings with every Coucou entry removed, and nothing else changed.
/// For Kiro that is no file at all, written as `null`.
fn without_ours(t: Target, existing: &Value) -> Value {
    if t == Target::Kiro {
        return Value::Null;
    }
    let mut root = existing.as_object().cloned().unwrap_or_default();
    if t == Target::Claude {
        without_our_statusline(&mut root);
    }
    let Some(hooks) = root.get("hooks").and_then(Value::as_object).cloned() else {
        return Value::Object(root);
    };
    let mut out = Map::new();
    for (event, value) in hooks {
        match value.as_array() {
            Some(list) => {
                let kept: Vec<Value> =
                    list.iter().filter(|e| !entry_is_ours(e)).cloned().collect();
                if !kept.is_empty() {
                    out.insert(event, Value::Array(kept));
                }
            }
            None => {
                out.insert(event, value);
            }
        }
    }
    if out.is_empty() {
        root.remove("hooks");
    } else {
        root.insert("hooks".into(), Value::Object(out));
    }
    Value::Object(root)
}

fn pretty(v: &Value) -> String {
    if v.is_null() {
        return String::new();
    }
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// Down to the second: installing then uninstalling in the same minute must not
/// quietly overwrite the first backup.
fn stamp() -> String {
    let t = platform::local_time();
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

/// None for Kiro: its file is Coucou's own, there is nobody else's work to keep.
fn backup_path(t: Target) -> Option<PathBuf> {
    let p = settings_path(t);
    let name = p.file_name()?.to_string_lossy().to_string();
    (t != Target::Kiro).then(|| p.with_file_name(format!("{name}.bak-{}", stamp())))
}

/// Identifies the exact bytes a preview was computed from. FNV-1a is plenty:
/// the question is only "is this still the file I showed the user?".
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{hash:016x}")
}

fn current_fingerprint(t: Target) -> String {
    match std::fs::read(settings_path(t)) {
        Ok(bytes) => fingerprint(&bytes),
        Err(_) => fingerprint(b""),
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn status(t: Target) -> HookStatus {
    let current = read_settings_lossy(t);
    let installed = match t {
        Target::Kiro => settings_path(t).exists() && current.to_string().contains(MARKER),
        _ => current
            .get("hooks")
            .and_then(Value::as_object)
            .map(|hooks| {
                hooks
                    .values()
                    .filter_map(Value::as_array)
                    .flatten()
                    .any(entry_is_ours)
            })
            .unwrap_or(false),
    };
    let outdated = installed
        && t != Target::Kiro
        && (events(t).iter().any(|(event, _)| {
            !current["hooks"][*event].as_array().is_some_and(|list| list.iter().any(entry_is_ours))
        }) || t == Target::Claude
            && (!current["hooks"]["PreToolUse"]
                .as_array()
                .is_some_and(|list| list.iter().any(is_question_entry))
                || statusline_missing(&current)));
    let hook_path = settings::hook_exe_path();
    HookStatus {
        installed,
        outdated,
        settings_path: settings_path(t).to_string_lossy().to_string(),
        hook_ready: hook_path.exists(),
        hook_path: hook_path.to_string_lossy().to_string(),
    }
}

/// What the file holds now, as the diff should show it: nothing when absent.
fn shown(t: Target, current: &Value) -> String {
    if settings_path(t).exists() { pretty(current) } else { String::new() }
}

pub fn preview(t: Target, install: bool) -> Result<HookPreview, String> {
    let current = read_settings(t)?;
    let next = if install { merged(t, &current) } else { without_ours(t, &current) };
    Ok(HookPreview {
        diff: unified_diff(&shown(t, &current), &pretty(&next)),
        backup: backup_path(t).map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        settings_path: settings_path(t).to_string_lossy().to_string(),
        fingerprint: current_fingerprint(t),
    })
}

/// Writes the merged (or cleaned) settings after taking a dated backup.
///
/// `fingerprint` is the one the preview was computed from. If the file changed
/// in between — another tool, another window, the user's own editor — we stop
/// and make them look at a fresh diff, because the only thing worse than not
/// installing the hooks is silently reverting somebody else's edit.
pub fn write(t: Target, install: bool, fingerprint: &str) -> Result<String, String> {
    let path = settings_path(t);
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    // Read before the backup: an unreadable file must abort before we touch
    // anything at all.
    let current = read_settings(t)?;
    if current_fingerprint(t) != fingerprint {
        return Err(format!(
            "{} changed since the preview. Nothing was written — review the new diff.",
            path.display()
        ));
    }

    let backup = backup_path(t);
    if let (Some(backup), true) = (&backup, path.exists()) {
        std::fs::copy(&path, backup).map_err(|e| format!("backup failed: {e}"))?;
    }
    let backup = backup.map(|p| p.to_string_lossy().to_string()).unwrap_or_default();

    let next = if install { merged(t, &current) } else { without_ours(t, &current) };
    if next.is_null() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("remove failed: {e}")),
            _ => Ok(backup),
        };
    }
    let mut text = pretty(&next);
    text.push('\n');

    // A dotfiles setup often makes settings.json a symlink: write to the file it
    // points at, so the link survives the rename below.
    #[cfg(unix)]
    let path = std::fs::canonicalize(&path).unwrap_or(path);

    // Write beside the target and rename over it: a crash or a full disk leaves
    // the original settings.json intact rather than half a file.
    let temp = path.with_extension(format!("json.coucou-{}", std::process::id()));
    if let Err(err) = write_like(&temp, &path, text.as_bytes()) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("write failed: {err}"));
    }
    if let Err(err) = std::fs::rename(&temp, &path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("write failed: {err}"));
    }
    Ok(backup)
}

/// Writes `bytes` to `temp`, which is about to replace `original`.
///
/// On Linux a fresh file would get the umask's 0644, and settings.json can hold
/// API keys in its `env` block: the new file is created readable by us only,
/// then given the original's permissions, so the rename never widens them.
fn write_like(temp: &Path, original: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(temp)?;
    file.write_all(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(original)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0o600);
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = original;
    Ok(())
}

/// Copies the relay (coucou-hook.exe / coucou-hook) into the local data dir's
/// bin/ on launch. In a bundled install it comes from the app resources; in
/// `tauri dev` it sits next to the app binary in the workspace target directory.
///
/// Every candidate is tried rather than just the first, because getting this
/// wrong is silent and fatal: `resources` used to be a glob, which made NSIS
/// mirror the source path into `_up_\target\release\`, no candidate matched, and
/// the relay was simply never installed. It only looked healthy on a developer
/// machine, where a leftover copy from `tauri dev` was already sitting in bin/.
pub fn ensure_hook_exe(app: &AppHandle) {
    let dest = settings::hook_exe_path();
    let Some(dir) = dest.parent() else { return };
    // Nobody else may swap the relay Claude Code runs: its folder is ours only.
    if platform::ensure_private_dir(&settings::local_dir()).is_err()
        || std::fs::create_dir_all(dir).is_err()
    {
        return;
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = app.path().resolve(platform::HOOK_EXE, tauri::path::BaseDirectory::Resource) {
        candidates.push(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // Installed build, then `tauri dev` (target/debug) next to the
            // release hook the pre-build step produces.
            candidates.push(parent.join(platform::HOOK_EXE));
            candidates.push(parent.join("../release").join(platform::HOOK_EXE));
            // Belt and braces: where the old glob form used to land it.
            candidates.push(parent.join("_up_/target/release").join(platform::HOOK_EXE));
        }
    }

    let tried: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
    let Some(src) = candidates.into_iter().find(|p| p.exists()) else {
        crate::log::line(format!(
            "{} not found — Claude Code hooks cannot work. Looked in: {}",
            platform::HOOK_EXE,
            tried.join(", ")
        ));
        return;
    };
    install_relay(&src, &dest);
}

#[cfg(windows)]
fn install_relay(src: &Path, dest: &Path) {
    let same = match (std::fs::metadata(src), std::fs::metadata(dest)) {
        (Ok(a), Ok(b)) => a.len() == b.len() && a.modified().ok() == b.modified().ok(),
        _ => false,
    };
    if same {
        return;
    }
    // A hook may be running right now and hold the file open; keeping the old
    // copy is fine, it is the same relay.
    if let Err(err) = std::fs::copy(src, dest) {
        if !dest.exists() {
            crate::log::line(format!("could not install {}: {err}", platform::HOOK_EXE));
        }
    }
}

/// Linux does not keep the modification time on copy, so the contents decide.
/// The new relay is written beside the old one and renamed over it: a hook
/// starting at that moment runs either the old relay or the new one, never half
/// of one, and a relay that is running right now does not block the update.
#[cfg(unix)]
fn install_relay(src: &Path, dest: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if matches!((std::fs::read(src), std::fs::read(dest)), (Ok(a), Ok(b)) if a == b) {
        return;
    }
    let temp = dest.with_extension(format!("new-{}", std::process::id()));
    let result = std::fs::copy(src, &temp)
        .and_then(|_| std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755)))
        .and_then(|_| std::fs::rename(&temp, dest));
    if let Err(err) = result {
        let _ = std::fs::remove_file(&temp);
        crate::log::line(format!("could not install {}: {err}", platform::HOOK_EXE));
    }
}

// ── Minimal unified diff (LCS) ────────────────────────────────────────────────

/// settings.json is short, so a plain O(n·m) LCS is the simplest honest diff.
fn unified_diff(before: &str, after: &str) -> String {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let (n, m) = (a.len(), b.len());

    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut out: Vec<String> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(format!("  {}", a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    while i < n {
        out.push(format!("- {}", a[i]));
        i += 1;
    }
    while j < m {
        out.push(format!("+ {}", b[j]));
        j += 1;
    }

    // Keep three lines of context around each change so the panel stays readable.
    let changed: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with('+') || l.starts_with('-'))
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return "No change.".into();
    }
    let mut keep = vec![false; out.len()];
    for idx in changed {
        let lo = idx.saturating_sub(3);
        let hi = (idx + 4).min(out.len());
        for k in lo..hi {
            keep[k] = true;
        }
    }
    let mut result = String::new();
    let mut gap = false;
    for (idx, line) in out.iter().enumerate() {
        if keep[idx] {
            result.push_str(line);
            result.push('\n');
            gap = false;
        } else if !gap {
            result.push_str("  …\n");
            gap = true;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHERE: &str = "settings.json";

    #[test]
    fn a_utf8_bom_is_stripped_not_treated_as_corruption() {
        // PowerShell 5's `Set-Content -Encoding utf8` produces exactly this.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"model":"opus","hooks":{}}"#);
        let parsed = parse_settings(&bytes, WHERE).expect("a BOM must not defeat the parser");
        assert_eq!(parsed["model"], "opus");
    }

    #[test]
    fn unreadable_content_is_an_error_never_an_empty_object() {
        // This is the whole bug: returning {} here meant `merged()` produced a
        // file containing nothing but Coucou's hooks, and the write replaced
        // everything the user had.
        for bad in [&b"{ not json"[..], &b"[1,2,3]"[..], &b"\"a string\""[..]] {
            assert!(
                parse_settings(bad, WHERE).is_err(),
                "content we cannot use must refuse, not come back empty"
            );
        }
    }

    #[test]
    fn empty_and_whitespace_files_start_from_nothing() {
        assert_eq!(parse_settings(b"", WHERE).unwrap(), json!({}));
        assert_eq!(parse_settings(b"  
	 ", WHERE).unwrap(), json!({}));
    }

    #[test]
    fn merging_keeps_every_other_setting_and_every_foreign_hook() {
        let existing = serde_json::json!({
            "model": "claude-opus-5",
            "theme": "dark",
            "enabledPlugins": ["a", "b"],
            "hooks": {
                "PreToolUse": [
                    { "hooks": [{ "type": "command", "command": "someone-elses-tool.exe" }] }
                ],
                "SomeEventWeDoNotTouch": [
                    { "hooks": [{ "type": "command", "command": "keep-me.exe" }] }
                ]
            }
        });

        let after = merged(Target::Claude, &existing);
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["enabledPlugins"], serde_json::json!(["a", "b"]));

        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(
            pre.iter().any(|e| serde_json::to_string(e).unwrap().contains("someone-elses-tool.exe")),
            "another tool's hook was dropped"
        );
        assert!(pre.iter().any(entry_is_ours), "our own hook was not added");
        let ask = pre.iter().find(|e| is_question_entry(e)).expect("the question hook was not added");
        assert!(ask["hooks"][0]["command"].as_str().unwrap().ends_with("--ask PreToolUse"));
        assert_eq!(ask["hooks"][0]["timeout"], QUESTION_TIMEOUT);
        assert_eq!(
            merged(Target::Claude, &after)["hooks"]["PreToolUse"].as_array().unwrap().len(),
            pre.len(),
            "merging twice duplicated an entry"
        );
        assert!(after["hooks"]["SomeEventWeDoNotTouch"].is_array());
        assert!(!after.to_string().contains("--agent"), "Claude commands carry no agent flag");

        // And removing ours puts it back exactly as it was.
        let cleaned = without_ours(Target::Claude, &after);
        assert_eq!(cleaned, existing);
    }

    #[test]
    fn the_status_line_wraps_the_users_own_and_gives_it_back() {
        let none = merged(Target::Claude, &json!({}));
        assert!(none["statusLine"]["command"].as_str().unwrap().ends_with("--statusline"));
        assert!(!statusline_missing(&none));
        assert_eq!(without_ours(Target::Claude, &none), json!({}));

        let mine = json!({ "statusLine": { "type": "command", "command": "~/bin/line.sh \"$X\"", "padding": 2 } });
        assert!(statusline_missing(&mine));
        let wrapped = merged(Target::Claude, &mine);
        let command = wrapped["statusLine"]["command"].as_str().unwrap();
        assert!(command.contains("--statusline --wrap "));
        assert_eq!(statusline_wrapped(command).as_deref(), Some("~/bin/line.sh \"$X\""));
        assert_eq!(wrapped["statusLine"]["padding"], 2);
        // Reinstalling keeps the same wrapped line instead of wrapping ours.
        let again = merged(Target::Claude, &wrapped);
        let again = again["statusLine"]["command"].as_str().unwrap();
        assert_eq!(statusline_wrapped(again).as_deref(), Some("~/bin/line.sh \"$X\""));
        let restored = without_ours(Target::Claude, &wrapped);
        assert_eq!(restored["statusLine"], mine["statusLine"]);

        // A status line without a command is left alone.
        let odd = json!({ "statusLine": { "type": "static" } });
        assert_eq!(merged(Target::Claude, &odd)["statusLine"], odd["statusLine"]);
        assert!(!statusline_missing(&odd));
        // Cursor has no status line.
        assert!(merged(Target::Cursor, &json!({})).get("statusLine").is_none());
    }

    #[test]
    fn cursor_hooks_merge_flat_and_keep_other_tools() {
        let existing = serde_json::json!({
            "version": 1,
            "hooks": {
                "preToolUse": [{ "command": "orca-hook.sh", "timeout": 10 }],
                "afterAgentResponse": [{ "command": "keep-me.sh" }]
            }
        });
        let after = merged(Target::Cursor, &existing);
        let pre = after["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(pre[0]["command"], "orca-hook.sh");
        assert!(pre[1]["command"].as_str().unwrap().contains("--agent cursor preToolUse"));
        assert_eq!(after["hooks"]["beforeShellExecution"][0]["timeout"], 120);
        // Merging twice does not duplicate. Counted, not compared: another test
        // moves the home directory, so the relay path can change mid-test.
        let again = merged(Target::Cursor, &after);
        for (event, _) in CURSOR_EVENTS {
            assert_eq!(again["hooks"][*event].as_array().map(Vec::len), after["hooks"][*event].as_array().map(Vec::len));
        }
        assert_eq!(without_ours(Target::Cursor, &after), existing);
        // A missing file gets the schema version.
        assert_eq!(merged(Target::Cursor, &json!({}))["version"], 1);
    }

    #[test]
    fn kiro_file_is_ours_alone() {
        let after = merged(Target::Kiro, &json!({}));
        assert_eq!(after["version"], "v1");
        let hooks = after["hooks"].as_array().unwrap();
        assert_eq!(hooks.len(), KIRO_EVENTS.len());
        assert!(hooks[0]["action"]["command"].as_str().unwrap().contains("--agent kiro SessionStart"));
        assert!(without_ours(Target::Kiro, &after).is_null());
    }

    #[test]
    fn a_fingerprint_notices_any_change() {
        assert_eq!(fingerprint(b"{}"), fingerprint(b"{}"));
        assert_ne!(fingerprint(b"{}"), fingerprint(b"{ }"));
        assert_ne!(fingerprint(b""), fingerprint(b"{}"));
    }

    #[cfg(unix)]
    #[test]
    fn the_hook_path_is_one_shell_word_whatever_it_contains() {
        assert_eq!(sh_quote("/home/a b/x"), "'/home/a b/x'");
        // $, backticks, backslashes and double quotes stay literal in single quotes.
        assert_eq!(sh_quote(r#"/h/$(id)`x`\"y"#), r#"'/h/$(id)`x`\"y'"#);
        // A single quote closes, escapes and reopens.
        assert_eq!(sh_quote("/h/it's"), r"'/h/it'\''s'");
    }

    /// settings.json can carry API keys in its `env` block: rewriting it must
    /// never make it readable by more people than before.
    #[cfg(unix)]
    #[test]
    fn rewriting_settings_never_widens_its_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("coucou-perm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("settings.json");
        let temp = dir.join("settings.json.new");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

        for wanted in [0o600, 0o640, 0o644] {
            std::fs::write(&original, b"{}").unwrap();
            std::fs::set_permissions(&original, std::fs::Permissions::from_mode(wanted)).unwrap();
            let _ = std::fs::remove_file(&temp);
            write_like(&temp, &original, b"{\"a\":1}").unwrap();
            assert_eq!(mode(&temp), wanted, "the rewrite must keep {wanted:o}");
        }

        // No original: ours only.
        std::fs::remove_file(&original).unwrap();
        let _ = std::fs::remove_file(&temp);
        write_like(&temp, &original, b"{}").unwrap();
        assert_eq!(mode(&temp), 0o600);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Everything filesystem-shaped lives in one test on purpose: it points
    /// the home directory at a temp directory, and that is process-wide.
    #[test]
    fn writing_backs_up_preserves_and_refuses_a_changed_file() {
        let tmp = std::env::temp_dir().join(format!("coucou-hooks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".claude")).unwrap();
        std::env::set_var(platform::HOME_VAR, &tmp);

        let c = Target::Claude;
        let path = settings_path(c);
        assert!(path.starts_with(&tmp), "the test must not touch the real home");

        // A real-shaped file, written the way PowerShell 5 would: UTF-8 with BOM.
        let original = r#"{"model":"claude-opus-5","theme":"dark","tui":{"x":1},"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"other-tool.exe"}]}]}}"#;
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(original.as_bytes());
        std::fs::write(&path, &bytes).unwrap();

        // Install.
        let plan = preview(c, true).expect("a BOM must not stop the preview");
        assert!(plan.diff.contains("coucou-hook"), "the diff must show what changes");
        let backup = write(c, true, &plan.fingerprint).expect("install should succeed");

        // The backup holds the original bytes, BOM and all.
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);

        // Everything else survived, and so did the other tool's hook.
        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["tui"]["x"], 1);
        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(pre.iter().any(|e| serde_json::to_string(e).unwrap().contains("other-tool.exe")));
        assert!(status(c).installed);

        // A file that moved since the preview is refused, and left alone.
        let stale = preview(c, false).unwrap();
        std::fs::write(&path, br#"{"model":"someone-else-edited-this"}"#).unwrap();
        let err = write(c, false, &stale.fingerprint).unwrap_err();
        assert!(err.contains("changed since the preview"), "got: {err}");
        let untouched: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(untouched["model"], "someone-else-edited-this");

        // Content we cannot parse is refused before anything is written.
        std::fs::write(&path, b"{ broken").unwrap();
        assert!(preview(c, true).is_err());
        assert!(write(c, true, "whatever").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");

        // Kiro: the file appears on install and is gone on uninstall.
        let k = Target::Kiro;
        assert!(!status(k).installed);
        let plan = preview(k, true).unwrap();
        assert!(plan.backup.is_empty());
        write(k, true, &plan.fingerprint).unwrap();
        assert!(status(k).installed);
        let plan = preview(k, false).unwrap();
        write(k, false, &plan.fingerprint).unwrap();
        assert!(!settings_path(k).exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
