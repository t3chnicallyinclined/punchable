//! Schemas on the wire and answers against them: the SDK's `transform_schema` (what `output_config.format`
//! accepts: the Python SDK's `anthropic/lib/_parse/_transform.py`, ported branch by branch), the tolerant
//! `parse_json` of model text, the `jsonschema` validation, and the instruction text the retry sends.

use serde_json::{Map, Value};

use crate::error::LlmError;
use crate::pyfmt;

/// The string formats the API's structured output keeps; any other goes into the description.
const SUPPORTED_STRING_FORMATS: &[&str] = &[
    "date-time",
    "time",
    "date",
    "duration",
    "email",
    "hostname",
    "uri",
    "ipv4",
    "ipv6",
    "uuid",
];

const SUPPORTED_TYPES: &[&str] = &["object", "array", "string", "integer", "number", "boolean", "null"];

/// The SDK's `transform_schema`: the subset of JSON Schema the API enforces, every other keyword appended to the
/// `description` as `{key: value, ...}` so the model might still follow it.
pub fn transform_schema(schema: &Value) -> Result<Value, LlmError> {
    let Value::Object(input) = schema else {
        return Err(LlmError::BadSchema("Schema must be a JSON object.".into()));
    };
    let mut rest = input.clone();
    let mut strict = Map::new();

    if let Some(r) = rest.shift_remove("$ref")
        && !r.is_null()
    {
        strict.insert("$ref".into(), r);
        return Ok(Value::Object(strict));
    }
    if let Some(Value::Object(defs)) = rest.shift_remove("$defs") {
        let mut strict_defs = Map::new();
        for (name, s) in &defs {
            strict_defs.insert(name.clone(), transform_schema(s)?);
        }
        strict.insert("$defs".into(), Value::Object(strict_defs));
    }

    let type_ = rest.shift_remove("type").filter(|t| !t.is_null());
    let any_of = rest.shift_remove("anyOf");
    let one_of = rest.shift_remove("oneOf");
    let all_of = rest.shift_remove("allOf");
    let type_name: Option<String> = match &type_ {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => return Err(LlmError::BadSchema(format!("Unsupported schema type: {other}"))),
    };

    let variants = |v: Option<Value>| -> Option<Vec<Value>> {
        match v {
            Some(Value::Array(a)) => Some(a),
            _ => None,
        }
    };
    let transform_all = |a: Vec<Value>| -> Result<Value, LlmError> {
        Ok(Value::Array(
            a.iter().map(transform_schema).collect::<Result<Vec<_>, _>>()?,
        ))
    };
    if let Some(a) = variants(any_of) {
        strict.insert("anyOf".into(), transform_all(a)?);
    } else if let Some(a) = variants(one_of) {
        strict.insert("anyOf".into(), transform_all(a)?);
    } else if let Some(a) = variants(all_of) {
        strict.insert("allOf".into(), transform_all(a)?);
    } else {
        let Some(t) = &type_name else {
            return Err(LlmError::BadSchema(
                "Schema must have a 'type', 'anyOf', 'oneOf', or 'allOf' field.".into(),
            ));
        };
        if !SUPPORTED_TYPES.contains(&t.as_str()) {
            return Err(LlmError::BadSchema(format!("Unsupported schema type: {t:?}")));
        }
        strict.insert("type".into(), Value::String(t.clone()));
    }

    if let Some(e @ Value::Array(_)) = rest.shift_remove("enum") {
        strict.insert("enum".into(), e);
    }
    if let Some(d) = rest.shift_remove("description").filter(|d| !d.is_null()) {
        strict.insert("description".into(), d);
    }
    if let Some(t) = rest.shift_remove("title").filter(|t| !t.is_null()) {
        strict.insert("title".into(), t);
    }

    match type_name.as_deref() {
        Some("object") => {
            let mut props = Map::new();
            match rest.shift_remove("properties") {
                Some(Value::Object(p)) => {
                    for (k, s) in &p {
                        props.insert(k.clone(), transform_schema(s)?);
                    }
                }
                Some(other) => {
                    return Err(LlmError::BadSchema(format!(
                        "'properties' must be an object, not {other}"
                    )));
                }
                None => {}
            }
            strict.insert("properties".into(), Value::Object(props));
            rest.shift_remove("additionalProperties");
            strict.insert("additionalProperties".into(), Value::Bool(false));
            if let Some(r) = rest.shift_remove("required").filter(|r| !r.is_null()) {
                strict.insert("required".into(), r);
            }
        }
        Some("string") => {
            if let Some(f) = rest.shift_remove("format")
                && truthy(&f)
            {
                match &f {
                    Value::String(s) if SUPPORTED_STRING_FORMATS.contains(&s.as_str()) => {
                        strict.insert("format".into(), f);
                    }
                    _ => {
                        rest.insert("format".into(), f); // back, so it lands in the description
                    }
                }
            }
        }
        Some("array") => {
            if let Some(items) = rest.shift_remove("items").filter(|i| !i.is_null()) {
                strict.insert("items".into(), transform_schema(&items)?);
            }
            if let Some(m) = rest.shift_remove("minItems").filter(|m| !m.is_null()) {
                if is_zero_or_one(&m) {
                    strict.insert("minItems".into(), m);
                } else {
                    rest.insert("minItems".into(), m);
                }
            }
        }
        _ => {}
    }

    if !rest.is_empty() {
        let head = strict
            .get("description")
            .map(|d| format!("{}\n\n", pyfmt::py_str(d)))
            .unwrap_or_default();
        let items: Vec<String> = rest.iter().map(|(k, v)| format!("{k}: {}", pyfmt::py_str(v))).collect();
        strict.insert(
            "description".into(),
            Value::String(format!("{head}{{{}}}", items.join(", "))),
        );
    }
    Ok(Value::Object(strict))
}

