use codex_client::Client;
use std::path::{Path, PathBuf};

struct Temp(PathBuf);
impl Temp {
    fn new(label: &str) -> Self {
        let p = std::env::temp_dir().join(format!("codex-client-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn authenticates_streams_and_rejects_automatic_approvals() {
    let tmp = Temp::new("chat");
    let mut client = Client::start(Path::new(env!("CARGO_BIN_EXE_mock-codex")), &tmp.0).unwrap();
    assert!(client.is_chatgpt_authenticated().unwrap());
    assert_eq!(client.chat("", "Hello", &tmp.0).unwrap(), "Fixture reply");
}

#[test]
fn api_key_authentication_cannot_silently_bill_in_chatgpt_mode() {
    let tmp = Temp::new("api");
    std::fs::write(tmp.0.join("api-mode"), b"").unwrap();
    let mut client = Client::start(Path::new(env!("CARGO_BIN_EXE_mock-codex")), &tmp.0).unwrap();
    assert!(!client.is_chatgpt_authenticated().unwrap());
    assert!(client
        .chat("", "Hello", &tmp.0)
        .unwrap_err()
        .contains("Sign in with ChatGPT"));
}

#[test]
fn refuses_relative_or_missing_executable() {
    let tmp = Temp::new("path");
    assert!(Client::start(Path::new("codex.exe"), &tmp.0).is_err());
    assert!(Client::start(&tmp.0.join("missing.exe"), &tmp.0).is_err());
}

#[test]
fn rejects_legacy_server_that_silently_ignores_read_restrictions() {
    let tmp = Temp::new("legacy");
    let exe = tmp.0.join("legacy-codex.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_mock-codex"), &exe).unwrap();
    assert!(codex_client::verify_restricted_reads(&exe)
        .unwrap_err()
        .contains("restricted file reads"));
}
