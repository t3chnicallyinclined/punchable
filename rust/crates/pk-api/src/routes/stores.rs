//! Reads of one creative store with no notebook (T9 round 8): `GET /api/profile/voice` (`profile_api.view`, his
//! voice profile as data), `/api/suggestions/not-a-joke` (`suggest_api.items`: the memos the triage judge flagged,
//! not kept, still jokes in this request's view) and `/api/jobs/<id>` (`job_brief` + the result).

use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use serde_json::{Map, Value, json};

use super::creative::items_of;
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;

/// `memolink.NOTJOKE_MIN`, as the rule's words print it.
const NOTJOKE_MIN: &str = "0.75";
/// `VOICE_ID`.
const VOICE_ID: &str = "profile:voice";

/// `data.by_kind(by)`: `owner`, `import`, `ai`, or null when the row names nobody.
pub fn by_kind(by: Option<&Value>, owner: &str) -> Value {
    match by.and_then(Value::as_str).filter(|b| !b.is_empty()) {
        None => Value::Null,
        Some(b) if b == owner => json!("owner"),
        Some(b) if b.starts_with("import:") => json!("import"),
        Some(_) => json!("ai"),
    }
}

/// `profile_api.view(doc, worker=False)`.
pub(crate) fn voice_view(doc: Option<&Map<String, Value>>, owner: &str) -> Value {
    let text = doc.and_then(|d| d.get("text")).filter(|t| !t.is_null());
    let mut out = match (doc, text) {
        (Some(d), Some(t)) => {
            json!({"kind": d.get("kind"), "text": t, "updated_at": d.get("updated_at"), "by": d.get("by"),
                   "chars": t.as_str().map_or(0, |s| s.chars().count())})
        }
        _ => json!({"kind": "voice", "text": null, "updated_at": null, "by": null, "chars": 0}),
    };
    out["by_kind"] = by_kind(out.get("by"), owner);
    out["learned"] = doc
        .and_then(|d| d.get("learned"))
        .filter(|l| pk_core::py::truthy(Some(l)))
        .cloned()
        .unwrap_or_else(pk_creative::reducers::learned::empty);
    out["previous"] = match doc.and_then(|d| d.get("previous")) {
        Some(Value::Object(p)) => {
            let mut p = p.clone();
            p.insert("by_kind".into(), by_kind(p.get("by"), owner));
            Value::Object(p)
        }
        Some(v) => v.clone(),
        None => Value::Null,
    };
    out
}

/// `profile_api.view(doc, worker=True, laughs=...)`: the worker's copy: no `previous`, his laugh lines instead.
pub(crate) fn voice_worker_view(doc: Option<&Map<String, Value>>, owner: &str, laughs: Vec<Value>) -> Value {
    let mut out = voice_view(doc, owner);
    if let Some(m) = out.as_object_mut() {
        m.remove("previous");
        m.insert("laugh_lines".into(), Value::Array(laughs));
    }
    out
}

/// `GET /api/profile/voice`: his voice profile, its previous version (the app's Undo) and what was learned.
pub async fn voice(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    super::pick(&q)?;
    let items = items_of(&st, |s| &s.profile).await?;
    let doc = items.get(VOICE_ID).and_then(Value::as_object);
    Ok(ok(&voice_view(doc, &st.config().owner_id)))
}

