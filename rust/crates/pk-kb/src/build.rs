//! The decision builder: every write the comic (or an importer, or the AI) makes goes through [`build`], which
//! validates the request against the current [`View`] and returns the line to append (`before` and `after` images).
//! [`undo_line`] is the inverse line an undo appends. Mirrors `tools/explorer/kb.py` `KB._build` and `KB.undo`;
//! the messages are the ones the app shows.

use chrono::NaiveDate;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha1::{Digest, Sha1};
use std::sync::LazyLock;

use crate::error::{KbError, KbResult};
use crate::ids::{edge_id, slugify};
use crate::model::{EDGE_KINDS, LABEL_KINDS, PERF_KINDS, Rec, STAGES, bucket, public, py_repr, truthy};
use crate::view::View;

static DATE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").expect("regex"));
static UNIT_ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^unit:[A-Za-z0-9._-]{1,80}$").expect("regex"));

/// A validated decision, ready to be stamped (`ts`, `reason`, `by`) and appended.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Built {
    /// `edge` | `node`.
    pub target: &'static str,
    /// The node or edge id (for an add: the id the new record gets).
    pub id: String,
    /// `add` | `delete` | `confirm` | `reject` | `edit`.
    pub action: &'static str,
    /// The record as it was (public fields), or null for an add.
    pub before: Option<Map<String, Value>>,
    /// The fields that change (or the whole new record), or null for a delete.
    pub after: Option<Map<String, Value>>,
}

/// The current UTC time the way the Python writes it: `2026-10-29T12:00:00+00:00`.
pub fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

