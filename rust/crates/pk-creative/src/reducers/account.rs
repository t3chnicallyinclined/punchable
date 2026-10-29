//! His account (`creative_api.account_apply`): the settings he can change (item `settings`) and his deletion
//! requests (item `deletion`). A request only records that he asked; an owner deletes by hand.

use serde_json::{Value, json};

use super::{list_mut, unknown_op};
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::pyish::{Obj, get_or_null, or_else};
use crate::store::{Items, Reducer};

/// The settings he can change (`SETTINGS_DEFAULTS`' keys), in order.
pub const SETTING_KEYS: [&str; 4] = ["provider", "training", "upload_video", "cellular"];
/// The default provider.
pub const DEFAULT_PROVIDER: &str = "hosted";
/// The providers the app offers (`PROVIDERS`).
pub const PROVIDERS: [&str; 4] = ["hosted", "chatgpt", "key", "custom"];

/// `{"id": id, **SETTINGS_DEFAULTS, "updated_at": None}`.
fn defaults(id: &str) -> Value {
    json!({"id": id, "provider": DEFAULT_PROVIDER, "training": false, "upload_video": false, "cellular": false,
           "updated_at": null})
}

/// `check_settings(d)`: provider plus three switches; unknown keys refused.
pub fn check_settings(d: &Obj) -> CreativeResult<()> {
    if d.is_empty() {
        return Err(CreativeError::refused(
            "send one or more of provider, training, upload_video, cellular",
        ));
    }
    let mut bad: Vec<&str> = d
        .keys()
        .map(String::as_str)
        .filter(|k| !SETTING_KEYS.contains(k))
        .collect();
    bad.sort_unstable();
    if !bad.is_empty() {
        return Err(CreativeError::refused(format!(
            "unknown setting(s): {}",
            bad.join(", ")
        )));
    }
    if let Some(p) = d.get("provider")
        && !p.as_str().is_some_and(|p| PROVIDERS.contains(&p))
    {
        return Err(CreativeError::refused(format!(
            "provider must be one of {}",
            PROVIDERS.join(", ")
        )));
    }
    for k in ["training", "upload_video", "cellular"] {
        if let Some(v) = d.get(k)
            && !v.is_boolean()
        {
            return Err(CreativeError::refused(format!("{k} must be true or false")));
        }
    }
    Ok(())
}

/// The account reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Account;

impl Reducer for Account {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, i, d) = (ev.op.as_str(), ev.id.as_str(), ev.data()?);
        match op {
            "settings" => {
                if i != "settings" {
                    return Err(CreativeError::refused("settings live on item 'settings'"));
                }
                let it = items
                    .entry(i.to_string())
                    .or_insert_with(|| defaults(i))
                    .as_object_mut()
                    .ok_or_else(|| CreativeError::faulted("'NoneType' object has no attribute 'update'"))?;
                check_settings(d)?;
                for (k, v) in d {
                    it.insert(k.clone(), v.clone());
                }
                it.insert("updated_at".into(), json!(ev.ts));
            }
            "request_deletion" => {
                if i != "deletion" {
                    return Err(CreativeError::refused("deletion requests live on item 'deletion'"));
                }
                let it = items
                    .entry(i.to_string())
                    .or_insert_with(|| json!({"id": i, "requests": []}))
                    .as_object_mut()
                    .ok_or_else(|| CreativeError::faulted("'NoneType' object is not subscriptable"))?;
                list_mut(it, "requests")?.push(json!({
                    "at": ev.ts, "by_sub": get_or_null(d, "by_sub"),
                    "would_delete": or_else(d, "would_delete", json!([])), "status": "requested",
                }));
            }
            _ => return Err(unknown_op(op)),
        }
        Ok(())
    }
}
