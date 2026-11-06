//! The engine's store: [`pk_engine::EngineStore`] over the official `surrealdb` client, reading his material through
//! the tables and indexes `db/schema.surql` already defines (`unit.text`, `joke.title`, `joke.summary`,
//! `performance.text` full-text with the `punch_en` analyzer; the three bge HNSW indexes). One round trip per read:
//! the legs return record ids with their score or distance, then each distinct record is fetched once with its text
//! and its bit.
//!
//! Ported from upstream's `Store::retrieve` / `retrieve_facts` (`upstream-ops/src/store.rs`, Rust branch 9dfedad):
//! the any-word full-text query (each word its own `@n@` match, the scores summed, so a record with more and rarer
//! words ranks higher; `@@` with several words would require all of them), now async and over four fields instead
//! of one, plus the vector legs upstream does not have. Each full-text leg is ranked on its own index's scale, so
//! the four are fused by reciprocal rank (never by comparing two indexes' scores, upstream's cross-project lesson),
//! ties broken by the vector leg, never by table name; the vector legs share one scale (cosine distance) and are
//! merged by it.
//!
//! Hidden records (`hidden = true`: pasted text that is not his, `db/DB.md`) are read by no leg and never serve as a
//! lesson's evidence. The engine's own tables (`pk_engine::SCHEMA`) are applied only by
//! [`Store::apply_engine_schema`], never on connect.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use pk_engine::fact::{Checked, FactIn, Grade, Kind, check_fact};
use pk_engine::{EngineStore, Hit, Read, ReadQuery};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use surrealdb::types::{Array, Number, Object, Value};

use crate::error::{Result, StoreError};
use crate::ids::parse_node_rid;
use crate::store::{Store, vars};
use crate::value::from_db;

/// At most this many distinct words of a question go to the full-text legs (upstream's `query_words`), so one
/// read stays one statement per leg.
pub const MAX_WORDS: usize = 16;
/// The HNSW search breadth, as `api-dev/search_api.py` uses it.
pub const KNN_EF: usize = 150;
/// Reciprocal rank fusion's constant for the four full-text legs (`search::rrf`'s in `search_api.py`).
pub const TEXT_RRF_K: f64 = 60.0;
/// The RRF constant `search::rrf` is called with (`search_all`'s).
pub const FUSED_RRF_K: u32 = 60;
/// Edge statuses that do not link a record to a bit.
const DEAD: &str = "['rejected', 'ruled_out', 'superseded']";

/// The full-text legs, in order: (name, table, field, skips hidden rows).
const TEXT_LEGS: [(&str, &str, &str, bool); 4] = [
    ("ut", "unit", "text", true),
    ("jt", "joke", "title", true),
    ("js", "joke", "summary", true),
    ("pt", "performance", "text", false),
];
/// The vector legs, in order: (name, table, skips hidden rows).
const VECTOR_LEGS: [(&str, &str, bool); 3] = [("uv", "unit", true), ("jv", "joke", true), ("pv", "performance", false)];

/// The words a question is about (`pk_engine::rerank::question_words`: Ask's `QSTOP` and English function words
/// left out, once each), at most [`MAX_WORDS`].
pub fn query_words(question: &str) -> Vec<String> {
    let mut words = pk_engine::rerank::question_words(question);
    words.truncate(MAX_WORDS);
    words
}

