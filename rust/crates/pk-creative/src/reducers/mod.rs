//! One reducer per creative store, each the port of a Python `*_apply` with its ops, its item shape and its refusal
//! texts: [`jobs`] (`creative_api.job_apply`), [`story`] (`creative/story._apply`), [`workingset`]
//! (`creative/workingset._apply`), [`venues`], [`account`] (`creative_api`), [`merges`] (`creative/merge.store_apply`),
//! [`chats`] (`ask_api.chat_apply`), [`intents`] (`say_api.intent_apply`), [`suggestions`]
//! (`suggest_api.store_apply`), [`profile`] (`profile_api.profile_apply`, with [`learned`]). [`for_store`] is the
//! `APPLY` map: a reducer by store name.

pub mod account;
pub mod chats;
pub mod intents;
pub mod jobs;
pub mod learned;
pub mod merges;
pub mod profile;
pub mod story;
pub mod suggestions;
pub mod venues;
pub mod workingset;

use serde_json::Value;

use crate::error::{CreativeError, CreativeResult};
use crate::pyish::Obj;
use crate::store::{Items, Reducer};
use pk_core::py;

/// The store names, as the API and the `creative_event` mirror key them.
pub const STORES: [&str; 10] = [
    "storyboards",
    "workingsets",
    "venues",
    "jobs",
    "account",
    "merges",
    "chats",
    "intents",
    "suggestions",
    "profile",
];

/// The reducer for a store name (`creative_api.APPLY` after every route module registered). `owner_id` is the
/// actor id of his own edits (`PK_OWNER_ID`), which storyboards and working sets compare `by` against.
pub fn for_store(name: &str, owner_id: &str) -> Option<Box<dyn Reducer + Send + Sync>> {
    Some(match name {
        "storyboards" => Box::new(story::Storyboards::new(owner_id)),
        "workingsets" => Box::new(workingset::Workingsets::new(owner_id)),
        "venues" => Box::new(venues::Venues),
        "jobs" => Box::new(jobs::Jobs),
        "account" => Box::new(account::Account),
        "merges" => Box::new(merges::Merges),
        "chats" => Box::new(chats::Chats),
        "intents" => Box::new(intents::Intents),
        "suggestions" => Box::new(suggestions::Suggestions),
        "profile" => Box::new(profile::Profile),
        _ => return None,
    })
}

/// `raise ValueError(f"unknown op {op}")`.
fn unknown_op(op: &str) -> CreativeError {
    CreativeError::refused(format!("unknown op {op}"))
}

/// The item `id` as an object, when it exists.
fn item_mut<'a>(items: &'a mut Items, id: &str) -> Option<&'a mut Obj> {
    items.get_mut(id).and_then(Value::as_object_mut)
}

/// The item `id` as an object, when it exists.
fn item<'a>(items: &'a Items, id: &str) -> Option<&'a Obj> {
    items.get(id).and_then(Value::as_object)
}

/// `d[k]` as a list to mutate, or the `KeyError`; a non-list is the Python's `AttributeError` on the next call.
fn list_mut<'a>(d: &'a mut Obj, k: &str) -> CreativeResult<&'a mut Vec<Value>> {
    match d.get_mut(k) {
        None => Err(CreativeError::missing(k)),
        Some(Value::Array(a)) => Ok(a),
        Some(v) => Err(CreativeError::faulted(format!(
            "'{}' object has no attribute 'append'",
            py::type_name(v)
        ))),
    }
}

/// `d[k]` as a list to read, or the `KeyError`.
fn list_of<'a>(d: &'a Obj, k: &str) -> CreativeResult<&'a Vec<Value>> {
    match d.get(k) {
        None => Err(CreativeError::missing(k)),
        Some(Value::Array(a)) => Ok(a),
        Some(v) => Err(CreativeError::faulted(format!(
            "'{}' object is not iterable",
            py::type_name(v)
        ))),
    }
}

/// `d[k]` as an object to mutate, or the `KeyError`.
fn obj_mut<'a>(d: &'a mut Obj, k: &str) -> CreativeResult<&'a mut Obj> {
    match d.get_mut(k) {
        None => Err(CreativeError::missing(k)),
        Some(Value::Object(o)) => Ok(o),
        Some(v) => Err(CreativeError::faulted(format!(
            "'{}' object has no attribute 'setdefault'",
            py::type_name(v)
        ))),
    }
}
