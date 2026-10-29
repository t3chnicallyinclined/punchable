//! The differential against the Python it replaces: `rust/tools/gen_obs_goldens.py` runs a scripted sequence
//! against `api-dev/metrics.py` and `worker/metrics.py`, scrubs synthetic crash events with `api-dev/scrub.py` and
//! writes what it saw; these tests run the same script through pk-obs and compare as parsed JSON.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::LazyLock;

use pk_obs::labels::{METHODS, job_stats, kind_label, model_label, route_pattern, status_class, task_label};
use pk_obs::metrics::{DB_BUCKETS, DEFAULT_BUCKETS, JOB_BUCKETS};
use pk_obs::scrub::{
    HEADER_ALLOW, MAX_MESSAGE, REDACTED, SENSITIVE_KEYS, flatten_keys, scrub_breadcrumb, scrub_event, scrub_request,
    scrub_text_value, scrub_value, sensitive_key,
};
use pk_obs::{ApiMetrics, BudgetStatus, CONTENT_TYPE, Usage, WorkerMetrics};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};

fn golden(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name);
    serde_json::from_str(
        &std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_obs_goldens.py)", p.display())),
    )
    .expect("golden is JSON")
}

/// A readable diff on failure: the first path where two JSON values differ.
fn first_diff(rust: &Value, python: &Value, path: &str) -> Option<String> {
    match (rust, python) {
        (Value::Object(ro), Value::Object(po)) => {
            for key in ro.keys().chain(po.keys().filter(|k| !ro.contains_key(*k))) {
                match (ro.get(key), po.get(key)) {
                    (Some(rv), Some(pv)) => {
                        if let Some(d) = first_diff(rv, pv, &format!("{path}.{key}")) {
                            return Some(d);
                        }
                    }
                    (Some(rv), None) => return Some(format!("{path}.{key}: rust has {rv}, python has nothing")),
                    (None, Some(pv)) => return Some(format!("{path}.{key}: rust has nothing, python has {pv}")),
                    (None, None) => {}
                }
            }
            None
        }
        (Value::Array(ra), Value::Array(pa)) => {
            if ra.len() != pa.len() {
                return Some(format!("{path}: rust has {} items, python has {}", ra.len(), pa.len()));
            }
            ra.iter()
                .zip(pa)
                .enumerate()
                .find_map(|(i, (rv, pv))| first_diff(rv, pv, &format!("{path}[{i}]")))
        }
        (Value::Number(rn), Value::Number(pn)) => {
            let (fr, fp) = (rn.as_f64().unwrap_or(f64::NAN), pn.as_f64().unwrap_or(f64::NAN));
            ((fr - fp).abs() > 1e-9).then(|| format!("{path}: rust {rn}, python {pn}"))
        }
        _ => (rust != python).then(|| format!("{path}: rust {rust}, python {python}")),
    }
}

fn same(what: &str, rust: &Value, python: &Value) {
    if let Some(d) = first_diff(rust, python, what) {
        panic!("{what} differs from the Python golden at {d}");
    }
}

// ------------------------------------------------------------------------------------------------- exposition

/// One metric family as both sides compare it (the generator's `parse_exposition`).
#[derive(Serialize, Default)]
struct Family {
    #[serde(rename = "type")]
    kind: String,
    help: String,
    labels: Vec<String>,
    buckets: Vec<f64>,
    series: Vec<Vec<(String, String)>>,
    values: BTreeMap<String, f64>,
    counts: BTreeMap<String, u64>,
}

static SAMPLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{(.*)\})? (\S+)$").expect("regex"));
static PAIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(\w+)="((?:[^"\\]|\\.)*)""#).expect("regex"));
const CLOCK: [&str; 2] = [
    "pk_jobs_waiting_transcriber_oldest_seconds",
    "pk_worker_heartbeat_age_seconds",
];

fn unescape(v: &str) -> String {
    v.replace("\\n", "\n").replace("\\\"", "\"").replace("\\\\", "\\")
}