/// One full-text leg: `field` of `table`, any word, scores summed, ids only. Names and numbers are this module's
/// constants; the words are bound.
fn text_leg(name: &str, table: &str, field: &str, hidden: bool, n: usize, depth: usize) -> String {
    if n == 0 {
        return format!("LET ${name} = [];");
    }
    let score = (1..=n)
        .map(|i| format!("search::score({i})"))
        .collect::<Vec<_>>()
        .join(" + ");
    let matches = (1..=n)
        .map(|i| format!("{field} @{i}@ $w{i}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let hid = if hidden { " AND hidden != true" } else { "" };
    format!(
        "LET ${name} = SELECT id, ({score}) AS score FROM {table} WHERE ({matches}){hid} \
         ORDER BY score DESC LIMIT {depth};"
    )
}

/// One vector leg over `table`'s `embedding`, ids only.
fn vector_leg(name: &str, table: &str, hidden: bool, depth: usize, vector: bool) -> String {
    if !vector {
        return format!("LET ${name} = [];");
    }
    let hid = if hidden { " AND hidden != true" } else { "" };
    format!(
        "LET ${name} = SELECT id, vector::distance::knn() AS distance FROM {table} \
         WHERE embedding <|{depth},{KNN_EF}|> $qvec{hid} ORDER BY distance;"
    )
}

/// What each table's records carry: the record, its words, its date, its bit and the bit's title.
fn unit_select() -> String {
    format!(
        "id, text, date, (->part_of[WHERE status NOT IN {DEAD}]->joke)[0] AS bit, \
         (->part_of[WHERE status NOT IN {DEAD}]->joke)[0].title AS title"
    )
}

const JOKE_SELECT: &str = "id, title, summary, first_seen AS date, id AS bit";

fn perf_select() -> String {
    format!(
        "id, text, name AS title, set.date AS date, \
         array::concat((->performed_as[WHERE status NOT IN {DEAD}]->joke), \
         (<-performed_as[WHERE status NOT IN {DEAD}]<-joke))[0] AS bit"
    )
}

/// The whole read as one batch: seven legs, `SurrealDB`'s own fusion of them (`search::rrf`, grouped as
/// `api-dev/search_api.py` `search_all` groups them: bits, then moments), the distinct ids per table, each record
/// once. The corpus count is not
/// here: three full scans cost ~110 ms a read on the owner's desktop (measured 2026-11-02), so it is
/// [`Store::corpus_size`], which a caller counts once and caches.
fn read_sql(n: usize, depth: usize, vector: bool) -> String {
    let mut sql = String::new();
    for (name, table, field, hidden) in TEXT_LEGS {
        sql.push_str(&text_leg(name, table, field, hidden, n, depth));
    }
    for (name, table, hidden) in VECTOR_LEGS {
        sql.push_str(&vector_leg(name, table, hidden, depth, vector));
    }
    let _ = write!(
        sql,
        "LET $uids = array::union($ut.id, $uv.id); \
         LET $jids = array::union(array::union($jt.id, $js.id), $jv.id); \
         LET $pids = array::union($pt.id, $pv.id); \
         LET $fb = search::rrf([$ut, $jt, $js, $uv, $jv], {depth}, {FUSED_RRF_K}); \
         LET $fm = search::rrf([$pt, $pv], {depth}, {FUSED_RRF_K}); \
         RETURN {{ legs: [$ut, $jt, $js, $pt, $uv, $jv, $pv], fused: [$fb, $fm], \
         units: (SELECT {} FROM $uids), jokes: (SELECT {JOKE_SELECT} FROM $jids), perfs: (SELECT {} FROM $pids) }};",
        unit_select(),
        perf_select()
    );
    sql
}

fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn str_of(row: &Json, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// A record row as a hit (no ranks yet). A joke's words are its typed idea, else its title.
fn hit_of(table: &str, row: &Json) -> Option<Hit> {
    let key = str_of(row, "id")?;
    let (text, title) = if table == "joke" {
        let title = str_of(row, "title");
        (str_of(row, "summary").or_else(|| title.clone())?, title)
    } else {
        (str_of(row, "text")?, str_of(row, "title"))
    };
    Some(Hit {
        hash: sha256_hex(&text),
        kind: Some(table.to_owned()),
        bit: str_of(row, "bit"),
        date: str_of(row, "date"),
        key,
        title,
        text,
        ..Hit::default()
    })
}

fn into_rows(v: Option<Json>) -> Vec<Json> {
    match v {
        Some(Json::Array(rows)) => rows,
        _ => Vec::new(),
    }
}

/// The legs merged over the fetched records: the four full-text legs fused by reciprocal rank (ties: nearer in the
/// vector leg, then key), the vector legs merged by distance (ties: key), and the store's given order (the better of
/// a record's two ranks, then the other, then key). A leg row whose record was not fetched is dropped.
fn merge(text_legs: &[Vec<Json>], vector_legs: &[Vec<Json>], mut records: HashMap<String, Hit>) -> Vec<Hit> {
    let mut fused: HashMap<String, (f64, Option<f64>)> = HashMap::new();
    for rows in text_legs {
        for (i, row) in rows.iter().enumerate() {
            let Some(key) = str_of(row, "id") else { continue };
            let e = fused.entry(key).or_insert((0.0, None));
            e.0 += 1.0 / (TEXT_RRF_K + i as f64 + 1.0);
            let score = row.get("score").and_then(Json::as_f64);
            if score.is_some_and(|s| e.1.is_none_or(|b| s > b)) {
                e.1 = score;
            }
        }
    }
    let mut near: Vec<(f64, String)> = vector_legs
        .iter()
        .flatten()
        .filter_map(|row| Some((row.get("distance").and_then(Json::as_f64)?, str_of(row, "id")?)))
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut vrank: HashMap<String, (usize, f64)> = HashMap::new();
    for (d, key) in near {
        let next = vrank.len() + 1;
        vrank.entry(key).or_insert((next, d));
    }
    let nearness = |k: &String| vrank.get(k).map_or(usize::MAX, |r| r.0);
    let mut text_order: Vec<(&String, &(f64, Option<f64>))> = fused.iter().collect();
    text_order.sort_by(|a, b| {
        b.1.0
            .total_cmp(&a.1.0)
            .then_with(|| nearness(a.0).cmp(&nearness(b.0)))
            .then_with(|| a.0.cmp(b.0))
    });
    let mut out: BTreeMap<String, Hit> = BTreeMap::new();
    for (i, (key, (_, score))) in text_order.into_iter().enumerate() {
        if let Some(mut h) = records.remove(key) {
            h.text_rank = Some(i + 1);
            h.score = *score;
            out.insert(key.clone(), h);
        }
    }
    for (key, (rank, d)) in &vrank {
        let h = match out.get_mut(key) {
            Some(h) => h,
            None => match records.remove(key) {
                Some(h) => out.entry(key.clone()).or_insert(h),
                None => continue,
            },
        };
        h.vector_rank = Some(*rank);
        h.distance = Some(*d);
    }
    let mut hits: Vec<Hit> = out.into_values().collect();
    hits.sort_by(|a, b| {
        let ranks = |h: &Hit| {
            let (t, v) = (h.text_rank.unwrap_or(usize::MAX), h.vector_rank.unwrap_or(usize::MAX));
            (t.min(v), t.max(v))
        };
        ranks(a).cmp(&ranks(b)).then_with(|| a.key.cmp(&b.key))
    });
    hits
}

/// The store's own fused order: `search::rrf`'s groups (bits, then moments) merged by `rrf_score`, ties to the
/// earlier group, then the earlier place; rank 1 = best.
fn fused_ranks(groups: Vec<Vec<Json>>) -> HashMap<String, usize> {
    let mut all: Vec<(f64, usize, usize, String)> = groups
        .into_iter()
        .enumerate()
        .flat_map(|(g, rows)| {
            rows.into_iter().enumerate().filter_map(move |(i, row)| {
                Some((
                    row.get("rrf_score").and_then(Json::as_f64).unwrap_or(0.0),
                    g,
                    i,
                    str_of(&row, "id")?,
                ))
            })
        })
        .collect();
    all.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut out = HashMap::new();
    for (_, _, _, key) in all {
        let next = out.len() + 1;
        out.entry(key).or_insert(next);
    }
    out
}

/// `$w1..$wn`, one per word.
fn words_vars(words: &[String]) -> Object {
    let mut o = Object::new();
    for (i, w) in words.iter().enumerate() {
        o.insert(format!("w{}", i + 1), Value::String(w.clone()));
    }
    o
}

fn grade_str(g: Grade) -> &'static str {
    match g {
        Grade::Unknown => "UNKNOWN",
        Grade::Inferred => "INFERRED",
        Grade::Confirmed => "CONFIRMED",
    }
}

fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Fact => "fact",
        Kind::Decision => "decision",
        Kind::Eliminated => "eliminated",
    }
}

