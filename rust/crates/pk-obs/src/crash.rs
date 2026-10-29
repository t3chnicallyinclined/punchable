//! Crash reporting: `sentry` pointed at the self-hosted Bugsink, with every event and breadcrumb passed through the
//! scrubber ([`crate::scrub`]) before it leaves (the Rust home of `api-dev/crash.py`).
//!
//! `SENTRY_DSN` unset or empty means off: the process runs exactly as before and [`report`] is a no-op. The release
//! is `<service>@<version>+<sha12>`; the environment is `SENTRY_ENVIRONMENT` (default `production`). No PII, no
//! traces, 30 breadcrumbs. The Rust SDK never attaches local variables or source context, so the Python's
//! `include_local_variables=False` / `include_source_context=False` have nothing to switch off; the scrubber drops
//! `vars` and `context_line` from every frame anyway.

use std::sync::Arc;

use sentry::protocol::{Breadcrumb, Event, LogEntry, Mechanism, User, Values};
use sentry::types::Dsn;
use sentry::{ClientInitGuard, ClientOptions, Level};
use serde_json::{Map, Value, json};

use crate::scrub::{scrub_breadcrumb, scrub_event, scrub_text, sensitive_key};
use crate::sha12;

/// The environment when `SENTRY_ENVIRONMENT` is unset.
pub const DEFAULT_ENVIRONMENT: &str = "production";
/// Breadcrumbs kept per event.
pub const MAX_BREADCRUMBS: usize = 30;

/// `<service>@<version>+<sha12>`, e.g. `punchable-api@0.1.0+0123456789ab` (`sha` empty = `dev`).
pub fn release(service: &str, version: &str, sha: &str) -> String {
    format!("{service}@{version}+{}", sha12(sha))
}

/// `SENTRY_ENVIRONMENT`, or [`DEFAULT_ENVIRONMENT`].
pub fn environment() -> String {
    std::env::var("SENTRY_ENVIRONMENT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_ENVIRONMENT.to_string())
}

/// The client options `init_sentry` uses, in one place so a test can read them.
pub fn options(dsn: Dsn, release: String, environment: &str) -> ClientOptions {
    ClientOptions {
        dsn: Some(dsn),
        release: Some(release.into()),
        environment: Some(environment.to_string().into()),
        before_send: Some(Arc::new(before_send)),
        before_breadcrumb: Some(Arc::new(before_breadcrumb)),
        send_default_pii: false,
        max_breadcrumbs: MAX_BREADCRUMBS,
        traces_sample_rate: 0.0,
        attach_stacktrace: false,
        default_integrations: true,
        ..ClientOptions::default()
    }
}

/// Start crash reporting when `SENTRY_DSN` is set. `None` means off. Keep the guard alive for the process: dropping
/// it flushes and closes the client. Never fails: a DSN that does not parse is logged and reporting stays off.
pub fn init_sentry(service: &str, version: &str, sha: &str, env: &str) -> Option<ClientInitGuard> {
    let dsn = std::env::var("SENTRY_DSN").unwrap_or_default();
    init_sentry_with(&dsn, service, version, sha, env)
}

/// [`init_sentry`] with the DSN given instead of read from the environment.
pub fn init_sentry_with(dsn: &str, service: &str, version: &str, sha: &str, env: &str) -> Option<ClientInitGuard> {
    let dsn = dsn.trim();
    if dsn.is_empty() {
        return None;
    }
    let dsn: Dsn = match dsn.parse() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("crash reporting: SENTRY_DSN does not parse ({e}); off");
            return None;
        }
    };
    let rel = release(service, version, sha);
    let host = dsn.host().to_string();
    let guard = sentry::init(options(dsn, rel.clone(), env));
    tracing::info!(release = %rel, host = %host, environment = %env, "crash reporting on");
    Some(guard)
}

/// Null entries the Python scrubber writes (`threads.values[].{id,crashed,stacktrace}`) that the typed SDK event
/// cannot hold: dropped so the scrubbed JSON deserialises back into an [`Event`].
fn sdk_shape(mut v: Value) -> Value {
    if let Some(Value::Array(threads)) = v.pointer_mut("/threads/values") {
        for t in threads.iter_mut() {
            if let Value::Object(t) = t {
                t.retain(|_, x| !x.is_null());
            }
        }
    }
    v
}

fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// The typed fallback when the JSON round trip fails: the same rules applied to the SDK's structs directly.
fn bare(mut e: Event<'static>) -> Event<'static> {
    e.request = None;
    e.extra.clear();
    e.contexts.clear();
    e.modules.clear();
    e.stacktrace = None;
    e.template = None;
    e.breadcrumbs = Values::default();
    e.threads = Values::default();
    e.tags.retain(|k, _| !sensitive_key(k));
    e.user = e.user.and_then(|u| {
        u.id.map(|id| User {
            id: Some(head(&id, 64)),
            ..User::default()
        })
    });
    e.message = e.message.map(|m| scrub_text(&m));
    e.logentry = e.logentry.map(|l| LogEntry {
        message: scrub_text(&l.message),
        ..LogEntry::default()
    });
    for exc in &mut e.exception.values {
        exc.ty = head(&exc.ty, 100);
        exc.value = Some(scrub_text(exc.value.as_deref().unwrap_or("")));
        exc.raw_stacktrace = None;
        exc.mechanism = exc.mechanism.take().map(|m| Mechanism {
            ty: m.ty,
            handled: m.handled,
            ..Mechanism::default()
        });
        if let Some(st) = &mut exc.stacktrace {
            st.registers.clear();
            for f in &mut st.frames {
                f.vars.clear();
                f.context_line = None;
                f.pre_context.clear();
                f.post_context.clear();
            }
        }
    }
    e
}

/// `before_send`: the event with the scrubber's rules applied (at the JSON level, then back into the SDK's type).
/// Always returns an event: a crash is always reported, just bare.
pub fn before_send(event: Event<'static>) -> Option<Event<'static>> {
    let Ok(value) = serde_json::to_value(&event) else {
        return Some(bare(event));
    };
    let scrubbed = sdk_shape(scrub_event(&value));
    Some(serde_json::from_value::<Event<'static>>(scrubbed).unwrap_or_else(|_| bare(event)))
}

fn bare_breadcrumb(mut bc: Breadcrumb) -> Breadcrumb {
    bc.message = None;
    bc.data.retain(|k, v| {
        matches!(
            k.as_str(),
            "route" | "status" | "status_code" | "method" | "reason_class" | "duration_ms"
        ) && matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_))
    });
    bc
}

/// `before_breadcrumb`: the breadcrumb with the scrubber's rules applied; `None` drops a breadcrumb that kept
/// nothing.
pub fn before_breadcrumb(bc: Breadcrumb) -> Option<Breadcrumb> {
    let Ok(value) = serde_json::to_value(&bc) else {
        return Some(bare_breadcrumb(bc));
    };
    let scrubbed = scrub_breadcrumb(&value);
    if scrubbed.as_object().is_some_and(Map::is_empty) {
        return None;
    }
    Some(serde_json::from_value::<Breadcrumb>(scrubbed).unwrap_or_else(|_| bare_breadcrumb(bc)))
}

/// One handled error (the API's 500 paths; a worker job that failed). `route` is a route pattern, never the raw
/// path; `tags` are short closed-set values (the worker's job kind, whether it was retried), never an id or text:
/// a tag whose name sounds private is dropped anyway. A no-op without a client.
pub fn report(
    err: &(dyn std::error::Error + 'static),
    route: Option<&str>,
    status: Option<u16>,
    tags: &[(&str, &str)],
) -> sentry::types::Uuid {
    sentry::with_scope(
        |scope| {
            if let Some(r) = route {
                scope.set_tag("route", r);
            }
            if let Some(s) = status {
                scope.set_tag("status", s);
            }
            for (k, v) in tags {
                if !sensitive_key(k) {
                    scope.set_tag(k, head(v, 64));
                }
            }
        },
        || sentry::capture_error(err),
    )
}

/// One request as a breadcrumb: route pattern, method and status, nothing else.
pub fn breadcrumb(route: &str, method: &str, status: u16) {
    sentry::add_breadcrumb(Breadcrumb {
        category: Some("http".to_string()),
        level: if status >= 500 { Level::Warning } else { Level::Info },
        data: [
            ("route", json!(route)),
            ("method", json!(method)),
            ("status", json!(status)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
        ..Breadcrumb::default()
    });
}