/// Validate a request against the view. `now` is the timestamp the builder stamps new jokes and units with
/// (injected so goldens and replays agree). The request is `{target, action, id?, after?, reason?}` as the app
/// posts it.
pub fn build(v: &View, req: &Value, now: &str, owner_id: &str) -> KbResult<Built> {
    let target = req.get("target").and_then(Value::as_str);
    let action = req.get("action").and_then(Value::as_str);
    let (Some(target), Some(action)) = (target, action) else {
        return Err(KbError::bad(
            "target must be edge|node and action confirm|reject|edit|add|delete",
        ));
    };
    if !matches!(target, "edge" | "node") || !matches!(action, "confirm" | "reject" | "edit" | "add" | "delete") {
        return Err(KbError::bad(
            "target must be edge|node and action confirm|reject|edit|add|delete",
        ));
    }
    let after: Rec = match req.get("after") {
        None | Some(Value::Null) => Rec::new(),
        Some(Value::Object(o)) => o.clone(),
        Some(_) => return Err(KbError::bad("after must be an object")),
    };
    match req.get("reason") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s.chars().count() <= 4000 => {}
        Some(_) => return Err(KbError::bad("reason must be text under 4000 characters")),
    }
    let reason = req.get("reason").and_then(Value::as_str).unwrap_or("");

    let need_node = |nid: Option<&Value>, types: Option<&[&str]>| -> KbResult<&Rec> {
        let id = nid.and_then(Value::as_str);
        let n = id.and_then(|s| v.nodes.get(s));
        match n {
            Some(n) if !id.is_some_and(|s| v.alias.contains_key(s)) => {
                if let Some(ts) = types {
                    let t = n.get("type").and_then(Value::as_str).unwrap_or("");
                    if !ts.contains(&t) {
                        return Err(KbError::bad(format!(
                            "{} is a {}, expected {}",
                            id.unwrap_or(""),
                            py_none(n.get("type")),
                            ts.join("/")
                        )));
                    }
                }
                Ok(n)
            }
            _ => Err(KbError::with_status(404, format!("unknown node {}", py_repr(nid)))),
        }
    };
    let kind_fields = |src: &Rec| -> KbResult<Map<String, Value>> {
        let kind = src.get("kind").and_then(Value::as_str);
        if !kind.is_some_and(|k| EDGE_KINDS.contains(&k)) {
            return Err(KbError::bad(format!("unknown kind {}", py_repr(src.get("kind")))));
        }
        let kind = kind.expect("checked");
        let mut out = Map::new();
        out.insert("kind".into(), json!(kind));
        if kind == "custom" {
            let ck = src
                .get("custom_kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if ck.is_empty() || ck.chars().count() > 60 {
                return Err(KbError::bad(
                    "a custom connection needs a name (custom_kind), up to 60 characters",
                ));
            }
            out.insert("custom_kind".into(), json!(ck));
        } else {
            out.insert("custom_kind".into(), Value::Null);
        }
        Ok(out)
    };

    if target == "edge" {
        if action == "add" {
            let (src, dst) = (after.get("src"), after.get("dst"));
            need_node(src, None)?;
            need_node(dst, None)?;
            if src == dst {
                return Err(KbError::bad("a connection needs two different ends"));
            }
            let kf = kind_fields(&after)?;
            if kf["kind"] == "has_label" && v.ntype(dst.and_then(Value::as_str)) != Some("label") {
                return Err(KbError::bad("has_label must point at a label"));
            }
            if reason.trim().is_empty() {
                return Err(KbError::bad("say why (reason) when you add a connection"));
            }
            let (s, d) = (
                src.and_then(Value::as_str).expect("checked"),
                dst.and_then(Value::as_str).expect("checked"),
            );
            let kind_for_id = kf
                .get("custom_kind")
                .and_then(Value::as_str)
                .unwrap_or_else(|| kf["kind"].as_str().expect("str"));
            let eid = edge_id(s, kind_for_id, d);
            if let Some(ex) = v.edges.get(&eid) {
                return Err(KbError::with_status(
                    409,
                    format!(
                        "that connection already exists ({}); confirm or edit it instead",
                        bucket(ex.get("status")).as_str()
                    ),
                ));
            }
            let mut rec = Map::new();
            rec.insert("src".into(), json!(s));
            rec.insert("dst".into(), json!(d));
            for (k, x) in &kf {
                if !x.is_null() {
                    rec.insert(k.clone(), x.clone());
                }
            }
            rec.insert("status".into(), json!("confirmed"));
            rec.insert("confidence".into(), json!(1.0));
            rec.insert("reason".into(), json!(reason.trim()));
            rec.insert("evidence".into(), json!([]));
            rec.insert("source".into(), json!(owner_id));
            return Ok(Built {
                target: "edge",
                id: eid,
                action: "add",
                before: None,
                after: Some(rec),
            });
        }
        let eid = req.get("id").and_then(Value::as_str);
        let Some(e) = eid.and_then(|id| v.edges.get(id)) else {
            return Err(KbError::with_status(
                404,
                format!("unknown edge {}", py_repr(req.get("id"))),
            ));
        };
        let eid = eid.expect("present").to_string();
        let snap = public(e);
        match action {
            "confirm" => {
                return Ok(Built {
                    target: "edge",
                    id: eid,
                    action: "confirm",
                    before: Some(snap),
                    after: Some(obj(&[("status", json!("confirmed"))])),
                });
            }
            "reject" => {
                return Ok(Built {
                    target: "edge",
                    id: eid,
                    action: "reject",
                    before: Some(snap),
                    after: Some(obj(&[("status", json!("rejected"))])),
                });
            }
            "delete" => {
                if !truthy(e.get("_by_tristan")) {
                    return Err(KbError::bad(
                        "only connections you added can be deleted; reject this one instead",
                    ));
                }
                return Ok(Built {
                    target: "edge",
                    id: eid,
                    action: "delete",
                    before: Some(snap),
                    after: None,
                });
            }
            _ => {}
        }
        // edit
        let mut ch: Map<String, Value> = Map::new();
        if after.contains_key("kind") {
            for (k, x) in kind_fields(&after)? {
                ch.insert(k, x);
            }
        }
        for end in ["src", "dst"] {
            if let Some(x) = after.get(end)
                && Some(x) != e.get(end)
            {
                need_node(Some(x), None)?;
                ch.insert(end.into(), x.clone());
            }
        }
        let s2 = ch.get("src").or_else(|| e.get("src"));
        let d2 = ch.get("dst").or_else(|| e.get("dst"));
        if s2 == d2 {
            return Err(KbError::bad("a connection needs two different ends"));
        }
        let kind_now = ch.get("kind").or_else(|| e.get("kind")).and_then(Value::as_str);
        if kind_now == Some("has_label") && v.ntype(d2.and_then(Value::as_str)) != Some("label") {
            return Err(KbError::bad("has_label must point at a label"));
        }
        if ch.is_empty() && reason.trim().is_empty() {
            return Err(KbError::bad("nothing changed"));
        }
        let mut ch: Map<String, Value> = ch
            .into_iter()
            .filter(|(k, x)| !(x.is_null() && e.get(k).is_none_or(Value::is_null)))
            .collect();
        ch.insert("status".into(), json!("confirmed"));
        return Ok(Built {
            target: "edge",
            id: eid,
            action: "edit",
            before: Some(snap),
            after: Some(ch),
        });
    }

    // nodes
    if action == "add" {
        let t = after.get("type").and_then(Value::as_str);
        return match t {
            Some("label") => {
                let name = after
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() || name.chars().count() > 80 {
                    return Err(KbError::bad("a label needs a name (up to 80 characters)"));
                }
                let kind = after
                    .get("kind")
                    .and_then(Value::as_str)
                    .filter(|k| !k.is_empty())
                    .unwrap_or("custom");
                if !LABEL_KINDS.contains(&kind) {
                    return Err(KbError::bad(format!(
                        "unknown label kind {}",
                        py_repr(after.get("kind"))
                    )));
                }
                let nid = format!("label:{kind}:{}", slugify(&name));
                if v.nodes.contains_key(&nid) {
                    return Err(KbError::with_status(
                        409,
                        format!("label {nid} already exists; pick it instead"),
                    ));
                }
                let definition = after
                    .get("definition")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let rec = obj(&[
                    ("type", json!("label")),
                    ("kind", json!(kind)),
                    ("name", json!(name)),
                    ("definition", json!(definition)),
                ]);
                Ok(Built {
                    target: "node",
                    id: nid,
                    action: "add",
                    before: None,
                    after: Some(rec),
                })
            }
            Some("joke") => {
                let title = after
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if title.is_empty() || title.chars().count() > 120 {
                    return Err(KbError::bad("a joke or idea needs a title (up to 120 characters)"));
                }
                let base = format!("joke:{}", slugify(&title));
                let mut nid = base.clone();
                let mut k = 2;
                while v.nodes.contains_key(&nid) {
                    nid = format!("{base}-{k}");
                    k += 1;
                }
                let stage = after
                    .get("stage")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("idea");
                if !STAGES.contains(&stage) {
                    return Err(KbError::bad(format!("unknown stage {}", py_repr(after.get("stage")))));
                }
                let summary = after
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let day = &now[..10];
                let rec = obj(&[
                    ("type", json!("joke")),
                    ("title", json!(title)),
                    ("stage", json!(stage)),
                    ("summary", json!(summary)),
                    ("first_seen", json!(day)),
                    ("last_seen", json!(day)),
                ]);
                Ok(Built {
                    target: "node",
                    id: nid,
                    action: "add",
                    before: None,
                    after: Some(rec),
                })
            }
            Some("unit") => {
                let text = after
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if text.is_empty() || text.chars().count() > 4000 {
                    return Err(KbError::bad("a line needs its text (up to 4000 characters)"));
                }
                let nid = match after.get("id").and_then(Value::as_str) {
                    Some(id) if UNIT_ID_RE.is_match(id) => id.to_string(),
                    _ => {
                        let digest = Sha1::digest(format!("{text}|{now}|{}", v.nodes.len()).as_bytes());
                        format!("unit:{}", &hex::encode(digest)[..12])
                    }
                };
                if v.nodes.contains_key(&nid) {
                    return Err(KbError::with_status(409, format!("{nid} already exists")));
                }
                let date = match after.get("date").and_then(Value::as_str) {
                    Some(d) if DATE_RE.is_match(d) => d.to_string(),
                    _ => now[..10].to_string(),
                };
                let kind: String = py_str(after.get("kind"), "tag").chars().take(40).collect();
                let source: String = py_str(after.get("source"), "app").chars().take(40).collect();
                let mut rec = obj(&[
                    ("type", json!("unit")),
                    ("text", json!(text)),
                    ("kind", json!(kind)),
                    ("source", json!(source)),
                    ("date", json!(date)),
                ]);
                if let Some(dt) = after.get("doc_title").and_then(Value::as_str)
                    && !dt.trim().is_empty()
                {
                    rec.insert(
                        "doc_title".into(),
                        json!(dt.trim().chars().take(200).collect::<String>()),
                    );
                }
                Ok(Built {
                    target: "node",
                    id: nid,
                    action: "add",
                    before: None,
                    after: Some(rec),
                })
            }
            _ => Err(KbError::bad("you can add a label, a joke/idea or a line (unit)")),
        };
    }
    let nidv = req.get("id");
    let n = need_node(nidv, None)?;
    let nid = nidv.and_then(Value::as_str).expect("checked").to_string();
    let snap = public(n);
    if action == "delete" {
        if !truthy(n.get("_by_tristan")) {
            return Err(KbError::bad("only things you added can be deleted"));
        }
        return Ok(Built {
            target: "node",
            id: nid,
            action: "delete",
            before: Some(snap),
            after: None,
        });
    }
    if action == "confirm" || action == "reject" {
        let status = if action == "confirm" { "confirmed" } else { "rejected" };
        return Ok(Built {
            target: "node",
            id: nid,
            action: if action == "confirm" { "confirm" } else { "reject" },
            before: Some(snap),
            after: Some(obj(&[("status", json!(status))])),
        });
    }
    let ntype = n.get("type").and_then(Value::as_str).unwrap_or("");
    let allowed: &[&str] = match ntype {
        "joke" => &["stage", "title", "summary", "hidden", "short_name"],
        "label" => &["name", "definition", "merged_into"],
        "set" => &["show_date", "venue", "audio_unreliable"],
        "performance" => &["kind"],
        _ => &[],
    };
    let mut ch: Map<String, Value> = Map::new();
    for (k, x) in &after {
        if !allowed.contains(&k.as_str()) {
            return Err(KbError::bad(format!(
                "can't edit {} on a {}",
                py_repr(Some(&json!(k))),
                py_none(n.get("type"))
            )));
        }
        let xs = x.as_str();
        match k.as_str() {
            "stage" if !xs.is_some_and(|s| STAGES.contains(&s)) => {
                return Err(KbError::bad(format!("unknown stage {}", py_repr(Some(x)))));
            }
            "merged_into" if !x.is_null() => {
                need_node(Some(x), Some(&["label"]))?;
                if xs == Some(nid.as_str()) {
                    return Err(KbError::bad("can't merge a label into itself"));
                }
            }
            "name" | "title" if xs.is_none_or(|s| s.trim().is_empty()) => {
                return Err(KbError::bad(format!("{k} can't be empty")));
            }
            "show_date" if !x.is_null() => {
                let Some(s) = xs.filter(|s| DATE_RE.is_match(s)) else {
                    return Err(KbError::bad("show_date must be YYYY-MM-DD (or null to clear it)"));
                };
                if NaiveDate::parse_from_str(s, "%Y-%m-%d").is_err() {
                    return Err(KbError::bad(format!(
                        "show_date {} is not a real date",
                        py_repr(Some(x))
                    )));
                }
            }
            "venue" if !xs.is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 120) => {
                return Err(KbError::bad("venue must be 1-120 characters"));
            }
            "audio_unreliable" if !x.is_boolean() => {
                return Err(KbError::bad("audio_unreliable must be true or false"));
            }
            "hidden" if !x.is_boolean() => return Err(KbError::bad("hidden must be true or false")),
            "short_name"
                if !x.is_null() && !xs.is_some_and(|s| !s.trim().is_empty() && s.trim().chars().count() <= 24) =>
            {
                return Err(KbError::bad(
                    "short_name must be 1-24 characters (or null for the default)",
                ));
            }
            "kind" if !xs.is_some_and(|s| PERF_KINDS.contains(&s)) => {
                return Err(KbError::bad(format!("kind must be one of {}", PERF_KINDS.join(", "))));
            }
            _ => {}
        }
        // Python's `x != n.get(k)`: an absent field reads as None, so null == absent and nothing changes
        if *x != *n.get(k).unwrap_or(&Value::Null) {
            ch.insert(k.clone(), xs.map_or_else(|| x.clone(), |s| json!(s.trim())));
        }
    }
    if ch.is_empty() {
        return Err(KbError::bad("nothing changed"));
    }
    Ok(Built {
        target: "node",
        id: nid,
        action: "edit",
        before: Some(snap),
        after: Some(ch),
    })
}

