//! Independent adversarial tests. Uses only synthetic tokens and in-memory storage.
use crate::storage::{self, Store};
use crate::{Credentials, Manager};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Default)]
struct FaultStore {
    values: Mutex<HashMap<String, String>>,
    fail_set: Mutex<Option<String>>,
    fail_remove: Mutex<Option<String>>,
}
impl Store for FaultStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.values.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        if self
            .fail_set
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|suffix| key.ends_with(suffix))
        {
            return Err("synthetic write failure".into());
        }
        self.values.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn remove(&self, key: &str) -> Result<(), String> {
        if self.fail_remove.lock().unwrap().as_deref() == Some(key) {
            return Err("synthetic deletion failure".into());
        }
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

#[test]
fn journal_chunk_write_failure_keeps_prior_session() {
    let store = FaultStore::default();
    storage::save(&store, "old synthetic session").unwrap();
    *store.fail_set.lock().unwrap() = Some(":1".into());
    assert!(storage::save(&store, &"new".repeat(1000)).is_err());
    assert_eq!(
        storage::load(&store).unwrap(),
        Some("old synthetic session".into())
    );
    assert_eq!(
        store.values.lock().unwrap().len(),
        2,
        "partial new journal must be removed"
    );
}

#[test]
fn journal_missing_part_is_never_accepted_as_session() {
    let store = FaultStore::default();
    storage::save(&store, &"synthetic".repeat(300)).unwrap();
    let key = store
        .values
        .lock()
        .unwrap()
        .keys()
        .find(|key| key.ends_with(":1"))
        .unwrap()
        .clone();
    store.values.lock().unwrap().remove(&key);
    assert!(storage::load(&store).is_err());
}

#[test]
fn journal_rejects_corrupt_or_attacker_selected_manifest() {
    for value in [
        "not json".to_string(),
        serde_json::json!({"version":"../another-credential","parts":1}).to_string(),
        serde_json::json!({"version":"a".repeat(43),"parts":0}).to_string(),
        serde_json::json!({"version":"a".repeat(43),"parts":65}).to_string(),
    ] {
        let store = FaultStore::default();
        store
            .values
            .lock()
            .unwrap()
            .insert("record-index".into(), value);
        assert!(storage::load(&store).is_err());
        assert!(storage::save(&store, "replacement").is_err());
    }
}

#[test]
fn journal_preserves_unicode_and_enforces_windows_utf16_limits() {
    let store = FaultStore::default();
    let original = "🦀漢é".repeat(1000);
    storage::save(&store, &original).unwrap();
    assert_eq!(storage::load(&store).unwrap(), Some(original));
    assert!(store
        .values
        .lock()
        .unwrap()
        .iter()
        .filter(|(key, _)| key.starts_with("record:"))
        .all(|(_, value)| value.encode_utf16().count() <= 900));
}

#[test]
fn journal_size_limit_failure_preserves_prior_record() {
    let store = FaultStore::default();
    storage::save(&store, "old synthetic session").unwrap();
    assert!(storage::save(&store, &"x".repeat(900 * 64 + 1)).is_err());
    assert_eq!(
        storage::load(&store).unwrap(),
        Some("old synthetic session".into())
    );
    assert_eq!(store.values.lock().unwrap().len(), 2);
}

#[test]
fn logout_index_deletion_failure_reports_error_and_retains_recoverable_record() {
    let store = FaultStore::default();
    storage::save(&store, "synthetic refresh token").unwrap();
    *store.fail_remove.lock().unwrap() = Some("record-index".into());
    assert!(storage::clear(&store).is_err());
    assert_eq!(
        storage::load(&store).unwrap(),
        Some("synthetic refresh token".into())
    );
}

fn synthetic_credentials() -> Credentials {
    Credentials {
        client_id: "oaiapp_fixture".into(),
        subject: "synthetic-user".into(),
        email: Some("synthetic@example.invalid".into()),
        id_token: "synthetic-id-token-marker".into(),
        access_token: "synthetic-access-token-marker".into(),
        refresh_token: String::new(),
        expires_at: crate::now() + 3600,
        earliest_refresh_at: None,
        pending_refresh: None,
        scopes: vec!["resource.invoke".into(), "chatgpt.tokens.use.direct".into()],
    }
}

fn manager_with(record: &Credentials) -> (Manager, Arc<FaultStore>) {
    let store = Arc::new(FaultStore::default());
    storage::save(store.as_ref(), &serde_json::to_string(record).unwrap()).unwrap();
    (Manager::with_store(store.clone()).unwrap(), store)
}

#[test]
fn identity_only_and_similarly_named_scopes_never_grant_plan_usage() {
    for scopes in [
        vec!["openid".into(), "profile".into()],
        vec!["resource.invoke".into()],
        vec!["chatgpt.tokens.use.direct".into()],
        vec![
            "resource.invoke".into(),
            "chatgpt.tokens.use.direct.attacker".into(),
        ],
    ] {
        let mut record = synthetic_credentials();
        record.scopes = scopes;
        let (manager, _) = manager_with(&record);
        assert!(manager.session().connected);
        assert!(!manager.session().sharing);
        assert!(
            manager.access().is_err(),
            "identity-only access must reject before any HTTP request"
        );
    }
    let (manager, _) = manager_with(&synthetic_credentials());
    assert!(manager.session().sharing);
    assert!(manager.access().is_ok());
}

#[test]
fn session_projection_contains_only_public_metadata() {
    let (manager, _) = manager_with(&synthetic_credentials());
    let public = serde_json::to_string(&manager.session()).unwrap();
    for marker in [
        "synthetic-id-token-marker",
        "synthetic-access-token-marker",
        "refresh_token",
        "access_token",
        "id_token",
        "authUrl",
    ] {
        assert!(
            !public.contains(marker),
            "secret marker {marker} crossed Session projection"
        );
    }
    assert!(public.contains("clientId"));
    assert!(public.contains("synthetic-user"));
}

#[test]
fn logout_invalidates_inflight_generation_and_retains_only_registration() {
    let (manager, store) = manager_with(&synthetic_credentials());
    let generation = manager.session().generation;
    manager.logout().unwrap();
    assert!(!manager.session().connected);
    assert!(!manager.session().sharing);
    assert!(manager.check_generation(generation).is_err());
    assert_eq!(
        manager.session().client_id.as_deref(),
        Some("oaiapp_fixture")
    );
    let persisted = storage::load(store.as_ref()).unwrap().unwrap();
    assert!(!persisted.contains("synthetic-id-token-marker"));
    assert!(!persisted.contains("synthetic-access-token-marker"));
    assert!(!Manager::with_store(store).unwrap().session().connected);
}

#[test]
fn logout_write_failure_still_disables_session_in_memory() {
    let (manager, store) = manager_with(&synthetic_credentials());
    *store.fail_set.lock().unwrap() = Some("record-index".into());
    assert!(manager.logout().is_err());
    assert!(
        !manager.session().connected,
        "persist failure must never leave the logged-out access token usable"
    );
    assert!(manager.access().is_err());
}

#[test]
fn replayed_or_wrong_attempt_ids_are_rejected_without_http() {
    let (manager, _) = manager_with(&synthetic_credentials());
    assert!(manager.finish_login("foreign-attempt").is_err());
    assert!(manager.cancel_login("foreign-attempt").is_err());
    let discovery = crate::oauth::Discovery {
        issuer: crate::oauth::ISSUER.into(),
        authorization_endpoint: format!("{}/api/accounts/authorize", crate::oauth::ISSUER),
        token_endpoint: format!("{}/api/accounts/oauth/token", crate::oauth::ISSUER),
        jwks_uri: format!("{}/.well-known/jwks.json", crate::oauth::ISSUER),
        revocation_endpoint: format!("{}/api/accounts/oauth/revoke", crate::oauth::ISSUER),
        id_token_signing_alg_values_supported: vec!["RS256".into()],
    };
    let attempt = Arc::new(
        crate::oauth::prepare(discovery, &manager.host_id, None, 0)
            .unwrap()
            .0,
    );
    attempt
        .finishing
        .store(true, std::sync::atomic::Ordering::Release);
    manager.inner.lock().unwrap().pending = Some(attempt.clone());
    assert!(manager.finish_login(&attempt.id).is_err());
    manager.cancel_login(&attempt.id).unwrap();
    assert!(attempt.cancelled.load(std::sync::atomic::Ordering::Acquire));
    assert!(manager.finish_login(&attempt.id).is_err());
    assert!(manager.cancel_login(&attempt.id).is_err());
    assert!(manager.check_generation(0).is_err());
}

#[test]
fn host_identity_persists_independently_of_account_and_rejects_corruption() {
    let (manager, store) = manager_with(&synthetic_credentials());
    let first = manager.host_id.clone();
    assert!(first.starts_with("urn:uuid:"));
    assert_eq!(first.as_bytes()[23], b'4');
    manager.logout().unwrap();
    assert_eq!(Manager::with_store(store.clone()).unwrap().host_id, first);
    store
        .values
        .lock()
        .unwrap()
        .insert("host-id".into(), "https://attacker.invalid".into());
    assert!(Manager::with_store(store).is_err());
}

#[test]
fn logout_invalidates_session_before_waiting_for_credential_gate() {
    let (manager, _) = manager_with(&synthetic_credentials());
    let manager = Arc::new(manager);
    let gate = manager.auth_gate.lock().unwrap();
    let worker_manager = manager.clone();
    let worker = std::thread::spawn(move || worker_manager.logout());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while manager.session().generation == 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let invalidated = manager.session().generation != 0 && !manager.session().connected;
    drop(gate);
    worker.join().unwrap().unwrap();
    assert!(
        invalidated,
        "logout must disable the session while an earlier operation holds the rotation gate"
    );
}

#[test]
fn earliest_refresh_future_uses_valid_access_and_rejects_expired_access_before_http() {
    let mut record = synthetic_credentials();
    record.refresh_token = "synthetic-refresh-marker".into();
    record.earliest_refresh_at = Some(serde_json::json!(crate::now() + 3600));
    record.expires_at = crate::now() + 30;
    let (manager, _) = manager_with(&record);
    assert!(
        manager.access().is_ok(),
        "still-valid token should be usable without premature refresh"
    );
    record.expires_at = crate::now() - 1;
    let (manager, _) = manager_with(&record);
    assert!(
        manager.access().is_err(),
        "expired token must reject before contacting token endpoint"
    );
    for invalid in [
        serde_json::json!(-1),
        serde_json::json!({"time":1}),
        serde_json::json!("not-a-timestamp"),
        serde_json::json!(u64::MAX),
    ] {
        assert!(crate::refresh_not_before(Some(&invalid)).is_err());
    }
    assert_eq!(
        crate::refresh_not_before(Some(&serde_json::json!("2026-10-01T00:00:00Z"))).unwrap(),
        Some(1_790_812_800)
    );
}

#[test]
fn unverified_pending_rotation_does_not_advertise_plan_authorization() {
    let mut record = synthetic_credentials();
    record.pending_refresh = Some(crate::PendingRefresh {
        access_token: "synthetic-pending-access-marker".into(),
        refresh_token: "synthetic-pending-refresh-marker".into(),
        id_token: Some("unverified-id-token".into()),
        expires_at: crate::now() + 3600,
        scopes: record.scopes.clone(),
        earliest_refresh_at: None,
    });
    let (manager, _) = manager_with(&record);
    assert!(!manager.session().sharing);
    let public = serde_json::to_string(&manager.session()).unwrap();
    assert!(!public.contains("synthetic-pending-access-marker"));
    assert!(!public.contains("synthetic-pending-refresh-marker"));
    assert!(!public.contains("unverified-id-token"));
}
