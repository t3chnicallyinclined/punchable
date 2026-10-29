//! The scrubber by property: whatever the shape of an event, no sensitive key (listed or matching the hint) keeps
//! a value through `scrub_value`, and a private string placed under sensitive keys never reaches the output of
//! `scrub_event`. The red arm proves the check is load-bearing: with `text` taken off the rule, or with the scrubber
//! bypassed, the same string leaks and the same assertion fails.

use pk_obs::scrub::{REDACTED, SENSITIVE_KEYS, scrub_event, scrub_value, scrub_value_by, sensitive_key};
use proptest::prelude::*;
use serde_json::{Map, Value, json};

/// The string that must never come out: it stands for his words.
const SENTINEL: &str = "PRIVATE-WORDS-7f3a";

/// Keys the hint regex catches although they are not listed.
const HINTED: [&str; 8] = [
    "joke_text",
    "search_query",
    "x-title",
    "HTTP_AUTHORIZATION",
    "Set-Cookie",
    "api_token",
    "PasswordHash",
    "authz",
];
/// Keys that must survive untouched.
const SAFE: [&str; 10] = [
    "route", "status", "method", "lineno", "function", "kind", "n", "ok", "release", "level",
];

fn key() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::sample::select(SENSITIVE_KEYS.to_vec()).prop_map(str::to_string),
        1 => prop::sample::select(HINTED.to_vec()).prop_map(str::to_string),
        4 => prop::sample::select(SAFE.to_vec()).prop_map(str::to_string),
        1 => "[a-z_]{1,12}",
    ]
}

fn leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        "[a-z ]{0,20}".prop_map(Value::String),
        Just(Value::String(SENTINEL.to_string())),
    ]
}

fn value() -> impl Strategy<Value = Value> {
    leaf().prop_recursive(5, 64, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec((key(), inner), 0..6)
                .prop_map(|pairs| Value::Object(pairs.into_iter().collect::<Map<String, Value>>())),
        ]
    })
}

/// Every value under a sensitive key becomes the sentinel; returns whether there was one.
fn poison(v: &mut Value) -> bool {
    match v {
        Value::Object(o) => {
            let mut any = false;
            for (k, x) in o.iter_mut() {
                if sensitive_key(k) {
                    *x = Value::String(SENTINEL.to_string());
                    any = true;
                } else {
                    any |= poison(x);
                }
            }
            any
        }
        Value::Array(a) => a.iter_mut().fold(false, |acc, x| poison(x) | acc),
        _ => false,
    }
}

/// Every object key at any depth that is sensitive maps to exactly `REDACTED`.
fn sensitive_values_redacted(v: &Value) -> bool {
    match v {
        Value::Object(o) => o.iter().all(|(k, x)| {
            if sensitive_key(k) {
                x == &Value::String(REDACTED.to_string())
            } else {
                sensitive_values_redacted(x)
            }
        }),
        Value::Array(a) => a.iter().all(sensitive_values_redacted),
        _ => true,
    }
}

/// The sentinel wrapped in every place an event carries free-form data.
fn event_around(payload: &Value) -> Value {
    json!({
        "event_id": "e", "level": "error", "message": format!("boom '{SENTINEL}' tail"),
        "request": {"method": "POST", "url": format!("https://x/api?q={SENTINEL}"), "data": payload,
                    "headers": {"Authorization": SENTINEL, "Content-Type": "application/json", "X-Title": SENTINEL},
                    "cookies": {"s": SENTINEL}, "query_string": SENTINEL, "env": payload},
        "exception": {"values": [{"type": "E", "value": format!("failed: \"{SENTINEL}\"\n{SENTINEL}"),
                                  "stacktrace": {"frames": [{"function": "f", "lineno": 1, "vars": payload,
                                                             "context_line": SENTINEL, "pre_context": [SENTINEL]}]},
                                  "mechanism": {"type": "generic", "data": payload}}]},
        "breadcrumbs": {"values": [{"category": "http", "message": SENTINEL, "data": payload},
                                   {"category": "http", "data": {"route": "/api/x", "status": 500, "body": SENTINEL}}]},
        "tags": {"route": "/api/x", "transcript": SENTINEL, "nested": payload},
        "extra": payload,
        "user": {"id": "owner", "email": SENTINEL, "username": SENTINEL, "ip_address": SENTINEL},
        "contexts": {"runtime": payload, "os": {"name": SENTINEL}, "custom": payload},
        "threads": {"values": [{"id": 1, "crashed": true, "stacktrace": {"frames": [{"function": "g", "vars": payload}]}}]},
        "modules": payload, "fingerprint": [SENTINEL], "culprit": SENTINEL,
    })
}

