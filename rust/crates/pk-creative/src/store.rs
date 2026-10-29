//! An append-only event log replayed through a reducer into `items`, the way every creative store works
//! (`creative/store.py`): a log is a sequence of [`Event`] lines, `items` is the replay, an append is validated on a
//! copy so a refused event never lands. Where the lines live (a `.jsonl` file, `creative_event` rows) is the caller's
//! business: this type takes lines or values in and hands events back out.

use serde_json::{Value, json};

use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::pyish::Obj;

/// The replayed state of a store: item id to item, in first-seen order.
pub type Items = Obj;

/// One store's rules: fold one event into `items`, or refuse it leaving `items` as it was found (a refused append
/// is applied to a copy, so partial mutation before the refusal is harmless, exactly as in the Python).
pub trait Reducer {
    /// Apply `ev` to `items`.
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()>;
}

impl<R: Reducer + ?Sized> Reducer for Box<R> {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        (**self).apply(items, ev)
    }
}

impl<R: Reducer + ?Sized> Reducer for &R {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        (**self).apply(items, ev)
    }
}

/// The lines of a log as JSON values, the way `Store.events()` reads a file: blank lines and a torn last line (not
/// JSON) are skipped, never fatal.
pub fn parse_lines<I, S>(lines: I) -> Vec<Value>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    lines
        .into_iter()
        .filter_map(|l| {
            let l = l.as_ref().trim();
            if l.is_empty() {
                None
            } else {
                serde_json::from_str(l).ok()
            }
        })
        .collect()
}

/// A store: its reducer, its replayed items and the events behind them.
pub struct Store<R> {
    reducer: R,
    items: Items,
    events: Vec<Event>,
}

impl<R> std::fmt::Debug for Store<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("items", &self.items.len())
            .field("events", &self.events.len())
            .finish_non_exhaustive()
    }
}

impl<R: Reducer> Store<R> {
    /// An empty store.
    pub fn new(reducer: R) -> Self {
        Self {
            reducer,
            items: Items::new(),
            events: Vec::new(),
        }
    }

    /// Replay the text lines of a log (`<name>.jsonl`).
    pub fn replay<I, S>(reducer: R, lines: I) -> CreativeResult<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::from_values(reducer, parse_lines(lines))
    }

    /// Replay parsed lines (what a `creative_event` mirror holds). A value that is not an event, or an event the
    /// reducer refuses, stops the replay with its position.
    pub fn from_values<I>(reducer: R, values: I) -> CreativeResult<Self>
    where
        I: IntoIterator<Item = Value>,
    {
        let mut store = Self::new(reducer);
        for (index, v) in values.into_iter().enumerate() {
            let ev = Event::from_value(v).map_err(|e| CreativeError::Replay {
                index,
                msg: e.msg().to_string(),
            })?;
            store.replay_one(index, ev)?;
        }
        Ok(store)
    }

    /// Replay events already parsed.
    pub fn from_events<I>(reducer: R, events: I) -> CreativeResult<Self>
    where
        I: IntoIterator<Item = Event>,
    {
        let mut store = Self::new(reducer);
        for (index, ev) in events.into_iter().enumerate() {
            store.replay_one(index, ev)?;
        }
        Ok(store)
    }

    fn replay_one(&mut self, index: usize, ev: Event) -> CreativeResult<()> {
        self.reducer
            .apply(&mut self.items, &ev)
            .map_err(|e| CreativeError::Replay {
                index,
                msg: e.msg().to_string(),
            })?;
        self.events.push(ev);
        Ok(())
    }

    /// Append one event: validated on a copy of `items` first, so a refused event changes nothing and is not
    /// recorded. Returns the item the event named, as `Store.append` does.
    pub fn append(&mut self, op: &str, id: &str, data: Value, by: &str, ts: &str) -> CreativeResult<Option<&Value>> {
        self.append_event(Event::new(op, id, data, by, ts))
    }

    /// [`Store::append`] with the event built by the caller.
    pub fn append_event(&mut self, ev: Event) -> CreativeResult<Option<&Value>> {
        let mut probe = self.items.clone();
        self.reducer.apply(&mut probe, &ev)?;
        self.items = probe;
        let id = ev.id.clone();
        self.events.push(ev);
        Ok(self.items.get(&id))
    }

    /// The replayed state.
    pub fn items(&self) -> &Items {
        &self.items
    }

    /// One item.
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.items.get(id)
    }

    /// How many events are behind `items` (`n_events`).
    pub fn n_events(&self) -> usize {
        self.events.len()
    }

    /// Every event that landed, in order; the last one is what a caller persists after an append.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// The last event that landed.
    pub fn last_event(&self) -> Option<&Event> {
        self.events.last()
    }

    /// The reducer.
    pub fn reducer(&self) -> &R {
        &self.reducer
    }

    /// The derived snapshot (`<name>.json`): `{version, updated_at, events, items}`.
    pub fn snapshot(&self, updated_at: &str) -> Value {
        json!({"version": 1, "updated_at": updated_at, "events": self.n_events(), "items": self.items})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A counter per id; refuses the op `bad`.
    struct Count;
    impl Reducer for Count {
        fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
            if ev.op == "bad" {
                return Err(CreativeError::refused("no"));
            }
            let n = items.get(&ev.id).and_then(Value::as_i64).unwrap_or(0);
            items.insert(ev.id.clone(), json!(n + 1));
            Ok(())
        }
    }

    #[test]
    fn torn_lines_are_skipped_and_refusals_never_land() {
        let lines = [
            r#"{"ts":"t","op":"inc","id":"a","by":"owner","data":{}}"#,
            "",
            r#"{"ts":"t","op":"inc","id":"a","by":"owner","data":{}}"#,
            r#"{"ts":"t","op":"inc","id":"b","by":"ow"#,
        ];
        let mut s = Store::replay(Count, lines).unwrap();
        assert_eq!(s.n_events(), 2);
        assert_eq!(s.get("a"), Some(&json!(2)));
        assert!(s.append("bad", "a", json!({}), "owner", "t").is_err());
        assert_eq!(s.n_events(), 2);
        assert_eq!(s.append("inc", "a", json!({}), "owner", "t").unwrap(), Some(&json!(3)));
        assert_eq!(s.snapshot("now")["events"], json!(3));
    }

    #[test]
    fn a_refused_line_stops_a_replay_at_its_index() {
        let lines = [r#"{"ts":"t","op":"inc","id":"a"}"#, r#"{"ts":"t","op":"bad","id":"a"}"#];
        let e = Store::replay(Count, lines).unwrap_err();
        assert_eq!(
            e,
            CreativeError::Replay {
                index: 1,
                msg: "no".into()
            }
        );
        let e = Store::from_values(Count, [json!(3)]).unwrap_err();
        assert!(matches!(e, CreativeError::Replay { index: 0, .. }));
    }
}
