// Preferences, stored as plain JSON in %APPDATA%\CoucouCodex\settings.json.
// No secret ever lands here — API keys live in the Windows Credential Manager.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub sound_enabled: bool,
    pub sound_volume: f64,
    pub auto_close_interval: f64,
    pub absence_interval: f64,
    pub active_integrations: Vec<String>,
    /// "primary" = the main display, "cursor" = whichever display the mouse is on.
    pub screen: String,
    pub autostart: bool,
    pub hooks_installed: bool,
    /// Optional model ID. ChatGPT selects from its authorised catalog;
    /// API uses its documented default when empty.
    /// Defaulted explicitly so a settings.json written by an older build still loads.
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_backend")]
    pub chat_backend: String,
    #[serde(default)]
    pub codex_path: String,
}

fn default_model() -> String {
    String::new()
}

fn default_backend() -> String {
    "chatgpt".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sound_enabled: true,
            sound_volume: 0.12,
            auto_close_interval: 15.0,
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
            chat_backend: default_backend(),
            codex_path: String::new(),
        }
    }
}

/// %APPDATA%\CoucouCodex
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("CoucouCodex")
}

/// %LOCALAPPDATA%\CoucouCodex — where coucou-hook.exe and the log live.
pub fn local_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("CoucouCodex")
}

pub fn hook_exe_path() -> PathBuf {
    local_dir().join("bin").join("coucou-hook.exe")
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn load() -> Settings {
    let mut loaded = match std::fs::read(settings_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Settings::default(),
    };
    // The original session backend was intentionally gated on CLI compatibility.
    // Move that selection to official SIWC; OAuth consent is still required.
    if loaded.chat_backend == "codex" {
        loaded.chat_backend = default_backend();
        loaded.model.clear();
    }
    loaded
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(settings_path(), json)
}

/// Called while the settings mutex is held. A chat control cannot restore a
/// stale billing backend or overwrite unrelated preferences from another window.
pub fn with_chat_model(
    current: &Settings,
    expected_backend: &str,
    expected_model: &str,
    model: &str,
) -> Result<Settings, String> {
    if current.chat_backend != expected_backend || current.model != expected_model {
        return Err("Chat settings changed. Choose the model again.".into());
    }
    if !matches!(current.chat_backend.as_str(), "chatgpt" | "api")
        || model.len() > 128
        || model.chars().any(char::is_whitespace)
        || (current.chat_backend == "api" && !matches!(model, "" | "gpt-5.6-sol"))
    {
        return Err("Choose an available chat model.".into());
    }
    let mut next = current.clone();
    next.model = model.into();
    Ok(next)
}

#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    fn inline_choice_preserves_current_preferences_and_backend() {
        let mut current = Settings::default();
        current.auto_close_interval = 10.0;
        current.sound_volume = 0.42;
        current.active_integrations = vec!["synthetic-integration".into()];
        current.codex_path = "C:/synthetic/codex.exe".into();
        let next = with_chat_model(&current, "chatgpt", "", "gpt-5.6-luna").unwrap();
        assert_eq!(next.model, "gpt-5.6-luna");
        let mut next_without_model = next;
        next_without_model.model.clear();
        assert_eq!(
            serde_json::to_value(next_without_model).unwrap(),
            serde_json::to_value(current).unwrap()
        );
    }

    #[test]
    fn stale_choice_cannot_restore_api_billing_or_an_external_model() {
        let current = Settings::default();
        assert!(with_chat_model(&current, "api", "", "gpt-5.6-sol").is_err());
        assert!(with_chat_model(&current, "chatgpt", "gpt-6-astra", "gpt-5.6-luna").is_err());
        assert_eq!(current.chat_backend, "chatgpt");
        assert_eq!(current.model, "");
    }

    #[test]
    fn malformed_choices_are_rejected_without_reflecting_input() {
        let current = Settings::default();
        for bad in ["synthetic-private-marker bad".to_string(), "x".repeat(129)] {
            let error = with_chat_model(&current, "chatgpt", "", &bad).unwrap_err();
            assert!(!error.contains(&bad));
        }
        let mut api = current;
        api.chat_backend = "api".into();
        assert!(with_chat_model(&api, "api", "", "gpt-6-astra").is_err());
        assert!(with_chat_model(&api, "api", "", "gpt-5.6-sol").is_ok());
    }
}
