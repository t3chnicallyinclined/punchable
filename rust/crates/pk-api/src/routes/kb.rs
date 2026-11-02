//! The knowledge-base reads of `server.py` `do_GET`: `/api/labels`, `/api/decisions`, `/api/meta`, `/api/pipeline`.

use axum::extract::{RawQuery, State};
use axum::response::Response;
use indexmap::IndexMap;
use pk_kb::decision::actor_of;
use serde_json::{Map, Value, json};

use crate::error::{ApiResult, ok};
use crate::index::{RATINGS, STAGES};
use crate::query::{Query, py_slice};
use crate::state::AppState;

/// `creative/data.py` `by_kind`: `owner`, `import` or `ai` (None when the row names nobody).
pub fn by_kind(by: Option<&str>, owner_id: &str) -> Value {
    match by {
        None | Some("") => Value::Null,
        Some(b) if b == owner_id => json!("owner"),
        Some(b) if b.starts_with("import:") => json!("import"),
        Some(_) => json!("ai"),
    }
}

/// A dict key the way `json.dumps` writes a non-string key (`None` -> `"null"`, `True` -> `"true"`).
fn key_of(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "null".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(other) => pk_core::py::str(other),
    }
}

/// `GET /api/labels`.
pub async fn labels(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, idx) = super::prepare(&st, &q).await?;
    let v = &idx.view;
    let get = |n: &Map<String, Value>, k: &str| n.get(k).cloned().unwrap_or(Value::Null);
    let labels: Vec<Value> = v
        .labels
        .iter()
        .filter_map(|id| v.nodes.get(id).map(|n| (id, n)))
        .map(|(id, n)| json!({"id": id, "name": get(n, "name"), "kind": get(n, "kind"), "definition": get(n, "definition")}))
        .collect();
    Ok(ok(&json!({ "labels": labels })))
}

/// `GET /api/decisions?limit=`: the newest first, at most 500.
pub async fn decisions(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, idx) = super::prepare(&st, &q).await?;
    let limit = q.int("limit", "30")?.min(500);
    let view = &idx.view;
    let owner = &st.config().owner_id;
    let range = py_slice(view.decisions.len(), Some(limit.saturating_neg()), None);
    let ds: Vec<Value> = range
        .rev()
        .map(|i| {
            let line = &view.decisions[i];
            let actor = actor_of(line, owner);
            let mut m = line.0.clone();
            m.insert("index".into(), json!(i));
            m.insert("undone".into(), json!(view.is_undone(i)));
            m.insert("actor".into(), actor.map_or(Value::Null, |a| json!(a)));
            m.insert("by_kind".into(), by_kind(actor, owner));
            Value::Object(m)
        })
        .collect();
    Ok(ok(&json!({"decisions": ds, "total": view.decisions.len()})))
}

/// `GET /api/meta?labels=`: stage counts, who touched what, the top labels, the totals.
pub async fn meta(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, idx) = super::prepare(&st, &q).await?;
    let v = &idx.view;
    let mut counts: IndexMap<String, u64> = IndexMap::new();
    for j in &v.jokes {
        *counts
            .entry(key_of(v.nodes.get(j).and_then(|n| n.get("stage"))))
            .or_default() += 1;
    }
    let mut lab: IndexMap<&str, u64> = IndexMap::new();
    for r in &idx.summaries {
        for lid in &r.label_ids {
            *lab.entry(lid.as_str()).or_default() += 1;
        }
    }
    let mut top: Vec<(&str, u64)> = lab.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let take = q.int("labels", "12")?;
    let top_labels: Vec<Value> = top[py_slice(top.len(), None, Some(take))]
        .iter()
        .map(|(k, n)| {
            let node = v.nodes.get(*k);
            let get = |f: &str| node.and_then(|x| x.get(f)).cloned().unwrap_or(Value::Null);
            json!({"id": k, "name": get("name"), "kind": get("kind"), "jokes": n})
        })
        .collect();
    let mut ai_counts: IndexMap<String, u64> = IndexMap::new();
    for r in idx.summaries.iter().filter(|r| r.summary.stage_ai) {
        *ai_counts
            .entry(r.summary.stage.clone().unwrap_or_else(|| "null".into()))
            .or_default() += 1;
    }
    let who: Map<String, Value> = ["ai", "confirmed", "edited"]
        .iter()
        .map(|w| {
            (
                (*w).to_string(),
                json!(idx.summaries.iter().filter(|r| r.summary.who == *w).count()),
            )
        })
        .collect();
    Ok(ok(&json!({
        "stages": STAGES, "stage_counts": counts, "ai_stage_counts": ai_counts, "who_counts": who, "ratings": RATINGS,
        "top_labels": top_labels,
        "counts": {
            "jokes": v.jokes.len(), "labels": v.labels.len(), "edges": v.edges.len(), "sets": idx.sets.len(),
            "performances": idx.performances, "decisions": v.decisions.len(),
        },
    })))
}

/// `GET /api/pipeline?limit=`: the Bits list by stage.
pub async fn pipeline(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, idx) = super::prepare(&st, &q).await?;
    let mut out = Map::new();
    for stage in STAGES {
        let xs: Vec<&crate::index::Row> = idx
            .summaries
            .iter()
            .filter(|r| r.summary.stage.as_deref() == Some(stage))
            .collect();
        let lim = q.int("limit", "60")?;
        let items: Vec<Value> = xs[py_slice(xs.len(), None, Some(lim))]
            .iter()
            .map(|r| {
                let x = &r.summary;
                json!({"id": x.id, "title": x.title, "short_name": x.short_name, "stage": x.stage, "sets": x.sets,
                       "hit_sets": x.hit_sets, "ai_hit_sets": x.ai_hit_sets, "versions": x.versions, "stage_ai": x.stage_ai})
            })
            .collect();
        out.insert(stage.to_string(), json!({"total": xs.len(), "items": items}));
    }
    Ok(ok(&json!({ "stages": out })))
}

/// `GET /api/_t9/summaries` (feature `diagnostics` only): every Bits-list row, in order, as `/api/jokes` would page
/// them before the creative layer adds `duration_s`.
#[cfg(feature = "diagnostics")]
pub async fn summaries(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, idx) = super::prepare(&st, &q).await?;
    let items: Vec<&pk_wire::api::JokeSummary> = idx.summaries.iter().map(|r| &r.summary).collect();
    Ok(ok(&json!({ "items": items })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_kind_in_three_words() {
        assert_eq!(by_kind(Some("owner"), "owner"), json!("owner"));
        assert_eq!(by_kind(Some("import:gsync"), "owner"), json!("import"));
        assert_eq!(by_kind(Some("ai:riff"), "owner"), json!("ai"));
        assert_eq!(by_kind(Some("someone"), "owner"), json!("ai"));
        assert_eq!(by_kind(Some(""), "owner"), Value::Null);
        assert_eq!(by_kind(None, "owner"), Value::Null);
    }

    #[test]
    fn dict_keys_as_json_dumps_writes_them() {
        assert_eq!(key_of(None), "null");
        assert_eq!(key_of(Some(&json!(true))), "true");
        assert_eq!(key_of(Some(&json!("idea"))), "idea");
    }
}