fn parse_exposition(text: &str) -> BTreeMap<String, Family> {
    let mut out: BTreeMap<String, Family> = BTreeMap::new();
    let mut types: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# HELP ") {
            let (name, help) = rest.split_once(' ').unwrap_or((rest, ""));
            out.entry(name.to_string()).or_default().help = unescape(help);
            continue;
        }
        if let Some(rest) = line.strip_prefix("# TYPE ") {
            let (name, kind) = rest.split_once(' ').unwrap_or((rest, ""));
            out.entry(name.to_string()).or_default().kind = kind.to_string();
            types.insert(name.to_string(), kind.to_string());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let m = SAMPLE
            .captures(line)
            .unwrap_or_else(|| panic!("not a sample line: {line}"));
        let sample = &m[1];
        let labels = m.get(3).map_or("", |x| x.as_str());
        let value = &m[4];
        let (base, suffix) = ["_bucket", "_sum", "_count"]
            .into_iter()
            .find_map(|s| {
                let base = sample.strip_suffix(s)?;
                (types.get(base).map(String::as_str) == Some("histogram")).then_some((base, s))
            })
            .unwrap_or((sample, ""));
        let fam = out
            .get_mut(base)
            .unwrap_or_else(|| panic!("sample before its TYPE: {line}"));
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut le: Option<String> = None;
        for c in PAIR.captures_iter(labels) {
            if &c[1] == "le" {
                le = Some(unescape(&c[2]));
            } else {
                pairs.push((c[1].to_string(), unescape(&c[2])));
            }
        }
        pairs.sort();
        for (k, _) in &pairs {
            if !fam.labels.contains(k) {
                fam.labels.push(k.clone());
            }
        }
        if !fam.series.contains(&pairs) {
            fam.series.push(pairs.clone());
        }
        let key = serde_json::to_string(&pairs).expect("key");
        match suffix {
            "_bucket" => {
                if let Some(le) = le
                    && le != "+Inf"
                {
                    let b: f64 = le.parse().expect("le");
                    if !fam.buckets.contains(&b) {
                        fam.buckets.push(b);
                    }
                }
            }
            "_count" => {
                fam.counts.insert(key, value.parse().expect("count"));
            }
            "_sum" => {}
            _ => {
                if !CLOCK.contains(&base) {
                    fam.values.insert(key, value.parse().expect("value"));
                }
            }
        }
    }
    for fam in out.values_mut() {
        fam.labels.sort();
        fam.buckets.sort_by(f64::total_cmp);
        fam.series.sort();
    }
    out
}

/// Python's `int(code)` on the generator's mixed status cases, so `status_class(i64)` sees what the Python saw.
fn py_int(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f.trunc() as i64))
            .unwrap_or(-1),
        Value::Bool(b) => i64::from(*b),
        Value::String(s) => s.trim().parse().unwrap_or(-1),
        _ => -1,
    }
}

/// Python's `int(v or 0)` with a `0` on failure (`worker/metrics.py` `_n`).
fn py_count(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n.as_u64().or_else(|| n.as_f64().map(|f| f.trunc() as u64)).unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        Some(Value::Bool(true)) => 1,
        _ => 0,
    }
}

fn opt_str(v: &Value) -> Option<&str> {
    v.as_str()
}

