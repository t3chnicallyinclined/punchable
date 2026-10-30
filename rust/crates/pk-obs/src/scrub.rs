//! The crash-report scrubber: what a Sentry-compatible event may carry out of the API or the worker, at the JSON
//! level (the Rust home of `api-dev/scrub.py`).
//!
//! The comic's material never leaves through an error report. Before any event is sent it loses every request body
//! and query string, every header but a short allow-list, every field at any depth whose name says it holds his
//! words (see [`SENSITIVE_KEYS`] and the key hint), breadcrumb `data` beyond route and status, `user` beyond an
//! opaque id, and the local variables and source context on stack frames. The exception type, a route-free bounded
//! message, the frames (file, line, function) and the tags survive: that is what tells us the service crashed and
//! where. [`scrub_value_by`] takes the key rule as a parameter so a test can prove the rule is load-bearing.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use pk_core::py;
use pk_core::text::head;
use regex::Regex;
use serde_json::{Map, Value, json};

/// Field names that hold his words or a secret, at any depth, in any case, with `-` or `_`.
pub const SENSITIVE_KEYS: [&str; 38] = [
    "text",
    "quote",
    "transcript",
    "query",
    "q",
    "title",
    "body",
    "content",
    "prompt",
    "answer",
    "lines",
    "summary",
    "name",
    "venue",
    "filename",
    "original_name",
    "reason",
    "note",
    "notes",
    "idea",
    "data",
    "payload",
    "vars",
    "token",
    "secret",
    "password",
    "authorization",
    "cookie",
    "cookies",
    "set-cookie",
    "api_key",
    "apikey",
    "key",
    "email",
    "user",
    "username",
    "sub",
    "ip_address",
];
/// The request headers an event may keep (compared lower-cased).
pub const HEADER_ALLOW: [&str; 6] = [
    "content-type",
    "content-length",
    "user-agent",
    "accept",
    "host",
    "x-request-id",
];
/// What a sensitive key's value is replaced with.
pub const REDACTED: &str = "[scrubbed]";
/// The longest message (in characters) an event carries.
pub const MAX_MESSAGE: usize = 300;
/// The longest string value `scrub_value` keeps before cutting it.
const MAX_STRING: usize = 1000;
/// Objects and arrays are cut to this many entries.
const MAX_WIDTH: usize = 64;
/// Nesting past this depth is replaced whole.
const MAX_DEPTH: usize = 8;

static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"'[^']*'|"[^"]*""#).expect("a fixed regex"));
static KEY_HINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?i)(text|quote|transcript|query|title|secret|token|password|cookie|auth)").expect("a fixed regex")
});

/// Is this field name one whose value must not leave? Case- and separator-insensitive, plus a hint regex over the
/// name (`joke_text`, `search_query`, `HTTP_AUTHORIZATION` are all sensitive).
pub fn sensitive_key(k: &str) -> bool {
    let s = k.trim().to_lowercase().replace('-', "_");
    let dashed = s.replace('_', "-");
    let listed = SENSITIVE_KEYS.iter().any(|x| *x == s || *x == dashed);
    listed || KEY_HINT.is_match(&s)
}

/// An exception message or log line: the first line only (a `SurrealError` appends the statement after a newline),
/// quoted literals replaced (a `KeyError`'s key, a parser's snippet, a search term in a query), bounded.
pub fn scrub_text(s: &str) -> String {
    let first = s.split('\n').next().unwrap_or("");
    head(&QUOTED.replace_all(first, "'...'"), MAX_MESSAGE)
}

/// Python's `str(v or "")`: a falsy value is empty; the rest prints as Python does (`True`, `1e-05`).
fn text_of(v: Option<&Value>) -> String {
    v.filter(|x| py::truthy(Some(x))).map_or_else(String::new, py::str)
}

/// [`scrub_text`] over a JSON value (an exception's `value` may be null or a number).
pub fn scrub_text_value(v: Option<&Value>) -> String {
    scrub_text(&text_of(v))
}

