//! KB dicts and `SurrealDB` rows, both ways, exactly as `db/kbmap.py` maps them, so the Rust API serves the same
//! dicts the files backend did: a node is `<table from its id prefix>:<key>` plus its fields with the private replay
//! markers `_foo` stored as `x_foo`; an edge is `<kind>:<hash>` with `src`/`dst` as `in`/`out`; a decision line is a
//! `decision` row with `id` as `target_id` and any unknown key under `extra`; a laugh is a `laugh` row keyed by
//! `[set, start_ms, ord]`. DB-only fields (`ord`, `working`, `working_sets`, `embedding`, `embedded_sha`, `run_id`)
//! never appear in a KB dict. `NULL` stays `null`; `NONE` (absent) stays absent.

use pk_kb::{Decision, EdgeId, NodeId, NodeKind, Rec};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use surrealdb::types::{Array, Number, Object, RecordId, Value};

use crate::error::{Result, StoreError};
use crate::ids::{EDGE_TABLES, key_str, node_kind_of_table, node_rid, parse_node_rid, rid_str, table_of};
use crate::value::{as_i64, as_str, from_db, to_db};

/// Fields a node row carries that are not part of the KB node.
pub const DB_ONLY_NODE: [&str; 5] = ["ord", "working", "working_sets", "embedding", "embedded_sha"];
/// Fields an edge row carries that are not part of the KB edge.
pub const DB_ONLY_EDGE: [&str; 2] = ["ord", "run_id"];

/// The keys of a decision line that have their own column, in the order `row_to_decision` writes them back.
pub const DECISION_KEYS: [&str; 9] = [
    "ts", "target", "id", "action", "before", "after", "reason", "by", "undo_of",
];

fn priv_out(k: &str) -> String {
    k.strip_prefix('_')
        .map_or_else(|| k.to_string(), |rest| format!("x_{rest}"))
}

fn priv_in(k: &str) -> String {
    k.strip_prefix("x_")
        .map_or_else(|| k.to_string(), |rest| format!("_{rest}"))
}

fn object(v: Value, what: &str) -> Result<Object> {
    match v {
        Value::Object(o) => Ok(o),
        other => Err(StoreError::mapping(format!("{what} is not an object: {other:?}"))),
    }
}

fn record_id(o: &mut Object, key: &str, what: &str) -> Result<RecordId> {
    match o.remove(key) {
        Some(Value::RecordId(r)) => Ok(r),
        other => Err(StoreError::mapping(format!(
            "{what}: '{key}' is not a record id: {other:?}"
        ))),
    }
}

fn int(o: &mut Object, key: &str, what: &str) -> Result<i64> {
    let v = o.remove(key).unwrap_or(Value::None);
    as_i64(&v).ok_or_else(|| StoreError::mapping(format!("{what}: '{key}' is not a number: {v:?}")))
}

fn string(o: &mut Object, key: &str, what: &str) -> Result<String> {
    match o.remove(key) {
        Some(Value::String(s)) => Ok(s),
        other => Err(StoreError::mapping(format!(
            "{what}: '{key}' is not a string: {other:?}"
        ))),
    }
}