impl Store {
    /// Apply the engine's tables (`pk_engine::SCHEMA`). Explicit only: never on connect, never by a migration runner;
    /// on the hosted database by his go, through the same path as `db/schema_extra.surql` (ADR 0009, ADR 0013).
    pub async fn apply_engine_schema(&self) -> Result<()> {
        self.query(pk_engine::SCHEMA, Object::new()).await?;
        Ok(())
    }

    /// How many records of his material a read can reach (units and bits not hidden, moments with text). Three full
    /// scans: count once and cache, never per read.
    pub async fn corpus_size(&self) -> Result<usize> {
        let v = from_db(
            self.last(
                "RETURN array::len((SELECT VALUE id FROM unit WHERE hidden != true)) \
                 + array::len((SELECT VALUE id FROM joke WHERE hidden != true)) \
                 + array::len((SELECT VALUE id FROM performance WHERE text != NONE));",
                Object::new(),
            )
            .await?,
        );
        v.as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| StoreError::mapping(format!("the corpus count is not a number: {v}")))
    }

    /// Run the evidence rule over a proposed lesson and write it. The quoted record's words are fetched here (a
    /// hidden record, pasted text that is not his, is refused as evidence), so the rule never trusts a caller's copy
    /// of his material. `signed`: his own signed-in session writes it (the API sets it from the session, never from
    /// the request). Returns the lesson's id.
    pub async fn add_lesson(&self, fact: &FactIn, signed: bool) -> Result<String> {
        let record_text = match &fact.quote {
            Some(q) => {
                let rid = parse_node_rid(&q.record)?;
                let row = from_db(
                    self.last(
                        "SELECT text, summary, title, hidden FROM ONLY $r;",
                        vars([("r", Value::RecordId(rid))]),
                    )
                    .await?,
                );
                if row.get("hidden").and_then(Json::as_bool) == Some(true) {
                    return Err(StoreError::Refused {
                        status: 409,
                        msg: format!(
                            "{} is hidden (not his words): it cannot be a lesson's evidence",
                            q.record
                        ),
                    });
                }
                str_of(&row, "text")
                    .or_else(|| str_of(&row, "summary"))
                    .or_else(|| str_of(&row, "title"))
            }
            None => None,
        };
        let checked = check_fact(fact, record_text.as_deref(), signed).map_err(|e| StoreError::Refused {
            status: 422,
            msg: e.to_string(),
        })?;
        self.write_lesson(&checked).await
    }

    async fn write_lesson(&self, c: &Checked) -> Result<String> {
        let mut sql =
            String::from("CREATE lesson SET text = $text, kind = $kind, grade = $grade, source = $source, by = $by");
        let mut pairs: Vec<(&str, Value)> = vec![
            ("text", Value::String(c.text().to_owned())),
            ("kind", Value::String(kind_str(c.kind()).to_owned())),
            ("grade", Value::String(grade_str(c.grade()).to_owned())),
            ("source", Value::String(c.source().to_owned())),
            ("by", Value::String(c.by().to_owned())),
            ("signed", Value::Bool(c.signed())),
        ];
        sql.push_str(", signed = $signed");
        if let Some(s) = c.owner_statement() {
            sql.push_str(", owner_statement = $stmt");
            pairs.push(("stmt", Value::String(s.to_owned())));
        }
        if let Some(q) = c.evidence() {
            sql.push_str(", evidence = $evidence, quote = $quote");
            pairs.push(("evidence", Value::RecordId(parse_node_rid(&q.record)?)));
            pairs.push(("quote", Value::String(q.text.clone())));
        }
        sql.push_str(" RETURN id;");
        let rows = self.write(&sql, vars(pairs), Some(1)).await?;
        let row = from_db(rows.into_iter().next().unwrap_or(Value::None));
        str_of(&row, "id").ok_or_else(|| StoreError::mapping("CREATE lesson returned no id"))
    }
}

