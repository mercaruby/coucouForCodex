//! Credential Manager journal. No plaintext files or shared Codex credentials.
use serde::{Deserialize, Serialize};

const SERVICE: &str = "local.coucou-codex.chatgpt";
const MAX_PARTS: usize = 64;
const PART_UTF16: usize = 900;

#[derive(Serialize, Deserialize)]
struct Index {
    version: String,
    parts: usize,
}

pub(crate) trait Store: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn remove(&self, key: &str) -> Result<(), String>;
}

pub(crate) struct CredentialManager;
#[cfg(windows)]
impl Store for CredentialManager {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        use windows_sys::Win32::{
            Foundation::{GetLastError, ERROR_NOT_FOUND},
            Security::Credentials::*,
        };
        let target = target(key);
        let mut credential = std::ptr::null_mut();
        unsafe {
            if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) == 0 {
                return if GetLastError() == ERROR_NOT_FOUND {
                    Ok(None)
                } else {
                    Err("Could not read protected ChatGPT credentials.".into())
                };
            }
            let result = (|| {
                let entry = &*credential;
                if entry.Persist != CRED_PERSIST_LOCAL_MACHINE
                    || entry.CredentialBlobSize > 2560
                    || entry.CredentialBlobSize % 2 != 0
                {
                    return Err(INVALID.into());
                }
                if entry.CredentialBlobSize == 0 {
                    return Ok(Some(String::new()));
                }
                if entry.CredentialBlob.is_null() {
                    return Err(INVALID.into());
                }
                let bytes = std::slice::from_raw_parts(
                    entry.CredentialBlob,
                    entry.CredentialBlobSize as usize,
                );
                let units = bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>();
                String::from_utf16(&units)
                    .map(Some)
                    .map_err(|_| INVALID.into())
            })();
            CredFree(credential.cast());
            result
        }
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        use windows_sys::Win32::Security::Credentials::*;
        let mut target = target(key);
        let mut user = "Coucou Codex"
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut bytes = value
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        if bytes.len() > 2560 {
            return Err("Protected credentials exceed their size limit.".into());
        }
        let entry = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            UserName: user.as_mut_ptr(),
            CredentialBlobSize: bytes.len() as u32,
            CredentialBlob: bytes.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        let success = unsafe { CredWriteW(&entry, 0) != 0 };
        bytes.fill(0);
        if success {
            Ok(())
        } else {
            Err("Could not save protected ChatGPT credentials.".into())
        }
    }
    fn remove(&self, key: &str) -> Result<(), String> {
        use windows_sys::Win32::{
            Foundation::{GetLastError, ERROR_NOT_FOUND},
            Security::Credentials::*,
        };
        let target = target(key);
        if unsafe {
            CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) != 0
                || GetLastError() == ERROR_NOT_FOUND
        } {
            Ok(())
        } else {
            Err("Could not remove protected ChatGPT credentials.".into())
        }
    }
}
#[cfg(windows)]
fn target(key: &str) -> Vec<u16> {
    format!("{SERVICE}:{key}")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}
#[cfg(not(windows))]
impl Store for CredentialManager {
    fn get(&self, _: &str) -> Result<Option<String>, String> {
        Err("Windows Credential Manager is required.".into())
    }
    fn set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("Windows Credential Manager is required.".into())
    }
    fn remove(&self, _: &str) -> Result<(), String> {
        Err("Windows Credential Manager is required.".into())
    }
}
const INVALID: &str =
    "Protected ChatGPT credentials are invalid. See docs/SECURITY-OAUTH.md for recovery.";

fn index(store: &dyn Store) -> Result<Option<Index>, String> {
    let Some(value) = store.get("record-index")? else {
        return Ok(None);
    };
    let index: Index = serde_json::from_str(&value).map_err(|_| INVALID)?;
    if index.parts == 0
        || index.parts > MAX_PARTS
        || index.version.len() != 43
        || !index
            .version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(INVALID.into());
    }
    Ok(Some(index))
}

pub(crate) fn load(store: &dyn Store) -> Result<Option<String>, String> {
    let Some(index) = index(store)? else {
        return Ok(None);
    };
    let mut record = String::new();
    for part in 0..index.parts {
        let value = store
            .get(&format!("record:{}:{part}", index.version))?
            .ok_or(INVALID)?;
        if value.encode_utf16().count() > PART_UTF16 {
            return Err("Protected credentials exceed their size limit.".into());
        }
        record.push_str(&value);
    }
    Ok(Some(record))
}

