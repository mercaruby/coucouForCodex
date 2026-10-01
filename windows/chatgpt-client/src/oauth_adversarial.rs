//! Independent tests of OAuth trust boundaries using a synthetic signing key.
use super::*;
use base64::engine::general_purpose::STANDARD;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../test-fixtures/synthetic-rsa.json")).unwrap()
}
fn key() -> DecodingKey {
    let fixture = fixture();
    DecodingKey::from_rsa_components(
        fixture["n"].as_str().unwrap(),
        fixture["e"].as_str().unwrap(),
    )
    .unwrap()
}
fn claims() -> Value {
    json!({"iss":ISSUER,"aud":"oaiapp_fixture","sub":"synthetic-user","iat":now(),"exp":now()+300,"nonce":"expected-nonce"})
}
fn signed(claims: &Value) -> String {
    let bytes = STANDARD
        .decode(fixture()["privateDer"].as_str().unwrap())
        .unwrap();
    encode(
        &Header::new(Algorithm::RS256),
        claims,
        &EncodingKey::from_rsa_der(&bytes),
    )
    .unwrap()
}
fn verified(claims: &Value) -> Result<(String, Option<String>), String> {
    verify_identity(
        &signed(claims),
        &key(),
        "oaiapp_fixture",
        Some("expected-nonce"),
    )
}

#[test]
fn valid_signed_identity_succeeds_but_mutated_signature_fails() {
    assert_eq!(verified(&claims()).unwrap().0, "synthetic-user");
    let token = signed(&claims());
    let mut segments = token.split('.').map(str::to_owned).collect::<Vec<_>>();
    let first = if segments[2].starts_with('A') {
        "B"
    } else {
        "A"
    };
    segments[2].replace_range(..1, first);
    assert!(verify_identity(
        &segments.join("."),
        &key(),
        "oaiapp_fixture",
        Some("expected-nonce")
    )
    .is_err());
}

#[test]
fn identity_rejects_wrong_issuer_audience_nonce_subject_and_timestamps() {
    for (field, invalid) in [
        ("iss", json!("https://auth.openai.com.attacker.invalid")),
        ("aud", json!("oaiapp_other")),
        ("nonce", json!("foreign-nonce")),
        ("sub", json!("")),
        ("sub", json!("s".repeat(513))),
        ("iat", json!(now() + 3600)),
        ("exp", json!(now() - 120)),
        ("nbf", json!(now() + 3600)),
    ] {
        let mut value = claims();
        value[field] = invalid;
        assert!(
            verified(&value).is_err(),
            "invalid claim {field} was accepted"
        );
    }
    for field in ["iss", "aud", "exp", "iat", "sub", "nonce"] {
        let mut value = claims();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            verified(&value).is_err(),
            "missing claim {field} was accepted"
        );
    }
}

#[test]
fn multiple_audiences_require_exact_authorized_party() {
    let mut value = claims();
    value["aud"] = json!(["oaiapp_fixture", "another-client"]);
    assert!(verified(&value).is_err());
    value["azp"] = json!("another-client");
    assert!(verified(&value).is_err());
    value["azp"] = json!("oaiapp_fixture");
    assert!(verified(&value).is_ok());
    value["aud"] = json!("oaiapp_fixture");
    value["azp"] = json!("another-client");
    assert!(verified(&value).is_err());
}

#[test]
fn algorithm_confusion_and_unsigned_tokens_are_rejected_without_exposing_input() {
    let secret_marker = "synthetic-secret-marker-not-a-real-token";
    let hs256 = encode(
        &Header::new(Algorithm::HS256),
        &claims(),
        &EncodingKey::from_secret(secret_marker.as_bytes()),
    )
    .unwrap();
    for token in [
        hs256,
        format!(
            "eyJhbGciOiJub25lIn0.{}.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims()).unwrap()),
            secret_marker
        ),
    ] {
        let error = verify_identity(&token, &key(), "oaiapp_fixture", Some("expected-nonce"))
            .err()
            .unwrap();
        assert!(!error.contains(secret_marker));
        assert!(!error.contains(&token));
    }
}