/// `option<string | null>` read back: absent and `NULL` are both `None`.
fn opt_string(o: &mut Object, key: &str, what: &str) -> Result<Option<String>> {
    match o.remove(key) {
        None | Some(Value::None | Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(other) => Err(StoreError::mapping(format!(
            "{what}: '{key}' is not a string: {other:?}"
        ))),
    }
}

fn opt_int(o: &mut Object, key: &str, what: &str) -> Result<Option<i64>> {
    match o.remove(key) {
        None | Some(Value::None | Value::Null) => Ok(None),
        Some(v) => as_i64(&v)
            .map(Some)
            .ok_or_else(|| StoreError::mapping(format!("{what}: '{key}' is not a number: {v:?}"))),
    }
}

fn opt_bool(o: &mut Object, key: &str, what: &str) -> Result<Option<bool>> {
    match o.remove(key) {
        None | Some(Value::None | Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(b)),
        Some(other) => Err(StoreError::mapping(format!("{what}: '{key}' is not a bool: {other:?}"))),
    }
}

/// The remaining fields of a row as KB fields: `NONE` dropped, `x_` back to `_`, record ids as KB id strings,
/// DB-only keys skipped.
fn fields_in(fields: Object, db_only: &[&str], into: &mut Rec) {
    for (k, v) in fields {
        if v.is_none() || db_only.contains(&k.as_str()) {
            continue;
        }
        into.insert(priv_in(&k), from_db(v));
    }
}

// ---------------------------------------------------------------- nodes

/// A row of one of the six node tables: `{id, ord, ...fields}`. `fields` holds everything else as stored
/// (`x_` names, `performance.set` as a record id, the DB-only fields when read back).
#[derive(Debug, Clone, PartialEq)]
pub struct NodeRow {
    /// The table, from the id prefix.
    pub kind: NodeKind,
    /// `<table>:<key>`.
    pub id: RecordId,
    /// The files backend's insertion order.
    pub ord: i64,
    /// Every other field.
    pub fields: Object,
}

impl NodeRow {
    /// `node_to_row`: a KB node (`graph.json` shape) at position `ord`. The node's `type` must agree with its id
    /// prefix; a field that collides with a DB-only name is an error.
    pub fn from_node(n: &Rec, ord: i64) -> Result<Self> {
        let id_s = n.get("id").and_then(Json::as_str).unwrap_or("");
        let id: NodeId = id_s.parse()?;
        let want = id.type_name();
        if n.get("type").and_then(Json::as_str) != Some(want) {
            return Err(StoreError::mapping(format!(
                "node '{id_s}' has type {} but its id prefix means '{want}'",
                n.get("type")
                    .map_or_else(|| "None".to_string(), |t| format!("'{}'", from_db(to_db(t))))
            )));
        }
        let mut fields = Object::new();
        for (k, v) in n {
            if k == "id" || k == "type" {
                continue;
            }
            let k2 = priv_out(k);
            if DB_ONLY_NODE.contains(&k2.as_str()) {
                return Err(StoreError::mapping(format!(
                    "node '{id_s}': field '{k}' collides with a DB-only field"
                )));
            }
            fields.insert(k2, to_db(v));
        }
        if id.kind() == NodeKind::Perf
            && let Some(Value::String(s)) = fields.get("set")
        {
            let set = parse_node_rid(s)?;
            fields.insert("set", Value::RecordId(set));
        }
        Ok(Self {
            kind: id.kind(),
            id: node_rid(&id),
            ord,
            fields,
        })
    }

    /// A row read back from the database (`SELECT * FROM joke`): `id` and `ord` are required.
    pub fn from_record(v: Value) -> Result<Self> {
        let mut o = object(v, "node row")?;
        let id = record_id(&mut o, "id", "node row")?;
        let tb = table_of(&id);
        let kind = node_kind_of_table(&tb)
            .ok_or_else(|| StoreError::mapping(format!("node row: '{tb}' is not a node table")))?;
        let ord = int(&mut o, "ord", &format!("node row {}", rid_str(&id)))?;
        Ok(Self {
            kind,
            id,
            ord,
            fields: o,
        })
    }

    /// The row as `CONTENT`.
    pub fn content(&self) -> Value {
        let mut o = self.fields.clone();
        o.insert("id", Value::RecordId(self.id.clone()));
        o.insert("ord", self.ord);
        Value::Object(o)
    }

    /// The table name.
    pub fn table(&self) -> &'static str {
        self.kind.type_name()
    }

    /// `row_to_node`: the KB node this row is (`id`, `type`, the fields without the DB-only ones).
    pub fn into_node(self) -> Rec {
        let mut out = Rec::new();
        out.insert("id".into(), Json::String(rid_str(&self.id)));
        out.insert("type".into(), Json::String(self.kind.type_name().to_string()));
        fields_in(self.fields, &DB_ONLY_NODE, &mut out);
        out
    }

    /// The `working` flag and `working_sets` of a joke row (DB-derived, `fn::recompute_working`).
    pub fn working(&self) -> (bool, Vec<String>) {
        working_of(&self.fields)
    }
}

/// `(working, hit sets as KB ids)` from a joke row's fields.
pub fn working_of(fields: &Object) -> (bool, Vec<String>) {
    let w = matches!(fields.get("working"), Some(Value::Bool(true)));
    let sets = match fields.get("working_sets") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|s| match s {
                Value::RecordId(r) => Some(rid_str(r)),
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    (w, sets)
}

// ---------------------------------------------------------------- edges

/// The ten edge kinds, each a `RELATION` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// A unit is part of a joke.
    PartOf,
    /// One is a variant of the other.
    VariantOf,
    /// A performance is a joke on stage.
    PerformedAs,
    /// A tag.
    TagOf,
    /// The same shape.
    SameShape,
    /// A callback.
    CallbackTo,
    /// The same premise.
    SamePremise,
    /// A node has a label.
    HasLabel,
    /// A segue.
    Segue,
    /// The comic's own kind (`custom_kind` names it).
    Custom,
}

