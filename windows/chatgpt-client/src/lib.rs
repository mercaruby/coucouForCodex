//! Official Sign in with ChatGPT for a local personal app.
//! Own OAuth registration and protected credentials; no Codex token imports.
mod oauth;
mod responses;
mod storage;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{atomic::Ordering, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage::{CredentialManager, Store};

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn random_value() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Credentials {
    client_id: String,
    subject: String,
    email: Option<String>,
    id_token: String,
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    scopes: Vec<String>,
    #[serde(default)]
    earliest_refresh_at: Option<Value>,
    #[serde(default)]
    pending_refresh: Option<PendingRefresh>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PendingRefresh {
    access_token: String,
    refresh_token: String,
    id_token: Option<String>,
    expires_at: u64,
    scopes: Vec<String>,
    earliest_refresh_at: Option<Value>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub connected: bool,
    pub sharing: bool,
    pub email: Option<String>,
    pub subject: Option<String>,
    pub client_id: Option<String>,
    pub generation: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub slug: String,
    pub display_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStart {
    pub attempt_id: String,
    pub auth_url: String,
}

pub struct Reply {
    pub text: String,
    pub generation: u64,
}

struct Inner {
    generation: u64,
    record: Option<Credentials>,
    pending: Option<Arc<oauth::Attempt>>,
    revocation_uncertain: bool,
}

pub struct Manager {
    store: Arc<dyn Store>,
    inner: Mutex<Inner>,
    // Serializes all credential journal reads/writes and refresh rotations.
    auth_gate: Mutex<()>,
    host_id: String,
}

impl Manager {
    pub fn new() -> Result<Self, String> {
        Self::with_store(Arc::new(CredentialManager))
    }

    fn with_store(store: Arc<dyn Store>) -> Result<Self, String> {
        let host_id = match store.get("host-id")? {
            Some(id) if valid_host(&id) => id,
            Some(_) => return Err("The app's ChatGPT host identifier is invalid.".into()),
            None => {
                let mut bytes = [0u8; 16];
                OsRng.fill_bytes(&mut bytes);
                bytes[6] = (bytes[6] & 0x0f) | 0x40;
                bytes[8] = (bytes[8] & 0x3f) | 0x80;
                let h = bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let id = format!(
                    "urn:uuid:{}-{}-{}-{}-{}",
                    &h[..8],
                    &h[8..12],
                    &h[12..16],
                    &h[16..20],
                    &h[20..]
                );
                store.set("host-id", &id)?;
                id
            }
        };
        let record=storage::load(store.as_ref())?.map(|record|serde_json::from_str::<Credentials>(&record)
            .map_err(|_| "Protected ChatGPT credentials are invalid. See docs/SECURITY-OAUTH.md for recovery.")).transpose()?;
        if record.as_ref().is_some_and(|record| {
            record.client_id.is_empty()
                || record.client_id == oauth::DYNAMIC_CLIENT
                || record.client_id.len() > 256
                || record.scopes.len() > 64
        }) {
            return Err("Protected ChatGPT credentials are invalid. See docs/SECURITY-OAUTH.md for recovery.".into());
        }
        Ok(Self {
            store,
            inner: Mutex::new(Inner {
                generation: 0,
                record,
                pending: None,
                revocation_uncertain: false,
            }),
            auth_gate: Mutex::new(()),
            host_id,
        })
    }

    pub fn session(&self) -> Session {
        let inner = self.inner.lock().unwrap();
        let record = inner.record.as_ref();
        let connected = record
            .is_some_and(|record| !record.access_token.is_empty() && !record.subject.is_empty());
        Session {
            connected,
            sharing: connected
                && record.is_some_and(|record| record.pending_refresh.is_none() && sharing(record)),
            email: record.and_then(|record| record.email.clone()),
            subject: record
                .filter(|record| !record.subject.is_empty())
                .map(|record| record.subject.clone()),
            client_id: record.map(|record| record.client_id.clone()),
            generation: inner.generation,
        }
    }

    pub fn begin_login(&self) -> Result<LoginStart, String> {
        let _gate = self.auth_gate.lock().unwrap();
        let discovery = oauth::discovery(&oauth::http()?)?;
        let mut inner = self.inner.lock().unwrap();
        if let Some(pending) = inner.pending.take() {
            pending.cancelled.store(true, Ordering::Release);
        }
        inner.generation += 1;
        let (attempt, auth_url) = oauth::prepare(
            discovery,
            &self.host_id,
            inner.record.clone(),
            inner.generation,
        )?;
        let attempt_id = attempt.id.clone();
        inner.pending = Some(Arc::new(attempt));
        Ok(LoginStart {
            attempt_id,
            auth_url,
        })
    }

    pub fn cancel_login(&self, attempt_id: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        if !inner
            .pending
            .as_ref()
            .is_some_and(|attempt| attempt.id == attempt_id)
        {
            return Err("This sign-in attempt is no longer active.".into());
        }
        if let Some(attempt) = inner.pending.take() {
            attempt.cancelled.store(true, Ordering::Release);
        }
        inner.generation += 1;
        Ok(())
    }

    pub fn finish_login(&self, attempt_id: &str) -> Result<Session, String> {
        let attempt = {
            let inner = self.inner.lock().unwrap();
            inner
                .pending
                .as_ref()
                .filter(|attempt| attempt.id == attempt_id)
                .cloned()
                .ok_or("This sign-in attempt is no longer active.")?
        };
        if attempt.finishing.swap(true, Ordering::AcqRel) {
            return Err("This sign-in attempt is already being completed.".into());
        }
        let result = self.complete_login(&attempt);
        {
            let mut inner = self.inner.lock().unwrap();
            if inner
                .pending
                .as_ref()
                .is_some_and(|pending| pending.id == attempt_id)
            {
                inner.pending = None;
            }
        }
        result?;
        Ok(self.session())
    }

    fn complete_login(&self, attempt: &oauth::Attempt) -> Result<(), String> {
        let callback = oauth::wait_callback(attempt)?;
        let _gate = self.auth_gate.lock().unwrap();
        {
            let mut inner = self.inner.lock().unwrap();
            active_attempt(&inner, attempt)?;
            // Persist the issued registration before exchanging a one-use code.
            // If exchange fails, a new attempt can reuse the same registration.
            if attempt.client_id == oauth::DYNAMIC_CLIENT {
                let registration = Credentials {
                    client_id: callback.client_id.clone(),
                    subject: String::new(),
                    email: None,
                    id_token: String::new(),
                    access_token: String::new(),
                    refresh_token: String::new(),
                    expires_at: 0,
                    scopes: Vec::new(),
                    earliest_refresh_at: None,
                    pending_refresh: None,
                };
                save(self.store.as_ref(), &registration)?;
                inner.record = Some(registration);
            }
        }
        let client = oauth::http()?;
        let tokens = oauth::token_request(
            &client,
            &attempt.discovery,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", &callback.client_id),
                ("code", &callback.code),
                ("code_verifier", &attempt.verifier),
                ("redirect_uri", &attempt.redirect_uri),
                ("resource", oauth::RESOURCE),
            ],
        )?;
        let id_token = tokens
            .id_token
            .as_deref()
            .filter(|token| !token.is_empty())
            .ok_or("OpenAI did not supply a verifiable identity token.")?;
        let (subject, email) = oauth::identity(
            &client,
            &attempt.discovery,
            id_token,
            &callback.client_id,
            Some(&attempt.nonce),
        )?;
        if attempt
            .previous
            .as_ref()
            .is_some_and(|previous| !previous.subject.is_empty() && previous.subject != subject)
        {
            return Err(
                "The signed-in account does not match this saved ChatGPT registration.".into(),
            );
        }
        let scopes = scopes(tokens.scope.as_deref().unwrap_or(""));
        let refresh_token = tokens.refresh_token.clone().unwrap_or_default();
        if scopes.iter().any(|scope| scope == "offline_access") && refresh_token.is_empty() {
            return Err("OpenAI did not provide the requested renewable connection.".into());
        }
        refresh_not_before(tokens.earliest_refresh_at.as_ref())?;
        let record = Credentials {
            client_id: callback.client_id,
            subject,
            email,
            id_token: id_token.into(),
            access_token: tokens.access_token,
            refresh_token,
            expires_at: now() + tokens.expires_in,
            scopes,
            earliest_refresh_at: tokens.earliest_refresh_at,
            pending_refresh: None,
        };
        let mut inner = self.inner.lock().unwrap();
        active_attempt(&inner, attempt)?;
        save(self.store.as_ref(), &record)?;
        inner.record = Some(record);
        inner.generation += 1;
        Ok(())
    }

    fn access(&self) -> Result<(u64, Credentials), String> {
        let _gate = self.auth_gate.lock().unwrap();
        let (generation, mut record) = {
            let inner = self.inner.lock().unwrap();
            (
                inner.generation,
                inner
                    .record
                    .clone()
                    .ok_or("Continue with ChatGPT in Settings before sending a message.")?,
            )
        };
        if record.access_token.is_empty() || !sharing(&record) {
            return Err("Allow ChatGPT plan usage for this connection in Settings before sending a message.".into());
        }
        if record.pending_refresh.is_some() || record.expires_at <= now() + 60 {
            if record.refresh_token.is_empty() {
                return Err(
                    "This ChatGPT connection has expired. Continue with ChatGPT again.".into(),
                );
            }
            let earliest = refresh_not_before(record.earliest_refresh_at.as_ref())?;
            if record.pending_refresh.is_none() && earliest.is_some_and(|earliest| earliest > now())
            {
                if record.expires_at > now() + 5 {
                    return Ok((generation, record));
                }
                return Err("OpenAI has not yet allowed renewal of this connection. Wait before retrying or reconnect with ChatGPT.".into());
            }
            let client = oauth::http()?;
            let discovery = oauth::discovery(&client)?;
            let rotation = if let Some(rotation) = record.pending_refresh.clone() {
                rotation
            } else {
                {
                    let mut inner = self.inner.lock().unwrap();
                    if generation != inner.generation {
                        return Err("The ChatGPT connection changed. Send a new message.".into());
                    }
                    // A refresh grant may be consumed even if its HTTP response
                    // is lost. Retire it durably before sending the one-use grant.
                    let retired = disconnected(record.clone());
                    save(self.store.as_ref(), &retired)?;
                    inner.record = Some(retired);
                    inner.revocation_uncertain = true;
                }
                let tokens=oauth::token_request(&client,&discovery,&[("grant_type","refresh_token"),("client_id",&record.client_id),
                    ("refresh_token",&record.refresh_token),("resource",oauth::RESOURCE)])
                    .map_err(|_| "ChatGPT renewal could not be verified. Reconnect with ChatGPT; the previous refresh grant will not be retried.")?;
                let refresh_token=tokens.refresh_token.filter(|refresh|!refresh.is_empty()).ok_or("OpenAI did not return a rotated refresh token. Reconnect with ChatGPT; the previous grant will not be retried.")?;
                let rotation = PendingRefresh {
                    access_token: tokens.access_token,
                    refresh_token,
                    id_token: tokens.id_token,
                    expires_at: now() + tokens.expires_in,
                    scopes: tokens
                        .scope
                        .map(|scope| scopes(&scope))
                        .unwrap_or_else(|| record.scopes.clone()),
                    earliest_refresh_at: tokens.earliest_refresh_at,
                };
                record.pending_refresh = Some(rotation.clone());
                let mut inner = self.inner.lock().unwrap();
                if generation != inner.generation {
                    return Err("The ChatGPT connection changed. Send a new message.".into());
                }
                if let Err(error) = save(self.store.as_ref(), &record) {
                    // The previous refresh grant may already be consumed. Do not
                    // make it usable for a second attempt in this process.
                    inner.record = Some(disconnected(record.clone()));
                    return Err(format!("{error} Reconnect with ChatGPT; the previous refresh grant will not be retried."));
                }
                inner.record = Some(record.clone());
                inner.revocation_uncertain = false;
                rotation
            };
            // Validate after checkpointing the new grant so malformed metadata
            // or a temporary identity-key outage cannot replay the old grant.
            refresh_not_before(rotation.earliest_refresh_at.as_ref())?;
            if let Some(id_token) = rotation.id_token.as_deref() {
                let (subject, email) =
                    oauth::identity(&client, &discovery, id_token, &record.client_id, None)?;
                if subject != record.subject {
                    return Err(
                        "OpenAI renewed a different account. Reconnect before continuing.".into(),
                    );
                }
                record.id_token = id_token.into();
                record.email = email;
            }
            record.access_token = rotation.access_token;
            record.expires_at = rotation.expires_at;
            record.refresh_token = rotation.refresh_token;
            record.scopes = rotation.scopes;
            record.earliest_refresh_at = rotation.earliest_refresh_at;
            record.pending_refresh = None;
            let mut inner = self.inner.lock().unwrap();
            if generation != inner.generation {
                return Err("The ChatGPT connection changed. Send a new message.".into());
            }
            save(self.store.as_ref(), &record)?;
            inner.record = Some(record.clone());
            if !sharing(&record) {
                return Err("The renewed connection does not allow ChatGPT plan usage.".into());
            }
        }
        Ok((generation, record))
    }

    fn models_with(&self, record: &Credentials) -> Result<Vec<Model>, String> {
        let response = oauth::http()?
            .get("https://api.openai.com/v1/models")
            .bearer_auth(&record.access_token)
            .send()
            .map_err(|_| "Could not obtain models for your ChatGPT connection.")?;
        if !response.status().is_success() {
            return Err(responses::response_error(response));
        }
        // Catalog entries include substantial capability metadata that we do
        // not expose. Bound the complete catalog separately from OAuth tokens.
        let value: Value =
            oauth::bounded_json_with_limit(response, 4 * 1024 * 1024, "ChatGPT model catalog")?;
        let entries = value["models"]
            .as_array()
            .ok_or("ChatGPT returned an invalid model catalog.")?;
        if entries.len() > 1000 {
            return Err("ChatGPT model catalog exceeded the size limit.".into());
        }
        let models = entries
            .iter()
            .filter(|entry| entry["visibility"].as_str() == Some("list"))
            .filter_map(|entry| {
                let slug = entry["slug"].as_str()?;
                let display_name = entry["display_name"].as_str()?;
                if slug.is_empty()
                    || slug.len() > 128
                    || slug.chars().any(char::is_whitespace)
                    || display_name.len() > 256
                {
                    return None;
                }
                Some(Model {
                    slug: slug.into(),
                    display_name: display_name.into(),
                })
            })
            .collect();
        Ok(models)
    }

    pub fn models(&self) -> Result<Vec<Model>, String> {
        let (generation, record) = self.access()?;
        let models = self.models_with(&record)?;
        self.check_generation(generation)?;
        Ok(models)
    }

    pub fn respond(&self, model: &str, input: Value, instructions: &str) -> Result<Reply, String> {
        if !input.is_array()
            || serde_json::to_vec(&input)
                .map_err(|_| "Could not encode your message.")?
                .len()
                > 30_000_000
        {
            return Err("The message or attachment exceeds the size limit.".into());
        }
        let (generation, record) = self.access()?;
        let models = self.models_with(&record)?;
        let selected = select_model(&models, model)
            .ok_or("Choose an available model from your connected ChatGPT account in Settings.")?;
        self.check_generation(generation)?;
        let text = responses::request(
            &oauth::inference_http()?,
            &record.access_token,
            &selected.slug,
            input,
            instructions,
        )?;
        self.check_generation(generation)?;
        Ok(Reply { text, generation })
    }

    fn check_generation(&self, generation: u64) -> Result<(), String> {
        if self.inner.lock().unwrap().generation != generation {
            return Err("The ChatGPT connection changed. Send a new message.".into());
        }
        Ok(())
    }

    pub fn logout(&self) -> Result<(), String> {
        let (generation, previous) = {
            let mut inner = self.inner.lock().unwrap();
            inner.generation += 1;
            if let Some(attempt) = inner.pending.take() {
                attempt.cancelled.store(true, Ordering::Release);
            }
            let previous = inner.record.clone();
            inner.record = previous.clone().map(disconnected);
            (inner.generation, previous)
        };
        // Invalidation happens before waiting for an in-flight refresh. Its
        // later commit will see the new generation and cannot restore a token.
        let _gate = self.auth_gate.lock().unwrap();
        let Some(mut record) = previous else {
            let inner = self.inner.lock().unwrap();
            if generation != inner.generation {
                return Err(
                    "The ChatGPT connection changed. Check the current connection in Settings."
                        .into(),
                );
            }
            storage::clear(self.store.as_ref())?;
            return Ok(());
        };
        let token = record
            .pending_refresh
            .as_ref()
            .map(|rotation| rotation.refresh_token.as_str())
            .unwrap_or(&record.refresh_token);
        let revoke = if token.is_empty() {
            Ok(())
        } else {
            (|| {
                let client = oauth::http()?;
                let discovery = oauth::discovery(&client)?;
                let response=client.post(&discovery.revocation_endpoint).form(&[("token",token),
                    ("token_type_hint","refresh_token"),("client_id",record.client_id.as_str())]).send()
                    .map_err(|_| "Local credentials were disconnected, but OpenAI revocation could not be confirmed. Revoke app access in ChatGPT settings.")?;
                if response.status().as_u16() != 200 {
                    return Err("Local credentials were disconnected, but OpenAI revocation could not be confirmed. Revoke app access in ChatGPT settings.".into());
                }
                Ok(())
            })()
        };
        record = disconnected(record);
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            return Err("The ChatGPT connection changed during disconnection. Check the current connection in Settings.".into());
        }
        // Keep the verified registration so reconnecting preserves usage settings.
        save(self.store.as_ref(),&record).map_err(|_| "This app stopped using the session, but protected credential removal could not be confirmed. See docs/SECURITY-OAUTH.md for recovery and revoke app access in ChatGPT settings.")?;
        inner.record = Some(record);
        if inner.revocation_uncertain {
            return Err("This app stopped using the session, but OpenAI revocation could not be confirmed after renewal. Revoke app access in ChatGPT settings.".into());
        }
        revoke
    }
}

fn scopes(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .take(64)
        .map(str::to_string)
        .collect()
}

fn select_model<'a>(models: &'a [Model], requested: &str) -> Option<&'a Model> {
    if requested.is_empty() {
        models
            .iter()
            .find(|model| model.slug == "gpt-5.6-luna")
            .or_else(|| models.first())
    } else {
        models.iter().find(|model| model.slug == requested)
    }
}
fn sharing(record: &Credentials) -> bool {
    record
        .scopes
        .iter()
        .any(|scope| scope == "chatgpt.tokens.use.direct")
        && record.scopes.iter().any(|scope| scope == "resource.invoke")
}
fn active_attempt(inner: &Inner, attempt: &oauth::Attempt) -> Result<(), String> {
    if attempt.cancelled.load(Ordering::Acquire)
        || inner.generation != attempt.generation
        || !inner
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == attempt.id)
    {
        return Err("This OpenAI sign-in attempt was cancelled or replaced.".into());
    }
    Ok(())
}
fn save(store: &dyn Store, record: &Credentials) -> Result<(), String> {
    storage::save(
        store,
        &serde_json::to_string(record)
            .map_err(|_| "Could not encode protected ChatGPT credentials.")?,
    )
}
fn valid_host(value: &str) -> bool {
    let Some(uuid) = value.strip_prefix("urn:uuid:") else {
        return false;
    };
    uuid.len() == 36
        && uuid.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn disconnected(mut record: Credentials) -> Credentials {
    record.id_token.clear();
    record.access_token.clear();
    record.refresh_token.clear();
    record.expires_at = 0;
    record.scopes.clear();
    record.earliest_refresh_at = None;
    record.pending_refresh = None;
    record
}

fn refresh_not_before(value: Option<&Value>) -> Result<Option<u64>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let epoch = if let Some(epoch) = value.as_u64() {
        Some(epoch)
    } else if let Some(text) = value.as_str().filter(|text| text.len() <= 128) {
        text.parse::<u64>().ok().or_else(|| {
            chrono::DateTime::parse_from_rfc3339(text)
                .ok()
                .and_then(|time| u64::try_from(time.timestamp()).ok())
        })
    } else {
        None
    };
    let epoch = epoch
        .filter(|epoch| *epoch <= 253_402_300_799)
        .ok_or("OpenAI returned an invalid token renewal time.")?;
    Ok(Some(epoch))
}

#[cfg(test)]
mod adversarial;
