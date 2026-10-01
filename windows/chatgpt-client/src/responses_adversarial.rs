//! Independent SSE parser tests. No HTTP request is sent by this module.
use super::*;
use std::io::Cursor;

fn event(value: Value) -> String {
    format!(
        "event: {}\ndata: {value}\n\n",
        value["type"].as_str().unwrap_or("message")
    )
}
fn completed(text: &str) -> Value {
    json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}]}})
}
fn delta(text: &str) -> Value {
    json!({"type":"response.output_text.delta","delta":text})
}
fn parse(value: String) -> Result<String, String> {
    parse_sse(Cursor::new(value.into_bytes()))
}

#[test]
fn terminal_completed_is_required_and_provides_authoritative_text() {
    let stream = event(delta("partial text")) + &event(completed("confirmed text"));
    assert_eq!(parse(stream).unwrap(), "confirmed text");
    assert!(parse(event(delta("partial text"))).is_err());
    assert!(parse(event(delta("partial text")) + "data: [DONE]\n\n").is_err());
    let no_separator = format!("data: {}", completed("missing event boundary"));
    assert!(parse(no_separator).is_err());
}

#[test]
fn failure_after_partial_text_never_returns_partial_success_or_raw_error() {
    for kind in ["response.failed", "response.incomplete", "error"] {
        let failed = json!({"type":kind,"response":{"status":"failed","error":{"code":"unknown-code","message":"synthetic-secret-marker"}},"error":{"message":"synthetic-secret-marker"}});
        let error = parse(event(delta("unconfirmed")) + &event(failed))
            .err()
            .unwrap();
        assert!(!error.contains("synthetic-secret-marker"));
        assert!(!error.contains("unconfirmed"));
    }
}

#[test]
fn completed_with_wrong_status_or_no_text_is_rejected() {
    let mut failed = completed("text");
    failed["response"]["status"] = json!("failed");
    assert!(parse(event(failed)).is_err());
    assert!(parse(event(completed("  \n\t"))).is_err());
    assert!(parse(event(json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"function_call","name":"exfiltrate","arguments":"synthetic-secret-marker"}]}}))).is_err());
}

#[test]
fn invalid_json_utf8_and_interrupted_stream_are_rejected_without_reflection() {
    let error = parse("data: {synthetic-secret-marker}\n\n".into())
        .err()
        .unwrap();
    assert!(!error.contains("synthetic-secret-marker"));
    assert!(parse_sse(Cursor::new(vec![b'd', b'a', b't', b'a', b':', 0xff, b'\n'])).is_err());
    struct Broken;
    impl std::io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("synthetic-secret-marker"))
        }
    }
    let error = parse_sse(BufReader::new(Broken)).err().unwrap();
    assert!(!error.contains("synthetic-secret-marker"));
}

#[test]
fn sse_supports_crlf_comments_and_multiline_data_without_accepting_unknown_terminal() {
    let completed = serde_json::to_string_pretty(&completed("confirmed")).unwrap();
    let data = completed
        .lines()
        .map(|line| format!("data: {line}\r\n"))
        .collect::<String>();
    assert_eq!(
        parse(format!(
            ": heartbeat\r\nevent: response.completed\r\n{data}\r\n"
        ))
        .unwrap(),
        "confirmed"
    );
    assert!(parse(event(
        json!({"type":"response.done","response":{"status":"completed","text":"unconfirmed"}})
    ))
    .is_err());
}

#[test]
fn sse_reply_event_line_and_total_stream_limits_are_enforced() {
    assert!(parse(event(completed(&"x".repeat(MAX_REPLY + 1)))).is_err());
    assert!(parse(format!("data: {}\n\n", "x".repeat(MAX_EVENT + 1))).is_err());
    let multiline = "data: ".to_string() + &" ".repeat(MAX_EVENT / 2) + "\n";
    assert!(parse(multiline.repeat(3) + "\n").is_err());
    assert!(parse((":".to_string() + &" ".repeat(1_000_000) + "\n").repeat(17)).is_err());
}

#[test]
fn public_http_errors_do_not_reflect_credentials_or_trigger_backend_switch() {
    for status in [302, 400, 401, 403, 404, 429, 500, 503] {
        let error = http_error(status);
        assert!(!error.contains("Bearer"));
        assert!(!error.contains("API key"));
        assert!(!error.contains("synthetic-secret-marker"));
    }
}