/// `" ".join(text.split())`: Python's whitespace runs to one space, ends trimmed.
fn collapse(text: &str) -> String {
    text.split(pk_core::py::isspace)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `GET /api/suggestions/not-a-joke`: the "not a joke?" list, most confident first.
pub async fn not_a_joke(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    // `kb.view()`: this request's view (hidden bits are not in it unless asked), no Bits-list rows
    let (d, view) = super::prepare_view(&st, &q).await?;
    let kept = items_of(&st, |s| &s.suggestions).await?;
    // a jot on a topic is not also "to sort" (the Rant tab, spec §12.7), the same rule as `/api/tonight/screen`'s count
    let topics = items_of(&st, |s| &s.topics).await?;
    let held = pk_creative::reducers::topics::held_jots(&topics);
    let judge = super::creative::judge_text(&st, &d).await;
    let judged: Value = judge
        .as_deref()
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or(Value::Null);
    let mut out: Vec<Value> = Vec::new();
    if let Some(items) = judged.get("items").and_then(Value::as_object) {
        for (memo, x) in items {
            let Some(x) = x.as_object() else { continue };
            if !super::creative::flagged(x) {
                continue;
            }
            let jid = x.get("joke_id").and_then(Value::as_str).filter(|j| !j.is_empty());
            let Some((jid, j)) = jid.and_then(|j| view.nodes.get(j).map(|n| (j, n))) else {
                continue;
            };
            if j.get("type").and_then(Value::as_str) != Some("joke") || kept.contains_key(jid) || held.contains(jid) {
                continue;
            }
            let text = collapse(j.get("summary").and_then(Value::as_str).unwrap_or(""));
            let short: String = text.chars().take(400).collect();
            let more = if text.chars().count() > 400 { "…" } else { "" };
            out.push(
                json!({"joke_id": jid, "memo": memo, "title": j.get("title"), "text": format!("{short}{more}"),
                            "kind": x.get("kind"), "why": x.get("why"), "confidence": x.get("confidence"),
                            "source": "inferred"}),
            );
        }
    }
    let conf = |r: &Value| {
        pk_creative::corpus::num(r.get("confidence").filter(|c| pk_core::py::truthy(Some(c)))).unwrap_or(0.0)
    };
    out.sort_by(|a, b| {
        conf(b)
            .total_cmp(&conf(a))
            .then_with(|| a["joke_id"].as_str().cmp(&b["joke_id"].as_str()))
    });
    let n = out.len();
    Ok(ok(&json!({"items": out, "count": n,
                  "rule": format!("a voice memo Claude reads as a conversation or a note (confidence >= {NOTJOKE_MIN}); \
                                   nothing is hidden until you tap Hide")})))
}

use crate::writes::jobs::waiting_note;

/// `GET /api/jobs/<id>`: one job's brief and its result.
pub async fn job(State(st): State<AppState>, Path(jid): Path<String>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    super::pick(&q)?;
    let items = items_of(&st, |s| &s.jobs).await?;
    let j = items
        .get(&jid)
        .and_then(Value::as_object)
        .ok_or_else(|| ApiError::not_found("no such job"))?;
    let mut out = Map::new();
    for k in ["id", "kind", "status", "created_at", "done_at", "attempts", "error"] {
        out.insert(k.into(), j.get(k).cloned().unwrap_or(Value::Null));
    }
    if let Some(w) = j.get("waiting_for").filter(|w| pk_core::py::truthy(Some(w))) {
        out.insert("waiting_for".into(), w.clone());
        out.insert("note".into(), w.as_str().map_or(Value::Null, waiting_note));
    }
    out.insert("result".into(), j.get("result").cloned().unwrap_or(Value::Null));
    Ok(ok(&Value::Object(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_kind_is_owner_import_ai_or_nobody() {
        assert_eq!(by_kind(Some(&json!("owner")), "owner"), json!("owner"));
        assert_eq!(by_kind(Some(&json!("import:profile")), "owner"), json!("import"));
        assert_eq!(by_kind(Some(&json!("ai:learn")), "owner"), json!("ai"));
        assert_eq!(by_kind(Some(&json!("")), "owner"), Value::Null);
        assert_eq!(by_kind(None, "owner"), Value::Null);
    }

    #[test]
    fn an_empty_profile_has_the_python_shape() {
        let v = voice_view(None, "owner");
        assert_eq!(
            v,
            json!({"kind": "voice", "text": null, "updated_at": null, "by": null, "chars": 0, "by_kind": null,
                   "learned": {"kept": [], "dropped": [], "updated_at": null}, "previous": null})
        );
        let d = json!({"kind": "voice", "text": "héllo", "updated_at": "t", "by": "owner",
                       "previous": {"text": "x", "updated_at": "s", "by": "import:profile"}});
        let v = voice_view(d.as_object(), "owner");
        assert_eq!(v["chars"], 5);
        assert_eq!(v["previous"]["by_kind"], "import");
    }
}
