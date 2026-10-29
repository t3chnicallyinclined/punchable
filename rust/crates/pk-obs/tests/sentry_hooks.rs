//! The `sentry` hooks on the SDK's own types: a typed event with everything private in it goes through `before_send`
//! and comes out bare; a breadcrumb keeps its route and status and nothing else; the init helpers are off without a
//! DSN; the options carry the Python's settings; `tracing` installs once.

use std::time::SystemTime;

use pk_obs::crash::{
    DEFAULT_ENVIRONMENT, MAX_BREADCRUMBS, before_breadcrumb, before_send, init_sentry_with, options, release,
};
use pk_obs::scrub::REDACTED;
use pk_obs::{init_tracing, sha12};
use sentry::Level;
use sentry::protocol::{
    Breadcrumb, Context, Event, Exception, Frame, Request, RuntimeContext, Stacktrace, Thread, User, Values,
};
use serde_json::{Value, json};

const PRIVATE: &str = "sample bit text about pigeons";

fn typed_event() -> Event<'static> {
    Event {
        message: Some(format!("boom: '{PRIVATE}' and more")),
        request: Some(Request {
            url: Some(
                format!("https://example.test/api/decision?q={}", PRIVATE.replace(' ', "+"))
                    .parse()
                    .unwrap(),
            ),
            method: Some("POST".into()),
            data: Some(format!("{{\"text\": \"{PRIVATE}\"}}")),
            query_string: Some(format!("q={PRIVATE}")),
            cookies: Some("session=abc".into()),
            headers: [
                ("Authorization", "Bearer secret.sig"),
                ("Cookie", "session=abc"),
                ("Content-Type", "application/json"),
                ("X-Title", PRIVATE),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
            env: [("REMOTE_ADDR".to_string(), "203.0.113.9".to_string())]
                .into_iter()
                .collect(),
        }),
        exception: Values::from(vec![Exception {
            ty: "SurrealError".into(),
            value: Some(format!(
                "statement 0 failed: \"{PRIVATE}\"\n--- SELECT * FROM unit WHERE text @@ 'x'"
            )),
            stacktrace: Some(Stacktrace {
                frames: vec![Frame {
                    function: Some("do_post".into()),
                    filename: Some("server.rs".into()),
                    lineno: Some(1),
                    in_app: Some(true),
                    context_line: Some(PRIVATE.into()),
                    pre_context: vec![PRIVATE.into()],
                    vars: [("text".to_string(), json!(PRIVATE))].into_iter().collect(),
                    ..Frame::default()
                }],
                ..Stacktrace::default()
            }),
            ..Exception::default()
        }]),
        breadcrumbs: Values::from(vec![Breadcrumb {
            category: Some("http".into()),
            message: Some(PRIVATE.into()),
            data: [
                ("route", json!("/api/joke/{id}")),
                ("status", json!(500)),
                ("body", json!(PRIVATE)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
            ..Breadcrumb::default()
        }]),
        tags: [("route", "/api/decision"), ("transcript", PRIVATE)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        extra: [
            ("nested", json!({"deep": {"quote": PRIVATE, "n": 3}})),
            ("QUERY_STRING", json!(PRIVATE)),
            ("ok", json!("fine")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
        user: Some(User {
            id: Some("owner".into()),
            email: Some("owner@example.test".into()),
            username: Some("x".into()),
            ..User::default()
        }),
        contexts: [(
            "runtime".to_string(),
            Context::Runtime(Box::new(RuntimeContext {
                name: Some("rust".into()),
                version: Some("1.93".into()),
                ..RuntimeContext::default()
            })),
        )]
        .into_iter()
        .collect(),
        threads: Values::from(vec![Thread {
            id: Some(1i64.into()),
            crashed: true,
            stacktrace: Some(Stacktrace {
                frames: vec![Frame {
                    function: Some("f".into()),
                    vars: [("text".to_string(), json!(PRIVATE))].into_iter().collect(),
                    ..Frame::default()
                }],
                ..Stacktrace::default()
            }),
            ..Thread::default()
        }]),
        release: Some("punchable-api@0.1.0+abc".into()),
        environment: Some("production".into()),
        ..Event::default()
    }
}

#[test]
fn before_send_strips_everything_private_from_a_typed_event() {
    let out = before_send(typed_event()).expect("a crash is always reported");
    let v: Value = serde_json::to_value(&out).unwrap();
    let blob = v.to_string();
    for word in [
        PRIVATE,
        "owner@example.test",
        "203.0.113.9",
        "session=abc",
        "secret.sig",
        "pigeons",
    ] {
        assert!(!blob.contains(word), "{word} leaked: {blob}");
    }
    assert_eq!(v["request"]["url"], json!("https://example.test/api/decision"));
    assert_eq!(v["request"]["method"], json!("POST"));
    assert_eq!(v["request"]["headers"], json!({"Content-Type": "application/json"}));
    assert!(
        v["request"].get("data").is_none() && v["request"].get("cookies").is_none(),
        "{blob}"
    );
    assert_eq!(v["message"], json!("boom: '...' and more"));
    assert_eq!(v["exception"]["values"][0]["type"], json!("SurrealError"));
    assert_eq!(v["exception"]["values"][0]["value"], json!("statement 0 failed: '...'"));
    assert_eq!(
        v["exception"]["values"][0]["stacktrace"]["frames"][0],
        json!({"filename": "server.rs", "function": "do_post", "lineno": 1, "in_app": true})
    );
    assert_eq!(
        v["breadcrumbs"]["values"][0]["data"],
        json!({"route": "/api/joke/{id}", "status": 500})
    );
    assert!(v["breadcrumbs"]["values"][0].get("message").is_none());
    assert_eq!(v["tags"], json!({"route": "/api/decision"}));
    assert_eq!(
        v["extra"],
        json!({"nested": {"deep": {"quote": REDACTED, "n": 3}}, "QUERY_STRING": REDACTED, "ok": "fine"})
    );
    assert_eq!(v["user"], json!({"id": "owner"}));
    assert_eq!(
        v["contexts"]["runtime"],
        json!({"type": "runtime", "name": REDACTED, "version": "1.93"})
    );
    assert_eq!(
        v["threads"]["values"][0]["stacktrace"]["frames"],
        json!([{"function": "f"}])
    );
    assert_eq!(v["threads"]["values"][0]["crashed"], json!(true));
    assert_eq!(v["release"], json!("punchable-api@0.1.0+abc"));
    assert_eq!(out.event_id, typed_event_id_is_kept(&out));
}

fn typed_event_id_is_kept(out: &Event<'static>) -> sentry::types::Uuid {
    // the event id survives the round trip (serialised as a string, parsed back)
    let v: Value = serde_json::to_value(out).unwrap();
    v["event_id"].as_str().unwrap().parse().unwrap()
}

#[test]
fn before_send_keeps_a_bare_event_bare() {
    let e = Event {
        message: Some("plain".into()),
        ..Event::default()
    };
    let out = before_send(e.clone()).unwrap();
    assert_eq!(out.message.as_deref(), Some("plain"));
    assert_eq!(out.event_id, e.event_id);
    assert_eq!(out.level, e.level);
}

#[test]
fn before_breadcrumb_keeps_route_and_status_only() {
    let bc = Breadcrumb {
        ty: "http".into(),
        category: Some("http".into()),
        level: Level::Warning,
        message: Some(PRIVATE.into()),
        timestamp: SystemTime::UNIX_EPOCH,
        data: [
            ("route", json!("/api/x")),
            ("status", json!(500)),
            ("body", json!(PRIVATE)),
            ("query", json!("q")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
    };
    let out = before_breadcrumb(bc).expect("kept");
    assert_eq!(out.message, None);
    assert_eq!(out.category.as_deref(), Some("http"));
    assert_eq!(out.level, Level::Warning);
    assert_eq!(out.ty, "http");
    assert_eq!(
        serde_json::to_value(&out.data).unwrap(),
        json!({"route": "/api/x", "status": 500})
    );
}

#[test]
fn init_is_off_without_a_dsn_and_on_a_bad_one() {
    assert!(init_sentry_with("", "punchable-api", "0.1.0", "abc", "production").is_none());
    assert!(init_sentry_with("   ", "punchable-api", "0.1.0", "abc", "production").is_none());
    assert!(init_sentry_with("not a dsn", "punchable-api", "0.1.0", "abc", "production").is_none());
}

#[test]
fn release_and_options_carry_the_python_settings() {
    assert_eq!(
        release("punchable-api", "0.1.0", "0123456789abcdef0123"),
        "punchable-api@0.1.0+0123456789ab"
    );
    assert_eq!(release("punchable-worker", "0.1.0", ""), "punchable-worker@0.1.0+dev");
    assert_eq!(sha12("abc"), "abc");
    assert_eq!(DEFAULT_ENVIRONMENT, "production");
    let dsn: sentry::types::Dsn = "https://public@sentry.example.test/1".parse().unwrap();
    let o = options(dsn, release("punchable-api", "0.1.0", "abc"), "preview");
    assert_eq!(o.release.as_deref(), Some("punchable-api@0.1.0+abc"));
    assert_eq!(o.environment.as_deref(), Some("preview"));
    assert!(!o.send_default_pii);
    assert_eq!(o.max_breadcrumbs, MAX_BREADCRUMBS);
    assert_eq!(MAX_BREADCRUMBS, 30);
    assert!(o.traces_sample_rate.abs() < f32::EPSILON);
    assert!(!o.attach_stacktrace);
    assert!(o.default_integrations);
    assert!(o.before_send.is_some() && o.before_breadcrumb.is_some());
    // the hook installed is the scrubber
    let hook = o.before_send.unwrap();
    let out = hook(typed_event()).unwrap();
    assert!(!serde_json::to_string(&out).unwrap().contains(PRIVATE));
}

#[test]
fn tracing_installs_once() {
    let first = init_tracing();
    let second = init_tracing();
    assert!(first.is_ok() || second.is_err());
    assert!(second.is_err(), "a second install must be refused");
    tracing::info!("a line through the installed subscriber");
}