fn discovery_fixture() -> Discovery {
    Discovery {
        issuer: ISSUER.into(),
        authorization_endpoint: format!("{ISSUER}/api/accounts/authorize"),
        token_endpoint: format!("{ISSUER}/api/accounts/oauth/token"),
        jwks_uri: format!("{ISSUER}/.well-known/jwks.json"),
        revocation_endpoint: format!("{ISSUER}/api/accounts/oauth/revoke"),
        id_token_signing_alg_values_supported: vec!["RS256".into()],
    }
}
fn attempt() -> Attempt {
    prepare(
        discovery_fixture(),
        "urn:uuid:synthetic-fixture-host",
        None,
        0,
    )
    .unwrap()
    .0
}

#[test]
fn authorization_url_binds_pkce_nonce_state_and_local_prebound_listener() {
    let (attempt, url) = prepare(
        discovery_fixture(),
        "urn:uuid:synthetic-fixture-host",
        None,
        42,
    )
    .unwrap();
    let parsed = Url::parse(&url).unwrap();
    let params: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(
        params["code_challenge"],
        URL_SAFE_NO_PAD.encode(Sha256::digest(attempt.verifier.as_bytes()))
    );
    assert_eq!(params["state"], attempt.state);
    assert_eq!(params["nonce"], attempt.nonce);
    assert_ne!(attempt.state, attempt.nonce);
    assert_ne!(attempt.verifier, attempt.state);
    assert_eq!(attempt.verifier.len(), 43);
    assert_eq!(attempt.state.len(), 43);
    assert_eq!(params["resource"], RESOURCE);
    assert_eq!(params["client_id"], DYNAMIC_CLIENT);
    assert!(!params.contains_key("id_token_hint"));
    assert!(
        TcpListener::bind(attempt.listener.local_addr().unwrap()).is_err(),
        "callback port must already be owned"
    );
    assert_eq!(attempt.generation, 42);
}

#[test]
fn callback_rejects_path_confusion_duplicate_encoded_parameters_and_registration_changes() {
    let state = "known-state";
    for target in [
        "/auth/callback/extra?state=known-state&code=c&client_id=oaiapp_fixture",
        "/auth/callback?state=known-state&%73tate=known-state&code=c&client_id=oaiapp_fixture",
        "/auth/callback?state=known-state&code=c&code=d&client_id=oaiapp_fixture",
        "/auth/callback?state=known-state&code=c&client_id=oaiapp_fixture#ignored",
        "/auth/callback?state=foreign&code=c&client_id=oaiapp_fixture",
    ] {
        assert!(
            callback(target, state, DYNAMIC_CLIENT).unwrap().is_none(),
            "accepted {target}"
        );
    }
    for client in [
        "dynamic_agent_client",
        "oaiapp_foreign",
        "attacker.invalid/path",
        "",
        "client%0d%0aHeader:x",
    ] {
        let target = format!("/auth/callback?state=known-state&code=c&client_id={client}");
        assert!(callback(&target, state, "oaiapp_fixture").is_err());
    }
    assert!(callback(
        "/auth/callback?state=known-state&code=&client_id=oaiapp_fixture",
        state,
        DYNAMIC_CLIENT
    )
    .is_err());
}

fn request_result(attempt: &Attempt, request: String) -> Result<Option<Callback>, String> {
    let address = attempt.listener.local_addr().unwrap();
    let sender = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(address).unwrap();
        // Accept before request bytes arrive: Windows accepts may inherit the
        // listener's nonblocking mode, which timeout settings alone do not fix.
        std::thread::sleep(Duration::from_millis(20));
        stream.write_all(request.as_bytes()).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
    });
    let (mut stream, _) = loop {
        match attempt.listener.accept() {
            Ok(value) => break value,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::yield_now()
            }
            Err(error) => panic!("{error}"),
        }
    };
    let result = read_callback(&mut stream, attempt);
    sender.join().unwrap();
    result
}

#[test]
fn callback_http_requires_get_exact_host_and_no_request_body_or_cross_origin_headers() {
    let attempt = attempt();
    let host = attempt
        .redirect_uri
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let target = format!(
        "/auth/callback?state={}&code=synthetic-code&client_id=oaiapp_fixture",
        attempt.state
    );
    for request in [
        format!("POST {target} HTTP/1.1\r\nHost: {host}\r\n\r\n"),
        format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n"),
        format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nHost: {host}\r\n\r\n"),
        format!(
            "GET {target} HTTP/1.1\r\nHost: {host}\r\nOrigin: https://attacker.invalid\r\n\r\n"
        ),
        format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nTransfer-Encoding: chunked\r\n\r\n"),
        format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nContent-Length: 1\r\n\r\nx"),
    ] {
        assert!(request_result(&attempt, request).unwrap().is_none());
    }
    assert!(request_result(
        &attempt,
        format!("GET {target} HTTP/1.1\r\nHost: {host}\r\n\r\n")
    )
    .unwrap()
    .is_some());
}

