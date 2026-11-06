//! Film it's two sessions (`creative_api.Routes.joke_rant` / `joke_hook`, `_cut_input`): `POST /api/jokes/<id>/rant
//! {unit, punch?, fresh?}` and `POST /api/jokes/<id>/hook {unit, exclude?, punch?, fresh?}` queue a `rant` / `hook`
//! job over one version of the bit as the KB holds it (never text the phone sends), split into the app's lines; the
//! version's cut is read from its source (`rant`, `rant_…`). The legal rules run on the version first.

use std::sync::LazyLock;

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_wire::worker::{HookInput, RantInput};
use regex::Regex;
use serde_json::{Map, Value};
use sha1::{Digest as _, Sha1};

use super::{Creative, Kb, Req, detached, http, is_id, refuse_legal, request};
use crate::auth::Caller;
use crate::error::{ApiResult, json_response};
use crate::state::AppState;

/// `CUT_RE = re.compile(r"^rant(_[a-z_]*)?$")` (`$` also before a final newline).
static CUT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\Arant(_[a-z_]*)?\n?\z").unwrap_or_else(|e| unreachable!("constant regex: {e}")));

/// `cut_of(source)`: `rant` for a rant source, else `stage`.
pub fn cut_of(source: Option<&Value>) -> &'static str {
    if source.and_then(Value::as_str).is_some_and(|s| CUT_RE.is_match(s)) {
        "rant"
    } else {
        "stage"
    }
}

/// `cut_lines(text)`: split on `\r?\n`, trailing whitespace dropped, blank lines left out.
pub fn cut_lines(text: &str) -> Vec<String> {
    text.split('\n')
        .map(|p| p.strip_suffix('\r').unwrap_or(p))
        .filter(|p| !py::strip(p).is_empty())
        .map(|p| p.trim_end_matches(py::isspace).to_string())
        .collect()
}

/// The version's input: `{joke_id, title, unit, cut, lines, punch}`.
async fn cut_input(st: &AppState, hidden: bool, jid: &str, body: &Map<String, Value>) -> ApiResult<RantInput> {
    if !is_id(jid) {
        return Err(http(400, "bad joke id"));
    }
    let unit = match body.get("unit") {
        Some(Value::String(u)) if is_id(u) => u.clone(),
        _ => return Err(http(400, "unit is required (the version to work on)")),
    };
    // under kb_lock as the Python reads its view under kb.lock (the `Kb` contract, writes/mod.rs) [review]
    let (_, view) = {
        let _kb = st.kb_lock().lock().await;
        Kb::new(st.clone(), hidden).view().await?
    };
    if view.ntype(Some(jid)) != Some("joke") {
        return Err(http(404, format!("no such joke {jid}")));
    }
    if !view
        .joke_units
        .get(jid)
        .is_some_and(|us| us.iter().any(|(u, _)| *u == unit))
    {
        return Err(http(404, "that version is not on this bit"));
    }
    let node = view.node(&unit);
    let text = node
        .and_then(|x| x.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let lines = cut_lines(&text);
    if lines.is_empty() {
        return Err(http(400, "that version has no lines"));
    }
    let punch = match body.get("punch") {
        None | Some(Value::Null) => None,
        Some(raw) => match raw
            .as_i64()
            .and_then(|x| u32::try_from(x).ok())
            .filter(|x| (*x as usize) < lines.len())
        {
            Some(x) => Some(x),
            None => return Err(http(400, "punch must be the index of a line of that version")),
        },
    };
    refuse_legal(st, [text.as_str()])?;
    let title = view
        .node(jid)
        .and_then(|j| j.get("title"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(RantInput {
        joke_id: jid.to_string(),
        title,
        unit,
        cut: cut_of(node.and_then(|x| x.get("source"))).to_string(),
        lines,
        punch,
    })
}

fn punch_key(p: Option<u32>) -> String {
    p.map_or_else(|| "-".to_string(), |x| x.to_string())
}

async fn queue(
    st: AppState,
    kind: &'static str,
    key: String,
    input: Value,
    fresh: bool,
) -> ApiResult<(StatusCode, Value)> {
    let owner = st.config().owner_id.clone();
    let _cr = st.creative_lock().lock().await;
    let cr = Creative::new(st.clone());
    super::jobs::enqueue(&cr, kind, &key, input, fresh, &owner).await
}

/// `POST /api/jokes/<id>/rant {unit, punch?, fresh?}`.
pub async fn rant(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(jid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q, b } = request(st, raw.as_deref(), &headers, body).await?;
    let inp = cut_input(&st, q.include_hidden(), &jid, &b).await?;
    let key = format!("rant|{}|{}|{}", inp.joke_id, inp.unit, punch_key(inp.punch));
    let input = serde_json::to_value(&inp).map_err(|e| http(500, e.to_string()))?;
    let fresh = py::truthy(b.get("fresh"));
    let (s, out) = detached(async move { queue(st, "rant", key, input, fresh).await }).await?;
    Ok(json_response(s, &out))
}

/// `POST /api/jokes/<id>/hook {unit, exclude?, punch?, fresh?}`.
pub async fn hook(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(jid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q, b } = request(st, raw.as_deref(), &headers, body).await?;
    let inp = cut_input(&st, q.include_hidden(), &jid, &b).await?;
    let bad = || http(400, "exclude must be a list of openers (at most 6)");
    let exclude: Vec<String> = match b.get("exclude") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(xs)) if xs.len() <= 6 => {
            let mut out = Vec::with_capacity(xs.len());
            for x in xs {
                match x.as_str().map(py::strip).filter(|s| !s.is_empty() && py::len(s) <= 300) {
                    Some(s) => out.push(s.to_string()),
                    None => return Err(bad()),
                }
            }
            out
        }
        Some(_) => return Err(bad()),
    };
    let tail = &hex::encode(Sha1::digest(exclude.join("\n").as_bytes()))[..10];
    let key = format!("hook|{}|{}|{}|{tail}", inp.joke_id, inp.unit, punch_key(inp.punch));
    let full = HookInput {
        joke_id: inp.joke_id,
        title: inp.title,
        unit: inp.unit,
        cut: inp.cut,
        lines: inp.lines,
        punch: inp.punch,
        exclude,
    };
    let input = serde_json::to_value(&full).map_err(|e| http(500, e.to_string()))?;
    let fresh = py::truthy(b.get("fresh"));
    let (s, out) = detached(async move { queue(st, "hook", key, input, fresh).await }).await?;
    Ok(json_response(s, &out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_cut_of_a_version_is_its_source() {
        for s in ["rant", "rant_session", "rant_hook", "rant\n"] {
            assert_eq!(cut_of(Some(&json!(s))), "rant", "{s}");
        }
        for s in [
            json!("app"),
            json!("hook"),
            json!("rants"),
            json!("rant-x"),
            json!(""),
            json!(3),
            Value::Null,
        ] {
            assert_eq!(cut_of(Some(&s)), "stage", "{s}");
        }
        assert_eq!(cut_of(None), "stage");
    }

    #[test]
    fn lines_are_the_apps_lines() {
        assert_eq!(cut_lines("a\r\n\n  b  \n \n"), ["a", "  b"]);
        assert_eq!(cut_lines("x\ry\nz"), ["x\ry", "z"]);
        assert!(cut_lines("   \n  ").is_empty());
    }
}
