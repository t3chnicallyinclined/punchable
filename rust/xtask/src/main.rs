//! `cargo xtask <task>`: the gates (fmt, `cargo deny` for advisories / licenses / the ADR 0010 bans, clippy -D warnings
//! per crate, tests per crate) and, later, the differential runners against the Python (docs/RUST.md "Migration
//! order"). One place, so CI and a laptop run the same thing. `cargo deny` needs cargo-deny on the PATH (rust/deny.toml
//! says where it comes from); a missing one fails the gate rather than skipping the check.
use std::process::Command;

fn run(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("cargo").args(args).status()?;
    anyhow::ensure!(status.success(), "cargo {} failed", args.join(" "));
    Ok(())
}

const CRATES: &[&str] = &[
    "pk-core",
    "pk-kb",
    "pk-creative",
    "pk-store",
    "pk-llm",
    "pk-embed",
    "pk-obs",
    "pk-api",
    "pk-worker",
];

fn main() -> anyhow::Result<()> {
    let task = std::env::args().nth(1).unwrap_or_else(|| "gate".to_string());
    match task.as_str() {
        "gate" => {
            run(&["fmt", "--all", "--check"])?;
            run(&["deny", "check"])?;
            for c in CRATES {
                run(&["clippy", "-p", c, "--all-targets", "--", "-D", "warnings"])?;
                run(&["test", "-p", c])?;
            }
            Ok(())
        }
        "deny" => run(&["deny", "check"]),
        other => anyhow::bail!("unknown task {other}; tasks: gate, deny"),
    }
}
