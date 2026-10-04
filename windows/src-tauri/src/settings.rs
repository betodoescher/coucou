// Preferences, stored as plain JSON in settings.json under platform::config_dir().
// No secret ever lands here — API keys live in the OS keychain (see secrets.rs).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub sound_enabled: bool,
    pub sound_volume: f64,
    pub auto_close_interval: f64,
    /// The compact island hides after a minute without the mouse. Off: it stays.
    #[serde(default = "default_true")]
    pub auto_hide: bool,
    pub absence_interval: f64,
    pub active_integrations: Vec<String>,
    /// "primary" = the main display, "cursor" = whichever display the mouse is on.
    pub screen: String,
    pub autostart: bool,
    pub hooks_installed: bool,
    /// Claude model used by the chat. Changeable in the settings window.
    /// Defaulted explicitly so a settings.json written by an older build still loads.
    #[serde(default = "default_model")]
    pub model: String,
    /// Who may answer in the chat, tried in this order until one answers:
    /// "cursor", "kiro" (that CLI and the user's plan), "anthropic" (API key).
    #[serde(default = "default_chat_providers")]
    pub chat_providers: Vec<String>,
    #[serde(default = "default_cursor_model")]
    pub cursor_model: String,
    #[serde(default = "default_kiro_model")]
    pub kiro_model: String,
    /// Cursor and Kiro shell commands wait for Allow/Deny on the island.
    /// Off: they run as the agent decides, the island only watches.
    #[serde(default)]
    pub agent_approvals: bool,
    /// Where the user dragged the island: top-left of the panel, in logical
    /// pixels from the top-left of its display. None = top centre. Owned by
    /// Rust: save_settings never takes it from a page.
    #[serde(default)]
    pub island_offset: Option<(f64, f64)>,
    /// City for the home card's weather (Open-Meteo). Empty: no weather, no request.
    #[serde(default)]
    pub weather_city: String,
    /// Mochi's outfit: "auto" follows the seasons. Same values as macOS.
    #[serde(default = "default_outfit")]
    pub mochi_outfit: String,
}

fn default_outfit() -> String {
    "auto".into()
}

fn default_true() -> bool {
    true
}

fn default_model() -> String {
    crate::claude::DEFAULT_MODEL.to_string()
}

fn default_chat_providers() -> Vec<String> {
    vec!["anthropic".into()]
}

fn default_cursor_model() -> String {
    crate::cursor_chat::DEFAULT_MODEL.to_string()
}

fn default_kiro_model() -> String {
    crate::kiro_chat::DEFAULT_MODEL.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sound_enabled: true,
            sound_volume: 0.12,
            auto_close_interval: 15.0,
            auto_hide: true,
            absence_interval: 180.0,
            active_integrations: vec![
                "integration_resend".into(),
                "integration_n8n".into(),
                "integration_vercel".into(),
                "integration_github".into(),
            ],
            screen: "primary".into(),
            autostart: false,
            hooks_installed: false,
            model: default_model(),
            chat_providers: default_chat_providers(),
            cursor_model: default_cursor_model(),
            kiro_model: default_kiro_model(),
            agent_approvals: false,
            island_offset: None,
            weather_city: String::new(),
            mochi_outfit: default_outfit(),
        }
    }
}

pub use crate::platform::{config_dir, local_dir};

pub fn hook_exe_path() -> PathBuf {
    local_dir().join("bin").join(crate::platform::HOOK_EXE)
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn load() -> Settings {
    match std::fs::read(settings_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let dir = config_dir();
    crate::platform::ensure_private_dir(&dir)?;
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(settings_path(), json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_settings_keep_auto_hide_on() {
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json.as_object_mut().unwrap().remove("autoHide");
        let loaded: Settings = serde_json::from_value(json).unwrap();
        assert!(loaded.auto_hide);
    }

    #[test]
    fn older_settings_dress_mochi_for_the_seasons() {
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json.as_object_mut().unwrap().remove("mochiOutfit");
        let loaded: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(loaded.mochi_outfit, "auto");
    }
}
