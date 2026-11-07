//! Venues and the map of performances (`creative/venues.py`, T10 round 5): the rooms he has played, each with its
//! sets, what worked there and what worked elsewhere, and a pin for the map. The pins come from a geocoder's cache
//! only: the hosted service never sends a query (`PK_GEOCODE` unset), so a room whose query is not in the cache is
//! listed as "not geocoded yet". Fetching from the network is the Python's `Geocoder._http` and is not ported; the
//! cache is the `srcfile` row `kb/creative/geocode_cache.json` and the street addresses he supplied are
//! `kb/creative/venue_addresses.json`. An address row with no `address` is ignored here (the Python fails with a
//! `KeyError` on it).
//!
//! Output is built as JSON values in the Python's key order, because the routes hand it to the app as it is.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::corpus::{Corpus, Perf, num, round_to, s, venue_short};
use crate::util::slug;
use pk_core::py;

/// `USER_AGENT`.
pub const USER_AGENT: &str = "Punchable/0.1 (personal)";
/// `UNCERTAIN`: words that say the set's metadata guessed the room.
const UNCERTAIN: [&str; 5] = ["unknown", "likely", "maybe", "(show)", "?"];
/// `VENUE_TYPES`.
const VENUE_TYPES: [&str; 14] = [
    "theatre",
    "arts_centre",
    "nightclub",
    "bar",
    "pub",
    "restaurant",
    "cafe",
    "casino",
    "events_venue",
    "community_centre",
    "music_venue",
    "comedy_club",
    "hotel",
    "resort",
];

static MAYBE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)maybe ([^)'"]+)"#).unwrap_or_else(|e| unreachable!("constant regex: {e}")));
static LIKELY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"likely ([A-Za-z ]+)").unwrap_or_else(|e| unreachable!("constant regex: {e}")));

/// `STATES`.
fn state(place: &str) -> &str {
    match place {
        "NYC" => "New York, NY",
        "NY" => "New York",
        "CT" => "Connecticut",
        "NJ" => "New Jersey",
        other => other,
    }
}

/// What `parse_venue` makes of a raw room name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedVenue {
    /// The room's name.
    pub name: String,
    /// The most specific query.
    pub query: String,
    /// Every query to try, most specific first, none twice.
    pub queries: Vec<String>,
    /// Why the name is a guess, when it is.
    pub uncertain_reasons: Vec<String>,
}

/// `x[n:]` on code points.
fn from_char(x: &str, n: usize) -> String {
    x.chars().skip(n).collect()
}

/// `parse_venue(raw)`: "Mabel's Comedy Club, Springfield" to a name and the queries to look it up by.
pub fn parse_venue(raw: &str) -> ParsedVenue {
    let v = py::strip(raw);
    let low = v.to_lowercase();
    let mut reasons = Vec::new();
    if UNCERTAIN.iter().any(|u| low.contains(u)) {
        reasons.push(format!("venue name is a guess in the set metadata: '{v}'"));
    }
    let (mut name, mut query);
    if let Some(m) = MAYBE.captures(v) {
        name = py::strip(&m[1]).to_string();
        query = name.clone();
    } else {
        let base = py::strip(v.split('(').next().unwrap_or(""));
        let parts: Vec<&str> = base.split(',').map(py::strip).filter(|p| !p.is_empty()).collect();
        name = parts.first().map_or_else(|| v.to_string(), |p| (*p).to_string());
        let mut place = parts.get(1).map_or_else(String::new, |p| (*p).to_string());
        if place.is_empty() {
            place = LIKELY
                .captures(v)
                .map_or_else(String::new, |m| py::strip(&m[1]).to_string());
        }
        let place = state(&place).to_string();
        query = if place.is_empty() {
            name.clone()
        } else {
            format!("{name}, {place}")
        };
    }
    if name.to_lowercase().starts_with("comix at ") {
        name = from_char(&name, 9);
        query = from_char(&query, 9);
    }
    let mut queries = vec![query.clone()];
    if let Some((_, rest)) = query.split_once(',') {
        let place = py::strip(rest);
        queries.push(format!("{name}, {}", py::strip(place.split(',').next().unwrap_or(""))));
    }
    queries.push(name.clone());
    let mut seen = Vec::new();
    for q in queries {
        if !seen.contains(&q) {
            seen.push(q);
        }
    }
    ParsedVenue {
        name,
        query,
        queries: seen,
        uncertain_reasons: reasons,
    }
}

/// The geocoder's cache, read only (`Geocoder.cache`, `Geocoder.search(query, offline=True)`).
#[derive(Debug, Clone, Default)]
pub struct Geocoder {
    cache: Map<String, Value>,
}

