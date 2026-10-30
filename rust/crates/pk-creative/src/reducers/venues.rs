//! Venues (`creative_api.venue_apply`): where he moves or renames a map pin. Ops: `pin {lat, lon}`, `rename {name}`,
//! `reset`.

use serde_json::{Value, json};

use super::unknown_op;
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::pyish::strip_or_empty;
use crate::store::{Items, Reducer};
use pk_core::py;

/// The venue reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Venues;

fn number(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        _ => None,
    }
}

impl Reducer for Venues {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, i, d) = (ev.op.as_str(), ev.id.as_str(), ev.data()?);
        let v = items
            .entry(i.to_string())
            .or_insert_with(
                || json!({"venue_id": i, "lat": null, "lon": null, "name": null, "by": null, "updated_at": null}),
            )
            .as_object_mut()
            .ok_or_else(|| CreativeError::faulted("'NoneType' object does not support item assignment"))?;
        match op {
            "pin" => {
                let (Some(lat), Some(lon)) = (number(d.get("lat")), number(d.get("lon"))) else {
                    return Err(CreativeError::refused("lat and lon must be numbers"));
                };
                if !((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)) {
                    return Err(CreativeError::refused("lat/lon out of range"));
                }
                v.insert("lat".into(), json!(lat));
                v.insert("lon".into(), json!(lon));
            }
            "rename" => {
                let name = strip_or_empty(d, "name")?;
                if name.is_empty() || py::len(name) > 120 {
                    return Err(CreativeError::refused("name must be 1-120 characters"));
                }
                v.insert("name".into(), json!(name));
            }
            "reset" => {
                v.insert("lat".into(), Value::Null);
                v.insert("lon".into(), Value::Null);
                v.insert("name".into(), Value::Null);
            }
            _ => return Err(unknown_op(op)),
        }
        v.insert("by".into(), ev.by.clone());
        v.insert("updated_at".into(), json!(ev.ts));
        Ok(())
    }
}