impl EngineStore for Store {
    type Error = StoreError;

    async fn read(&self, q: ReadQuery<'_>) -> Result<Read> {
        let words = query_words(q.question);
        if words.is_empty() && q.vector.is_none() {
            return Ok(Read::default());
        }
        let mut bound = words_vars(&words);
        if let Some(v) = q.vector {
            let floats: Vec<Value> = v.iter().map(|x| Value::Number(Number::Float(f64::from(*x)))).collect();
            bound.insert("qvec", Value::Array(Array::from(floats)));
        }
        let sql = read_sql(words.len(), q.depth.max(1), q.vector.is_some());
        let Json::Object(mut out) = from_db(self.last(&sql, bound).await?) else {
            return Err(StoreError::mapping("the engine read returned no object"));
        };
        let mut legs = into_rows(out.remove("legs")).into_iter().map(|l| into_rows(Some(l)));
        let text: Vec<Vec<Json>> = legs.by_ref().take(TEXT_LEGS.len()).collect();
        let vector: Vec<Vec<Json>> = legs.collect();
        let mut records: HashMap<String, Hit> = HashMap::new();
        for (field, table) in [("units", "unit"), ("jokes", "joke"), ("perfs", "performance")] {
            for row in into_rows(out.remove(field)) {
                if let Some(h) = hit_of(table, &row) {
                    records.insert(h.key.clone(), h);
                }
            }
        }
        let fused = fused_ranks(
            into_rows(out.remove("fused"))
                .into_iter()
                .map(|g| into_rows(Some(g)))
                .collect(),
        );
        let mut hits = merge(&text, &vector, records);
        for h in &mut hits {
            h.fused_rank = fused.get(&h.key).copied();
        }
        // Not counted on the read path (see `read_sql`); a caller that reports "not read" uses `corpus_size`.
        Ok(Read { hits, corpus_total: 0 })
    }

