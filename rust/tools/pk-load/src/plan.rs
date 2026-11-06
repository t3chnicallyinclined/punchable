//! What one run asks for: a fixed, weighted mix of the app's own GET routes (the paths `app/src/lib` calls, with the
//! query strings it sends), over ids found on the server first, plus two kinds of writes when asked. The mix is
//! deterministic (no randomness): request `i` takes template `i mod 40` and an id chosen by a stride, so two runs
//! over the same data ask for the same things in the same order, and a Python run and a Rust run are comparable.

use serde_json::Value;

/// One request, or one short write sequence, in the plan.
#[derive(Debug, Clone)]
pub enum Step {
    /// A read: `GET /api/<segments...>?<query>`.
    Get {
        /// The route group the report rolls it into (`bits`, `search`, ...).
        group: &'static str,
        /// The route as a template (`/api/jokes/{id}`): what the report lists per row.
        route: &'static str,
        /// The path segments after `/api`, each percent-encoded when the URL is built.
        segments: Vec<String>,
        /// The query pairs, encoded when the URL is built.
        query: Vec<(&'static str, String)>,
    },
    /// `POST /api/ideas`: a typed jot, the commonest write the app makes.
    Jot {
        /// Which jot of the run (its words differ per jot).
        n: usize,
    },
    /// A set list's life: `POST /api/boards`, then `PATCH` archive, unarchive and delete (the board ends deleted).
    BoardCycle {
        /// Which cycle of the run.
        n: usize,
        /// Two bits to put on the list.
        jokes: Vec<String>,
    },
}

/// The ids the reads are spread over, read from the server before the run.
#[derive(Debug, Default, Clone)]
pub struct Ids {
    /// Bit ids (`joke:...`), from `GET /api/jokes`.
    pub jokes: Vec<String>,
    /// Taped set ids, from `GET /api/sets`.
    pub sets: Vec<String>,
    /// Live set list ids, from `GET /api/boards`.
    pub boards: Vec<String>,
}

impl Ids {
    /// The `id` strings of the array under `key` in `body` (`{"jokes": [{"id": ...}, ...]}`); none when the shape
    /// differs.
    pub fn of(body: &Value, key: &str) -> Vec<String> {
        body.get(key)
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Which id a template needs.
#[derive(Debug, Clone, Copy)]
enum Need {
    None,
    Joke,
    Set,
    Board,
    Query,
}

/// What a template's builder returns: the path segments after `/api` and the query pairs.
type Built = (Vec<String>, Vec<(&'static str, String)>);

/// One read template: group, route, how often per 40 requests, the id it needs and how to build it.
struct Template {
    group: &'static str,
    route: &'static str,
    weight: usize,
    need: Need,
    build: fn(&str) -> Built,
}

fn segs(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

/// The read mix: weights sum to 40; the routes and query strings are the app's (`app/src/lib/*.ts`).
const TEMPLATES: &[Template] = &[
    Template {
        group: "health",
        route: "/api/health",
        weight: 2,
        need: Need::None,
        build: |_| (segs(&["health"]), vec![]),
    },
    Template {
        group: "tonight",
        route: "/api/today",
        weight: 2,
        need: Need::None,
        build: |_| (segs(&["today"]), vec![]),
    },
    Template {
        group: "tonight",
        route: "/api/tonight/screen",
        weight: 3,
        need: Need::None,
        build: |_| (segs(&["tonight", "screen"]), vec![]),
    },
    Template {
        group: "bits",
        route: "/api/jokes?sort=recent&limit=500",
        weight: 3,
        need: Need::None,
        build: |_| {
            (
                segs(&["jokes"]),
                vec![("sort", "recent".into()), ("limit", "500".into())],
            )
        },
    },
    Template {
        group: "bits",
        route: "/api/jokes/{id}",
        weight: 7,
        need: Need::Joke,
        build: |id| (segs(&["jokes", id]), vec![]),
    },
    Template {
        group: "bits",
        route: "/api/jokes/{id}/connections",
        weight: 2,
        need: Need::Joke,
        build: |id| (segs(&["jokes", id, "connections"]), vec![("cap", "12".into())]),
    },
    Template {
        group: "bits",
        route: "/api/merge/candidates?joke={id}",
        weight: 1,
        need: Need::Joke,
        build: |id| (segs(&["merge", "candidates"]), vec![("joke", id.into())]),
    },
    Template {
        group: "search",
        route: "/api/search?q={q}",
        weight: 3,
        need: Need::Query,
        build: |q| (segs(&["search"]), vec![("q", q.into()), ("limit", "6".into())]),
    },
    Template {
        group: "search",
        route: "/api/jokes?q={q}&mode=hybrid",
        weight: 2,
        need: Need::Query,
        build: |q| {
            (
                segs(&["jokes"]),
                vec![("q", q.into()), ("mode", "hybrid".into()), ("limit", "10".into())],
            )
        },
    },
    Template {
        group: "search",
        route: "/api/jokes/{id}/similar",
        weight: 2,
        need: Need::Joke,
        build: |id| (segs(&["jokes", id, "similar"]), vec![("limit", "5".into())]),
    },
    Template {
        group: "search",
        route: "/api/notebook/related?q={q}",
        weight: 1,
        need: Need::Query,
        build: |q| {
            (
                segs(&["notebook", "related"]),
                vec![("q", q.into()), ("cap", "10".into())],
            )
        },
    },
    Template {
        group: "sets",
        route: "/api/sets",
        weight: 2,
        need: Need::None,
        build: |_| (segs(&["sets"]), vec![]),
    },
    Template {
        group: "sets",
        route: "/api/sets/{id}/analysis",
        weight: 2,
        need: Need::Set,
        build: |id| (segs(&["sets", id, "analysis"]), vec![]),
    },
    Template {
        group: "sets",
        route: "/api/mysets",
        weight: 1,
        need: Need::None,
        build: |_| (segs(&["mysets"]), vec![]),
    },
    Template {
        group: "sets",
        route: "/api/boards",
        weight: 2,
        need: Need::None,
        build: |_| (segs(&["boards"]), vec![]),
    },
    Template {
        group: "sets",
        route: "/api/boards/{id}",
        weight: 1,
        need: Need::Board,
        build: |id| (segs(&["boards", id]), vec![]),
    },
    Template {
        group: "profile",
        route: "/api/profile/voice",
        weight: 1,
        need: Need::None,
        build: |_| (segs(&["profile", "voice"]), vec![]),
    },
    Template {
        group: "profile",
        route: "/api/memos",
        weight: 1,
        need: Need::None,
        build: |_| (segs(&["memos"]), vec![]),
    },
    Template {
        group: "profile",
        route: "/api/settings",
        weight: 1,
        need: Need::None,
        build: |_| (segs(&["settings"]), vec![]),
    },
    Template {
        group: "map",
        route: "/api/map/cloud?cap=1600 (warm)",
        weight: 1,
        need: Need::None,
        build: |_| (segs(&["map", "cloud"]), vec![("cap", "1600".into())]),
    },
];

/// The route a template falls back to when this server has none of the ids it needs.
const FALLBACK_ROUTE: &str = "/api/jokes?sort=recent&limit=500";

/// One write per this many requests when writes are on (jots and board cycles alternate).
pub const WRITE_EVERY: usize = 20;

/// The plan: `n` steps over `ids` and `queries`; with `writes`, every [`WRITE_EVERY`]th step is a write instead.
/// A template whose ids are missing on this server (no sets, no boards) falls back to the bits list, so the count
/// stays `n`; the report says which routes ran.
pub fn build(ids: &Ids, queries: &[String], n: usize, writes: bool) -> Vec<Step> {
    let cycle: Vec<&Template> = TEMPLATES
        .iter()
        .flat_map(|t| std::iter::repeat_n(t, t.weight))
        .collect();
    let Some(fallback) = TEMPLATES.iter().find(|t| t.route == FALLBACK_ROUTE) else {
        return Vec::new();
    };
    let (mut jots, mut boards) = (0, 0);
    let mut uses = vec![0usize; cycle.len()];
    (0..n)
        .map(|i| {
            if writes && i % WRITE_EVERY == WRITE_EVERY - 1 && !ids.jokes.is_empty() {
                let k = i / WRITE_EVERY;
                return if k.is_multiple_of(2) {
                    jots += 1;
                    Step::Jot { n: jots }
                } else {
                    boards += 1;
                    let a = &ids.jokes[k % ids.jokes.len()];
                    let b = &ids.jokes[(k * 7 + 3) % ids.jokes.len()];
                    Step::BoardCycle {
                        n: boards,
                        jokes: vec![a.clone(), b.clone()],
                    }
                };
            }
            // spread the id choice with a stride co-prime to typical list lengths
            let slot = i % cycle.len();
            let t = cycle[slot];
            // each template walks its own id list in a fixed stride, so a route meets as many distinct ids as it can
            let k = uses[slot];
            uses[slot] += 1;
            let pick = |list: &[String]| (!list.is_empty()).then(|| list[(k * 31 + slot * 7) % list.len()].clone());
            let arg = match t.need {
                Need::None => Some(String::new()),
                Need::Joke => pick(&ids.jokes),
                Need::Set => pick(&ids.sets),
                Need::Board => pick(&ids.boards),
                Need::Query => pick(queries),
            };
            let (t, arg) = match arg {
                Some(a) => (t, a),
                None => (fallback, String::new()),
            };
            let (segments, query) = (t.build)(&arg);
            Step::Get {
                group: t.group,
                route: t.route,
                segments,
                query,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Ids {
        Ids {
            jokes: (0..50).map(|i| format!("joke:j{i}")).collect(),
            sets: vec!["2024-01-01_x".into()],
            boards: vec![],
        }
    }

    #[test]
    fn the_weights_make_a_cycle_of_forty() {
        assert_eq!(TEMPLATES.iter().map(|t| t.weight).sum::<usize>(), 40);
    }

    #[test]
    fn the_plan_is_deterministic_and_the_right_length() {
        let q = vec!["dog".to_string()];
        let a = build(&ids(), &q, 400, true);
        let b = build(&ids(), &q, 400, true);
        assert_eq!(a.len(), 400);
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        let writes = a.iter().filter(|s| !matches!(s, Step::Get { .. })).count();
        assert_eq!(writes, 400 / WRITE_EVERY);
        assert!(
            build(&ids(), &q, 400, false)
                .iter()
                .all(|s| matches!(s, Step::Get { .. }))
        );
    }

    #[test]
    fn a_missing_id_list_falls_back_to_the_bits_list() {
        let plan = build(&ids(), &[], 40, false);
        assert!(plan.iter().all(|s| match s {
            Step::Get { route, .. } => !route.contains("{q}") && *route != "/api/boards/{id}",
            _ => false,
        }));
    }

    #[test]
    fn ids_are_read_from_the_list_shape() {
        let body = serde_json::json!({"jokes": [{"id": "joke:a"}, {"id": 3}, {"title": "x"}, {"id": "joke:b"}]});
        assert_eq!(Ids::of(&body, "jokes"), vec!["joke:a", "joke:b"]);
        assert_eq!(Ids::of(&body, "sets").len(), 0);
    }
}