#[test]
fn api_exposition_matches_the_python_registry() {
    let g = golden("exposition.json");
    let api = ApiMetrics::new(g["version"].as_str().unwrap(), g["sha"].as_str().unwrap()).unwrap();
    for r in g["api"]["requests"].as_array().unwrap() {
        api.record(
            r[0].as_str().unwrap(),
            r[1].as_str().unwrap(),
            py_int(&r[2]),
            r[3].as_f64().unwrap(),
        );
    }
    api.in_flight.inc();
    api.in_flight.inc();
    api.in_flight.dec();
    api.live_clients.set(2);
    api.db_duration.observe(0.003);
    api.db_duration.observe(0.3);
    let jobs = g["api"]["jobs"].as_array().unwrap().clone();
    api.set_jobs_source(move || jobs.clone());
    let text = api.render().unwrap();
    same("api", &json!(parse_exposition(&text)), &g["api"]["metrics"]);
    assert!(text.contains("pk_jobs_waiting_transcriber_oldest_seconds "), "{text}");
    assert!(
        text.contains(r#"pk_build_info{sha="0123456789ab",version="0.1.0"} 1"#),
        "{text}"
    );
    for word in ["abc123", "def456", "joke:", "words", "0a1b2c3d4e5f"] {
        assert!(!text.contains(word), "{word} leaked into {text}");
    }
}

#[test]
fn worker_exposition_matches_the_python_registry() {
    let g = golden("exposition.json");
    let w = WorkerMetrics::new(g["worker_version"].as_str().unwrap(), g["sha"].as_str().unwrap()).unwrap();
    w.heartbeat();
    w.job(Some("riff")).done();
    w.job(Some("ask_chat")).failed(true);
    w.job(Some("weird kind!")).failed(false);
    w.job(None).done();
    for c in g["worker"]["charges"].as_array().unwrap() {
        let u = &c[1];
        let usage = Usage {
            input_tokens: py_count(u.get("input_tokens")),
            output_tokens: py_count(u.get("output_tokens")),
            cache_read_input_tokens: py_count(u.get("cache_read_input_tokens")),
            cache_creation_input_tokens: py_count(u.get("cache_creation_input_tokens")),
        };
        w.on_charge(opt_str(&c[0]), &usage, c[2].as_f64().unwrap(), opt_str(&c[3]));
    }
    let b = &g["worker"]["budget"];
    let status = BudgetStatus {
        daily_usd: b["daily_usd"].as_f64().unwrap(),
        spent_usd: b["spent_usd"].as_f64().unwrap(),
        over: b["over"].as_bool().unwrap(),
    };
    w.attach_budget(move || Some(status));
    let text = w.render().unwrap();
    same("worker", &json!(parse_exposition(&text)), &g["worker"]["metrics"]);
    assert!(text.contains("pk_worker_heartbeat_age_seconds 0."), "{text}");
}

#[test]
fn heartbeat_is_nan_before_the_first_touch() {
    let w = WorkerMetrics::new("0.1.0", "").unwrap();
    let text = w.render().unwrap();
    assert!(text.contains("pk_worker_heartbeat_age_seconds NaN"), "{text}");
    assert!(
        text.contains(r#"pk_worker_build_info{sha="dev",version="0.1.0"} 1"#),
        "{text}"
    );
    assert!(w.heartbeat_age().is_none());
    w.heartbeat();
    assert!(w.heartbeat_age().is_some());
}

#[test]
fn constants_match_the_python() {
    let g = golden("labels.json");
    same("default_buckets", &json!(DEFAULT_BUCKETS), &g["default_buckets"]);
    same("job_buckets", &json!(JOB_BUCKETS), &g["job_buckets"]);
    same("methods", &json!(METHODS), &g["methods"]);
    assert_eq!(CONTENT_TYPE, g["content_type"].as_str().unwrap());
    let e = golden("exposition.json");
    same(
        "db_buckets",
        &json!(DB_BUCKETS),
        &e["api"]["metrics"]["pk_db_query_duration_seconds"]["buckets"],
    );
}

// ----------------------------------------------------------------------------------------------------- labels

#[test]
fn route_pattern_matches_the_python() {
    let g = golden("labels.json");
    for case in g["route_pattern"].as_array().unwrap() {
        let (path, want) = (case[0].as_str().unwrap(), case[1].as_str().unwrap());
        assert_eq!(route_pattern(path), want, "route_pattern({path:?})");
    }
}

#[test]
fn status_class_matches_the_python() {
    let g = golden("labels.json");
    for case in g["status_class"].as_array().unwrap() {
        assert_eq!(
            status_class(py_int(&case[0])),
            case[1].as_str().unwrap(),
            "status_class({})",
            case[0]
        );
    }
}

#[test]
fn worker_label_helpers_match_the_python() {
    let g = golden("labels.json");
    for case in g["kind_label"].as_array().unwrap() {
        assert_eq!(
            kind_label(opt_str(&case[0])),
            case[1].as_str().unwrap(),
            "kind_label({})",
            case[0]
        );
    }
    for case in g["model_label"].as_array().unwrap() {
        assert_eq!(
            model_label(opt_str(&case[0])),
            case[1].as_str().unwrap(),
            "model_label({})",
            case[0]
        );
    }
    assert_eq!(task_label(Some("riff")), "riff");
    assert_eq!(task_label(None), "?");
    assert_eq!(task_label(Some("")), "?");
    assert_eq!(task_label(Some(&"t".repeat(40))), "t".repeat(40));
    assert_eq!(task_label(Some(&"t".repeat(41))), "other");
}

#[test]
fn job_stats_match_the_python() {
    let g = golden("labels.json");
    for case in g["job_stats"].as_array().unwrap() {
        let jobs: Vec<Value> = case[0].as_array().cloned().unwrap_or_default();
        let st = job_stats(&jobs, 1_800_000_000.0);
        let queued: Value = json!(st.queued);
        same("queued", &queued, &case[1]["queued"]);
        assert_eq!(st.waiting, case[1]["waiting"].as_u64().unwrap());
        if st.waiting > 0 {
            assert!(st.oldest_waiting_s > 24.0 * 3600.0, "{}", st.oldest_waiting_s);
        } else {
            assert!(st.oldest_waiting_s.abs() < f64::EPSILON, "{}", st.oldest_waiting_s);
        }
    }
}

// --------------------------------------------------------------------------------------------------- scrubber

#[test]
fn scrub_rules_match_the_python_constants() {
    let g = golden("scrub.json");
    let mut keys: Vec<&str> = SENSITIVE_KEYS.to_vec();
    keys.sort_unstable();
    same("sensitive_keys", &json!(keys), &g["sensitive_keys"]);
    let mut allow: Vec<&str> = HEADER_ALLOW.to_vec();
    allow.sort_unstable();
    same("header_allow", &json!(allow), &g["header_allow"]);
    assert_eq!(REDACTED, g["redacted"].as_str().unwrap());
    assert_eq!(MAX_MESSAGE, g["max_message"].as_u64().unwrap() as usize);
}

#[test]
fn scrub_event_and_scrub_value_match_the_python_over_the_golden_events() {
    let g = golden("scrub.json");
    let private = g["private"].as_object().unwrap();
    for ev in g["events"].as_array().unwrap() {
        let name = ev["name"].as_str().unwrap();
        let scrubbed = scrub_event(&ev["event"]);
        same(&format!("scrub_event[{name}]"), &scrubbed, &ev["scrubbed"]);
        same(
            &format!("scrub_value[{name}]"),
            &scrub_value(&ev["event"]),
            &ev["scrub_value"],
        );
        let keys: Vec<String> = flatten_keys(&scrubbed).into_iter().collect();
        same(&format!("flat_keys[{name}]"), &json!(keys), &ev["flat_keys"]);
        let blob = serde_json::to_string(&scrubbed).unwrap();
        for (k, v) in private {
            assert!(!blob.contains(v.as_str().unwrap()), "[{name}] {k} leaked: {blob}");
        }
        for word in ["owner@example.test", "203.0.113.9", "session=abc"] {
            assert!(!blob.contains(word), "[{name}] {word} leaked: {blob}");
        }
    }
    for case in g["scrub_event_passthrough"].as_array().unwrap() {
        same("passthrough", &scrub_event(&case[0]), &case[1]);
    }
}

#[test]
fn small_scrub_rules_match_the_python() {
    let g = golden("scrub.json");
    for case in g["scrub_text"].as_array().unwrap() {
        assert_eq!(
            scrub_text_value(Some(&case[0])),
            case[1].as_str().unwrap(),
            "scrub_text({})",
            case[0]
        );
    }
    for case in g["sensitive_key"].as_array().unwrap() {
        let k = case[0].as_str().unwrap();
        assert_eq!(sensitive_key(k), case[1].as_bool().unwrap(), "sensitive_key({k:?})");
    }
    for case in g["scrub_request"].as_array().unwrap() {
        same("scrub_request", &scrub_request(&case[0]), &case[1]);
    }
    for case in g["scrub_breadcrumb"].as_array().unwrap() {
        same("scrub_breadcrumb", &scrub_breadcrumb(&case[0]), &case[1]);
    }
}
