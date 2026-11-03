//! The Desk's list rules (`server.py` `GET /api/jokes`): the filters (`q` over `View.search`, `stage`, every
//! `label=`, `who`), the five sorts of `sort_jokes` with their tie rules, and the page bounds. Pure functions over the
//! index rows; the route adds the creative layer (the attached notes it hides, each row's duration).

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use pk_wire::api::JokeSummary;

use crate::error::ApiError;
use crate::index::{AppIndex, Row};
use crate::query::Query;

/// `MAX_JOKES_PAGE`: the Desk pages 500 at a time; a whole-catalogue page is allowed.
pub const MAX_JOKES_PAGE: i64 = 5000;
/// `JOKE_SORTS`.
pub const JOKE_SORTS: [&str; 5] = ["recent", "performed", "strongest", "written", "untouched"];

/// The rows the filters keep, in the Desk's order (`items` before `desk_filter`). `ranked` is the fused search's
/// answer for `?q=&mode=hybrid` (joke ids, best first): only those rows, in that order; otherwise `q` is the
/// view's word search, in the Desk's order.
pub fn filtered<'a>(idx: &'a AppIndex, q: &Query, ranked: Option<&[String]>) -> Vec<&'a Row> {
    let mut items: Vec<&Row> = idx.summaries.iter().collect();
    let qs = pk_core::py::strip(q.get("q").unwrap_or(""));
    if let Some(ranked) = ranked {
        let rank: HashMap<&str, usize> = ranked.iter().enumerate().map(|(i, j)| (j.as_str(), i)).collect();
        items.retain(|x| rank.contains_key(x.summary.id.as_str()));
        items.sort_by_key(|x| rank[x.summary.id.as_str()]);
    } else if !qs.is_empty() {
        let hits: HashSet<String> = idx.view.search(qs).into_iter().collect();
        items.retain(|x| hits.contains(&x.summary.id));
    }
    if let Some(stage) = q.get("stage") {
        items.retain(|x| x.summary.stage.as_deref() == Some(stage));
    }
    // repeated `label=`: every one must be on the joke
    for lab in q.all("label") {
        items.retain(|x| x.label_ids.iter().any(|l| l == lab));
    }
    if let Some(who @ ("ai" | "confirmed" | "edited")) = q.get("who") {
        items.retain(|x| x.summary.who == who);
    }
    items
}

/// `s[:n]` over characters.
fn head(s: Option<&str>, n: usize) -> &str {
    let s = s.unwrap_or("");
    s.char_indices().nth(n).map_or(s, |(i, _)| &s[..i])
}

/// `recent_key`: the newest of his last touch, the newest version, the last night on stage and the first day.
/// Date-only values compare correctly against same-day timestamps.
pub fn recent_key(x: &JokeSummary) -> &str {
    [
        head(x.touched.as_deref(), 19),
        head(x.last_seen.as_deref(), 10),
        head(x.last_performed.as_deref(), 10),
        head(x.first_seen.as_deref(), 10),
    ]
    .into_iter()
    .max()
    .unwrap_or("")
}

fn opt(s: Option<&String>) -> &str {
    s.map_or("", String::as_str)
}

/// `(last_performed or "", last_seen or "")`.
fn performed_key(x: &JokeSummary) -> (&str, &str) {
    (opt(x.last_performed.as_ref()), opt(x.last_seen.as_ref()))
}