impl Geocoder {
    /// Over the cache file's text; no file, or a file that is not an object, is an empty cache.
    pub fn from_text(text: Option<&str>) -> Self {
        let cache = text
            .and_then(|t| serde_json::from_str::<Value>(t).ok())
            .and_then(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .unwrap_or_default();
        Self { cache }
    }

    /// `search(query, offline=True)`: the cached results, or `None` when the query was never sent.
    pub fn search(&self, query: &str) -> Option<Vec<Value>> {
        let hit = self.cache.get(query)?;
        Some(
            hit.get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        )
    }
}

/// `_km(a, b)`: the great-circle distance between two results.
fn km(a: &Value, b: &Value) -> f64 {
    let f = |v: &Value, k: &str| num(v.get(k)).unwrap_or(0.0).to_radians();
    let (la1, lo1, la2, lo2) = (f(a, "lat"), f(a, "lon"), f(b, "lat"), f(b, "lon"));
    let h = ((la2 - la1) / 2.0).sin().powi(2) + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    6371.0 * 2.0 * h.sqrt().asin()
}

/// `geocode_venue(geo, raw, offline=True)`: where the cache says a room is, and why that may be wrong.
pub fn geocode_venue(geo: &Geocoder, addresses: &Map<String, Value>, raw: &str) -> Map<String, Value> {
    let mut pv = parse_venue(raw);
    if let Some(addr) = addresses
        .get(&pv.name.to_lowercase())
        .filter(|a| py::truthy(Some(a)))
        .and_then(|a| a.get("address"))
        .map(py::str)
    {
        let mut qs = vec![addr.clone()];
        for q in &pv.queries {
            if !qs.contains(q) {
                qs.push(q.clone());
            }
        }
        pv.queries = qs;
        pv.query = addr;
    }
    let (mut res, mut used, mut tried) = (None, pv.query.clone(), Vec::new());
    for q in &pv.queries {
        res = geo.search(q);
        used.clone_from(q);
        tried.push(q.clone());
        if res.as_ref().is_some_and(|r| !r.is_empty()) {
            break;
        }
    }
    let mut out = Map::new();
    out.insert("raw".into(), json!(raw));
    out.insert("name".into(), json!(pv.name));
    out.insert("query".into(), json!(used));
    out.insert("queries_tried".into(), json!(tried));
    out.insert("status".into(), json!("inferred"));
    out.insert("source".into(), json!("nominatim"));
    for k in ["lat", "lon", "display_name"] {
        out.insert(k.into(), Value::Null);
    }
    let mut reasons: Vec<String> = pv.uncertain_reasons;
    let finish = |mut out: Map<String, Value>, reasons: Vec<String>, amb: bool| {
        out.insert("ambiguous".into(), json!(amb));
        out.insert("ambiguous_reasons".into(), json!(reasons));
        out
    };
    let Some(res) = res else {
        reasons.push("not geocoded yet (offline)".into());
        return finish(out, reasons, true);
    };
    if res.is_empty() {
        let quoted: Vec<String> = tried.iter().map(|q| py::quote(q)).collect();
        reasons.push(format!("Nominatim found nothing for {}", quoted.join(" / ")));
        return finish(out, reasons, true);
    }
    if used == pv.name && pv.queries.len() > 1 {
        reasons.push(format!("matched only on the bare name '{used}'"));
    }
    let top = &res[0];
    let get = |k: &str| top.get(k).cloned().unwrap_or(Value::Null);
    out.insert("lat".into(), get("lat"));
    out.insert("lon".into(), get("lon"));
    out.insert("display_name".into(), get("display_name"));
    out.insert(
        "osm".into(),
        json!(format!("{}/{}", py::str(&get("osm_type")), py::str(&get("osm_id")))),
    );
    out.insert("osm_type".into(), get("type"));
    let far: Vec<&Value> = res[1..].iter().filter(|r| km(top, r) > 2.0).collect();
    if let Some(first) = far.first() {
        let name: String = py::str(&first.get("display_name").cloned().unwrap_or(Value::Null))
            .chars()
            .take(80)
            .collect();
        reasons.push(format!(
            "{} other match(es) more than 2 km away (e.g. {name})",
            far.len()
        ));
    }
    let ty = s_of(top, "type");
    let cat = s_of(top, "category");
    if !ty.is_some_and(|t| VENUE_TYPES.contains(&t))
        && !cat.is_some_and(|c| matches!(c, "amenity" | "leisure" | "tourism"))
    {
        reasons.push(format!(
            "top match is a {}/{}, not a venue",
            py::str(&get("category")),
            py::str(&get("type"))
        ));
    }
    let amb = !reasons.is_empty();
    finish(out, reasons, amb)
}

fn s_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// Python's float comparison: `-0.0` equals `0.0`.
fn fcmp(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

/// `mean(xs)` over the numbers that are not none.
fn mean(xs: impl IntoIterator<Item = Option<f64>>) -> Option<f64> {
    let xs: Vec<f64> = xs.into_iter().flatten().collect();
    (!xs.is_empty()).then(|| xs.iter().sum::<f64>() / xs.len() as f64)
}

/// `r2(x)`.
fn r2(x: Option<f64>) -> Value {
    x.map_or(Value::Null, |x| json!(round_to(x, 2)))
}

/// `Perf.venue` as the string the Python keys its dicts by.
fn venue_of(p: &Perf) -> String {
    py::str(&p.venue)
}

/// `heat_color(h)`.
pub fn heat_color(h: Option<f64>) -> &'static str {
    match h {
        None => "#9ca3af",
        Some(h) if h >= 1.2 => "#22c55e",
        Some(h) if h >= 0.8 => "#f5b544",
        Some(_) => "#fca5a5",
    }
}

/// `venue_stats(c)`: one entry per room, rooms in name order.
#[allow(clippy::too_many_lines)]
pub fn venue_stats(c: &Corpus) -> Vec<Map<String, Value>> {
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for k in c.set_keys() {
        by.entry(py::str(&c.sets[&k].venue)).or_default().push(k);
    }
    // per joke: its nights that have a room rate, by room
    let mut jv: IndexMap<&str, IndexMap<String, Vec<&Perf>>> = IndexMap::new();
    for (jid, ps) in &c.perfs {
        for p in ps.iter().filter(|p| p.vs_room.is_some()) {
            jv.entry(jid.as_str())
                .or_default()
                .entry(venue_of(p))
                .or_default()
                .push(p);
        }
    }
    let rank = |x: &(&str, &Perf)| {
        let p = x.1;
        let evidence: Vec<Value> = c
            .seg_evidence(&p.main_seg)
            .into_iter()
            .filter(|e| py::truthy(Some(e)))
            .collect();
        json!({"joke_id": x.0, "title": c.title(x.0), "date": p.date, "vs_room": p.vs_room,
               "laugh_s_per_min": p.laugh_s_per_min, "ai_rating": p.ai_rating, "evidence": evidence})
    };
    let mut out = Vec::new();
    for (venue, keys) in &by {
        let mut sets = Vec::new();
        for k in keys {
            let info = &c.sets[k];
            let sm = |f: &str| info.summary.get(f).cloned().unwrap_or(Value::Null);
            let mapped = |f: &str| {
                let b = sm(f);
                if py::truthy(Some(&b)) {
                    json!(c.map_joke(&py::str(&b)))
                } else {
                    b
                }
            };
            sets.push(json!({"set": k, "date": info.date, "title": info.title,
                             "laugh_s_per_min": sm("laugh_s_per_min"), "room_heat": sm("room_heat"),
                             "pct_laughing": sm("pct_laughing"), "material_s": sm("material_s"),
                             "best": mapped("best"), "weakest": mapped("weakest")}));
        }
        let mut here: Vec<(&str, &Perf)> = jv
            .iter()
            .flat_map(|(jid, vs)| vs.get(venue).into_iter().flatten().map(move |p| (*jid, *p)))
            .collect();
        let vs = |x: &(&str, &Perf)| x.1.vs_room.unwrap_or(0.0);
        here.sort_by(|a, b| fcmp(-vs(a), -vs(b)).then_with(|| a.0.cmp(b.0)));
        let mut diff: Vec<(f64, String, Value)> = Vec::new();
        for (jid, per) in &jv {
            if per.contains_key(venue) && per.len() > 1 {
                // vs_room clipped at 3.0: a near-silent room inflates it
                let clip = |p: &&Perf| p.vs_room.map(|v| v.min(3.0));
                let h = mean(per[venue].iter().map(clip));
                let el = mean(
                    per.iter()
                        .filter(|(v, _)| *v != venue)
                        .flat_map(|(_, ps)| ps.iter().map(clip)),
                );
                let n_else: usize = per.iter().filter(|(v, _)| *v != venue).map(|(_, ps)| ps.len()).sum();
                let (h, el) = (h.unwrap_or(0.0), el.unwrap_or(0.0));
                let delta = round_to(h - el, 2);
                diff.push((
                    delta,
                    (*jid).to_string(),
                    json!({"joke_id": jid, "title": c.title(jid), "here_vs_room": round_to(h, 2),
                           "elsewhere_vs_room": round_to(el, 2), "delta": delta, "n_here": per[venue].len(),
                           "n_elsewhere": n_else, "low_n": per[venue].len() < 2 || n_else < 2}),
                ));
            }
        }
        diff.sort_by(|a, b| fcmp(-a.0, -b.0).then_with(|| a.1.cmp(&b.1)));
        let works_here: Vec<Value> = diff.iter().filter(|d| d.0 > 0.2).take(5).map(|d| d.2.clone()).collect();
        let mut ascending: Vec<&(f64, String, Value)> = diff.iter().collect();
        ascending.sort_by(|a, b| fcmp(a.0, b.0).then_with(|| a.1.cmp(&b.1)));
        let works_elsewhere: Vec<Value> = ascending
            .iter()
            .filter(|d| d.0 < -0.2)
            .take(5)
            .map(|d| d.2.clone())
            .collect();
        let mut worst = here.clone();
        worst.sort_by(|a, b| fcmp(vs(a), vs(b)).then_with(|| a.0.cmp(b.0)));
        let name = parse_venue(venue).name;
        let num_of = |sets: &[Value], k: &str| mean(sets.iter().map(|s| num(s.get(k))));
        let mut v = Map::new();
        v.insert("venue_id".into(), json!(format!("venue:{}", slug(&name, 40))));
        v.insert("raw".into(), json!(venue));
        v.insert("name".into(), json!(name));
        v.insert(
            "dates".into(),
            json!(sets.iter().map(|s| s["date"].clone()).collect::<Vec<_>>()),
        );
        v.insert("n_sets".into(), json!(sets.len()));
        v.insert("avg_laugh_s_per_min".into(), r2(num_of(&sets, "laugh_s_per_min")));
        v.insert("avg_room_heat".into(), r2(num_of(&sets, "room_heat")));
        v.insert("sets".into(), Value::Array(sets));
        v.insert(
            "best_jokes".into(),
            Value::Array(here.iter().take(3).map(&rank).collect()),
        );
        v.insert(
            "worst_jokes".into(),
            Value::Array(worst.iter().take(3).map(&rank).collect()),
        );
        v.insert("works_here".into(), Value::Array(works_here));
        v.insert("works_elsewhere".into(), Value::Array(works_elsewhere));
        v.insert(
            "note".into(),
            json!("vs_room compares a joke with the rest of that night's set; one set per room is a small sample."),
        );
        out.push(v);
    }
    out
}

/// What `build(c, geo, offline=True)` makes, before his edits.
#[derive(Debug, Clone)]
pub struct Built {
    /// `generated_at`.
    pub generated_at: String,
    /// The rooms, each with its `geo`.
    pub venues: Vec<Map<String, Value>>,
    /// The pins that are guesses, with why.
    pub ambiguous: Vec<Value>,
    /// The map's features.
    pub features: Vec<Value>,
    /// `geocoder.cache`: where the Python's cache file is.
    pub cache_path: String,
}

/// `build(c, geo, offline=True)`.
pub fn build(c: &Corpus, geo: &Geocoder, addresses: &Map<String, Value>, cache_path: &str, now: &str) -> Built {
    let mut venues = venue_stats(c);
    let (mut features, mut ambiguous) = (Vec::new(), Vec::new());
    for v in &mut venues {
        let raw = s(v, "raw").unwrap_or("").to_string();
        let g = geocode_venue(geo, addresses, &raw);
        let amb = py::truthy(g.get("ambiguous"));
        let get = |k: &str| g.get(k).cloned().unwrap_or(Value::Null);
        if amb {
            ambiguous.push(
                json!({"venue_id": v["venue_id"], "raw": raw, "reasons": get("ambiguous_reasons"),
                                  "picked": get("display_name")}),
            );
        }
        let geometry = if get("lat").is_null() {
            Value::Null
        } else {
            json!({"type": "Point", "coordinates": [get("lon"), get("lat")]})
        };
        features.push(json!({
            "type": "Feature", "geometry": geometry,
            "properties": {"venue_id": v["venue_id"], "name": v["name"], "raw": raw, "status": get("status"),
                           "ambiguous": amb, "n_sets": v["n_sets"], "dates": v["dates"],
                           "avg_laugh_s_per_min": v["avg_laugh_s_per_min"], "avg_room_heat": v["avg_room_heat"],
                           "color": heat_color(num(v.get("avg_room_heat"))),
                           "label": venue_short(Some(&raw), None)}}));
        v.insert("geo".into(), Value::Object(g));
    }
    Built {
        generated_at: now.to_string(),
        venues,
        ambiguous,
        features,
        cache_path: cache_path.to_string(),
    }
}

impl Built {
    /// `venue_overlay(built, edits)`: his pin moves and renames on top of the geocoder's inferred pins. `edits` is the
    /// `venues` store's items.
    #[must_use]
    pub fn overlay(mut self, edits: &Map<String, Value>) -> Self {
        let edit = |id: &str| edits.get(id).and_then(Value::as_object).filter(|e| !e.is_empty());
        let moved = |e: &Map<String, Value>| !e.get("lat").is_none_or(Value::is_null);
        for f in &mut self.features {
            let id = f["properties"]["venue_id"].as_str().unwrap_or("").to_string();
            let Some(e) = edit(&id) else { continue };
            if moved(e) {
                f["geometry"] = json!({"type": "Point", "coordinates": [e.get("lon"), e.get("lat")]});
                f["properties"]["status"] = json!("his");
                f["properties"]["ambiguous"] = json!(false);
            }
            if py::truthy(e.get("name")) {
                f["properties"]["name"] = e["name"].clone();
                f["properties"]["label"] = e["name"].clone();
            }
            f["properties"]["edited_at"] = e.get("updated_at").cloned().unwrap_or(Value::Null);
        }
        for v in &mut self.venues {
            let id = s(v, "venue_id").unwrap_or("").to_string();
            let Some(e) = edit(&id) else { continue };
            let g = v.get("geo").and_then(Value::as_object).cloned().unwrap_or_default();
            if moved(e) {
                let mut g = g;
                g.insert("lat".into(), e["lat"].clone());
                g.insert("lon".into(), e.get("lon").cloned().unwrap_or(Value::Null));
                g.insert("status".into(), json!("his"));
                g.insert("ambiguous".into(), json!(false));
                v.insert("geo".into(), Value::Object(g));
            }
            if py::truthy(e.get("name")) {
                v.insert("name".into(), e["name"].clone());
            }
            let edited: Map<String, Value> = ["lat", "lon", "name", "updated_at"]
                .iter()
                .map(|k| ((*k).to_string(), e.get(*k).cloned().unwrap_or(Value::Null)))
                .collect();
            v.insert("edited".into(), Value::Object(edited));
        }
        let moved_ids: Vec<&String> = edits
            .iter()
            .filter(|(_, e)| e.as_object().is_some_and(moved))
            .map(|(k, _)| k)
            .collect();
        self.ambiguous
            .retain(|a| !moved_ids.iter().any(|m| Some(m.as_str()) == a["venue_id"].as_str()));
        self
    }