/// Python truthiness of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => false,
        Value::Bool(true) => true,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `min_items == 0 or min_items == 1` in Python, where `False == 0` and `True == 1`.
fn is_zero_or_one(v: &Value) -> bool {
    match v {
        Value::Bool(_) => true,
        Value::Number(n) => n
            .as_f64()
            .is_some_and(|f| f.abs() < f64::EPSILON || (f - 1.0).abs() < f64::EPSILON),
        _ => false,
    }
}

/// `providers.py` `parse_json`: a JSON object from model text, tolerating ```` ```json ```` fences and leading or
/// trailing prose (the first `{` to the last `}`). The error is the parser's words.
pub fn parse_json(text: &str) -> Result<Value, String> {
    let mut t = text.trim();
    if t.starts_with("```") {
        if let Some((_, rest)) = t.split_once('\n') {
            t = rest;
        }
        let r = t.trim_end();
        if let Some(stripped) = r.strip_suffix("```") {
            t = stripped;
        }
    }
    match serde_json::from_str::<Value>(t) {
        Ok(v) => Ok(v),
        Err(e) => {
            let (Some(a), Some(b)) = (t.find('{'), t.rfind('}')) else {
                return Err(e.to_string());
            };
            if b <= a {
                return Err(e.to_string());
            }
            serde_json::from_str::<Value>(&t[a..=b]).map_err(|e| e.to_string())
        }
    }
}

/// `jsonschema.validate(data, schema)`: the draft the schema names, else 2020-12. The error is the validator's words.
pub fn validate(data: &Value, schema: &Value) -> Result<(), String> {
    jsonschema::validate(schema, data).map_err(|e| e.to_string())
}

/// The sentence the retry sends after an invalid answer, the schema embedded as Python prints it.
pub fn schema_instructions(schema: &Value) -> String {
    format!(
        "Reply with ONE JSON object only, no prose and no code fences, matching this JSON Schema:\n{}",
        pyfmt::dumps(schema)
    )
}

/// `parse_json` then `validate`, the error cut to 200 characters the way `Router._attempt` records it.
pub fn parse_and_validate(text: &str, schema: &Value) -> Result<Value, LlmError> {
    let data = parse_json(text).map_err(|e| LlmError::Schema(cut(&e, 200)))?;
    validate(&data, schema).map_err(|e| LlmError::Schema(cut(&e, 200)))?;
    Ok(data)
}

/// The first `n` characters (Python slices by code point).
pub fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn leftovers_go_into_the_description() {
        let s = json!({"type": "integer", "minimum": 1, "maximum": 10, "description": "A number"});
        assert_eq!(
            transform_schema(&s).unwrap(),
            json!({"type": "integer", "description": "A number\n\n{minimum: 1, maximum: 10}"})
        );
        let s = json!({"type": "string", "format": "slug"});
        assert_eq!(
            transform_schema(&s).unwrap(),
            json!({"type": "string", "description": "{format: slug}"})
        );
    }

    #[test]
    fn a_schema_without_a_type_is_refused() {
        assert!(matches!(
            transform_schema(&json!({"minimum": 1})),
            Err(LlmError::BadSchema(_))
        ));
        assert!(matches!(
            transform_schema(&json!({"type": ["string", "null"]})),
            Err(LlmError::BadSchema(_))
        ));
    }

    #[test]
    fn parse_json_tolerates_fences_and_prose() {
        assert_eq!(parse_json("```json\n{\"a\": 1}\n```").unwrap(), json!({"a": 1}));
        assert_eq!(
            parse_json("Sure: {\"a\": {\"b\": 2}} done").unwrap(),
            json!({"a": {"b": 2}})
        );
        assert!(parse_json("no braces").is_err());
        assert!(parse_json("{\"a\": 1} and {\"b\": 2}").is_err());
    }

    #[test]
    fn validation_speaks() {
        let schema = json!({"type": "object", "properties": {"q": {"type": "array"}}, "required": ["q"]});
        assert!(validate(&json!({"q": []}), &schema).is_ok());
        assert!(validate(&json!({"q": "x"}), &schema).is_err());
        assert!(validate(&json!({}), &schema).is_err());
    }
}
