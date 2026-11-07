//! The venues port against `creative/venues.py` and `creative_api.venue_overlay`: `rust/tools/gen_creative_goldens.py`
//! runs the Python over the synthetic Ask notebook (five rooms, a geocoder cache with a far second match, an empty
//! answer, a match that is not a venue, a street address he supplied, and his pin moves and renames) and records
//! `parse_venue`, `venue_stats`, `build`, the overlay and `geocode_venue` for every room. These tests run the same
//! inputs through `pk_creative::venues` and compare as parsed JSON.

mod common;

use common::{golden, same};
use indexmap::IndexMap;
use pk_creative::corpus::{Corpus, Parts};
use pk_creative::venues::{Geocoder, build, geocode_venue, heat_color, parse_venue, venue_stats};
use serde_json::{Map, Value, json};

fn corpus() -> Corpus {
    let inp = golden("ask_inputs.json");
    let analyses: IndexMap<String, Value> = serde_json::from_value(inp["analyses"].clone()).expect("analyses");
    let decisions: Vec<Value> = serde_json::from_value(inp["decisions"].clone()).expect("decisions");
    Corpus::new(Parts {
        graph: inp["graph"].clone(),
        analyses,
        decisions,
        owner: "owner".into(),
        ..Parts::default()
    })
}

fn obj(v: &Value) -> Map<String, Value> {
    v.as_object().expect("an object").clone()
}

#[test]
fn room_names_become_the_pythons_queries() {
    let g = golden("venue_rooms.json");
    for raw in g["raws"].as_array().expect("raws") {
        let raw = raw.as_str().expect("a string");
        let p = parse_venue(raw);
        let want = &g["parsed"][raw];
        let got =
            json!({"name": p.name, "query": p.query, "queries": p.queries, "uncertain_reasons": p.uncertain_reasons});
        same(&format!("parse_venue({raw:?})"), &got, want);
    }
    for (k, v) in g["colors"].as_object().expect("colors") {
        let h: Option<f64> = k.parse().ok();
        assert_eq!(json!(heat_color(h)), *v, "heat_color({k})");
    }
}

#[test]
fn the_rooms_and_the_map_are_the_pythons() {
    let g = golden("venue_rooms.json");
    let c = corpus();
    let stats: Vec<Value> = venue_stats(&c).into_iter().map(Value::Object).collect();
    same("venue_stats", &Value::Array(stats), &g["stats"]);
    let cache = g["cache"].to_string();
    let addresses = obj(&g["addresses"]);
    let geo = Geocoder::from_text(Some(&cache));
    let built = build(&c, &geo, &addresses, "CACHE_PATH", "NOW");
    let whole = |b: &pk_creative::venues::Built| {
        let mut m = obj(&b.venues_body());
        m.insert("geojson".into(), b.geojson());
        Value::Object(m)
    };
    same("build", &whole(&built), &g["built"]);
    let edits = obj(&g["edits"]);
    same("overlay", &whole(&built.overlay(&edits)), &g["overlaid"]);
    for (raw, want) in g["geocoded"].as_object().expect("geocoded") {
        same(
            &format!("geocode_venue({raw:?})"),
            &Value::Object(geocode_venue(&geo, &addresses, raw)),
            want,
        );
    }
}
