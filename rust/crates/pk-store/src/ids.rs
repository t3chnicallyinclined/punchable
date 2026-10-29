//! KB ids and record ids, both ways: `perf:2015-02-18_x@1200` is the record `performance:⟨2015-02-18_x@1200⟩`
//! (the table from the prefix, the key the rest, colons kept), `e:<hash>` is `<kind>:<hash>` in the kind's table.
//! Parsing is [`pk_kb::ids`]; this adds the table side (`db/sdb.py` `node_rid`, `rid_str`, `edge_key`).

use pk_kb::{EdgeId, NodeId, NodeKind};
use surrealdb::types::{RecordId, RecordIdKey, ToSql};

use crate::error::{Result, StoreError};

/// The node tables, in the order `db/load.py` writes them.
pub const NODE_TABLES: [&str; 6] = ["joke", "unit", "set", "performance", "label", "list"];
/// The edge tables: one `RELATION` table per KB edge kind.
pub use pk_kb::model::EDGE_KINDS as EDGE_TABLES;

/// The node kind a table holds, if it is a node table.
pub fn node_kind_of_table(table: &str) -> Option<NodeKind> {
    Some(match table {
        "joke" => NodeKind::Joke,
        "unit" => NodeKind::Unit,
        "set" => NodeKind::Set,
        "performance" => NodeKind::Perf,
        "label" => NodeKind::Label,
        "list" => NodeKind::List,
        _ => return None,
    })
}

/// The record a node id names.
pub fn node_rid(id: &NodeId) -> RecordId {
    RecordId::new(id.type_name(), id.key())
}

/// The record a node id string names (`perf:x` is `performance:x`); an unknown prefix is an error.
pub fn parse_node_rid(s: &str) -> Result<RecordId> {
    let id: NodeId = s.parse()?;
    Ok(node_rid(&id))
}

/// The table of a record id.
pub fn table_of(r: &RecordId) -> String {
    String::from(r.table.clone())
}

/// A record key as text: a string key as is, anything else as `SurrealQL` prints it.
pub fn key_str(k: &RecordIdKey) -> String {
    match k {
        RecordIdKey::String(s) => s.clone(),
        RecordIdKey::Number(n) => n.to_string(),
        other => other.to_sql(),
    }
}

/// The KB id of a record: `performance:x` is `perf:x`; other tables keep their name as the prefix.
pub fn rid_str(r: &RecordId) -> String {
    let tb = table_of(r);
    let prefix = node_kind_of_table(&tb).map_or_else(|| tb.clone(), |k| k.prefix().to_string());
    format!("{prefix}:{}", key_str(&r.key))
}

/// The edge table of a KB kind, or an error naming the kind.
pub fn edge_table(kind: &str) -> Result<&'static str> {
    EDGE_TABLES
        .iter()
        .copied()
        .find(|k| *k == kind)
        .ok_or_else(|| StoreError::mapping(format!("unknown edge kind '{kind}'")))
}

/// The record an edge id names in its kind's table.
pub fn edge_rid(kind: &str, id: &EdgeId) -> Result<RecordId> {
    Ok(RecordId::new(edge_table(kind)?, id.key()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_round_trip() {
        let r = parse_node_rid("perf:2015-02-18_x@1200").unwrap();
        assert_eq!(table_of(&r), "performance");
        assert_eq!(key_str(&r.key), "2015-02-18_x@1200");
        assert_eq!(rid_str(&r), "perf:2015-02-18_x@1200");
        let r = parse_node_rid("label:topic:money").unwrap();
        assert_eq!(rid_str(&r), "label:topic:money");
        assert!(parse_node_rid("blob:x").is_err());
    }

    #[test]
    fn edge_ids() {
        let id: EdgeId = "e:0123456789ab".parse().unwrap();
        let r = edge_rid("has_label", &id).unwrap();
        assert_eq!(rid_str(&r), "has_label:0123456789ab");
        assert!(edge_rid("friend_of", &id).is_err());
    }
}
