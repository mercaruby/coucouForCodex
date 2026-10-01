use crate::{now, random_value, Credentials, HTTP_TIMEOUT};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use url::Url;

pub(crate) const ISSUER: &str = "https://auth.openai.com";
pub(crate) const RESOURCE: &str = "https://api.openai.com/v1";
pub(crate) const DYNAMIC_CLIENT: &str = "dynamic_agent_client";

#[derive(Clone, Deserialize)]
pub(crate) struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub revocation_endpoint: String,
    pub id_token_signing_alg_values_supported: Vec<String>,
}

pub(crate) fn http() -> Result<Client, String> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|_| "Could not initialize the secure OpenAI connection.".into())
}

pub(crate) fn inference_http() -> Result<Client, String> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|_| "Could not initialize the secure ChatGPT connection.".into())
}

#[cfg(test)]
pub(crate) fn bounded_json<T: DeserializeOwned>(
    response: reqwest::blocking::Response,
) -> Result<T, String> {
    bounded_json_with_limit(response, 262144, "OpenAI")
}

pub(crate) fn bounded_json_with_limit<T: DeserializeOwned>(
    response: reqwest::blocking::Response,
    limit: usize,
    stage: &str,
) -> Result<T, String> {
    let mut body = Vec::new();
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|_| format!("Could not read the {stage} response."))?;
    if body.len() > limit {
        return Err(format!("{stage} response exceeded the size limit."));
    }
    serde_json::from_slice(&body).map_err(|_| format!("{stage} returned an invalid response."))
}

pub(crate) fn discovery(client: &Client) -> Result<Discovery, String> {
    let response = client
        .get(format!("{ISSUER}/.well-known/openid-configuration"))
        .send()
        .map_err(|_| "Could not connect to OpenAI sign-in. Try again later.")?;
    if !response.status().is_success() {
        return Err("OpenAI sign-in discovery is unavailable.".into());
    }
    let discovery: Discovery =
        bounded_json_with_limit(response, 262144, "OpenAI sign-in discovery")?;
    if discovery.issuer != ISSUER
        || discovery.authorization_endpoint != format!("{ISSUER}/api/accounts/authorize")
        || discovery.token_endpoint != format!("{ISSUER}/api/accounts/oauth/token")
        || discovery.jwks_uri != format!("{ISSUER}/.well-known/jwks.json")
        || discovery.revocation_endpoint != format!("{ISSUER}/api/accounts/oauth/revoke")
        || !discovery
            .id_token_signing_alg_values_supported
            .iter()
            .any(|alg| alg == "RS256")
    {
        return Err(
            "OpenAI sign-in configuration changed. Update the companion before signing in.".into(),
        );
    }
    Ok(discovery)
}

pub(crate) struct Attempt {
    pub id: String,
    pub listener: TcpListener,
    pub cancelled: AtomicBool,
    pub finishing: AtomicBool,
    pub expires: Instant,
    pub state: String,
    pub nonce: String,
    pub verifier: String,
    pub redirect_uri: String,
    pub client_id: String,
    pub previous: Option<Credentials>,
    pub discovery: Discovery,
    pub generation: u64,
}