    /// The body of `GET /api/venues`.
    pub fn venues_body(&self) -> Value {
        json!({"generated_at": self.generated_at, "venues": self.venues, "ambiguous": self.ambiguous,
               "geocoder": {"service": "OpenStreetMap Nominatim", "user_agent": USER_AGENT, "requests_this_run": 0,
                            "cache": self.cache_path, "attribution": "Data \u{a9} OpenStreetMap contributors, ODbL"}})
    }

    /// The body of `GET /api/venues.geojson`.
    pub fn geojson(&self) -> Value {
        json!({"type": "FeatureCollection", "features": self.features})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_name_becomes_its_queries() {
        let p = parse_venue("Mabel's Comedy Club, Springfield");
        assert_eq!(p.name, "Mabel's Comedy Club");
        assert_eq!(p.queries, ["Mabel's Comedy Club, Springfield", "Mabel's Comedy Club"]);
        let p = parse_venue("Joe's (likely Brooklyn) ");
        assert_eq!((p.name.as_str(), p.query.as_str()), ("Joe's", "Joe's, Brooklyn"));
        assert_eq!(p.queries, ["Joe's, Brooklyn", "Joe's"]);
        assert!(p.uncertain_reasons[0].starts_with("venue name is a guess"));
        let p = parse_venue("Comix at Harbor, NYC");
        assert_eq!(p.name, "Harbor");
        assert_eq!(p.query, "Harbor, New York, NY");
        let p = parse_venue("maybe The Loft)");
        assert_eq!((p.name.as_str(), p.queries.len()), ("The Loft", 1));
    }

    #[test]
    fn heat_has_three_colours_and_a_grey() {
        assert_eq!(heat_color(None), "#9ca3af");
        assert_eq!(heat_color(Some(1.2)), "#22c55e");
        assert_eq!(heat_color(Some(0.8)), "#f5b544");
        assert_eq!(heat_color(Some(0.79)), "#fca5a5");
    }
}
