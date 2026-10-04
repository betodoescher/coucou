// Global keyboard shortcuts. Port of HotKeyCenter.swift and ShortcutLogic.swift.
//
// Each shortcut is the modifier chosen in Settings plus the action's own key.
// Windows and X11 grab them through the global-shortcut plugin. On a Wayland
// session an X11 grab only fires while an X11 window has the focus, so there
// they go through the desktop portal, and the desktop keeps the final say:
// it asks the user once and lets them change the keys in its own settings.
//
// Rust only reports which action fired; the island decides what it means.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::island::WINDOW_LABEL;
use crate::log;
use crate::settings::Settings;

pub const DEFAULT_MODIFIER: &str = "Ctrl+Shift+Alt";

/// Every action and its default key. The island opener starts off, as on macOS.
pub const ACTIONS: &[(&str, &str)] = &[
    ("toggleIsland", ""),
    ("openChat", "Space"),
    ("goToAlert", "A"),
    ("jumpToTerminal", "T"),
    ("nextPill", "]"),
    ("prevPill", "["),
    ("muteToggle", "M"),
    ("desktopToggle", "D"),
    ("wardrobeToggle", "G"),
];

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub action: String,
    /// What to press, as the system shows it.
    pub keys: String,
    /// "on", "off", "taken" (another app has it), "invalid", or "pending"
    /// while the desktop asks the user.
    pub state: String,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// "hotkey" (Coucou grabs the keys) or "portal" (the desktop does).
    pub backend: String,
    pub entries: Vec<Entry>,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);

pub fn status() -> Status {
    STATUS.lock().unwrap().clone().unwrap_or_default()
}

fn publish(app: &AppHandle, status: Status) {
    *STATUS.lock().unwrap() = Some(status.clone());
    let _ = app.emit("shortcuts-status", status);
}

/// The key an action uses: the user's, or its default.
pub fn key_of<'a>(settings: &'a Settings, action: &str) -> &'a str {
    if let Some(k) = settings.shortcuts.get(action) {
        return k.trim();
    }
    ACTIONS.iter().find(|(a, _)| *a == action).map(|(_, k)| *k).unwrap_or("")
}

/// `(action, accelerator)` for every shortcut that is on.
pub fn wanted(settings: &Settings) -> Vec<(String, String)> {
    ACTIONS
        .iter()
        .filter_map(|(action, _)| {
            let key = key_of(settings, action);
            (!key.is_empty()).then(|| (action.to_string(), format!("{}+{key}", settings.shortcut_modifier)))
        })
        .collect()
}

fn fire(app: &AppHandle, action: &str) {
    let _ = app.emit_to(WINDOW_LABEL, "shortcut", action.to_string());
}

/// Registers the shortcuts in `settings`, replacing whatever was registered.
pub fn apply(app: &AppHandle, settings: &Settings) {
    let wanted = wanted(settings);
    #[cfg(target_os = "linux")]
    if portal::wanted_here() {
        portal::bind(app.clone(), wanted);
        return;
    }
    grab(app, &wanted);
}

fn grab(app: &AppHandle, wanted: &[(String, String)]) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut entries = Vec::new();
    for (action, accel) in wanted {
        let state = match accel.parse::<Shortcut>() {
            Err(_) => "invalid",
            Ok(shortcut) => {
                let name = action.clone();
                let result = gs.on_shortcut(shortcut, move |app, _, event| {
                    if event.state == ShortcutState::Pressed {
                        fire(app, &name);
                    }
                });
                match result {
                    Ok(()) => "on",
                    Err(err) => {
                        log::line(format!("shortcut {accel} for {action} not available: {err}"));
                        "taken"
                    }
                }
            }
        };
        entries.push(Entry { action: action.clone(), keys: accel.clone(), state: state.into() });
    }
    publish(app, Status { backend: "hotkey".into(), entries });
}

/// The XDG shortcuts format the portal takes: `CTRL+SHIFT+ALT+space`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn xdg_trigger(accel: &str) -> Option<String> {
    let parts: Vec<&str> = accel.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last()?;
    let mut out = Vec::new();
    for m in mods {
        out.push(match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "CTRL",
            "alt" | "option" => "ALT",
            "shift" => "SHIFT",
            "super" | "meta" | "cmd" | "win" => "LOGO",
            _ => return None,
        }
        .to_string());
    }
    let keysym = match *key {
        "[" => "bracketleft".to_string(),
        "]" => "bracketright".to_string(),
        "," => "comma".to_string(),
        "." => "period".to_string(),
        "/" => "slash".to_string(),
        ";" => "semicolon".to_string(),
        "'" => "apostrophe".to_string(),
        "`" => "grave".to_string(),
        "-" => "minus".to_string(),
        "=" => "equal".to_string(),
        "\\" => "backslash".to_string(),
        "Enter" => "Return".to_string(),
        k if k.len() == 1 => k.to_ascii_lowercase(),
        k if k.eq_ignore_ascii_case("space") => "space".to_string(),
        k => k.to_string(),
    };
    out.push(keysym);
    Some(out.join("+"))
}

