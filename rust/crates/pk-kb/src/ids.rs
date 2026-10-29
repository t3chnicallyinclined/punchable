//! Typed ids. A node id is `<prefix>:<key>` (`joke:`, `unit:`, `set:`, `perf:`, `label:`, `list:`; the key may hold
//! colons); an edge id is `e:` plus 12 hex characters of the sha1 of `src|kind|dst`.

use std::fmt;
use std::str::FromStr;

use sha1::{Digest, Sha1};

use crate::error::KbError;

/// What a node is, by its id prefix. `Perf` is the `performance` table in the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// A bit: `joke:`.
    Joke,
    /// One written version of a bit: `unit:`.
    Unit,
    /// A recorded set: `set:`.
    Set,
    /// A stretch of a set's tape: `perf:` (table `performance`).
    Perf,
    /// A label: `label:`.
    Label,
    /// A list (tonight's): `list:`.
    List,
}

impl NodeKind {
    /// The id prefix.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Joke => "joke",
            Self::Unit => "unit",
            Self::Set => "set",
            Self::Perf => "perf",
            Self::Label => "label",
            Self::List => "list",
        }
    }

    /// The node `type` field and the database table.
    pub fn type_name(self) -> &'static str {
        match self {
            Self::Perf => "performance",
            other => other.prefix(),
        }
    }

    fn from_prefix(p: &str) -> Option<Self> {
        Some(match p {
            "joke" => Self::Joke,
            "unit" => Self::Unit,
            "set" => Self::Set,
            "perf" => Self::Perf,
            "label" => Self::Label,
            "list" => Self::List,
            _ => return None,
        })
    }
}

/// A node id, validated: a known prefix and a non-empty key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeId {
    kind: NodeKind,
    key: String,
}

impl NodeId {
    /// The kind, from the prefix.
    pub fn kind(&self) -> NodeKind {
        self.kind
    }

    /// Everything after the first colon (may itself hold colons).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The node `type` field and the database table (`perf:` is `performance`).
    pub fn type_name(&self) -> &'static str {
        self.kind.type_name()
    }
}

impl FromStr for NodeId {
    type Err = KbError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (p, key) = s
            .split_once(':')
            .ok_or_else(|| KbError::bad(format!("node id '{s}' has no prefix")))?;
        let kind =
            NodeKind::from_prefix(p).ok_or_else(|| KbError::bad(format!("node id '{s}': unknown prefix '{p}'")))?;
        if key.is_empty() {
            return Err(KbError::bad(format!("node id '{s}' has no key")));
        }
        Ok(Self {
            kind,
            key: key.to_string(),
        })
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind.prefix(), self.key)
    }
}

/// An edge id: `e:` + 12 hex characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EdgeId(String);

impl EdgeId {
    /// The 12 hex characters after `e:`.
    pub fn key(&self) -> &str {
        &self.0[2..]
    }
}

impl FromStr for EdgeId {
    type Err = KbError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ok = s.len() == 14
            && s.starts_with("e:")
            && s[2..].chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        if ok {
            Ok(Self(s.to_string()))
        } else {
            Err(KbError::bad(format!("edge id '{s}' is not e:<12 hex>")))
        }
    }
}

impl fmt::Display for EdgeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The edge id for `src`, `kind` (the custom kind for a custom edge) and `dst`: `e:` + sha1(`src|kind|dst`)[..12].
pub fn edge_id(src: &str, kind: &str, dst: &str) -> String {
    let digest = Sha1::digest(format!("{src}|{kind}|{dst}").as_bytes());
    format!("e:{}", &hex::encode(digest)[..12])
}

/// A url-ish slug of a title: lower-cased, runs of anything but `a-z0-9` become `-`, trimmed of `-`, at most 60
/// characters, `x` when nothing is left. The Python `slugify` (its `[^a-z0-9]+` runs over code points, so does this).
pub fn slugify(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut dash = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    let cut: String = trimmed.chars().take(60).collect();
    if cut.is_empty() { "x".to_string() } else { cut }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_parse_and_print() {
        let n: NodeId = "perf:2015-02-18_x@1200".parse().unwrap();
        assert_eq!(n.kind(), NodeKind::Perf);
        assert_eq!(n.key(), "2015-02-18_x@1200");
        assert_eq!(n.type_name(), "performance");
        assert_eq!(n.to_string(), "perf:2015-02-18_x@1200");
        let l: NodeId = "label:topic:money".parse().unwrap();
        assert_eq!(l.key(), "topic:money");
        assert!("blob:x".parse::<NodeId>().is_err());
        assert!("joke:".parse::<NodeId>().is_err());
        assert!("nocolon".parse::<NodeId>().is_err());
    }

    #[test]
    fn edge_ids() {
        assert!("e:0123456789ab".parse::<EdgeId>().is_ok());
        assert!("e:0123456789AB".parse::<EdgeId>().is_err());
        assert!("e:0123".parse::<EdgeId>().is_err());
        assert_eq!(edge_id("a", "k", "b").len(), 14);
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Parking meters"), "parking-meters");
        assert_eq!(slugify("  Hello, World!  "), "hello-world");
        assert_eq!(slugify(""), "x");
        assert_eq!(slugify("--x--"), "x");
        assert_eq!(slugify(&"a".repeat(80)).len(), 60);
    }
}
