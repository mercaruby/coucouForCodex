//! Independent error-policy fixtures. No HTTP, OAuth or provider requests.
use super::*;
use std::io::Cursor;

const SECRET: &str = "synthetic-secret-marker";
fn body_error(status: u16, value: Value) -> String {
    error_from_reader(
        status,
        "application/json",
        Cursor::new(value.to_string().into_bytes()),
    )
}
fn assert_public(message: &str) {
    assert!(!message.contains(SECRET));
    assert!(!message.contains("Bearer"));
    assert!(!message.contains("refresh_token"));
    assert!(!message.contains("API key"));
}
fn assert_no_reconnect(message: &str) {
    assert!(!message.to_ascii_lowercase().contains("reconnect"));
}

#[test]
fn sharing_limit_body_overrides_generic_403_without_reauthorization_or_percentage() {
    for value in [
        json!({"error":{"code":"subscription_sharing_usage_limit_exceeded","message":SECRET}}),
        json!({"response":{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":SECRET}}}),
    ] {
        let message = body_error(403, value);
        assert!(
            message.starts_with("OpenAI reported a ChatGPT sharing limit for this app or account.")
        );
        assert!(!message.contains('%'));
        assert_public(&message);
        assert_no_reconnect(&message);
    }
}

#[test]
fn usage_check_unavailable_does_not_invalidate_authorization() {
    let message = body_error(
        503,
        json!({"error":{"code":"subscription_sharing_usage_unavailable","detail":SECRET}}),
    );
    assert!(message.starts_with("ChatGPT usage could not be checked right now."));
    assert!(message.contains("keep your current connection"));
    assert_no_reconnect(&message);
    assert_public(&message);
}

#[test]
fn unknown_403_is_restriction_not_evidence_of_expired_login() {
    for value in [
        json!({}),
        json!({"error":{"code":"unknown-provider-code","message":format!("authorization permission {SECRET}"),"refresh_token":SECRET}}),
    ] {
        let message = body_error(403, value);
        assert!(message
            .starts_with("A ChatGPT policy or permission restriction prevented this request."));
        assert_no_reconnect(&message);
        assert_public(&message);
    }
    assert!(body_error(401, json!({})).contains("granted permissions"));
}

#[test]
fn untrusted_message_strings_metadata_and_invalid_code_types_do_not_fabricate_known_codes() {
    for value in [
        json!({"error":{"message":"subscription_sharing_usage_limit_exceeded"}}),
        json!({"metadata":{"code":"subscription_sharing_usage_limit_exceeded"}}),
        json!({"error":{"code":{"value":"subscription_sharing_usage_limit_exceeded"}}}),
        json!({"error":{"code":"SUBSCRIPTION_SHARING_USAGE_LIMIT_EXCEEDED"}}),
    ] {
        let message = body_error(403, value);
        assert!(!message.starts_with("OpenAI reported"));
        assert_no_reconnect(&message);
    }
}

#[test]
fn mime_diagnostics_are_allowlisted_without_reflecting_raw_header_values() {
    let payload =
        json!({"error":{"code":"subscription_sharing_usage_limit_exceeded","message":SECRET}})
            .to_string();
    let message = error_from_reader(403, SECRET, Cursor::new(payload));
    assert!(message.starts_with("OpenAI reported"));
    assert!(message.contains("format: other"));
    assert!(message.contains("code: subscription_sharing_usage_limit_exceeded"));
    assert_no_reconnect(&message);
    assert_public(&message);
}

#[test]
fn error_body_cap_accepts_exact_limit_but_oversized_payload_has_no_semantic_effect() {
    let payload = json!({"error":{"code":"subscription_sharing_usage_limit_exceeded"}}).to_string();
    let exact = format!("{}{}", payload, " ".repeat(65536 - payload.len()));
    let message = error_from_reader(403, "application/json", Cursor::new(exact.as_bytes()));
    assert!(message.starts_with("OpenAI reported"));
    let overflow = exact + " ";
    let message = error_from_reader(403, "application/json", Cursor::new(overflow));
    assert!(!message.starts_with("OpenAI reported"));
    assert_no_reconnect(&message);
    assert_public(&message);
}

#[test]
fn malformed_utf8_and_io_errors_are_redacted_and_do_not_suggest_reconnect_for_403() {
    for bytes in [format!("{{{SECRET}").into_bytes(), vec![0xff, 0xfe]] {
        let message = error_from_reader(403, "application/json", Cursor::new(bytes));
        assert_public(&message);
        assert_no_reconnect(&message);
    }
    struct Broken;
    impl std::io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other(SECRET))
        }
    }
    let message = error_from_reader(403, "application/json", Broken);
    assert_public(&message);
    assert_no_reconnect(&message);
}