impl EdgeKind {
    /// Every kind, in `EDGE_TABLES` order.
    pub const ALL: [Self; 10] = [
        Self::PartOf,
        Self::VariantOf,
        Self::PerformedAs,
        Self::TagOf,
        Self::SameShape,
        Self::CallbackTo,
        Self::SamePremise,
        Self::HasLabel,
        Self::Segue,
        Self::Custom,
    ];

    /// The KB kind and the table name.
    pub fn table(self) -> &'static str {
        EDGE_TABLES[Self::ALL.iter().position(|k| *k == self).expect("every kind is listed")]
    }

    /// The kind a KB `kind` string (and table name) is.
    pub fn parse(s: &str) -> Option<Self> {
        EDGE_TABLES.iter().position(|k| *k == s).map(|i| Self::ALL[i])
    }
}

/// A row of one of the ten edge tables: `{id, in, out, ord, run_id, ...fields}`.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeRow {
    /// The kind, and so the table.
    pub kind: EdgeKind,
    /// The 12 hex characters of the edge id.
    pub key: String,
    /// The source node (`src`).
    pub in_: RecordId,
    /// The destination node (`dst`).
    pub out: RecordId,
    /// The files backend's insertion order.
    pub ord: i64,
    /// The pipeline run that proposed it (`source: "auto:<run>"`), else `NULL`.
    pub run_id: Option<String>,
    /// Every other field.
    pub fields: Object,
}

impl EdgeRow {
    /// `edge_to_row`: a KB edge at position `ord`. `kind`, `id`, `src` and `dst` are required.
    pub fn from_edge(e: &Rec, ord: i64) -> Result<Self> {
        let id_s = e.get("id").and_then(Json::as_str).unwrap_or("");
        let kind_s = e.get("kind").and_then(Json::as_str).unwrap_or("");
        let kind = EdgeKind::parse(kind_s).ok_or_else(|| {
            StoreError::mapping(format!(
                "edge '{id_s}': unknown kind {}",
                e.get("kind")
                    .map_or_else(|| "None".to_string(), |k| format!("'{}'", from_db(to_db(k))))
            ))
        })?;
        let id: EdgeId = id_s.parse()?;
        let end = |k: &str| -> Result<RecordId> {
            let s = e
                .get(k)
                .and_then(Json::as_str)
                .ok_or_else(|| StoreError::mapping(format!("edge '{id_s}': '{k}' is not a node id")))?;
            parse_node_rid(s)
        };
        let in_ = end("src")?;
        let out = end("dst")?;
        let mut fields = Object::new();
        for (k, v) in e {
            if ["id", "src", "dst", "kind"].contains(&k.as_str()) {
                continue;
            }
            let k2 = priv_out(k);
            if DB_ONLY_EDGE.contains(&k2.as_str()) {
                return Err(StoreError::mapping(format!(
                    "edge '{id_s}': field '{k}' collides with a DB-only field"
                )));
            }
            fields.insert(k2, to_db(v));
        }
        let run_id = e
            .get("source")
            .and_then(Json::as_str)
            .and_then(|s| s.strip_prefix("auto:"))
            .map(str::to_string);
        Ok(Self {
            kind,
            key: id.key().to_string(),
            in_,
            out,
            ord,
            run_id,
            fields,
        })
    }

    /// A row read back from the database.
    pub fn from_record(v: Value) -> Result<Self> {
        let mut o = object(v, "edge row")?;
        let id = record_id(&mut o, "id", "edge row")?;
        let tb = table_of(&id);
        let kind = EdgeKind::parse(&tb)
            .ok_or_else(|| StoreError::mapping(format!("edge row: '{tb}' is not an edge table")))?;
        let what = format!("edge row {tb}:{}", key_str(&id.key));
        let in_ = record_id(&mut o, "in", &what)?;
        let out = record_id(&mut o, "out", &what)?;
        let ord = int(&mut o, "ord", &what)?;
        let run_id = opt_string(&mut o, "run_id", &what)?;
        Ok(Self {
            kind,
            key: key_str(&id.key),
            in_,
            out,
            ord,
            run_id,
            fields: o,
        })
    }

    /// The record id, `<kind>:<key>`.
    pub fn id(&self) -> RecordId {
        RecordId::new(self.kind.table(), self.key.as_str())
    }

    /// The row as `INSERT RELATION` content.
    pub fn content(&self) -> Value {
        let mut o = self.fields.clone();
        o.insert("id", Value::RecordId(self.id()));
        o.insert("in", Value::RecordId(self.in_.clone()));
        o.insert("out", Value::RecordId(self.out.clone()));
        o.insert("ord", self.ord);
        o.insert("run_id", self.run_id.clone().map_or(Value::Null, Value::String));
        Value::Object(o)
    }