/// Recursively replace the values of sensitive keys with [`REDACTED`]; cut depth, width and string length so a
/// scrubbed event stays small. `is_sensitive` is the key rule ([`sensitive_key`] in production).
pub fn scrub_value_by(v: &Value, is_sensitive: &dyn Fn(&str) -> bool) -> Value {
    fn go(v: &Value, depth: usize, is_sensitive: &dyn Fn(&str) -> bool) -> Value {
        if depth > MAX_DEPTH {
            return Value::String(REDACTED.to_string());
        }
        match v {
            Value::Object(o) => Value::Object(
                o.iter()
                    .take(MAX_WIDTH)
                    .map(|(k, x)| {
                        let scrubbed = if is_sensitive(k) {
                            Value::String(REDACTED.to_string())
                        } else {
                            go(x, depth + 1, is_sensitive)
                        };
                        (k.clone(), scrubbed)
                    })
                    .collect(),
            ),
            Value::Array(a) => Value::Array(
                a.iter()
                    .take(MAX_WIDTH)
                    .map(|x| go(x, depth + 1, is_sensitive))
                    .collect(),
            ),
            Value::String(s) => {
                if s.chars().count() <= MAX_STRING {
                    v.clone()
                } else {
                    Value::String(format!("{}...", head(s, MAX_STRING)))
                }
            }
            other => other.clone(),
        }
    }
    go(v, 0, is_sensitive)
}

/// [`scrub_value_by`] with the production rule, [`sensitive_key`].
pub fn scrub_value(v: &Value) -> Value {
    scrub_value_by(v, &sensitive_key)
}