fn clean(store: &dyn Store, index: &Index) -> Result<(), String> {
    let mut failed = false;
    for part in 0..index.parts {
        failed |= store
            .remove(&format!("record:{}:{part}", index.version))
            .is_err();
    }
    if failed {
        Err("Protected credential fragments could not all be removed. See docs/SECURITY-OAUTH.md for recovery.".into())
    } else {
        Ok(())
    }
}

pub(crate) fn save(store: &dyn Store, record: &str) -> Result<(), String> {
    let old = index(store)?;
    let mut parts = vec![String::new()];
    let mut units = 0;
    for character in record.chars() {
        if units + character.len_utf16() > PART_UTF16 {
            parts.push(String::new());
            units = 0;
        }
        parts.last_mut().unwrap().push(character);
        units += character.len_utf16();
    }
    let count = parts.len();
    if count == 0 || count > MAX_PARTS {
        return Err("Protected credentials exceed their size limit.".into());
    }
    let next = Index {
        version: crate::random_value(),
        parts: count,
    };
    for (part, value) in parts.iter().enumerate() {
        if let Err(error) = store.set(&format!("record:{}:{part}", next.version), value) {
            let _ = clean(store, &next);
            return Err(error);
        }
    }
    // Windows Credential Manager commits one entry atomically. The pointer is
    // published only after every new token/expiry/scope chunk has been saved.
    if let Err(error) = store.set(
        "record-index",
        &serde_json::to_string(&next).map_err(|_| "Could not encode protected credentials.")?,
    ) {
        let _ = clean(store, &next);
        return Err(error);
    }
    if let Some(old) = old {
        let _ = clean(store, &old);
    }
    Ok(())
}

pub(crate) fn clear(store: &dyn Store) -> Result<(), String> {
    let old = index(store)?;
    store.remove("record-index")?;
    if let Some(old) = old {
        clean(store, &old)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    #[derive(Default)]
    pub(crate) struct MemoryStore {
        pub values: Mutex<HashMap<String, String>>,
        pub fail_key: Mutex<Option<String>>,
    }
    impl Store for MemoryStore {
        fn get(&self, key: &str) -> Result<Option<String>, String> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> Result<(), String> {
            if self
                .fail_key
                .lock()
                .unwrap()
                .as_deref()
                .is_some_and(|fail| key.contains(fail))
            {
                return Err("Fixture storage failure".into());
            }
            assert!(value.encode_utf16().count() * 2 <= 2560);
            self.values.lock().unwrap().insert(key.into(), value.into());
            Ok(())
        }
        fn remove(&self, key: &str) -> Result<(), String> {
            self.values.lock().unwrap().remove(key);
            Ok(())
        }
    }
    #[test]
    fn credentials_larger_than_windows_blob_limit_are_committed_together() {
        let store = MemoryStore::default();
        let record = format!("access:{};refresh:{}", "a".repeat(9000), "r".repeat(5000));
        save(&store, &record).unwrap();
        assert_eq!(load(&store).unwrap(), Some(record));
        clear(&store).unwrap();
        assert!(load(&store).unwrap().is_none());
    }
    #[test]
    fn failed_manifest_update_keeps_previous_record_and_cleans_new_parts() {
        let store = MemoryStore::default();
        save(&store, "old credential").unwrap();
        *store.fail_key.lock().unwrap() = Some("record-index".into());
        assert!(save(&store, &"new".repeat(4000)).is_err());
        assert_eq!(load(&store).unwrap(), Some("old credential".into()));
        assert_eq!(store.values.lock().unwrap().len(), 2);
    }
    #[cfg(windows)]
    #[test]
    fn native_store_uses_host_local_protection_for_synthetic_entries() {
        use windows_sys::Win32::Security::Credentials::*;
        let key = format!("test:{}", crate::random_value());
        let store = CredentialManager;
        for value in [String::new(), "🦀".repeat(640)] {
            store.set(&key, &value).unwrap();
            assert_eq!(store.get(&key).unwrap(), Some(value));
            let target = target(&key);
            let mut credential = std::ptr::null_mut();
            unsafe {
                assert_ne!(
                    CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential),
                    0
                );
                assert_eq!((*credential).Persist, CRED_PERSIST_LOCAL_MACHINE);
                CredFree(credential.cast());
            }
        }
        store.remove(&key).unwrap();
        assert!(store.get(&key).unwrap().is_none());
    }
}