    /// `row_to_edge`: the KB edge this row is (`id`, `src`, `dst`, `kind`, the fields without the DB-only ones).
    pub fn into_edge(self) -> Rec {
        let mut out = Rec::new();
        out.insert("id".into(), Json::String(format!("e:{}", self.key)));
        out.insert("src".into(), Json::String(rid_str(&self.in_)));
        out.insert("dst".into(), Json::String(rid_str(&self.out)));
        out.insert("kind".into(), Json::String(self.kind.table().to_string()));
        fields_in(self.fields, &DB_ONLY_EDGE, &mut out);
        out
    }
}

// ---------------------------------------------------------------- decisions

/// A column that may be absent (`NONE`), `NULL`, or set: the three states a decision line's keys can be in
/// (`"undo_of": null` is not the same line as one without `undo_of`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field<T> {
    /// The key was not on the line.
    Absent,
    /// The key was `null`.
    Null,
    /// The key had a value of the column's type.
    Set(T),
}

impl<T> Field<T> {
    fn to_value(&self, f: impl FnOnce(&T) -> Value) -> Option<Value> {
        match self {
            Self::Absent => None,
            Self::Null => Some(Value::Null),
            Self::Set(t) => Some(f(t)),
        }
    }

    fn to_json(&self, f: impl FnOnce(&T) -> Json) -> Option<Json> {
        match self {
            Self::Absent => None,
            Self::Null => Some(Json::Null),
            Self::Set(t) => Some(f(t)),
        }
    }
}

fn field_string(o: &mut Object, key: &str, what: &str) -> Result<Field<String>> {
    Ok(match o.remove(key) {
        None | Some(Value::None) => Field::Absent,
        Some(Value::Null) => Field::Null,
        Some(Value::String(s)) => Field::Set(s),
        Some(other) => {
            return Err(StoreError::mapping(format!(
                "{what}: '{key}' is not a string: {other:?}"
            )));
        }
    })
}

fn field_object(o: &mut Object, key: &str, what: &str) -> Result<Field<Rec>> {
    Ok(match o.remove(key) {
        None | Some(Value::None) => Field::Absent,
        Some(Value::Null) => Field::Null,
        Some(Value::Object(obj)) => match from_db(Value::Object(obj)) {
            Json::Object(m) => Field::Set(m),
            _ => unreachable!("an object converts to an object"),
        },
        Some(other) => {
            return Err(StoreError::mapping(format!(
                "{what}: '{key}' is not an object: {other:?}"
            )));
        }
    })
}

fn field_int(o: &mut Object, key: &str, what: &str) -> Result<Field<i64>> {
    Ok(match o.remove(key) {
        None | Some(Value::None) => Field::Absent,
        Some(Value::Null) => Field::Null,
        Some(v) => Field::Set(
            as_i64(&v).ok_or_else(|| StoreError::mapping(format!("{what}: '{key}' is not a number: {v:?}")))?,
        ),
    })
}

/// A `decision` row: one line of `decisions.jsonl` at `seq`, its known keys in columns (`id` as `target_id`), any
/// other key under `extra`. A known key whose value has the wrong type goes to `extra` too (as the Python does).
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRow {
    /// The line's index among valid lines (what undo addresses).
    pub seq: i64,
    /// When.
    pub ts: Field<String>,
    /// `edge` | `node`.
    pub target: Field<String>,
    /// The node or edge id (the line's `id`).
    pub target_id: Field<String>,
    /// `add` | `delete` | `confirm` | `reject` | `edit`.
    pub action: Field<String>,
    /// The before image.
    pub before: Field<Rec>,
    /// The after image.
    pub after: Field<Rec>,
    /// Why.
    pub reason: Field<String>,
    /// Who.
    pub by: Field<String>,
    /// The decision this one undoes.
    pub undo_of: Field<i64>,
    /// Every other key of the line.
    pub extra: Option<Rec>,
}