#[cfg(target_os = "linux")]
mod portal {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    use tauri::AppHandle;
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

    use super::{fire, publish, xdg_trigger, Entry, Status};
    use crate::log;

    const DEST: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
    /// The desktop file Coucou installs: how the desktop names us in its dialog.
    const APP_ID: &str = "Coucou";

    static GENERATION: AtomicU64 = AtomicU64::new(0);
    static SESSION: Mutex<Option<(Connection, OwnedObjectPath)>> = Mutex::new(None);

    /// A Wayland session with the portal: an X11 grab would miss most keys.
    pub fn wanted_here() -> bool {
        std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t.eq_ignore_ascii_case("wayland"))
            && std::env::var("COUCOU_SHORTCUTS").as_deref() != Ok("x11")
    }

    pub fn bind(app: AppHandle, wanted: Vec<(String, String)>) {
        let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        let pending = wanted
            .iter()
            .map(|(action, accel)| Entry { action: action.clone(), keys: accel.clone(), state: "pending".into() })
            .collect();
        publish(&app, Status { backend: "portal".into(), entries: pending });
        std::thread::spawn(move || {
            if let Err(err) = run(&app, generation, &wanted) {
                log::line(format!("shortcuts portal: {err}"));
                let failed = wanted
                    .iter()
                    .map(|(action, accel)| Entry { action: action.clone(), keys: accel.clone(), state: "taken".into() })
                    .collect();
                publish(&app, Status { backend: "portal".into(), entries: failed });
            }
        });
    }

    /// Ends the previous session, so its keys stop firing and can be bound again.
    fn close_previous() {
        if let Some((conn, session)) = SESSION.lock().unwrap().take() {
            let _ = conn.call_method(Some(DEST), session.as_str(), Some("org.freedesktop.portal.Session"), "Close", &());
        }
    }

    fn run(app: &AppHandle, generation: u64, wanted: &[(String, String)]) -> zbus::Result<()> {
        close_previous();
        if wanted.is_empty() {
            publish(app, Status { backend: "portal".into(), entries: Vec::new() });
            return Ok(());
        }
        let conn = Connection::session()?;
        // A host app has to say who it is before its first portal call on this
        // connection. Older portals do not have the registry: nothing to say.
        let _ = conn.call_method(
            Some(DEST),
            PATH,
            Some("org.freedesktop.host.portal.Registry"),
            "Register",
            &(APP_ID, HashMap::<&str, Value>::new()),
        );
        let gs = Proxy::new(&conn, DEST, PATH, IFACE)?;

        let created = request(&conn, "CreateSession", |token| {
            let mut opts = HashMap::<&str, Value>::new();
            opts.insert("handle_token", token.into());
            opts.insert("session_handle_token", "coucou".into());
            gs.call_method("CreateSession", &(opts,)).map(|_| ())
        })?;
        let session = created
            .get("session_handle")
            .and_then(|v| {
                String::try_from(v.try_clone().ok()?)
                    .ok()
                    .or_else(|| OwnedObjectPath::try_from(v.try_clone().ok()?).ok().map(|p| p.to_string()))
            })
            .ok_or_else(|| zbus::Error::Failure("no session handle".into()))?;
        let session = OwnedObjectPath::try_from(session)?;
        *SESSION.lock().unwrap() = Some((conn.clone(), session.clone()));

        let shortcuts: Vec<(String, HashMap<&str, Value>)> = wanted
            .iter()
            .map(|(action, accel)| {
                let mut props = HashMap::<&str, Value>::new();
                props.insert("description", Value::from(describe(action)));
                if let Some(trigger) = xdg_trigger(accel) {
                    props.insert("preferred_trigger", Value::from(trigger));
                }
                (action.clone(), props)
            })
            .collect();
        let bound = request(&conn, "BindShortcuts", |token| {
            let mut opts = HashMap::<&str, Value>::new();
            opts.insert("handle_token", token.into());
            gs.call_method("BindShortcuts", &(ObjectPath::from(&session), &shortcuts, "", opts)).map(|_| ())
        });
        let bound = match bound {
            Ok(b) => b,
            Err(zbus::Error::Failure(msg)) if msg == "cancelled" => {
                log::line("shortcuts: not allowed by the user");
                let off = wanted
                    .iter()
                    .map(|(action, accel)| Entry { action: action.clone(), keys: accel.clone(), state: "off".into() })
                    .collect();
                publish(app, Status { backend: "portal".into(), entries: off });
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        let mut described: HashMap<String, String> = HashMap::new();
        if let Some(list) = bound.get("shortcuts").and_then(|v| v.try_clone().ok()) {
            if let Ok(list) = <Vec<(String, HashMap<String, OwnedValue>)>>::try_from(list) {
                for (id, props) in list {
                    let text = props
                        .get("trigger_description")
                        .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
                        .unwrap_or_default();
                    described.insert(id, text);
                }
            }
        }
        let entries = wanted
            .iter()
            .map(|(action, accel)| {
                let keys = described.get(action).filter(|t| !t.is_empty()).cloned().unwrap_or_else(|| accel.clone());
                Entry { action: action.clone(), keys, state: "on".into() }
            })
            .collect();
        publish(app, Status { backend: "portal".into(), entries });
        log::line(format!("shortcuts bound through the portal ({})", wanted.len()));

        for msg in gs.receive_signal("Activated")? {
            if GENERATION.load(Ordering::Relaxed) != generation {
                break;
            }
            let Ok((from, id, _, _)) = msg.body().deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>() else {
                continue;
            };
            if from == session {
                fire(app, &id);
            }
        }
        Ok(())
    }

    /// One portal request: listens for its Response before making the call,
    /// since the answer can come back before the call returns.
    fn request(
        conn: &Connection,
        what: &str,
        call: impl FnOnce(&str) -> zbus::Result<()>,
    ) -> zbus::Result<HashMap<String, OwnedValue>> {
        let token = format!("coucou_{}_{}", what.to_lowercase(), std::process::id());
        let sender = conn
            .unique_name()
            .map(|n| n.trim_start_matches(':').replace('.', "_"))
            .ok_or_else(|| zbus::Error::Failure("no bus name".into()))?;
        let path = format!("{PATH}/request/{sender}/{token}");
        let req = Proxy::new(conn, DEST, path.as_str(), "org.freedesktop.portal.Request")?;
        let mut responses = req.receive_signal("Response")?;
        call(&token)?;
        let msg = responses.next().ok_or_else(|| zbus::Error::Failure("no response".into()))?;
        let (code, results) = msg.body().deserialize::<(u32, HashMap<String, OwnedValue>)>()?;
        match code {
            0 => Ok(results),
            1 => Err(zbus::Error::Failure("cancelled".into())),
            _ => Err(zbus::Error::Failure(format!("{what} failed"))),
        }
    }

    fn describe(action: &str) -> &'static str {
        match action {
            "toggleIsland" => "Open or close the island",
            "openChat" => "Open the chat",
            "goToAlert" => "Go to the waiting permission or question",
            "jumpToTerminal" => "Bring the agent's window forward",
            "nextPill" => "Next pill",
            "prevPill" => "Previous pill",
            "muteToggle" => "Mute or unmute Mochi",
            "desktopToggle" => "Send Mochi to the desktop and back",
            "wardrobeToggle" => "Open or close the wardrobe",
            _ => "Coucou",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_follow_the_modifier_and_can_be_turned_off() {
        let mut s = Settings::default();
        let all = wanted(&s);
        assert!(all.contains(&("openChat".into(), "Ctrl+Shift+Alt+Space".into())));
        assert!(!all.iter().any(|(a, _)| a == "toggleIsland"), "off by default");

        s.shortcut_modifier = "Super+Alt".into();
        s.shortcuts.insert("goToAlert".into(), "".into());
        s.shortcuts.insert("toggleIsland".into(), "N".into());
        let all = wanted(&s);
        assert!(all.contains(&("toggleIsland".into(), "Super+Alt+N".into())));
        assert!(!all.iter().any(|(a, _)| a == "goToAlert"));
        assert!(all.contains(&("nextPill".into(), "Super+Alt+]".into())));
    }

    #[test]
    fn every_default_parses() {
        for (action, accel) in wanted(&Settings::default()) {
            assert!(accel.parse::<Shortcut>().is_ok(), "{action}: {accel}");
        }
    }

    #[test]
    fn portal_triggers_use_xdg_names() {
        assert_eq!(xdg_trigger("Ctrl+Shift+Alt+Space").as_deref(), Some("CTRL+SHIFT+ALT+space"));
        assert_eq!(xdg_trigger("Super+Alt+]").as_deref(), Some("LOGO+ALT+bracketright"));
        assert_eq!(xdg_trigger("Ctrl+Alt+A").as_deref(), Some("CTRL+ALT+a"));
        assert_eq!(xdg_trigger("Hyper+A"), None);
    }
}