pub(crate) fn prepare(
    discovery: Discovery,
    host_id: &str,
    previous: Option<Credentials>,
    generation: u64,
) -> Result<(Attempt, String), String> {
    // Binding before opening the browser prevents another process from taking
    // the callback port between URL creation and server startup.
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|_| "Could not open the local OpenAI sign-in callback.")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "Could not configure the local sign-in callback.")?;
    let port = listener
        .local_addr()
        .map_err(|_| "Could not find the sign-in callback port.")?
        .port();
    let client_id = previous
        .as_ref()
        .map(|record| record.client_id.clone())
        .unwrap_or_else(|| DYNAMIC_CLIENT.into());
    let attempt = Attempt {
        id: random_value(),
        listener,
        cancelled: AtomicBool::new(false),
        finishing: AtomicBool::new(false),
        expires: Instant::now() + Duration::from_secs(600),
        state: random_value(),
        nonce: random_value(),
        verifier: random_value(),
        redirect_uri: format!("http://127.0.0.1:{port}/auth/callback"),
        client_id,
        previous,
        discovery,
        generation,
    };
    let mut url = Url::parse(&attempt.discovery.authorization_endpoint)
        .map_err(|_| "Invalid OpenAI sign-in endpoint.")?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", &attempt.client_id)
            .append_pair("ext_agent_host_id", host_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &attempt.redirect_uri)
            .append_pair(
                "scope",
                "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
            )
            .append_pair("resource", RESOURCE)
            .append_pair("state", &attempt.state)
            .append_pair("nonce", &attempt.nonce)
            .append_pair("code_challenge_method", "S256")
            .append_pair(
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(attempt.verifier.as_bytes())),
            );
        if attempt.client_id == DYNAMIC_CLIENT {
            query.append_pair("agent_name_hint", "Coucou Codex");
        } else if let Some(email) = attempt
            .previous
            .as_ref()
            .and_then(|record| record.email.as_deref())
        {
            query.append_pair("login_hint", email);
        }
        // ID-token hints are deliberately omitted. They are credentials and
        // must not cross into the frontend or browser-launch diagnostics.
    }
    Ok((attempt, url.into()))
}

fn equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

pub(crate) struct Callback {
    pub code: String,
    pub client_id: String,
}

fn callback(target: &str, state: &str, saved_client: &str) -> Result<Option<Callback>, String> {
    let Some(query) = target.strip_prefix("/auth/callback?") else {
        return Ok(None);
    };
    if query.len() > 8192 || query.contains('#') {
        return Ok(None);
    }
    let mut params = HashMap::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if params
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Ok(None);
        }
    }
    if !params
        .get("state")
        .is_some_and(|received| equal(received, state))
    {
        return Ok(None);
    }
    if params.contains_key("error") {
        return Err("OpenAI sign-in was declined or cancelled. No credentials were saved.".into());
    }
    let code = params
        .remove("code")
        .filter(|code| !code.is_empty() && code.len() < 4096)
        .ok_or("OpenAI returned an incomplete sign-in callback.")?;
    let client_id = match params.remove("client_id") {
        Some(id)
            if !id.is_empty()
                && id != DYNAMIC_CLIENT
                && id.len() <= 256
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') =>
        {
            id
        }
        None if saved_client != DYNAMIC_CLIENT => saved_client.to_string(),
        _ => {
            return Err(
                "OpenAI did not complete the app registration. Try signing in again.".into(),
            )
        }
    };
    if saved_client != DYNAMIC_CLIENT && client_id != saved_client {
        return Err("OpenAI returned a different app registration. Sign-in was rejected.".into());
    }
    Ok(Some(Callback { code, client_id }))
}

fn browser_response(stream: &mut TcpStream, ok: bool) {
    let body = if ok {
        "OpenAI sign-in received. Return to Coucou Codex to check the connection."
    } else {
        "This sign-in request was not accepted. Return to Coucou Codex."
    };
    let status = if ok { "200 OK" } else { "400 Bad Request" };
    let _ = write!(stream,"HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",body.len());
}

