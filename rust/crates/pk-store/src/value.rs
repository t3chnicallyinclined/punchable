//! JSON and `SurrealDB` values, both ways, keeping what the Python keeps: `null` is `NULL`, an absent key is `NONE`
//! and stays absent, a record id becomes its KB id string on the way out. The SDK's own `into_json_value` folds
//! `NONE` into `null`, which is why this module exists.

use serde_json::Value as Json;
use surrealdb::types::{Array, Number, Object, Value};

use crate::ids::rid_str;

/// A JSON value as a `SurrealDB` value: `null` is `NULL`, integers stay integers, everything else is what it is.
pub fn to_db(v: &Json) -> Value {
    match v {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => n
            .as_i64()
            .map(Number::Int)
            .or_else(|| n.as_f64().map(Number::Float))
            .map_or_else(|| Value::String(n.to_string()), Value::Number),
        Json::String(s) => Value::String(s.clone()),
        Json::Array(a) => Value::Array(Array::from(a.iter().map(to_db).collect::<Vec<_>>())),
        Json::Object(o) => {
            let mut obj = Object::new();
            for (k, v) in o {
                obj.insert(k.clone(), to_db(v));
            }
            Value::Object(obj)
        }
    }
}

/// A `SurrealDB` value as JSON, the way `db/sdb.py` `strip_none` plus `rid_str` see it: `NONE` keys and array items
/// are dropped, `NULL` is `null`, a record id is its KB id string, a datetime is RFC 3339 text. A bare `NONE` is
/// `null` (there is no JSON for absence).
pub fn from_db(v: Value) -> Json {
    match v {
        Value::None | Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(b),
        Value::Number(Number::Int(i)) => Json::from(i),
        Value::Number(Number::Float(f)) => serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number),
        Value::String(s) => Json::String(s),
        Value::RecordId(r) => Json::String(rid_str(&r)),
        Value::Array(a) => Json::Array(
            a.into_inner()
                .into_iter()
                .filter(|x| !x.is_none())
                .map(from_db)
                .collect(),
        ),
        Value::Object(o) => Json::Object(
            o.into_inner()
                .into_iter()
                .filter(|(_, x)| !x.is_none())
                .map(|(k, x)| (k, from_db(x)))
                .collect(),
        ),
        other => other.into_json_value(),
    }
}

/// A `SurrealDB` number as `i64`, if it is one (an integral float counts).
pub fn as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.to_int(),
        _ => None,
    }
}

/// A `SurrealDB` number as `f64`, if it is one.
pub fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.to_f64(),
        _ => None,
    }
}

/// The string in a value, if it is one.
pub fn as_str(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) => Some(s.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use surrealdb::types::RecordId;

    #[test]
    fn null_and_none_are_kept_apart() {
        let mut o = Object::new();
        o.insert("a", Value::Null);
        o.insert("b", Value::None);
        o.insert(
            "c",
            Value::Array(Array::from(vec![Value::None, Value::Null, Value::Bool(true)])),
        );
        assert_eq!(from_db(Value::Object(o)), json!({"a": null, "c": [null, true]}));
        assert_eq!(to_db(&json!({"a": null})), {
            let mut o = Object::new();
            o.insert("a", Value::Null);
            Value::Object(o)
        });
    }

    #[test]
    fn numbers_and_ids() {
        assert_eq!(to_db(&json!(3)), Value::Number(Number::Int(3)));
        assert_eq!(to_db(&json!(0.5)), Value::Number(Number::Float(0.5)));
        assert_eq!(from_db(Value::Number(Number::Float(1.0))), json!(1.0));
        assert_eq!(
            from_db(Value::RecordId(RecordId::new("performance", "2015-02-18_x@1200"))),
            json!("perf:2015-02-18_x@1200")
        );
    }
}
