//! `GET /api/jokes/<id>/around?cap=6` (`docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.1; Rust only, no Python
//! twin): everything tied to one bit in five groups, his words on every card, and the sets the bit is in. One read
//! that joins what the page used to ask for one by one or not at all: the merge judge's group and the closest other
//! bits (`/api/merge/candidates?joke=`), the connection groups (`/api/jokes/<id>/connections`), what sounds like it
//! (`/api/jokes/<id>/similar`), the notes filed under it (the `merges` store), the newest night's different wording
//! (the notebook's `script_diff`), the set lists (`storyboards`) and the pairs he said are not connected (`links`).
//! The join itself is `pk_creative::around`; this module fetches, in the order of refusals the other id pages keep
//! (the data, `?backend=`, the cap, the id, the bit).
//!
//! "What sounds like it" needs the model or a word search: when that leg fails the page still answers with the rest
//! (it is a suggestion, like `/api/notebook/related`'s fused search).

use std::collections::HashSet;

use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use pk_creative::around::{Bit, Sources, build, wording};
use pk_creative::merge::{MergeState, candidates_for};
use pk_kb::View;
use serde_json::Value;

use super::creative::items_of;
use super::sets_detail::digits_in;
use crate::error::{ApiError, ApiResult, ok};
use crate::index::AppIndex;
use crate::query::Query;
use crate::search::{Like, moment_item, performed};
use crate::state::AppState;
use crate::writes::film::{cut_lines, cut_of};
use crate::writes::rant::versions_of;
use crate::writes::{Creative, is_id};

/// How many neighbours the "sounds like it" leg asks for.
const LIKE: i64 = 8;
/// How many of the judge's closest other bits are read.
const OTHERS: usize = 5;

