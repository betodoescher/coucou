// Which app an agent session runs in, so "Open terminal" brings that window
// back instead of opening an editor.
//
// coucou-hook is started by the agent, the agent by a shell, the shell by
// whatever shows it: a terminal, an editor, Orca. When the relay connects we
// walk up from its process to the first ancestor that is none of that plumbing,
// then climb to the top of that app's own tree — Electron editors run their
// terminals in a helper process with the same name as the main one.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::log;
use crate::platform::{self, Proc};

/// Shells, multiplexers, consoles and agent CLIs: between the relay and the app.
const PLUMBING: &[&str] = &[
    "coucou-hook", "sh", "bash", "zsh", "fish", "dash", "ksh", "mksh", "tcsh", "csh", "nu",
    "elvish", "xonsh", "pwsh", "powershell", "cmd", "conhost", "openconsole", "tmux", "screen",
    "zellij", "sudo", "doas", "su", "env", "script", "timeout", "nohup", "setsid", "node", "bun",
    "deno", "python", "python3", "uv", "npx", "npm", "pnpm", "yarn", "claude", "agent",
    "cursor-agent", "gemini", "codex", "kiro", "kiro-cli", "opencode", "ptyxis-agent", "wsl",
    "wslhost", "wslrelay",
];

/// Reaching one of these means no app shows the session (tmux, ssh, a service).
const ROOTS: &[&str] = &[
    "systemd", "init", "launchd", "sshd", "login", "gnome-shell", "plasmashell", "kwin_wayland",
    "explorer", "services", "svchost", "wininit", "winlogon",
];

/// Hosts seen since launch, by pid, with the name they had: a pid only counts
/// while it still belongs to the same app.
static HOSTS: Mutex<Option<HashMap<u32, String>>> = Mutex::new(None);

/// Index of the host in `chain`, which starts at the relay and goes up.
fn pick_host(chain: &[Proc]) -> Option<usize> {
    let start = chain.iter().position(|p| !PLUMBING.contains(&p.name.as_str()))?;
    let name = chain[start].name.as_str();
    if ROOTS.contains(&name) {
        return None;
    }
    let mut top = start;
    while chain.get(top + 1).is_some_and(|p| p.name == name) {
        top += 1;
    }
    Some(top)
}

/// Finds and remembers the app behind the relay process `relay`.
pub fn remember(relay: u32) -> Option<u32> {
    let chain = platform::ancestors(relay);
    let host = &chain[pick_host(&chain)?];
    let mut hosts = HOSTS.lock().unwrap();
    let hosts = hosts.get_or_insert_with(HashMap::new);
    if hosts.get(&host.pid) != Some(&host.name) {
        if hosts.len() > 64 {
            hosts.clear();
        }
        log::line(format!("session host {} ({})", host.name, host.pid));
        hosts.insert(host.pid, host.name.clone());
    }
    Some(host.pid)
}

/// Brings a remembered host's window to the front. False when it is gone or
/// cannot be raised from here.
pub fn bring_back(pid: u32) -> bool {
    let known = HOSTS.lock().unwrap().as_ref().and_then(|h| h.get(&pid).cloned());
    let Some(name) = known else { return false };
    if platform::ancestors(pid).first().map(|p| &p.name) != Some(&name) {
        return false;
    }
    let raised = platform::bring_to_front(pid);
    log::line(format!("bring back {name} ({pid}): {}", if raised { "ok" } else { "not possible" }));
    raised
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(names: &[&str]) -> Vec<Proc> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Proc { pid: 100 + i as u32, name: n.to_string() })
            .collect()
    }

    fn host(names: &[&str]) -> Option<String> {
        let c = chain(names);
        pick_host(&c).map(|i| c[i].name.clone())
    }

    #[test]
    fn finds_the_app_behind_the_shells() {
        let orca = chain(&["coucou-hook", "zsh", "node", "zsh", "orca-ide", "orca-ide", "zsh", "ptyxis-agent", "ptyxis", "systemd"]);
        assert_eq!(pick_host(&orca), Some(5), "the main Orca process, not its pty helper");
        assert_eq!(host(&["coucou-hook", "sh", "code", "code", "code", "systemd"]).as_deref(), Some("code"));
        assert_eq!(host(&["coucou-hook", "zsh", "claude", "zsh", "ptyxis-agent", "ptyxis", "systemd"]).as_deref(), Some("ptyxis"));
        assert_eq!(
            host(&["coucou-hook", "pwsh", "claude", "pwsh", "openconsole", "windowsterminal", "explorer"]).as_deref(),
            Some("windowsterminal"),
        );
    }

    #[test]
    fn no_host_without_a_window() {
        assert_eq!(host(&["coucou-hook", "bash", "node", "bash", "tmux", "systemd"]), None);
        assert_eq!(host(&["coucou-hook", "bash", "claude", "bash", "sshd", "sshd", "systemd"]), None);
        assert_eq!(host(&["coucou-hook", "bash"]), None);
    }
}