#[test]
fn callback_oversize_and_error_description_never_become_success_or_public_secrets() {
    assert!(callback(
        &format!(
            "/auth/callback?state=x&code={}&client_id=oaiapp_fixture",
            "s".repeat(9000)
        ),
        "x",
        DYNAMIC_CLIENT
    )
    .unwrap()
    .is_none());
    let error = callback(
        "/auth/callback?state=x&error=denied&error_description=synthetic-secret-marker",
        "x",
        DYNAMIC_CLIENT,
    )
    .err()
    .unwrap();
    assert!(!error.contains("synthetic-secret-marker"));
}

#[test]
fn slowly_dripped_callback_cannot_extend_attempt_deadline() {
    let mut attempt = attempt();
    attempt.expires = Instant::now() + Duration::from_millis(180);
    let address = attempt.listener.local_addr().unwrap();
    let sender = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(address).unwrap();
        for _ in 0..15 {
            if stream.write_all(b"G").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    });
    let (mut stream, _) = loop {
        match attempt.listener.accept() {
            Ok(value) => break value,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::yield_now()
            }
            Err(error) => panic!("{error}"),
        }
    };
    let started = Instant::now();
    assert!(read_callback(&mut stream, &attempt).unwrap().is_none());
    let elapsed = started.elapsed();
    drop(stream);
    sender.join().unwrap();
    assert!(
        elapsed >= Duration::from_millis(100),
        "nonblocking accepted stream rejected before deadline: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(800),
        "drip kept callback alive beyond deadline: {elapsed:?}"
    );
}

#[test]
fn cancelled_attempt_cannot_accept_callback_even_when_state_is_valid() {
    let attempt = attempt();
    attempt.cancelled.store(true, Ordering::Release);
    let host = attempt
        .redirect_uri
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let request = format!("GET /auth/callback?state={}&code=c&client_id=oaiapp_fixture HTTP/1.1\r\nHost: {host}\r\n\r\n", attempt.state);
    assert!(request_result(&attempt, request).is_err());
    assert!(wait_callback(&attempt).is_err());
}

fn fixture_http_json(body: String) -> reqwest::blocking::Response {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request);
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let _ = stream.write_all(response.as_bytes());
    });
    // Only a synthetic local HTTP response. No discovery endpoint or token
    // endpoint is overridden and no Bearer header is sent.
    http()
        .unwrap()
        .get(format!("http://{address}/fixture"))
        .send()
        .unwrap()
}

#[test]
fn oauth_json_bodies_are_bounded_and_errors_never_reflect_server_payload() {
    let small: Value = bounded_json(fixture_http_json("{\"ok\":true}".into())).unwrap();
    assert_eq!(small["ok"], json!(true));
    let large = serde_json::to_string(&json!({"secret":"s".repeat(262145)})).unwrap();
    assert!(bounded_json::<Value>(fixture_http_json(large)).is_err());
    let error = bounded_json::<Value>(fixture_http_json("synthetic-secret-marker".into()))
        .err()
        .unwrap();
    assert!(!error.contains("synthetic-secret-marker"));
}

#[test]
fn model_catalog_limit_accepts_large_catalog_but_enforces_four_mib() {
    const CATALOG_LIMIT: usize = 4 * 1024 * 1024;
    let body = serde_json::to_string(&json!({"models":[{"slug":"synthetic-model","visibility":"list","metadata":"m".repeat(300*1024)}]})).unwrap();
    assert!(bounded_json::<Value>(fixture_http_json(body.clone())).is_err());
    let catalog: Value = bounded_json_with_limit(
        fixture_http_json(body),
        CATALOG_LIMIT,
        "ChatGPT model catalog",
    )
    .unwrap();
    assert_eq!(catalog["models"][0]["slug"], "synthetic-model");
    let oversized =
        serde_json::to_string(&json!({"models":[{"metadata":"m".repeat(CATALOG_LIMIT)}]})).unwrap();
    let error = bounded_json_with_limit::<Value>(
        fixture_http_json(oversized),
        CATALOG_LIMIT,
        "ChatGPT model catalog",
    )
    .err()
    .unwrap();
    assert!(error.contains("model catalog"));
    assert!(error.contains("size limit"));
}