/// The request: method, the URL without its query string, and the allow-listed headers. Body, query string,
/// cookies and environment are gone.
pub fn scrub_request(req: &Value) -> Value {
    let mut out = Map::new();
    let Value::Object(req) = req else {
        return Value::Object(out);
    };
    if let Some(Value::String(m)) = req.get("method") {
        out.insert("method".into(), Value::String(m.clone()));
    }
    if let Some(Value::String(u)) = req.get("url") {
        out.insert(
            "url".into(),
            Value::String(u.split('?').next().unwrap_or("").to_string()),
        );
    }
    if let Some(Value::Object(hs)) = req.get("headers") {
        let kept: Map<String, Value> = hs
            .iter()
            .filter(|(k, _)| HEADER_ALLOW.contains(&k.to_lowercase().as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.insert("headers".into(), Value::Object(kept));
    }
    Value::Object(out)
}

/// A breadcrumb: timestamp, type, category and level, and of its `data` only the route, status, method, reason
/// class and duration (scalars). The message is gone.
pub fn scrub_breadcrumb(bc: &Value) -> Value {
    let mut out = Map::new();
    let Value::Object(bc) = bc else {
        return Value::Object(out);
    };
    for k in ["timestamp", "type", "category", "level"] {
        if let Some(v) = bc.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    if let Some(Value::Object(d)) = bc.get("data") {
        let mut keep = Map::new();
        for k in [
            "route",
            "status",
            "status_code",
            "method",
            "reason_class",
            "duration_ms",
        ] {
            if let Some(v @ (Value::String(_) | Value::Number(_) | Value::Bool(_))) = d.get(k) {
                keep.insert(k.into(), v.clone());
            }
        }
        if !keep.is_empty() {
            out.insert("data".into(), Value::Object(keep));
        }
    }
    Value::Object(out)
}

/// The frames of a stack trace: file, path, function, line, module, in-app. No `vars`, no `context_line`, no
/// `pre_context` / `post_context`. Non-objects are skipped; the last 50 frames are kept.
fn scrub_stacktrace(st: &Map<String, Value>) -> Value {
    let frames: Vec<Value> = match st.get("frames") {
        Some(Value::Array(fs)) => fs
            .iter()
            .skip(fs.len().saturating_sub(50))
            .filter_map(|f| match f {
                Value::Object(f) => Some(Value::Object(
                    ["filename", "abs_path", "function", "lineno", "module", "in_app"]
                        .into_iter()
                        .filter_map(|k| f.get(k).map(|v| (k.to_string(), v.clone())))
                        .collect(),
                )),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    json!({ "frames": frames })
}

/// The exception values: type (bounded), the scrubbed message, the frames and the mechanism's type and handled flag.
pub fn scrub_exception(exc: &Value) -> Value {
    let mut vals = Vec::new();
    if let Value::Object(exc) = exc
        && let Some(Value::Array(items)) = exc.get("values")
    {
        for v in items.iter().take(8) {
            let Value::Object(v) = v else { continue };
            let mut item = Map::new();
            item.insert("type".into(), Value::String(head(&text_of(v.get("type")), 100)));
            item.insert("value".into(), Value::String(scrub_text_value(v.get("value"))));
            if let Some(Value::Object(st)) = v.get("stacktrace") {
                item.insert("stacktrace".into(), scrub_stacktrace(st));
            }
            if let Some(Value::Object(m)) = v.get("mechanism") {
                let kept: Map<String, Value> = ["type", "handled"]
                    .into_iter()
                    .filter_map(|k| m.get(k).map(|v| (k.to_string(), v.clone())))
                    .collect();
                item.insert("mechanism".into(), Value::Object(kept));
            }
            vals.push(Value::Object(item));
        }
    }
    json!({ "values": vals })
}

/// `before_send`: the event to send (never nothing: a crash is always reported, just bare). A non-object passes
/// through unchanged.
pub fn scrub_event(event: &Value) -> Value {
    let Value::Object(ev) = event else { return event.clone() };
    let mut out = Map::new();
    for k in [
        "event_id",
        "timestamp",
        "platform",
        "level",
        "logger",
        "release",
        "environment",
        "server_name",
        "transaction",
        "sdk",
        "dist",
        "type",
    ] {
        if let Some(v) = ev.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    if let Some(m) = ev.get("message") {
        let text = match m {
            Value::String(s) => s.clone(),
            Value::Object(o) => text_of(o.get("formatted")),
            _ => String::new(),
        };
        out.insert("message".into(), Value::String(scrub_text(&text)));
    }
    if let Some(Value::Object(le)) = ev.get("logentry") {
        out.insert(
            "logentry".into(),
            json!({ "message": scrub_text_value(le.get("message")) }),
        );
    }
    if let Some(exc) = ev.get("exception") {
        out.insert("exception".into(), scrub_exception(exc));
    }
    if let Some(req) = ev.get("request") {
        out.insert("request".into(), scrub_request(req));
    }
    match ev.get("breadcrumbs") {
        Some(Value::Object(b)) => {
            let vals = match b.get("values") {
                Some(Value::Array(vs)) => vs
                    .iter()
                    .skip(vs.len().saturating_sub(30))
                    .map(scrub_breadcrumb)
                    .collect(),
                _ => Vec::new(),
            };
            out.insert("breadcrumbs".into(), json!({ "values": vals }));
        }
        Some(Value::Array(vs)) => {
            let vals: Vec<Value> = vs
                .iter()
                .skip(vs.len().saturating_sub(30))
                .map(scrub_breadcrumb)
                .collect();
            out.insert("breadcrumbs".into(), Value::Array(vals));
        }
        _ => {}
    }
    if let Some(Value::Object(tags)) = ev.get("tags") {
        let kept: Map<String, Value> = tags
            .iter()
            .filter(|(k, _)| !sensitive_key(k))
            .map(|(k, v)| (k.clone(), scrub_value(v)))
            .collect();
        out.insert("tags".into(), Value::Object(kept));
    }
    if let Some(Value::Object(cx)) = ev.get("contexts") {
        let mut ctx = Map::new();
        for k in ["runtime", "os", "device", "app", "trace"] {
            if let Some(v @ Value::Object(_)) = cx.get(k) {
                ctx.insert(k.into(), scrub_value(v));
            }
        }
        out.insert("contexts".into(), Value::Object(ctx));
    }
    if let Some(extra @ Value::Object(_)) = ev.get("extra") {
        out.insert("extra".into(), scrub_value(extra));
    }
    if let Some(Value::Object(u)) = ev.get("user")
        && let Some(id) = u.get("id")
        && !id.is_null()
    {
        let id = match id {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out.insert("user".into(), json!({ "id": head(&id, 64) }));
    }
    if let Some(Value::Object(th)) = ev.get("threads") {
        let vals: Vec<Value> = match th.get("values") {
            Some(Value::Array(ts)) => ts
                .iter()
                .take(16)
                .filter_map(|t| match t {
                    Value::Object(t) => Some(json!({
                        "id": t.get("id").cloned().unwrap_or(Value::Null),
                        "crashed": t.get("crashed").cloned().unwrap_or(Value::Null),
                        "stacktrace": match t.get("stacktrace") {
                            Some(Value::Object(st)) => scrub_stacktrace(st),
                            _ => Value::Null,
                        },
                    })),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        out.insert("threads".into(), json!({ "values": vals }));
    }
    Value::Object(out)
}

/// Every key name at any depth of a (scrubbed) event; the tests assert no sensitive key survives with a value.
pub fn flatten_keys(x: &Value) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    match x {
        Value::Object(o) => {
            for (k, v) in o {
                keys.insert(k.clone());
                keys.extend(flatten_keys(v));
            }
        }
        Value::Array(a) => {
            for v in a {
                keys.extend(flatten_keys(v));
            }
        }
        _ => {}
    }
    keys
}