impl DecisionRow {
    /// `decision_to_row`: the line at `seq`.
    pub fn from_decision(d: &Decision, seq: i64) -> Self {
        let mut row = Self {
            seq,
            ts: Field::Absent,
            target: Field::Absent,
            target_id: Field::Absent,
            action: Field::Absent,
            before: Field::Absent,
            after: Field::Absent,
            reason: Field::Absent,
            by: Field::Absent,
            undo_of: Field::Absent,
            extra: None,
        };
        let mut extra = Rec::new();
        for (k, v) in &d.0 {
            let str_field = |v: &Json| match v {
                Json::Null => Some(Field::Null),
                Json::String(s) => Some(Field::Set(s.clone())),
                _ => None,
            };
            let placed = match k.as_str() {
                "ts" => str_field(v).map(|f| row.ts = f),
                "target" => str_field(v).map(|f| row.target = f),
                "id" => str_field(v).map(|f| row.target_id = f),
                "action" => str_field(v).map(|f| row.action = f),
                "reason" => str_field(v).map(|f| row.reason = f),
                "by" => str_field(v).map(|f| row.by = f),
                "before" | "after" => {
                    let f = match v {
                        Json::Null => Some(Field::Null),
                        Json::Object(o) => Some(Field::Set(o.clone())),
                        _ => None,
                    };
                    f.map(|f| {
                        if k == "before" {
                            row.before = f;
                        } else {
                            row.after = f;
                        }
                    })
                }
                "undo_of" => match v {
                    Json::Null => Some(Field::Null),
                    Json::Number(n) => n.as_i64().map(Field::Set),
                    _ => None,
                }
                .map(|f| row.undo_of = f),
                _ => None,
            };
            if placed.is_none() {
                extra.insert(k.clone(), v.clone());
            }
        }
        if !extra.is_empty() {
            row.extra = Some(extra);
        }
        row
    }

    /// A row read back from the database.
    pub fn from_record(v: Value) -> Result<Self> {
        let mut o = object(v, "decision row")?;
        let seq = int(&mut o, "seq", "decision row")?;
        let what = format!("decision row seq {seq}");
        let extra = match o.remove("extra") {
            None | Some(Value::None | Value::Null) => None,
            Some(Value::Object(e)) => match from_db(Value::Object(e)) {
                Json::Object(m) => Some(m),
                _ => unreachable!("an object converts to an object"),
            },
            Some(other) => {
                return Err(StoreError::mapping(format!(
                    "{what}: 'extra' is not an object: {other:?}"
                )));
            }
        };
        Ok(Self {
            seq,
            ts: field_string(&mut o, "ts", &what)?,
            target: field_string(&mut o, "target", &what)?,
            target_id: field_string(&mut o, "target_id", &what)?,
            action: field_string(&mut o, "action", &what)?,
            before: field_object(&mut o, "before", &what)?,
            after: field_object(&mut o, "after", &what)?,
            reason: field_string(&mut o, "reason", &what)?,
            by: field_string(&mut o, "by", &what)?,
            undo_of: field_int(&mut o, "undo_of", &what)?,
            extra,
        })
    }

    /// The row as `CONTENT`.
    pub fn content(&self) -> Value {
        let mut o = Object::new();
        o.insert("seq", self.seq);
        let s = |v: &String| Value::String(v.clone());
        let obj = |v: &Rec| to_db(&Json::Object(v.clone()));
        for (k, v) in [
            ("ts", self.ts.to_value(s)),
            ("target", self.target.to_value(s)),
            ("target_id", self.target_id.to_value(s)),
            ("action", self.action.to_value(s)),
            ("before", self.before.to_value(obj)),
            ("after", self.after.to_value(obj)),
            ("reason", self.reason.to_value(s)),
            ("by", self.by.to_value(s)),
            ("undo_of", self.undo_of.to_value(|i| Value::Number(Number::Int(*i)))),
        ] {
            if let Some(v) = v {
                o.insert(k, v);
            }
        }
        if let Some(e) = &self.extra {
            o.insert("extra", to_db(&Json::Object(e.clone())));
        }
        Value::Object(o)
    }

    /// `row_to_decision`: the line again (keys in [`DECISION_KEYS`] order, then `extra`).
    pub fn into_decision(self) -> Decision {
        let mut out = Rec::new();
        let s = |v: &String| Json::String(v.clone());
        let obj = |v: &Rec| Json::Object(v.clone());
        for (k, v) in [
            ("ts", self.ts.to_json(s)),
            ("target", self.target.to_json(s)),
            ("id", self.target_id.to_json(s)),
            ("action", self.action.to_json(s)),
            ("before", self.before.to_json(obj)),
            ("after", self.after.to_json(obj)),
            ("reason", self.reason.to_json(s)),
            ("by", self.by.to_json(s)),
            ("undo_of", self.undo_of.to_json(|i| Json::from(*i))),
        ] {
            if let Some(v) = v {
                out.insert(k.to_string(), v);
            }
        }
        if let Some(e) = self.extra {
            out.extend(e);
        }
        Decision(out)
    }
}

// ---------------------------------------------------------------- creative events

/// sha256 of a line, hex: the `h` column (`creative_api._h`).
pub fn line_hash(line: &str) -> String {
    hex::encode(Sha256::digest(line.as_bytes()))
}