/// What a bit says now, as its page shows it: the newest stage version (a rant cut is another cut of the bit), else
/// the newest version of any cut; the unit and its lines. None for a bit with no written version.
pub fn newest_version(v: &View, jid: &str) -> Option<(String, Vec<String>)> {
    let vs: Vec<(&str, Vec<String>, &'static str)> = versions_of(v, jid)
        .into_iter()
        .map(|(u, n)| {
            (
                u,
                cut_lines(n.get("text").and_then(Value::as_str).unwrap_or("")),
                cut_of(n.get("source")),
            )
        })
        .filter(|(_, lines, _)| !lines.is_empty())
        .collect();
    vs.iter()
        .rev()
        .find(|(_, _, cut)| *cut == "stage")
        .or_else(|| vs.last())
        .map(|(u, lines, _)| ((*u).to_string(), lines.clone()))
}

/// "killed twice", "3 nights", "never on stage": the app's own words (`app/src/rules/bitLines.ts`).
fn meta_of(nights: u64, killed: u64) -> String {
    match (killed, nights) {
        (1, _) => "killed once".into(),
        (2, _) => "killed twice".into(),
        (k, _) if k > 2 => format!("killed {k} times"),
        (_, 0) => "never on stage".into(),
        (_, 1) => "1 night".into(),
        (_, n) => format!("{n} nights"),
    }
}

/// Another bit as a card shows it, or none for one this reader's view does not hold.
pub fn bit_of(idx: &AppIndex, jid: &str) -> Option<Bit> {
    let v = &idx.view;
    if v.ntype(Some(jid)) != Some("joke") {
        return None;
    }
    let n = v.node(jid)?;
    let row = idx.by_id.get(jid).and_then(|i| idx.summaries.get(*i));
    let title = n.get("title").and_then(Value::as_str).unwrap_or("");
    // the page's own cut: the title up to its first parenthesis
    let title = title.split(" (").next().unwrap_or(title).to_string();
    let (lines, written) = match newest_version(v, jid) {
        Some((_, lines)) => (lines, true),
        None => (cut_lines(n.get("summary").and_then(Value::as_str).unwrap_or("")), false),
    };
    let (nights, killed) = row.map_or((0, 0), |r| (r.summary.sets, r.summary.hit_sets));
    Some(Bit {
        title,
        lines,
        written,
        meta: meta_of(nights, killed),
    })
}

/// `GET /api/jokes/<id>/around[?cap=6]`.
pub async fn around(
    State(st): State<AppState>,
    Path(jid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, v) = super::prepare_view(&st, &q).await?;
    let cap = digits_in(q.get("cap").unwrap_or("6"), 1, 20, "cap must be 1-20")?;
    let cap = usize::try_from(cap).unwrap_or(20);
    if !is_id(&jid) {
        return Err(ApiError::bad_request("bad joke id"));
    }
    if v.ntype(Some(&jid)) != Some("joke") {
        return Err(ApiError::not_found("no such joke"));
    }
    let hidden = q.include_hidden();
    // the two halves do not wait for each other: the neighbours are a read of the database (and the model), the rest
    // is this generation's notebook and the stores
    let like = super::search::like_of(&st, &v, &jid, LIKE, hidden);
    let rest = async {
        let idx = d.index(hidden).await?;
        let c = d.corpus(&st).await?;
        let ix = d.merge_index(&st).await?;
        let (events, merges) = Creative::new(st.clone()).lines_and_items("merges").await?;
        let groups = d.merge_groups(&st, events.len(), &merges).await?;
        let boards = items_of(&st, |s| &s.storyboards).await?;
        let links = items_of(&st, |s| &s.links).await?;
        Ok::<_, ApiError>((idx, c, ix, merges, groups, boards, links))
    };
    let (like, rest) = tokio::join!(like, rest);
    let (idx, c, ix, merges, groups, boards, links) = rest?;
    let like = like.unwrap_or_else(|e| {
        tracing::warn!("around: what sounds like it failed: {}; the rest is answered", e.msg);
        Like::default()
    });
    let body = super::merge::blocking(&st, move || {
        let state = MergeState::from_items(&merges);
        let merge = candidates_for(&ix, &jid, &groups.judge, &state, OTHERS, groups.list());
        let conns = pk_creative::connections::joke_connections(&c.story(), &c, &jid, cap);
        let said = wording(&c, &jid);
        // the moments that sound like it and are not this bit's already
        let linked: HashSet<String> = performed(&v, &jid)
            .iter()
            .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
            .collect();
        let moments: Vec<Value> = like
            .moments
            .iter()
            .filter(|(pid, _)| !linked.contains(pid))
            .filter_map(|(pid, _)| moment_item(&v, pid, Value::Null))
            .map(Value::Object)
            .collect();
        let like_bits: Vec<String> = like.bits.iter().map(|(j, _)| j.clone()).collect();
        let newest = newest_version(&v, &jid);
        let none = Vec::new();
        let src = Sources {
            joke: &jid,
            unit: newest.as_ref().map(|(u, _)| u.as_str()),
            lines: newest.as_ref().map_or(&none, |(_, lines)| lines),
            connections: conns.as_ref(),
            merge: &merge,
            like_bits: &like_bits,
            like_moments: &moments,
            wording: said.as_ref(),
            boards: &boards,
            links: &links,
            merges: &state,
            cap,
        };
        build(&src, &|j| bit_of(&idx, j))
    })
    .await?;
    Ok(ok(&body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view() -> View {
        let g = json!({"version": 1, "nodes": [
            {"id": "joke:a", "type": "joke", "title": "Smart fridge (kitchen)", "summary": "The fridge tells me things"},
            {"id": "joke:b", "type": "joke", "title": "Ice maker", "summary": "The ice maker judges me.\nI can hear it."},
            {"id": "unit:a1", "type": "unit", "text": "My fridge has wifi.\n\nI know.  ", "date": "2026-01-02"},
            {"id": "unit:a2", "type": "unit", "text": "MY FRIDGE HAS WIFI!", "date": "2026-02-02", "source": "rant_session"},
            {"id": "unit:a0", "type": "unit", "text": "fridge, first pass", "date": "2025-01-01"},
            {"id": "unit:a3", "type": "unit", "text": "   ", "date": "2026-03-03"},
        ], "edges": [
            {"id": "e:1", "src": "unit:a1", "dst": "joke:a", "kind": "part_of", "status": "confirmed"},
            {"id": "e:2", "src": "unit:a2", "dst": "joke:a", "kind": "part_of", "status": "confirmed"},
            {"id": "e:3", "src": "unit:a0", "dst": "joke:a", "kind": "part_of", "status": "confirmed"},
            {"id": "e:4", "src": "unit:a3", "dst": "joke:a", "kind": "part_of", "status": "confirmed"},
        ]});
        View::new(&g, Vec::new(), Vec::new(), "owner")
    }

    #[test]
    fn the_newest_version_is_the_newest_stage_cut_with_words() {
        let v = view();
        assert_eq!(
            newest_version(&v, "joke:a"),
            Some(("unit:a1".into(), vec!["My fridge has wifi.".into(), "I know.".into()])),
            "not the newer rant cut, not the empty one"
        );
        assert_eq!(newest_version(&v, "joke:b"), None, "a typed idea has no version");
    }

    #[test]
    fn the_meta_uses_the_apps_words() {
        assert_eq!(meta_of(0, 0), "never on stage");
        assert_eq!(meta_of(1, 0), "1 night");
        assert_eq!(meta_of(5, 0), "5 nights");
        assert_eq!(meta_of(5, 1), "killed once");
        assert_eq!(meta_of(5, 2), "killed twice");
        assert_eq!(meta_of(5, 3), "killed 3 times");
    }
}