/// `sort_jokes(items, how)`: the Desk's sorts (`UX_SPEC` 4.2). Missing values go last; ties by title then id, so
/// pages are stable. Every sort is the tie order `(title.lower(), id)` first (each key computed once), then a stable
/// sort on the sort's own key: equal keys keep the tie order, which is what Python's stable `sorted` gives both for
/// `reverse = True` over a tie-sorted list and for a key that ends in the tie.
pub fn sort_jokes(items: &mut [&Row], how: &str) -> Result<(), ApiError> {
    if !JOKE_SORTS.contains(&how) {
        return Err(ApiError::bad_request(format!(
            "sort must be one of {}",
            JOKE_SORTS.join(", ")
        )));
    }
    items.sort_by_cached_key(|r| (r.summary.title.to_lowercase(), r.summary.id.clone()));
    match how {
        "recent" => items.sort_by(|a, b| recent_key(&b.summary).cmp(recent_key(&a.summary))),
        "performed" => items.sort_by(|a, b| performed_key(&b.summary).cmp(&performed_key(&a.summary))),
        // `(avg is None, -(avg or 0), -(nights or 0), *tie)`
        "strongest" => items.sort_by(|a, b| {
            let (a, b) = (&a.summary, &b.summary);
            a.avg_vs_room
                .is_none()
                .cmp(&b.avg_vs_room.is_none())
                .then_with(|| {
                    let (x, y) = (a.avg_vs_room.unwrap_or(0.0), b.avg_vs_room.unwrap_or(0.0));
                    y.partial_cmp(&x).unwrap_or(Ordering::Equal)
                })
                .then_with(|| b.nights.cmp(&a.nights))
        }),
        "written" => items.sort_by(|a, b| opt(b.summary.last_seen.as_ref()).cmp(opt(a.summary.last_seen.as_ref()))),
        // `(not last_seen, last_seen or "", *tie)`
        _ => items.sort_by(|a, b| {
            let (x, y) = (opt(a.summary.last_seen.as_ref()), opt(b.summary.last_seen.as_ref()));
            x.is_empty().cmp(&y.is_empty()).then_with(|| x.cmp(y))
        }),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, title: &str, f: impl FnOnce(&mut JokeSummary)) -> Row {
        let mut s: JokeSummary = serde_json::from_value(serde_json::json!({
            "id": id, "title": title, "stage": "idea", "summary": "", "versions": 0, "sets": 0, "hit_sets": 0,
            "ai_hit_sets": 0, "to_review": 0, "labels": [], "yours": false, "stage_ai": false, "who": "ai",
            "short_title": title, "short_name": title, "nights": 0,
        }))
        .unwrap();
        f(&mut s);
        Row {
            summary: s,
            label_ids: Vec::new(),
        }
    }

    fn ids(xs: &[&Row]) -> Vec<String> {
        xs.iter().map(|r| r.summary.id.clone()).collect()
    }

    #[test]
    fn recent_takes_the_newest_of_four_and_keeps_ties_by_title() {
        let rows = [
            row("j:1", "b", |s| s.last_seen = Some("2024-01-01".into())),
            row("j:2", "A", |s| s.touched = Some("2024-01-01T10:00:00.123+00:00".into())),
            row("j:3", "a", |s| s.last_seen = Some("2024-01-01".into())),
            row("j:4", "z", |_| {}),
        ];
        let mut xs: Vec<&Row> = rows.iter().collect();
        sort_jokes(&mut xs, "recent").unwrap();
        // the timestamp is newer than its own date; equal keys keep the tie order (title, then id)
        assert_eq!(ids(&xs), ["j:2", "j:3", "j:1", "j:4"]);
        assert_eq!(recent_key(&rows[1].summary), "2024-01-01T10:00:00");
    }

    #[test]
    fn strongest_and_untouched_put_missing_values_last() {
        let rows = [
            row("j:1", "a", |_| {}),
            row("j:2", "b", |s| {
                s.avg_vs_room = Some(1.2);
                s.nights = 1;
            }),
            row("j:3", "c", |s| {
                s.avg_vs_room = Some(1.2);
                s.nights = 3;
            }),
            row("j:4", "d", |s| s.avg_vs_room = Some(-0.5)),
        ];
        let mut xs: Vec<&Row> = rows.iter().collect();
        sort_jokes(&mut xs, "strongest").unwrap();
        assert_eq!(ids(&xs), ["j:3", "j:2", "j:4", "j:1"]);
        let rows = [
            row("j:1", "a", |_| {}),
            row("j:2", "b", |s| s.last_seen = Some("2023-01-01".into())),
            row("j:3", "c", |s| s.last_seen = Some("2022-01-01".into())),
        ];
        let mut xs: Vec<&Row> = rows.iter().collect();
        sort_jokes(&mut xs, "untouched").unwrap();
        assert_eq!(ids(&xs), ["j:3", "j:2", "j:1"]);
        assert_eq!(
            sort_jokes(&mut xs, "best").unwrap_err().msg,
            "sort must be one of recent, performed, strongest, written, untouched"
        );
    }

    #[test]
    fn head_counts_characters() {
        assert_eq!(head(Some("été-2024-01-01"), 3), "été");
        assert_eq!(head(None, 3), "");
        assert_eq!(head(Some("ab"), 10), "ab");
    }
}
