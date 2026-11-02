//! The fork's boundary, as a test that can fail: nothing that runs a process, reads the environment or the secrets
//! file, touches files or the network, or reaches for a secret came across from upstream (exec, secrets, census,
//! hooks, the MCP server, the window), and nothing may enter later.
//!
//! Two scans of this crate's own sources (`src/`, `schema/`), with `//` comments removed and every whitespace
//! character dropped, so `std :: process`, `use std::{fs, process}` and a path split across lines are caught: one
//! line by line (to name the line), one over the whole file joined (to catch the split). And a manifest check over
//! every dependency table, renamed packages included, plus no `build.rs`.
//!
//! Red arms (FORK.md): `the_scan_can_fail` (in process, over a temporary copy) and, by hand, adding
//! `std::process::Command::new("sh")` to any module.

use std::path::{Path, PathBuf};

/// What may never appear in the engine's code, whitespace removed, and why.
const BANNED: &[(&str, &str)] = &[
    ("std::process", "runs a process"),
    ("Command::new", "runs a process"),
    ("std::{", "a grouped std import (could hide fs, env, net, process)"),
    (
        "std::env",
        "reads the environment (a binary builds pk_core::config::Env once)",
    ),
    ("env!(", "reads the build environment"),
    ("option_env!(", "reads the build environment"),
    ("pk_core::config", "reads secrets.env and the environment"),
    ("Env::", "reads secrets.env and the environment"),
    ("from_process", "reads secrets.env and the environment"),
    ("std::fs", "touches files"),
    ("std::io", "touches files or streams"),
    ("File::", "touches files"),
    ("include_bytes!(", "embeds a file"),
    ("#[path", "pulls a module from outside src/"),
    ("std::net", "opens a socket"),
    ("TcpStream", "opens a socket"),
    (
        "tokio::",
        "a runtime in a pure crate (only #[tokio::test] in test modules)",
    ),
    ("reqwest", "talks HTTP"),
    ("ureq", "talks HTTP (upstream's blocking client)"),
    ("surrealdb", "talks to the database (pk-store's job)"),
    ("secrets.env", "names the secrets file"),
    ("owner_pass", "carries a password (upstream's owner login)"),
    ("unsafe", "unsafe code"),
];

/// The one `include_str!` the crate may hold: its own schema.
const SCHEMA_INCLUDE: &str = "include_str!(\"../schema/engine.surql\")";

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs" || e == "surql") {
            out.push(path);
        }
    }
}

/// A line's code: everything before `//` (a `//` inside a string ends the scan of that line early, so such a line
/// is also scanned whole by the file pass), with whitespace dropped and the allowed forms removed.
fn code_of(line: &str) -> String {
    let code: String = line
        .split("//")
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    code.replace("#[tokio::test]", "")
        .replace(&SCHEMA_INCLUDE.replace(' ', ""), "")
}

fn hits(code: &str, at: &str, found: &mut Vec<String>) {
    for (pat, why) in BANNED {
        if code.contains(pat) {
            found.push(format!("{at}: `{pat}` {why}"));
        }
    }
    if code.contains("include_str!(") {
        found.push(format!("{at}: `include_str!` other than the schema embeds a file"));
    }
}

fn violations(root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    for sub in ["src", "schema"] {
        files(&root.join(sub), &mut paths);
    }
    assert!(paths.len() >= 15, "the scan found the sources: {paths:?}");
    let mut found = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path).expect("readable source");
        let mut joined = String::new();
        let mut by_line = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let code = code_of(line);
            hits(&code, &format!("{}:{}", path.display(), n + 1), &mut by_line);
            joined.push_str(&code);
        }
        let mut whole = Vec::new();
        hits(&joined, &format!("{} (across lines)", path.display()), &mut whole);
        // The joined scan only adds what no single line showed (a path split over lines).
        let lines_saw = |w: &String| {
            by_line
                .iter()
                .any(|l| l.split_once(": `").map(|x| x.1) == w.split_once(": `").map(|x| x.1))
        };
        whole.retain(|w| !lines_saw(w));
        found.extend(by_line);
        found.extend(whole);
    }
    if root.join("build.rs").exists() {
        found.push("build.rs: a build script runs code at build time".to_owned());
    }
    found
}

#[test]
fn no_process_environment_file_network_or_secret_path_is_in_the_engine() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let found = violations(root);
    assert!(found.is_empty(), "the engine must stay pure:\n{}", found.join("\n"));
}

#[test]
fn the_scan_can_fail() {
    let dir = std::env::temp_dir().join(format!("pk-engine-boundary-{}", std::process::id()));
    let src = dir.join("src");
    std::fs::create_dir_all(&src).expect("temp dir");
    std::fs::create_dir_all(dir.join("schema")).expect("temp dir");
    for i in 0..15 {
        std::fs::write(
            src.join(format!("m{i}.rs")),
            "pub fn f() {}\n// std::process in a comment is fine\n",
        )
        .expect("write");
    }
    let bad = [
        (
            "a.rs",
            "pub fn g() { let _ = std::process::Command::new(\"sh\"); }\n",
            "std::process",
        ),
        ("b.rs", "use std :: { fs , process };\n", "std::{"),
        (
            "c.rs",
            "pub fn h() { let _ = std::\n    env::var(\"X\"); }\n",
            "std::env",
        ),
        (
            "d.rs",
            "pub fn k() { let _ = pk_core::config::Env::from_process(); }\n",
            "pk_core::config",
        ),
        (
            "e.rs",
            "const X: &[u8] = include_bytes!(\"../../secrets\");\n",
            "include_bytes!(",
        ),
        ("f.rs", "#[path = \"../../x.rs\"] mod x;\n", "#[path"),
    ];
    for (name, body, _) in bad {
        std::fs::write(src.join(name), body).expect("write");
    }
    let found = violations(&dir);
    std::fs::remove_dir_all(&dir).ok();
    for (name, _, pat) in bad {
        assert!(
            found.iter().any(|f| f.contains(name) && f.contains(pat)),
            "{name} with `{pat}` was not caught: {found:#?}"
        );
    }
    assert!(
        found.iter().all(|f| !f.contains("m0.rs")),
        "a comment is not code: {found:#?}"
    );
}

#[test]
fn no_dependency_table_holds_an_io_crate() {
    const IO: &[&str] = &[
        "tokio",
        "surrealdb",
        "reqwest",
        "ureq",
        "hyper",
        "pk-store",
        "pk-embed",
        "pk-llm",
        "pk-obs",
        "pk-api",
        "pk-worker",
        "ort",
        "tokenizers",
        "axum",
        "sentry",
        "dirs",
        "tempfile",
    ];
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).expect("manifest");
    let manifest: toml::Table = text.parse().expect("a TOML manifest");
    let mut tables: Vec<(String, &toml::Table)> = Vec::new();
    for key in ["dependencies", "build-dependencies"] {
        if let Some(t) = manifest.get(key).and_then(toml::Value::as_table) {
            tables.push((key.to_owned(), t));
        }
    }
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (cfg, t) in targets {
            for key in ["dependencies", "build-dependencies"] {
                if let Some(d) = t.get(key).and_then(toml::Value::as_table) {
                    tables.push((format!("target.{cfg}.{key}"), d));
                }
            }
        }
    }
    assert!(!tables.is_empty(), "the manifest has a dependencies table");
    for (where_, deps) in tables {
        for (name, spec) in deps {
            let package = spec.get("package").and_then(toml::Value::as_str).unwrap_or(name);
            assert!(
                !IO.contains(&package),
                "{where_}: {package} is an IO dependency of a pure crate"
            );
        }
    }
}