fn read_callback(stream: &mut TcpStream, attempt: &Attempt) -> Result<Option<Callback>, String> {
    // Windows accepted sockets can inherit the listener's nonblocking mode.
    stream
        .set_nonblocking(false)
        .map_err(|_| "Could not configure the local sign-in callback.")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| "Could not read the local sign-in callback.")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| "Could not write the local sign-in callback.")?;
    let mut bytes = Vec::new();
    let deadline = std::cmp::min(Instant::now() + Duration::from_secs(2), attempt.expires);
    loop {
        if attempt.cancelled.load(Ordering::Acquire) {
            return Err("OpenAI sign-in was cancelled.".into());
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(None);
        };
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| "Could not configure the local sign-in callback.")?;
        let mut buffer = [0_u8; 512];
        match stream.read(&mut buffer) {
            Ok(0) => return Ok(None),
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(_) => return Ok(None),
        }
        if bytes.len() > 16384 {
            return Ok(None);
        }
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = match std::str::from_utf8(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let mut lines = request.split("\r\n");
    let Some(line) = lines.next() else {
        return Ok(None);
    };
    let parts: Vec<_> = line.split(' ').collect();
    if parts.len() != 3 || parts[0] != "GET" || parts[2] != "HTTP/1.1" {
        return Ok(None);
    }
    let expected_host = attempt
        .redirect_uri
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let mut host = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Ok(None);
        };
        if name.eq_ignore_ascii_case("Host") {
            if host.replace(value.trim()).is_some() {
                return Ok(None);
            }
        }
        if name.eq_ignore_ascii_case("Transfer-Encoding")
            || name.eq_ignore_ascii_case("Origin")
            || (name.eq_ignore_ascii_case("Content-Length") && value.trim() != "0")
        {
            return Ok(None);
        }
    }
    if host != Some(expected_host) {
        return Ok(None);
    }
    callback(parts[1], &attempt.state, &attempt.client_id)
}

pub(crate) fn wait_callback(attempt: &Attempt) -> Result<Callback, String> {
    while Instant::now() < attempt.expires {
        if attempt.cancelled.load(Ordering::Acquire) {
            return Err("OpenAI sign-in was cancelled.".into());
        }
        match attempt.listener.accept() {
            Ok((mut stream, addr)) => {
                if !addr.ip().is_loopback() {
                    continue;
                }
                let result = read_callback(&mut stream, attempt);
                browser_response(&mut stream, matches!(result, Ok(Some(_))));
                match result {
                    Ok(Some(callback)) => return Ok(callback),
                    Err(error) => return Err(error),
                    _ => {}
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(30))
            }
            Err(_) => return Err("The local OpenAI sign-in callback failed.".into()),
        }
    }
    Err("OpenAI sign-in timed out. Start a new sign-in attempt.".into())
}

#[derive(Deserialize)]
pub(crate) struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub token_type: String,
    pub expires_in: u64,
    pub scope: Option<String>,
    pub earliest_refresh_at: Option<serde_json::Value>,
}

pub(crate) fn token_request(
    client: &Client,
    discovery: &Discovery,
    form: &[(&str, &str)],
) -> Result<Tokens, String> {
    let response = client
        .post(&discovery.token_endpoint)
        .form(form)
        .send()
        .map_err(|_| "Could not connect to OpenAI's token endpoint.")?;
    if !response.status().is_success() {
        return Err(
            "OpenAI could not complete or renew sign-in. Start a fresh sign-in attempt.".into(),
        );
    }
    let tokens: Tokens = bounded_json_with_limit(response, 262144, "OpenAI token exchange")?;
    if !tokens.token_type.eq_ignore_ascii_case("Bearer")
        || tokens.access_token.is_empty()
        || tokens.access_token.len() > 32768
        || tokens.expires_in == 0
        || tokens.expires_in > 604800
        || tokens
            .id_token
            .as_ref()
            .is_some_and(|value| value.len() > 32768)
        || tokens
            .refresh_token
            .as_ref()
            .is_some_and(|value| value.len() > 16384)
        || tokens
            .scope
            .as_ref()
            .is_some_and(|value| value.len() > 8192)
    {
        return Err("OpenAI returned an invalid token response.".into());
    }
    Ok(tokens)
}

#[derive(Deserialize, Serialize)]
struct Identity {
    sub: String,
    email: Option<String>,
    nonce: Option<String>,
    azp: Option<String>,
    iat: u64,
    aud: serde_json::Value,
}

