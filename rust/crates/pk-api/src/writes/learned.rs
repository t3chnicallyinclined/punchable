//! What his writes teach the voice profile (`profile_api.learn_kept`): a draft he kept as his own becomes one
//! `learn` event on `profile:voice` (the line kept, the drafts he passed over beside it), written on the write he
//! already made. Never an error: his save stands either way.

use std::collections::HashSet;

use pk_creative::reducers::learned::{counts, drafted, norm};
use serde_json::{Map, Value, json};

use super::Creative;
use crate::error::ApiResult;

/// `VOICE_ID`: the one profile document.
pub const VOICE_ID: &str = "profile:voice";

/// `learn_kept(cr, kb, text, bit, by, draft)`: the counts after the event, or `None` when nothing was written (his
/// own typed line, unflagged) or anything failed (logged, never answered).
pub async fn learn_kept(cr: &Creative, text: &str, bit: Option<&str>, by: &str, draft: bool) -> Option<Value> {
    match learn(cr, text, bit, by, draft).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("the learned section was not written: {e}");
            None
        }
    }
}

async fn learn(cr: &Creative, text: &str, bit: Option<&str>, by: &str, draft: bool) -> ApiResult<Option<Value>> {
    let jobs = cr.items("jobs").await?;
    let (kind, others) = drafted(&jobs, text);
    if kind.is_none() && !draft {
        return Ok(None);
    }
    let profile = cr.items("profile").await?;
    let learned = profile
        .get(VOICE_ID)
        .and_then(|d| d.get("learned"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    // a sibling he kept or dismissed earlier is already a signal of its own: only lines he never acted on
    let seen: HashSet<String> = ["kept", "dropped"]
        .iter()
        .filter_map(|k| learned.get(*k))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|e| e.get("text").and_then(Value::as_str))
        .map(norm)
        .collect();
    let others: Vec<&str> = others
        .iter()
        .map(String::as_str)
        .filter(|t| !seen.contains(&norm(t)))
        .collect();
    let bit = bit.map_or(Value::Null, |b| json!(b));
    let mut kept = Map::new();
    kept.insert("text".into(), json!(text));
    kept.insert("bit".into(), bit.clone());
    if kind.is_none() {
        kept.insert("edited".into(), json!(true));
    }
    let dropped: Vec<Value> = others.iter().map(|t| json!({"text": t, "bit": bit})).collect();
    let from = kind.as_deref().unwrap_or("draft_flag");
    let doc = cr
        .append(
            "profile",
            "learn",
            VOICE_ID,
            json!({"kept": [Value::Object(kept)], "dropped": dropped, "from": from}),
            by,
        )
        .await?;
    Ok(Some(counts(doc.get("learned"))))
}