fn leaks(v: &Value) -> bool {
    serde_json::to_string(v).expect("json").contains(SENTINEL)
}

proptest! {
    #[test]
    fn scrub_value_redacts_every_sensitive_key_at_any_depth(v in value()) {
        prop_assert!(sensitive_values_redacted(&scrub_value(&v)));
    }

    #[test]
    fn scrub_value_never_lets_a_poisoned_value_out(mut v in value()) {
        let had = poison(&mut v);
        let out = scrub_value(&v);
        prop_assert!(sensitive_values_redacted(&out));
        // red arm, inside the property: an input that carried the sentinel under a sensitive key leaks before
        // scrubbing, so the assertion above is doing work
        if had {
            prop_assert!(leaks(&v));
        }
        // tags and extra go through scrub_value in scrub_event: the sentinel under a sensitive key is gone
        let ev = scrub_event(&json!({"extra": v, "tags": {"route": "/x", "payload": v.clone()}}));
        prop_assert!(sensitive_values_redacted(&ev["extra"]));
        prop_assert_eq!(&ev["tags"], &json!({"route": "/x"}));
    }

    #[test]
    fn scrub_event_never_lets_the_sentinel_out_whatever_the_payload(mut payload in value()) {
        poison(&mut payload);
        // the payload may still carry the sentinel under a safe key (a leaf chosen at random): that is allowed
        // to survive under `extra`, so strip those before the whole-event check
        let payload = strip_sentinel_under_safe_keys(&payload);
        let ev = event_around(&payload);
        prop_assert!(leaks(&ev), "the input carries the sentinel");
        let out = scrub_event(&ev);
        prop_assert!(!leaks(&out), "leaked: {out}");
        prop_assert_eq!(&out["request"], &json!({"method": "POST", "url": "https://x/api", "headers": {"Content-Type": "application/json"}}));
        prop_assert_eq!(&out["user"], &json!({"id": "owner"}));
        prop_assert_eq!(&out["tags"]["route"], &json!("/api/x"));
        prop_assert!(out["tags"].get("transcript").is_none(), "a sensitive tag is dropped whole");
        prop_assert!(sensitive_values_redacted(&out["tags"]["nested"]));
        prop_assert_eq!(&out["exception"]["values"][0]["value"], &json!("failed: '...'"));
    }
}

/// The sentinel as a free leaf (not under a sensitive key) is legitimate data (`extra.ok = "..."` could be any
/// string); replace those leaves so the whole-event check is about the rule, not the random leaf.
fn strip_sentinel_under_safe_keys(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| {
                    // under a sensitive key the sentinel stays: that is the poison the scrubber must remove
                    let x = if sensitive_key(k) {
                        x.clone()
                    } else {
                        strip_sentinel_under_safe_keys(x)
                    };
                    (k.clone(), x)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(strip_sentinel_under_safe_keys).collect()),
        Value::String(s) if s == SENTINEL => Value::String("plain".to_string()),
        other => other.clone(),
    }
}

#[test]
fn red_arm_bypassing_the_scrubber_leaks() {
    let ev = event_around(&json!({"job": {"kind": "riff", "text": SENTINEL}, "nested": {"deep": {"quote": SENTINEL}}}));
    assert!(leaks(&ev), "the raw event carries his words");
    assert!(!leaks(&scrub_event(&ev)), "the scrubbed event does not");
    // the same check the Python keeps as a permanent test: with `text` taken off the rule, his words leak
    let without_text = |k: &str| k != "text" && sensitive_key(k);
    assert!(!without_text("text") && without_text("quote"));
    let out = scrub_value_by(&ev["extra"], &without_text);
    assert!(leaks(&out), "with `text` off the rule the words leak: {out}");
    assert!(!leaks(&scrub_value(&ev["extra"])), "with the real rule they do not");
    // and an identity "scrubber" leaks everything
    let identity = |_: &str| false;
    assert!(leaks(&scrub_value_by(&ev, &identity)));
}

#[test]
fn red_arm_the_property_check_rejects_a_scrubber_that_misses_a_key() {
    let v = json!({"ok": 1, "deep": [{"transcript": SENTINEL, "route": "/x"}]});
    assert!(sensitive_values_redacted(&scrub_value(&v)));
    let missed = scrub_value_by(&v, &|k| k != "transcript" && sensitive_key(k));
    assert!(
        !sensitive_values_redacted(&missed),
        "the property must fail on a miss: {missed}"
    );
}