pub(crate) fn identity(
    client: &Client,
    discovery: &Discovery,
    token: &str,
    client_id: &str,
    nonce: Option<&str>,
) -> Result<(String, Option<String>), String> {
    let header = decode_header(token).map_err(|_| "OpenAI identity verification failed.")?;
    if header.alg != Algorithm::RS256 {
        return Err("OpenAI identity signing algorithm is unsupported.".into());
    }
    let kid = header.kid.ok_or("OpenAI identity verification failed.")?;
    let response = client
        .get(&discovery.jwks_uri)
        .send()
        .map_err(|_| "Could not obtain OpenAI identity verification keys.")?;
    if !response.status().is_success() {
        return Err("Could not obtain OpenAI identity verification keys.".into());
    }
    let keys: JwkSet =
        bounded_json_with_limit(response, 262144, "OpenAI identity verification keys")?;
    let key = keys
        .find(&kid)
        .ok_or("OpenAI identity verification key was not found.")?;
    let key = DecodingKey::from_jwk(key).map_err(|_| "OpenAI identity verification failed.")?;
    verify_identity(token, &key, client_id, nonce)
}

fn verify_identity(
    token: &str,
    key: &DecodingKey,
    client_id: &str,
    nonce: Option<&str>,
) -> Result<(String, Option<String>), String> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[client_id]);
    validation.set_required_spec_claims(&["iss", "aud", "exp", "sub", "iat"]);
    validation.validate_nbf = true;
    validation.leeway = 30;
    let claims = decode::<Identity>(token, key, &validation)
        .map_err(|_| "OpenAI identity verification failed.")?
        .claims;
    if claims.sub.is_empty()
        || claims.sub.len() > 512
        || claims.iat > now() + 30
        || claims.azp.as_deref().is_some_and(|azp| azp != client_id)
        || (claims.aud.as_array().is_some_and(|aud| aud.len() > 1)
            && claims.azp.as_deref() != Some(client_id))
        || nonce.is_some_and(|expected| {
            !claims
                .nonce
                .as_deref()
                .is_some_and(|nonce| equal(nonce, expected))
        })
    {
        return Err("OpenAI identity verification failed.".into());
    }
    Ok((claims.sub, claims.email.filter(|email| email.len() <= 512)))
}

#[cfg(test)]
#[path = "oauth_adversarial.rs"]
mod independent_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unrelated_callback_does_not_consume_attempt() {
        assert!(callback(
            "/auth/callback?state=foreign&code=secret&client_id=oaiapp_fixture",
            "correct",
            DYNAMIC_CLIENT
        )
        .unwrap()
        .is_none());
        assert!(callback(
            "/auth/callback?state=correct&state=foreign&code=secret&client_id=oaiapp_fixture",
            "correct",
            DYNAMIC_CLIENT
        )
        .unwrap()
        .is_none());
    }
    #[test]
    fn registration_and_reconnection_require_exact_client_id() {
        assert!(callback("/auth/callback?state=x&code=c", "x", DYNAMIC_CLIENT).is_err());
        assert!(callback(
            "/auth/callback?state=x&code=c&client_id=dynamic_agent_client",
            "x",
            DYNAMIC_CLIENT
        )
        .is_err());
        assert!(callback(
            "/auth/callback?state=x&code=c&client_id=oaiapp_other",
            "x",
            "oaiapp_saved"
        )
        .is_err());
        assert_eq!(
            callback("/auth/callback?state=x&code=c", "x", "oaiapp_saved")
                .unwrap()
                .unwrap()
                .client_id,
            "oaiapp_saved"
        );
    }
    #[test]
    fn oauth_errors_are_redacted() {
        let error = callback(
            "/auth/callback?state=x&error=secret-token",
            "x",
            DYNAMIC_CLIENT,
        )
        .err()
        .unwrap();
        assert!(!error.contains("secret-token"));
    }
}