/// The inverse of decision `index`, as the line an undo appends (`ts` = `now`, `by`), or why it cannot be undone.
pub fn undo_line(view: &View, index: usize, now: &str, by: &str) -> KbResult<Map<String, Value>> {
    let Some(dec) = view.decisions.get(index) else {
        return Err(KbError::with_status(404, "no such decision"));
    };
    if view.is_undone(index) {
        return Err(KbError::with_status(409, "already undone"));
    }
    if dec.has_undo_of() {
        return Err(KbError::with_status(409, "that line is itself an undo"));
    }
    let did = dec.id();
    let cur = match dec.target() {
        Some("edge") => did.and_then(|id| view.edges.get(id)),
        _ => did.and_then(|id| view.nodes.get(id)),
    };
    let before = dec.before();
    let after = dec.after();
    let (action, inv_before, inv_after): (&str, Value, Value) = match dec.action() {
        Some("add") => (
            "delete",
            cur.map_or(Value::Object(after.clone()), |c| Value::Object(public(c))),
            Value::Null,
        ),
        Some("delete") => ("add", Value::Null, Value::Object(before.clone())),
        _ => (
            "edit",
            cur.map_or(Value::Null, |c| Value::Object(public(c))),
            Value::Object(
                after
                    .keys()
                    .map(|k| (k.clone(), before.get(k).cloned().unwrap_or(Value::Null)))
                    .collect(),
            ),
        ),
    };
    let reason = match dec.reason().filter(|r| !r.is_empty()) {
        Some(r) => format!("undo: {r}"),
        None => "undo".to_string(),
    };
    let mut out = Map::new();
    out.insert("ts".into(), json!(now));
    out.insert("target".into(), dec.0.get("target").cloned().unwrap_or(Value::Null));
    out.insert("id".into(), dec.0.get("id").cloned().unwrap_or(Value::Null));
    out.insert("action".into(), json!(action));
    out.insert("before".into(), inv_before);
    out.insert("after".into(), inv_after);
    out.insert("reason".into(), json!(reason));
    out.insert("by".into(), json!(by));
    out.insert("undo_of".into(), json!(index));
    Ok(out)
}

/// The decision line for a [`Built`]: stamped with `ts`, `reason` and `by`, the way `KB.decide` writes it.
pub fn stamp(b: &Built, now: &str, reason: &str, by: &str) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("ts".into(), json!(now));
    out.insert("target".into(), json!(b.target));
    out.insert("id".into(), json!(b.id));
    out.insert("action".into(), json!(b.action));
    out.insert("before".into(), b.before.clone().map_or(Value::Null, Value::Object));
    out.insert("after".into(), b.after.clone().map_or(Value::Null, Value::Object));
    out.insert("reason".into(), json!(reason.trim()));
    out.insert("by".into(), json!(by));
    out
}

fn obj(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
}

/// `str(x or default)`: a non-empty string value, else the default (the Python `str(after.get(k) or "tag")`).
fn py_str(v: Option<&Value>, default: &str) -> String {
    match v {
        Some(x) if truthy(Some(x)) => match x {
            Value::String(s) => s.clone(),
            Value::Bool(true) => "True".to_string(),
            other => other.to_string(),
        },
        _ => default.to_string(),
    }
}

/// How Python prints a value in an f-string: `None` for null, the string itself otherwise.
fn py_none(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}
