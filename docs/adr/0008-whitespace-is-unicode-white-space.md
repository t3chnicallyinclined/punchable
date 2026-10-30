# ADR 0008: whitespace means Unicode White_Space

Date: 2026-10-29. Status: accepted.

**Context.** Python's `str.isspace` includes U+001C to U+001F; Rust's `char::is_whitespace` does not
(`docs/RUST.md` fact 4).

**Decision.** The verbatim gate defines whitespace as Unicode White_Space; the differential allows this difference. A
Python behaviour that costs nothing to keep is still kept exactly; the allow-list is for things that cannot hold.
