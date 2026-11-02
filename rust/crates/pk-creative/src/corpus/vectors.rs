//! The bge-small vectors the notebook carries (`creative/data.py` `Vectors`, fed the way `worker/online.py`
//! `rebuild_vectors` feeds it): the `vectors/<cache>.partNNN.json` rows regrouped by cache (`vectors.f32` holds
//! `unit:` / `reveal:` keys, `vectors_v1.f32` the v1 `areveal:` keys; the first cache to name a key keeps it), then
//! every `vectors/extra_*.json` in name order for keys no cache had. float32, as numpy stores them.

use std::collections::HashMap;

use serde_json::Value;

/// The vector width.
pub const DIM: usize = 384;

/// One `vectors/<cache>.partNNN.json` file.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct VectorPart {
    /// `vectors.f32` or `vectors_v1.f32`.
    pub cache: String,
    /// Its ids file name.
    #[serde(default)]
    pub ids_file: String,
    /// The keys.
    pub ids: Vec<String>,
    /// One row per key.
    pub rows: Vec<Vec<f64>>,
}

/// Keys to float32 rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vectors {
    row: HashMap<String, usize>,
    m: Vec<f32>,
}

impl Vectors {
    /// From the part files (in path order) and the extra files (`(file name, parsed JSON)`, any order).
    pub fn from_parts(parts: &[VectorPart], extras: &[(String, Value)]) -> Self {
        let mut v = Self::default();
        let mut n = 0usize;
        for cache in ["vectors.f32", "vectors_v1.f32"] {
            let (mut ids, mut rows): (Vec<&String>, Vec<&Vec<f64>>) = (Vec::new(), Vec::new());
            for p in parts.iter().filter(|p| p.cache == cache) {
                ids.extend(&p.ids);
                rows.extend(&p.rows);
            }
            // numpy only reshapes a cache whose size is ids × 384
            if ids.is_empty() || rows.len() != ids.len() || rows.iter().any(|r| r.len() != DIM) {
                continue;
            }
            for (i, k) in ids.iter().enumerate() {
                v.row.entry((*k).clone()).or_insert(n + i);
            }
            v.m.extend(rows.iter().flat_map(|r| r.iter().map(|x| *x as f32)));
            n += ids.len();
        }
        let mut extras: Vec<&(String, Value)> = extras.iter().collect();
        extras.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, doc) in extras {
            let Some(rows) = doc.get("rows").and_then(Value::as_object) else {
                continue;
            };
            for (k, r) in rows {
                let Some(r) = r.as_array().filter(|r| r.len() == DIM) else {
                    continue;
                };
                if v.row.contains_key(k) {
                    continue;
                }
                v.row.insert(k.clone(), n);
                v.m.extend(r.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32));
                n += 1;
            }
        }
        v
    }

    /// Whether any vector is loaded.
    pub fn available(&self) -> bool {
        !self.row.is_empty()
    }

    /// How many keys.
    pub fn len(&self) -> usize {
        self.row.len()
    }

    /// No keys.
    pub fn is_empty(&self) -> bool {
        self.row.is_empty()
    }

    /// `get(key)`: the row.
    pub fn get(&self, key: &str) -> Option<&[f32]> {
        let i = *self.row.get(key)?;
        self.m.get(i * DIM..(i + 1) * DIM)
    }

    /// `reveal(uid)`: the v1 anatomy reveal, else the v0 reveal.
    pub fn reveal(&self, uid: &str) -> Option<&[f32]> {
        let bare = uid.strip_prefix("unit:").unwrap_or(uid);
        self.get(&format!("areveal:{bare}"))
            .or_else(|| self.get(&format!("reveal:{bare}")))
    }
}
