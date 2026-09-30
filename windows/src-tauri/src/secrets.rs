// API keys persist only in Windows Credential Manager. The settings form accepts
// them temporarily; IPC never returns their stored values to the frontend.

use keyring::Entry;

const SERVICE: &str = "local.coucou-codex";

/// Every key Coucou may store. Anything outside this list is refused.
pub const KNOWN_KEYS: &[&str] = &[
    "openai-api-key",
    "n8n-url",
    "n8n-api-key",
    "vercel-token",
    "github-token",
    "stripe-api-key",
    "resend-api-key",
    "notion-api-key",
    "calcom-api-key",
];

fn entry(key: &str) -> Option<Entry> {
    if !KNOWN_KEYS.contains(&key) {
        return None;
    }
    Entry::new(SERVICE, key).ok()
}

pub fn get(key: &str) -> Option<String> {
    entry(key)?.get_password().ok().filter(|v| !v.is_empty())
}

pub fn set(key: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return clear(key);
    }
    if key == "n8n-url" {
        let url = crate::integrations::validate_n8n_base(value)?;
        let entry = entry(key).ok_or_else(|| format!("unknown key {key}"))?;
        return entry.set_password(&url).map_err(|e| e.to_string());
    }
    let entry = entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    entry.set_password(value).map_err(|e| e.to_string())
}

pub fn clear(key: &str) -> Result<(), String> {
    let entry = entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn present(key: &str) -> bool {
    get(key).is_some()
}
