//! The clock: the ISO form every service writes (`2026-10-29T12:00:00+00:00`, what Python's
//! `datetime.now(timezone.utc).isoformat(timespec="seconds")` gives) and the UTC day the budget ledger keys on.
//! Code that stamps a time takes a [`Clock`] so a test passes a [`Fixed`] one; [`now_iso`] and [`today`] are the
//! [`System`] clock for the places that have no reason to be injected.

use chrono::{DateTime, Utc};

/// The timestamp format: seconds, UTC, the offset spelled `+00:00` (never `Z`).
pub const ISO_SECONDS: &str = "%Y-%m-%dT%H:%M:%S+00:00";
/// The day format the ledger keys on.
pub const DAY: &str = "%Y-%m-%d";

/// A source of the current UTC time.
pub trait Clock: Send + Sync {
    /// Now.
    fn now(&self) -> DateTime<Utc>;

    /// Now in [`ISO_SECONDS`].
    fn now_iso(&self) -> String {
        iso(self.now())
    }

    /// Today in [`DAY`].
    fn today(&self) -> String {
        day(self.now())
    }
}

/// The process clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct System;

impl Clock for System {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock stopped at one instant (tests, golden replays).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixed(pub DateTime<Utc>);

impl Fixed {
    /// A fixed clock from an RFC 3339 text (`2026-10-29T12:00:00+00:00`).
    pub fn parse(s: &str) -> Result<Self, chrono::ParseError> {
        DateTime::parse_from_rfc3339(s).map(|t| Self(t.with_timezone(&Utc)))
    }
}

impl Clock for Fixed {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// `t` in [`ISO_SECONDS`].
pub fn iso(t: DateTime<Utc>) -> String {
    t.format(ISO_SECONDS).to_string()
}

/// `t` in [`DAY`].
pub fn day(t: DateTime<Utc>) -> String {
    t.format(DAY).to_string()
}

/// The current UTC time the way the Python writes it: `2026-10-29T12:00:00+00:00`.
pub fn now_iso() -> String {
    System.now_iso()
}

/// Today's UTC day: `2026-10-29`.
pub fn today() -> String {
    System.today()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_clock_prints_the_python_form() {
        let c = Fixed::parse("2026-10-29T12:00:05+00:00").expect("parses");
        assert_eq!(c.now_iso(), "2026-10-29T12:00:05+00:00");
        assert_eq!(c.today(), "2026-10-29");
        let shifted = Fixed::parse("2026-10-29T23:30:00-02:00").expect("parses");
        assert_eq!(shifted.now_iso(), "2026-10-30T01:30:00+00:00", "normalised to UTC");
        assert!(Fixed::parse("yesterday").is_err());
    }

    #[test]
    fn the_system_clock_has_the_same_shape() {
        let s = now_iso();
        assert_eq!(s.len(), 25, "{s}");
        assert!(s.ends_with("+00:00"), "{s}");
        assert_eq!(today().len(), 10);
        let dynamic: Box<dyn Clock> = Box::new(System);
        assert!(dynamic.now_iso().ends_with("+00:00"));
    }
}