/// A `creative_event` row: one line of a creative store's jsonl, unique on `(store, seq)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreativeEvent {
    /// `storyboards` | `workingsets` | `venues` | `jobs` | ...
    pub store: String,
    /// The 0-based line number in `<store>.jsonl`.
    pub seq: i64,
    /// The event, exactly as written to the jsonl.
    pub line: String,
    /// sha256 of `line`.
    pub h: String,
    /// The event's `op`.
    pub op: Option<String>,
    /// The event's `id` (the item it changed).
    pub item: Option<String>,
    /// The event's `ts`.
    pub at: Option<String>,
}

impl CreativeEvent {
    /// A row for `line` at `seq` of `store`; `op`, `item` and `at` are read from the line, which must be a JSON object.
    pub fn new(store: &str, seq: i64, line: &str) -> Result<Self> {
        let ev: Json = serde_json::from_str(line)?;
        let pick = |k: &str| -> Result<Option<String>> {
            match ev.get(k) {
                None | Some(Json::Null) => Ok(None),
                Some(Json::String(s)) => Ok(Some(s.clone())),
                Some(other) => Err(StoreError::mapping(format!(
                    "creative event {store}/{seq}: '{k}' is not a string: {other}"
                ))),
            }
        };
        Ok(Self {
            store: store.to_string(),
            seq,
            line: line.to_string(),
            h: line_hash(line),
            op: pick("op")?,
            item: pick("id")?,
            at: pick("ts")?,
        })
    }

    /// The row as `CONTENT`: `op`, `item` and `at` are `NULL` when the line has none.
    pub fn content(&self) -> Value {
        let mut o = Object::new();
        o.insert("store", self.store.clone());
        o.insert("seq", self.seq);
        o.insert("line", self.line.clone());
        o.insert("h", self.h.clone());
        let opt = |v: &Option<String>| v.clone().map_or(Value::Null, Value::String);
        o.insert("op", opt(&self.op));
        o.insert("item", opt(&self.item));
        o.insert("at", opt(&self.at));
        Value::Object(o)
    }

    /// A row read back from the database.
    pub fn from_record(v: Value) -> Result<Self> {
        let mut o = object(v, "creative_event row")?;
        let what = "creative_event row";
        Ok(Self {
            store: string(&mut o, "store", what)?,
            seq: int(&mut o, "seq", what)?,
            line: string(&mut o, "line", what)?,
            h: string(&mut o, "h", what)?,
            op: opt_string(&mut o, "op", what)?,
            item: opt_string(&mut o, "item", what)?,
            at: opt_string(&mut o, "at", what)?,
        })
    }
}

// ---------------------------------------------------------------- srcfile

/// A `srcfile` row: a text file the hosted API needs, unique on `path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrcFile {
    /// Relative to the comedy folder, forward slashes.
    pub path: String,
    /// sha256 of `text`.
    pub sha256: String,
    /// `text` in bytes.
    pub bytes: i64,
    /// The file.
    pub text: String,
}

impl SrcFile {
    /// A row for `text` at `path` (`sha256` and `bytes` computed).
    pub fn new(path: &str, text: &str) -> Self {
        Self {
            path: path.to_string(),
            sha256: line_hash(text),
            bytes: i64::try_from(text.len()).unwrap_or(i64::MAX),
            text: text.to_string(),
        }
    }

    /// The row as `CONTENT`.
    pub fn content(&self) -> Value {
        let mut o = Object::new();
        o.insert("path", self.path.clone());
        o.insert("sha256", self.sha256.clone());
        o.insert("bytes", self.bytes);
        o.insert("text", self.text.clone());
        Value::Object(o)
    }

    /// A row read back from the database.
    pub fn from_record(v: Value) -> Result<Self> {
        let mut o = object(v, "srcfile row")?;
        let what = "srcfile row";
        Ok(Self {
            path: string(&mut o, "path", what)?,
            sha256: string(&mut o, "sha256", what)?,
            bytes: int(&mut o, "bytes", what)?,
            text: string(&mut o, "text", what)?,
        })
    }
}

// ---------------------------------------------------------------- meta:state

/// `meta:state`: the load and replay state. `gen` moves on every write, so a reader knows its snapshot is stale.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MetaState {
    /// The generation (`gen`).
    pub generation: Option<i64>,
    /// The next `ord` a decision-made record gets.
    pub ord_next: Option<i64>,
    /// Decisions whose target was gone and could not be revived.
    pub orphans: Option<i64>,
    /// A reload is running.
    pub loading: Option<bool>,
    /// What was loaded (`graph`, `graph_meta`, `counts`, ...), as JSON.
    pub load_info: Option<Json>,
    /// sha256 of the schema files last applied.
    pub schema_sha: Option<String>,
    /// sha256 of `schema_extra.surql` last applied.
    pub extra_sha: Option<String>,
}

