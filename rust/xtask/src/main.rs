//! `cargo xtask <task>`: the gates (fmt, clippy -D warnings per crate, tests per crate) and, later, the differential
//! runners against the Python (docs/RUST.md "Migration order"). One place, so CI and a laptop run the same thing.
use std::process::Command;

fn run(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("cargo").args(args).status()?;
    anyhow::ensure!(status.success(), "cargo {} failed", args.join(" "));
    Ok(())
}

const CRATES: &[&str] = &[
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
            for c in CRATES {
                run(&["clippy", "-p", c, "--all-targets", "--", "-D", "warnings"])?;
                run(&["test", "-p", c])?;
            }
            Ok(())
        }
        other => anyhow::bail!("unknown task {other}; tasks: gate"),
    }
}
