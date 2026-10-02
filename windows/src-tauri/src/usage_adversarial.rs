//! Independent read-only usage parser tests. Synthetic data; no CLI or OAuth.
use super::*;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};

fn public_usage(input: Value) -> Value {
    serde_json::to_value(parse_usage(&input).unwrap()).unwrap()
}
fn window(bucket: &Value, id: &str) -> Value {
    bucket["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == id)
        .unwrap()
        .clone()
}

#[test]
fn legacy_explicit_identity_and_real_zero_percent_are_preserved() {
    let output = public_usage(
        json!({"rateLimits":{"limitId":"codex","limitName":"Codex","planType":"plus",
        "primary":{"usedPercent":0,"windowDurationMins":300,"resetsAt":1790812800},
        "secondary":{"usedPercent":73.5,"windowDurationMins":10080,"resetsAt":1791417600}}}),
    );
    assert_eq!(output[0]["id"], "codex");
    assert_eq!(window(&output[0], "primary")["usedPercent"], json!(0.0));
    assert_eq!(window(&output[0], "secondary")["usedPercent"], json!(73.5));
    assert_eq!(
        window(&output[0], "primary")["windowDurationMins"],
        json!(300)
    );
    assert_eq!(
        window(&output[0], "primary")["resetsAt"],
        json!(1790812800u64)
    );
}

#[test]
fn legacy_without_identity_never_claims_it_is_codex_specific_usage() {
    let output = public_usage(json!({"rateLimits":{"primary":{"usedPercent":17}}}));
    assert_eq!(output[0]["id"], "legacy");
    assert_ne!(output[0]["id"], "codex");
}

#[test]
fn missing_and_null_metrics_remain_unknown_instead_of_zero_or_full_remaining() {
    for primary in [
        json!({}),
        json!({"usedPercent":null,"windowDurationMins":null,"resetsAt":null}),
    ] {
        let output = public_usage(json!({"rateLimits":{"limitId":"fixture","primary":primary}}));
        let primary = window(&output[0], "primary");
        assert!(primary["usedPercent"].is_null());
        assert!(primary["windowDurationMins"].is_null());
        assert!(primary["resetsAt"].is_null());
    }
}

#[test]
fn multiple_limit_ids_are_kept_separate_without_serializing_unknown_fields() {
    let output = public_usage(json!({"rateLimitsByLimitId":{
        "codex":{"limitId":"contradictory-inner-id","limitName":"Codex","primary":{"usedPercent":17},"access_token":"synthetic-secret-marker"},
        "code-review":{"limitName":"Code review","primary":{"usedPercent":91},"rawResponse":{"refresh_token":"synthetic-secret-marker"}}
    }}));
    let buckets = output.as_array().unwrap();
    assert_eq!(buckets.len(), 2);
    assert!(buckets.iter().any(|entry| entry["id"] == "codex"));
    assert!(buckets.iter().any(|entry| entry["id"] == "code-review"));
    assert!(!output.to_string().contains("synthetic-secret-marker"));
    assert!(!output.to_string().contains("access_token"));
    assert!(!output.to_string().contains("rawResponse"));
}

#[test]
fn explicitly_empty_limit_map_is_authoritative_over_legacy_snapshot() {
    let output = public_usage(
        json!({"rateLimitsByLimitId":{},"rateLimits":{"limitId":"codex","primary":{"usedPercent":99}}}),
    );
    assert!(output.as_array().unwrap().is_empty());
}

#[test]
fn absent_or_null_limit_map_can_use_the_explicit_legacy_snapshot() {
    for input in [
        json!({"rateLimitsByLimitId":null,"rateLimits":{"limitId":"fixture","primary":{"usedPercent":8}}}),
        json!({"rateLimits":{"limitId":"fixture","primary":{"usedPercent":8}}}),
    ] {
        let output = public_usage(input);
        assert_eq!(output[0]["id"], "fixture");
        assert_eq!(window(&output[0], "primary")["usedPercent"], json!(8.0));
    }
}

#[test]
fn malformed_authoritative_map_rejects_without_fallback_or_error_reflection() {
    for map in [json!("synthetic-secret-marker"), json!([]), json!(true)] {
        let input = json!({"rateLimitsByLimitId":map,"rateLimits":{"primary":{"usedPercent":12}}});
        let error = parse_usage(&input).err().unwrap();
        assert!(!error.contains("synthetic-secret-marker"));
        assert!(!error.contains("usedPercent"));
    }
}

#[test]
fn current_camel_and_snake_metrics_are_supported_but_explicit_null_has_priority() {
    let output = public_usage(json!({"rateLimits":{"limitId":"fixture",
        "primary":{"used_percent":26.5,"window_duration_mins":300,"resets_at":1790812800},
        "secondary":{"usedPercent":null,"used_percent":99,"windowDurationMins":null,"window_duration_mins":10080,"resetsAt":null,"resets_at":1791417600}}}));
    let primary = window(&output[0], "primary");
    assert_eq!(primary["usedPercent"], json!(26.5));
    assert_eq!(primary["windowDurationMins"], json!(300));
    assert_eq!(primary["resetsAt"], json!(1790812800u64));
    let secondary = window(&output[0], "secondary");
    assert!(secondary["usedPercent"].is_null());
    assert!(secondary["windowDurationMins"].is_null());
    assert!(secondary["resetsAt"].is_null());
}

#[test]
fn malformed_or_out_of_range_metrics_never_invent_valid_percent_or_date() {
    for invalid in [
        json!(-1),
        json!(101),
        json!("88"),
        json!(true),
        json!("NaN"),
        Value::Null,
    ] {
        let output = public_usage(json!({"rateLimits":{"primary":{"usedPercent":invalid}}}));
        assert!(window(&output[0], "primary")["usedPercent"].is_null());
    }
    for invalid in [json!(-1), json!(0), json!(1.5), json!("300"), json!(true)] {
        let output = public_usage(json!({"rateLimits":{"primary":{"windowDurationMins":invalid}}}));
        assert!(window(&output[0], "primary")["windowDurationMins"].is_null());
    }
    for invalid in [
        json!(-1),
        json!(1790812800.5),
        json!(1790812800000u64),
        json!(u64::MAX),
        json!("1790812800"),
    ] {
        let output = public_usage(json!({"rateLimits":{"primary":{"resetsAt":invalid}}}));
        assert!(window(&output[0], "primary")["resetsAt"].is_null());
    }
    let output = public_usage(
        json!({"rateLimits":{"primary":{"usedPercent":100,"resetsAt":253402300799u64}}}),
    );
    assert_eq!(window(&output[0], "primary")["usedPercent"], json!(100.0));
    assert_eq!(
        window(&output[0], "primary")["resetsAt"],
        json!(253402300799u64)
    );
}

#[test]
fn excessive_buckets_and_unsafe_labels_are_rejected_or_hidden() {
    let mut buckets = serde_json::Map::new();
    for index in 0..65 {
        buckets.insert(format!("fixture-{index}"), json!({}));
    }
    assert!(parse_usage(&json!({"rateLimitsByLimitId":buckets})).is_err());
    for id in ["x".repeat(129), "unsafe\r\nname".into(), String::new()] {
        let mut buckets = serde_json::Map::new();
        buckets.insert(id, json!({}));
        assert!(parse_usage(&json!({"rateLimitsByLimitId":buckets})).is_err());
    }
    let output = public_usage(
        json!({"rateLimits":{"limitId":"fixture","limitName":"unsafe\u{001b}name","planType":"p".repeat(65)}}),
    );
    assert!(output[0]["name"].is_null());
    assert!(output[0]["planType"].is_null());
}

// A child of the already-built test executable keeps an inert stdin open.
// It has no protocol logic, credentials, network, shell, or filesystem writes.
#[test]
fn hold_stdin_fixture() {
    if std::env::var_os("COUCOU_USAGE_TEST_HOLD_STDIN").is_some() {
        println!("COUCOU_USAGE_TEST_READY");
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn inert_server(timeout: Duration, messages: Vec<Value>) -> NativeServer {
    use std::os::windows::process::CommandExt;
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "usage::independent_tests::hold_stdin_fixture",
            "--nocapture",
        ])
        .env_clear()
        .env("COUCOU_USAGE_TEST_HOLD_STDIN", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(super::super::CREATE_NO_WINDOW)
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    // Discard the test runner banner before exercising the usage protocol.
    // The fixture emits no other bytes while its inert stdin stays open.
    let mut handshake = BufReader::new(&mut output);
    loop {
        let mut line = String::new();
        assert!(handshake.read_line(&mut line).unwrap() > 0);
        if line.trim() == "COUCOU_USAGE_TEST_READY" {
            break;
        }
    }
    drop(handshake);
    let pending = messages
        .iter()
        .map(|message| format!("{message}\n"))
        .collect::<String>()
        .into_bytes();
    NativeServer {
        child,
        input,
        output,
        total_bytes: pending.len(),
        pending,
        deadline: Instant::now() + timeout,
        messages: 0,
    }
}

#[test]
fn rpc_error_and_unexpected_server_operation_never_reflect_sensitive_payload() {
    for message in [
        json!({"id":2,"error":{"message":"synthetic-secret-marker","data":{"access_token":"synthetic-secret-marker"}}}),
        json!({"id":"request-approval","method":"item/commandExecution/requestApproval","params":{"command":"synthetic-secret-marker"}}),
    ] {
        let mut server = inert_server(Duration::from_millis(100), vec![message]);
        let error = server
            .request(2, "account/rateLimits/read", json!({}))
            .err()
            .unwrap();
        assert!(!error.contains("synthetic-secret-marker"));
        assert!(!error.contains("access_token"));
    }
}

#[test]
fn rpc_ignores_unrelated_results_but_limits_notification_floods() {
    let mut server = inert_server(
        Duration::from_millis(100),
        vec![
            json!({"id":99,"result":{"secret":"synthetic-secret-marker"}}),
            json!({"method":"account/rateLimits/updated","params":{}}),
            json!({"id":2,"result":{"rateLimits":null}}),
        ],
    );
    assert_eq!(
        server
            .request(2, "account/rateLimits/read", json!({}))
            .unwrap(),
        json!({"rateLimits":null})
    );
    let flood = (0..MAX_MESSAGES + 1)
        .map(|_| json!({"method":"fixture/notification","params":{}}))
        .collect();
    let mut server = inert_server(Duration::from_millis(100), flood);
    assert!(server
        .request(2, "account/rateLimits/read", json!({}))
        .err()
        .unwrap()
        .contains("message limit"));
}

#[test]
fn rpc_timeout_is_bounded_and_returns_no_invented_usage_result() {
    let mut server = inert_server(Duration::from_millis(25), Vec::new());
    let started = Instant::now();
    let error = server
        .request(2, "account/rateLimits/read", json!({}))
        .err()
        .unwrap();
    assert!(error.contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn native_resolver_rejects_relative_paths_scripts_and_fake_exe_wrappers() {
    let directory = std::env::temp_dir().join(format!(
        "coucou-usage-fixture-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let script = directory.join("codex.cmd");
    let fake = directory.join("codex.exe");
    std::fs::write(&script, b"@echo synthetic fixture").unwrap();
    std::fs::write(&fake, b"#!/usr/bin/env node\nsynthetic fixture").unwrap();
    assert!(native_executable(Path::new("codex.exe")).is_none());
    assert!(native_executable(&script).is_none());
    assert!(native_executable(&fake).is_none());
    let mut incomplete_pe = vec![0u8; 64];
    incomplete_pe[..2].copy_from_slice(b"MZ");
    std::fs::write(&fake, &incomplete_pe).unwrap();
    assert!(
        native_executable(&fake).is_none(),
        "MZ alone is not a native executable"
    );
    assert!(native_executable(&std::env::current_exe().unwrap()).is_some());
    std::fs::remove_file(script).unwrap();
    std::fs::remove_file(fake).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

fn cached_fixture(path: &str) -> UsageCache {
    let buckets =
        parse_usage(&json!({"rateLimits":{"limitId":"fixture","primary":{"usedPercent":42}}}))
            .unwrap();
    UsageCache(Mutex::new(Cached {
        path: path.into(),
        finished: Some(Instant::now() - MIN_REFRESH - Duration::from_millis(1)),
        snapshot: UsageSnapshot {
            status: "available".into(),
            source: SOURCE.into(),
            fetched_at: Some(1790812800),
            error: None,
            buckets,
        },
    }))
}

#[test]
fn cache_error_preserves_prior_timestamp_and_marks_stale_without_zeroing_percent() {
    let cache = cached_fixture("relative-untrusted.exe");
    let cached = cache.read("relative-untrusted.exe", false);
    assert_eq!(cached.status, "available");
    let stale = cache.read("relative-untrusted.exe", true);
    assert_eq!(stale.status, "stale");
    assert_eq!(stale.fetched_at, Some(1790812800));
    assert_eq!(stale.buckets[0].windows[0].used_percent, Some(42.0));
    assert!(stale.error.is_some());
}

#[test]
fn changing_configured_cli_discards_previous_account_usage_before_error() {
    let cache = cached_fixture("old-relative-untrusted.exe");
    let unavailable = cache.read("new-relative-untrusted.exe", false);
    assert_eq!(unavailable.status, "unavailable");
    assert!(unavailable.fetched_at.is_none());
    assert!(unavailable.buckets.is_empty());
    assert!(unavailable.error.is_some());
}

#[test]
fn immediate_forced_refresh_is_throttled_without_erasing_successful_snapshot() {
    let cache = cached_fixture("relative-untrusted.exe");
    cache.0.lock().unwrap().finished = Some(Instant::now());
    let snapshot = cache.read("relative-untrusted.exe", true);
    assert_eq!(snapshot.status, "available");
    assert!(snapshot.error.is_none());
    assert_eq!(snapshot.buckets[0].windows[0].used_percent, Some(42.0));
}

/// Explicit acceptance probe. Never runs in the ordinary test suite.
/// Uses the installed official executable or an operator-selected path; no inference,
/// account/read, credential-file access, login or token printing occurs here.
#[test]
#[ignore = "explicitly authorized read-only native Codex usage acceptance"]
fn official_native_usage_acceptance_read_only() {
    let configured = std::env::var("COUCOU_USAGE_READ_ONLY_EXE").unwrap_or_default();
    let path = resolve_executable(&configured).expect("trusted native executable required");
    let buckets = read_native(&path).expect("official read-only usage RPC must complete");
    let snapshot = UsageSnapshot {
        status: "available".into(),
        source: SOURCE.into(),
        fetched_at: Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        ),
        error: None,
        buckets,
    };
    let windows: Vec<_> = snapshot
        .buckets
        .iter()
        .flat_map(|bucket| &bucket.windows)
        .collect();
    println!(
        "USAGE_ACCEPTANCE_SUMMARY {}",
        json!({
            "source": snapshot.source,
            "metricsPresent": windows.iter().any(|window| window.used_percent.is_some()),
            "windowCount": windows.len(),
            "percentagesValid": windows.iter().all(|window| window.used_percent.is_none_or(|value| value.is_finite() && (0.0..=100.0).contains(&value))),
            "unitsValid": windows.iter().all(|window| window.window_duration_mins.is_none_or(|value| value > 0) && window.resets_at.is_none_or(|value| value <= 253402300799))
        })
    );
}