    async fn lessons(&self, question: &str, k: usize) -> Result<Vec<Hit>> {
        let words = query_words(question);
        if words.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let n = words.len();
        let score = (1..=n)
            .map(|i| format!("search::score({i})"))
            .collect::<Vec<_>>()
            .join(" + ");
        let matches = (1..=n)
            .map(|i| format!("text @{i}@ $w{i}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!(
            "SELECT id, text, grade, evidence, ({score}) AS score, \
             (evidence.text ?? evidence.summary ?? evidence.title) AS etext FROM lesson \
             WHERE ({matches}) AND status = 'live' AND (evidence = NONE OR evidence.hidden != true) \
             ORDER BY score DESC LIMIT {k};"
        );
        let rows = from_db(self.last(&sql, words_vars(&words)).await?);
        let mut hits = Vec::new();
        for row in into_rows(Some(rows)) {
            let Some(lesson) = str_of(&row, "text") else { continue };
            // A lesson with record evidence is shown as that record (gone: left out); one resting on his own
            // statement is its own candidate. Either way the check never takes the lesson's text as his words
            // (`pk_engine::check::Rules::PUNCHABLE`).
            let (key, text, kind) = match (str_of(&row, "evidence"), str_of(&row, "etext")) {
                (Some(e), Some(t)) => (e, t, "evidence"),
                (Some(_), None) => continue,
                (None, _) => match str_of(&row, "id") {
                    Some(id) => (id, lesson.clone(), "lesson"),
                    None => continue,
                },
            };
            hits.push(Hit {
                hash: sha256_hex(&text),
                key,
                kind: Some(kind.to_owned()),
                text,
                score: row.get("score").and_then(Json::as_f64),
                fact: Some(lesson),
                fact_grade: str_of(&row, "grade"),
                ..Hit::default()
            });
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn question_words_are_distinct_lowercase_and_capped() {
        assert_eq!(query_words("The dog, the DOG's lawyer!"), ["dog", "s", "lawyer"]);
        let many = (0..40).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
        assert_eq!(query_words(&many).len(), MAX_WORDS);
        assert_eq!(query_words("  ...  ").len(), 0);
    }

    fn recs(keys: &[(&str, &str)]) -> HashMap<String, Hit> {
        keys.iter()
            .filter_map(|(table, key)| hit_of(table, &json!({"id": key, "text": key, "title": key})))
            .map(|h| (h.key.clone(), h))
            .collect()
    }

    #[test]
    fn text_legs_fuse_by_rank_and_ties_go_to_the_nearer_record_not_the_table() {
        // Each record leads its own leg (a tie on rank); the vector leg decides, not the table name.
        let text = vec![
            vec![json!({"id": "unit:a", "score": 3.0})],
            vec![json!({"id": "joke:j", "score": 9.0})],
            vec![],
            vec![json!({"id": "perf:p", "score": 1.0})],
        ];
        let vector = vec![
            vec![json!({"id": "unit:a", "distance": 0.1})],
            vec![],
            vec![json!({"id": "perf:p", "distance": 0.3})],
        ];
        let hits = merge(
            &text,
            &vector,
            recs(&[("unit", "unit:a"), ("joke", "joke:j"), ("performance", "perf:p")]),
        );
        let by_text: BTreeMap<&str, Option<usize>> = hits.iter().map(|h| (h.key.as_str(), h.text_rank)).collect();
        assert_eq!(by_text["unit:a"], Some(1), "nearest in the vector leg wins the tie");
        assert_eq!(by_text["perf:p"], Some(2));
        assert_eq!(
            by_text["joke:j"],
            Some(3),
            "no vector rank: last of the tie, whatever its table"
        );
        assert_eq!(hits[0].key, "unit:a");
        // A record two legs agree on beats one that leads a single leg.
        let text = vec![
            vec![
                json!({"id": "unit:x", "score": 1.0}),
                json!({"id": "unit:y", "score": 0.5}),
            ],
            vec![json!({"id": "joke:y2", "score": 1.0})],
            vec![json!({"id": "joke:y2", "score": 1.0})],
            vec![],
        ];
        let hits = merge(
            &text,
            &[],
            recs(&[("unit", "unit:x"), ("unit", "unit:y"), ("joke", "joke:y2")]),
        );
        assert_eq!(
            hits.iter().find(|h| h.key == "joke:y2").and_then(|h| h.text_rank),
            Some(1)
        );
    }

    #[test]
    fn a_leg_row_without_its_record_is_dropped_and_a_joke_reads_as_its_idea() {
        let text = vec![vec![json!({"id": "unit:gone", "score": 1.0})], vec![], vec![], vec![]];
        let mut records = HashMap::new();
        let k = hit_of("joke", &json!({"id": "joke:k", "title": "K", "summary": "k idea"})).expect("a joke");
        records.insert(k.key.clone(), k);
        let vector = vec![vec![], vec![json!({"id": "joke:k", "distance": 0.2})], vec![]];
        let hits = merge(&text, &vector, records);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].text.as_str(), hits[0].title.as_deref()), ("k idea", Some("K")));
        assert_eq!((hits[0].text_rank, hits[0].vector_rank), (None, Some(1)));
        assert_eq!(
            hit_of("joke", &json!({"id": "joke:t", "title": "T"}))
                .expect("titled")
                .text,
            "T"
        );
        assert_eq!(hits[0].hash.len(), 64);
    }