#[test]
fn broker_deltas_are_returned_only_after_valid_completed_without_final_text() {
    for terminal in [
        json!({"type":"response.completed","response":{"status":"completed"}}),
        json!({"type":"response.completed","response":{"status":"completed","output":[]}}),
    ] {
        let stream = event(delta("confirmed ")) + &event(delta("text 🦀")) + &event(terminal);
        assert_eq!(parse(stream).unwrap(), "confirmed text 🦀");
    }
    let stream = event(delta("older text")) + &event(completed("authoritative final"));
    assert_eq!(parse(stream).unwrap(), "authoritative final");
}

#[test]
fn broker_partial_text_is_rejected_with_missing_failed_or_incomplete_terminal_status() {
    for status in [
        json!("failed"),
        json!("incomplete"),
        json!("in_progress"),
        Value::Null,
    ] {
        let terminal =
            json!({"type":"response.completed","response":{"status":status,"output":[]}});
        assert!(parse(event(delta("unconfirmed text")) + &event(terminal)).is_err());
    }
    let missing = json!({"type":"response.completed","response":{"output":[]}});
    assert!(parse(event(delta("unconfirmed text")) + &event(missing)).is_err());
    assert!(parse(event(delta("unconfirmed text"))).is_err());
    for kind in ["response.failed", "response.incomplete", "error"] {
        let terminal = json!({"type":kind,"response":{"status":"failed","output":[]}});
        assert!(parse(event(delta("unconfirmed text")) + &event(terminal)).is_err());
    }
    for response in [
        json!({"status":"completed","error":{"message":"synthetic-secret-marker"},"output":[]}),
        json!({"status":"completed","output":{}}),
        json!({"status":"completed","output":[{"type":"function_call","name":"exfiltrate","arguments":"synthetic-secret-marker"}]}),
        json!({"status":"completed","output":[{"type":"message","content":[{"type":"refusal","refusal":"synthetic-secret-marker"}]}]}),
        json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":false}]}]}),
    ] {
        let terminal = json!({"type":"response.completed","response":response});
        let error = parse(event(delta("unconfirmed text")) + &event(terminal))
            .err()
            .unwrap();
        assert!(!error.contains("synthetic-secret-marker"));
        assert!(!error.contains("unconfirmed text"));
    }
    let invalid_delta = json!({"type":"response.output_text.delta","delta":{"text":"unconfirmed"}});
    assert!(parse(event(invalid_delta) + &event(completed("authoritative"))).is_err());
}

#[test]
fn broker_aggregated_deltas_are_bounded_and_cannot_hide_oversized_final_text() {
    let terminal =
        json!({"type":"response.completed","response":{"status":"completed","output":[]}});
    let half = "x".repeat(MAX_REPLY / 2);
    let exact = event(delta(&half)) + &event(delta(&half)) + &event(terminal.clone());
    assert_eq!(parse(exact).unwrap().len(), MAX_REPLY);
    let overflow =
        event(delta(&half)) + &event(delta(&half)) + &event(delta("x")) + &event(terminal.clone());
    assert!(parse(overflow).is_err());
    let unicode = "🦀".repeat(MAX_REPLY / 4);
    assert!(parse(event(delta(&unicode)) + &event(delta("🦀")) + &event(terminal)).is_err());
    assert!(
        parse(event(delta("small partial")) + &event(completed(&"x".repeat(MAX_REPLY + 1))))
            .is_err()
    );
}

#[test]
fn content_type_classification_allows_missing_header_and_sanitizes_unknown_values() {
    use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
    let mut headers = HeaderMap::new();
    assert_eq!(content_type(&headers), "none");
    for value in [
        "text/event-stream",
        "Text/Event-Stream; charset=utf-8",
        " text/event-stream ; anything=synthetic-secret-marker",
    ] {
        headers.insert(CONTENT_TYPE, HeaderValue::from_str(value).unwrap());
        assert_eq!(content_type(&headers), "text/event-stream");
    }
    for (value, classification) in [
        ("Application/Json; charset=utf-8", "application/json"),
        ("TEXT/HTML", "text/html"),
        ("synthetic-secret-marker", "other"),
        ("", "other"),
    ] {
        headers.insert(CONTENT_TYPE, HeaderValue::from_str(value).unwrap());
        let public = content_type(&headers);
        assert_eq!(public, classification);
        assert!(!public.contains("synthetic-secret-marker"));
    }
    headers.insert(CONTENT_TYPE, HeaderValue::from_bytes(&[0xff]).unwrap());
    assert_eq!(content_type(&headers), "other");
}