#[test]
fn streamed_provider_limit_never_becomes_partial_success_or_reconnect() {
    for kind in ["response.failed", "response.incomplete", "error"] {
        let partial =
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"unconfirmed partial\"}\n\n";
        let failure = json!({"type":kind,"response":{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":SECRET}}});
        let stream = format!("{partial}data: {failure}\n\n");
        let message = parse_sse(Cursor::new(stream)).unwrap_err();
        assert!(
            message.starts_with("OpenAI reported a ChatGPT sharing limit for this app or account.")
        );
        assert!(!message.contains("unconfirmed partial"));
        assert_public(&message);
        assert_no_reconnect(&message);
    }
}

#[test]
fn nested_error_extraction_and_json_parser_depth_are_bounded() {
    let leaf = json!({"code":"subscription_sharing_usage_limit_exceeded","message":SECRET});
    let mut bounded = leaf.clone();
    for _ in 0..4 {
        bounded = json!({"error":bounded});
    }
    assert!(body_error(403, bounded.clone()).starts_with("OpenAI reported"));
    let excessive = json!({"error":bounded});
    let message = body_error(403, excessive);
    assert!(!message.starts_with("OpenAI reported"));
    assert_public(&message);
    let mut deep = leaf;
    for _ in 0..129 {
        deep = json!({"detail":deep});
    }
    let message = body_error(403, deep);
    assert!(!message.starts_with("OpenAI reported"));
    assert_public(&message);
    assert_no_reconnect(&message);
}

#[test]
fn error_reader_consumes_at_most_cap_plus_one_byte() {
    struct Endless(std::rc::Rc<std::cell::Cell<usize>>);
    impl std::io::Read for Endless {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            buffer.fill(b' ');
            self.0.set(self.0.get() + buffer.len());
            Ok(buffer.len())
        }
    }
    let consumed = std::rc::Rc::new(std::cell::Cell::new(0));
    let message = error_from_reader(403, "application/json", Endless(consumed.clone()));
    assert_eq!(consumed.get(), 65537);
    assert_public(&message);
    assert_no_reconnect(&message);
}

#[test]
fn http_429_uses_explicit_provider_quota_code_without_inventing_quota_for_unknown_code() {
    let known = body_error(
        429,
        json!({"error":{"code":"subscription_sharing_usage_limit_exceeded","message":SECRET}}),
    );
    assert!(known.starts_with("OpenAI reported a ChatGPT sharing limit"));
    assert!(known.contains("HTTP 429"));
    let unknown = body_error(429, json!({"error":{"code":"unknown","message":SECRET}}));
    assert!(unknown.starts_with("OpenAI reported a rate limit for this request."));
    assert!(!unknown.contains("sharing limit"));
    assert!(unknown.contains("code: unknown"));
    for message in [known, unknown] {
        assert_public(&message);
        assert_no_reconnect(&message);
    }
}

#[test]
fn completed_event_with_provider_error_never_returns_accumulated_text() {
    let event = json!({"type":"response.completed","response":{"status":"completed","output":[],"error":{"code":"subscription_sharing_usage_limit_exceeded","message":SECRET}}});
    let stream = format!("data: {{\"type\":\"response.output_text.delta\",\"delta\":\"unconfirmed partial\"}}\n\ndata: {event}\n\n");
    let message = parse_sse(Cursor::new(stream)).unwrap_err();
    assert!(message.starts_with("OpenAI reported a ChatGPT sharing limit"));
    assert!(!message.contains("unconfirmed partial"));
    assert_public(&message);
    assert_no_reconnect(&message);
}
