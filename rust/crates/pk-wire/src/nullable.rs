//! Absent is not null: a few Python responses leave a key out in one case and write `null` in another (a review
//! step's `ai_rating`, a build note's `suggest`, a clips view's `source`). [`Nullable`] keeps the three states apart
//! so a round trip writes back exactly what came in: `None` = no key, `Some(None)` = `null`, `Some(Some(v))` = `v`.
//!
//! Use it as
//! `#[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::nullable::deserialize")]`.

use serde::{Deserialize, Deserializer};

/// Absent (`None`), `null` (`Some(None)`) or a value (`Some(Some(v))`).
pub type Nullable<T> = Option<Option<T>>;

/// Deserialises a present key, `null` included, into the outer `Some`; `#[serde(default)]` supplies the absent case.
pub fn deserialize<'de, T, D>(d: D) -> Result<Nullable<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}