    #[test]
    fn the_stores_fusion_merges_bits_and_moments_by_rrf_score() {
        let bits = vec![
            json!({"id": "unit:a", "rrf_score": 0.03}),
            json!({"id": "joke:b", "rrf_score": 0.016}),
        ];
        let moments = vec![
            json!({"id": "perf:m", "rrf_score": 0.02}),
            json!({"id": "perf:n", "rrf_score": 0.016}),
        ];
        let r = fused_ranks(vec![bits, moments]);
        assert_eq!(
            (r["unit:a"], r["perf:m"], r["joke:b"], r["perf:n"]),
            (1, 2, 3, 4),
            "ties go to the bits"
        );
        assert!(fused_ranks(vec![]).is_empty());
    }

    #[test]
    fn the_sql_binds_words_and_names_only_constants() {
        let leg = text_leg("ut", "unit", "text", true, 2, 60);
        assert!(leg.contains("text @1@ $w1 OR text @2@ $w2"), "{leg}");
        assert!(leg.contains("search::score(1) + search::score(2)"));
        assert!(leg.ends_with("LIMIT 60;"));
        assert_eq!(text_leg("ut", "unit", "text", true, 0, 60), "LET $ut = [];");
        let v = vector_leg("pv", "performance", false, 30, true);
        assert!(v.contains("<|30,150|> $qvec") && !v.contains("hidden"), "{v}");
        let sql = read_sql(3, 60, true);
        assert!(
            sql.contains("FROM $uids") && !sql.contains("array::len"),
            "no full scan on the read path: {sql}"
        );
        assert!(!sql.contains("$w4"), "only the words given are referenced");
    }
}