impl MetaState {
    /// The row (or a partial `SELECT` of it) read back; `NONE` means never loaded.
    pub fn from_record(v: Value) -> Result<Option<Self>> {
        if v.is_none() {
            return Ok(None);
        }
        let mut o = object(v, "meta:state")?;
        let what = "meta:state";
        Ok(Some(Self {
            generation: opt_int(&mut o, "gen", what)?,
            ord_next: opt_int(&mut o, "ord_next", what)?,
            orphans: opt_int(&mut o, "orphans", what)?,
            loading: opt_bool(&mut o, "loading", what)?,
            load_info: match o.remove("load_info") {
                None | Some(Value::None | Value::Null) => None,
                Some(v) => Some(from_db(v)),
            },
            schema_sha: opt_string(&mut o, "schema_sha", what)?,
            extra_sha: opt_string(&mut o, "extra_sha", what)?,
        }))
    }
}

// ---------------------------------------------------------------- laughs

/// `(laugh key, column, accepts)`: the laugh keys with their own column.
const LAUGH_KEYS: [(&str, &str, LaughType); 9] = [
    ("id", "lid", LaughType::Str),
    ("start_ms", "start_ms", LaughType::Int),
    ("end_ms", "end_ms", LaughType::Int),
    ("strength", "strength", LaughType::Num),
    ("bars", "bars", LaughType::Int),
    ("kind", "kind", LaughType::Str),
    ("confidence", "confidence", LaughType::Num),
    ("segment", "segment", LaughType::Str),
    ("detail", "detail", LaughType::Any),
];

#[derive(Clone, Copy)]
enum LaughType {
    Str,
    Int,
    Num,
    Any,
}

impl LaughType {
    fn accepts(self, v: &Json) -> bool {
        match (self, v) {
            (Self::Int, Json::Number(n)) => n.is_i64() || n.is_u64(),
            (_, Json::Null) | (Self::Any, _) | (Self::Str, Json::String(_)) | (Self::Num, Json::Number(_)) => true,
            _ => false,
        }
    }
}

/// `laugh_to_row`: a laugh of `set` at position `ord`, keyed `laugh:[set, start_ms, ord]` (`start_ms` -1 when it
/// is not an integer). Keys without a column, or with a value of the wrong type, go under `extra`. A JSON bool is
/// never a number here (the Python's `isinstance(True, int)` quirk is not reproduced).
pub fn laugh_to_row(l: &Rec, set: &RecordId, ord: i64) -> Value {
    let mut row = Object::new();
    row.insert("set", Value::RecordId(set.clone()));
    row.insert("ord", ord);
    let mut extra = Rec::new();
    for (k, v) in l {
        match LAUGH_KEYS.iter().find(|(key, _, _)| key == k) {
            Some((_, col, ty)) if ty.accepts(v) => {
                row.insert(*col, to_db(v));
            }
            _ => {
                extra.insert(k.clone(), v.clone());
            }
        }
    }
    if !extra.is_empty() {
        row.insert("extra", to_db(&Json::Object(extra)));
    }
    let start = l.get("start_ms").and_then(Json::as_i64).unwrap_or(-1);
    row.insert(
        "id",
        Value::RecordId(RecordId::new(
            "laugh",
            Array::from(vec![
                Value::RecordId(set.clone()),
                Value::Number(Number::Int(start)),
                Value::Number(Number::Int(ord)),
            ]),
        )),
    );
    Value::Object(row)
}

/// `row_to_laugh`: the laugh dict again (`id`, `set`, `ord` dropped, `extra` merged in, `lid` back to `id`).
pub fn row_to_laugh(v: Value) -> Result<Rec> {
    let o = object(v, "laugh row")?;
    let mut out = Rec::new();
    for (k, v) in o {
        if v.is_none() || ["id", "set", "ord"].contains(&k.as_str()) {
            continue;
        }
        if k == "extra" {
            if let Json::Object(m) = from_db(v) {
                out.extend(m);
            }
            continue;
        }
        let key = LAUGH_KEYS
            .iter()
            .find(|(_, col, _)| *col == k)
            .map_or(k.as_str(), |(key, _, _)| key);
        out.insert(key.to_string(), from_db(v));
    }
    Ok(out)
}

/// The string in a row's field, for callers reading partial selects.
pub fn field_str<'a>(o: &'a Object, k: &str) -> Option<&'a str> {
    o.get(k).and_then(as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(v: Json) -> Rec {
        match v {
            Json::Object(m) => m,
            _ => panic!("object"),
        }
    }

    #[test]
    fn node_rows_both_ways() {
        let n = rec(
            json!({"id": "perf:p1", "type": "performance", "set": "set:s1", "start_ms": 1, "text": "x",
            "_by_tristan": true, "note": null}),
        );
        let row = NodeRow::from_node(&n, 7).unwrap();
        assert_eq!(row.table(), "performance");
        assert_eq!(
            row.fields.get("set"),
            Some(&Value::RecordId(RecordId::new("set", "s1")))
        );
        assert_eq!(row.fields.get("x_by_tristan"), Some(&Value::Bool(true)));
        assert_eq!(row.fields.get("note"), Some(&Value::Null));
        let back = NodeRow::from_record(row.content()).unwrap().into_node();
        assert_eq!(Json::Object(back), Json::Object(n));
        let bad = rec(json!({"id": "perf:p1", "type": "joke"}));
        assert!(NodeRow::from_node(&bad, 0).is_err());
        let bad = rec(json!({"id": "joke:j", "type": "joke", "ord": 3}));
        assert!(NodeRow::from_node(&bad, 0).is_err());
    }

    #[test]
    fn edge_rows_both_ways() {
        let e = rec(
            json!({"id": "e:aaaaaaaaaaa1", "kind": "part_of", "src": "unit:m1", "dst": "joke:meters",
            "status": "confirmed", "confidence": 0.9, "source": "auto:r1", "_edited": true}),
        );
        let row = EdgeRow::from_edge(&e, 3).unwrap();
        assert_eq!(row.run_id.as_deref(), Some("r1"));
        assert_eq!(rid_str(&row.id()), "part_of:aaaaaaaaaaa1");
        let back = EdgeRow::from_record(row.content()).unwrap().into_edge();
        assert_eq!(Json::Object(back), Json::Object(e));
        let bad = rec(json!({"id": "e:aaaaaaaaaaa1", "kind": "friend", "src": "unit:m1", "dst": "joke:meters"}));
        assert!(EdgeRow::from_edge(&bad, 0).is_err());
    }

    #[test]
    fn decision_rows_keep_null_and_absent_apart() {
        let d = Decision(rec(
            json!({"ts": "t", "target": "node", "id": "joke:x", "action": "edit",
            "before": {"a": 1}, "after": null, "reason": "", "by": "owner", "undo_of": null, "weird": [1], "ts2": 1.5}),
        ));
        let row = DecisionRow::from_decision(&d, 4);
        assert_eq!(row.after, Field::Null);
        assert_eq!(row.undo_of, Field::Null);
        assert_eq!(row.extra, Some(rec(json!({"weird": [1], "ts2": 1.5}))));
        let back = DecisionRow::from_record(row.content()).unwrap().into_decision();
        assert_eq!(Json::Object(back.0), Json::Object(d.0));
        let d2 = Decision(rec(
            json!({"target": "edge", "id": "e:1", "action": "confirm", "undo_of": 2.0}),
        ));
        let row2 = DecisionRow::from_decision(&d2, 5);
        assert_eq!(row2.undo_of, Field::Absent);
        assert_eq!(row2.extra, Some(rec(json!({"undo_of": 2.0}))));
    }

    #[test]
    fn creative_events() {
        let ev = CreativeEvent::new(
            "storyboards",
            0,
            r#"{"ts":"2026-01-01","op":"put","id":"sb1","by":"owner","data":{}}"#,
        )
        .unwrap();
        assert_eq!(ev.op.as_deref(), Some("put"));
        assert_eq!(ev.item.as_deref(), Some("sb1"));
        assert_eq!(ev.h.len(), 64);
        assert_eq!(CreativeEvent::from_record(ev.content()).unwrap(), ev);
        assert!(CreativeEvent::new("s", 0, "not json").is_err());
        let no_op = CreativeEvent::new("s", 1, r"{}").unwrap();
        assert_eq!(no_op.op, None);
    }

    #[test]
    fn laughs_both_ways() {
        let l = rec(
            json!({"id": "l1", "start_ms": 100, "end_ms": 200, "strength": 0.5, "bars": 2, "kind": "big",
            "confidence": "high", "detail": {"x": 1}, "odd": true}),
        );
        let set = RecordId::new("set", "s1");
        let row = laugh_to_row(&l, &set, 3);
        let Value::Object(o) = &row else { panic!() };
        assert_eq!(o.get("lid"), Some(&Value::String("l1".into())));
        assert!(o.get("confidence").is_none(), "a string confidence goes to extra");
        assert_eq!(Json::Object(row_to_laugh(row.clone()).unwrap()), Json::Object(l));
    }
}
