//! The command line (`gsync/__main__.py` and `gsync/__init__.py` `run_once`):
//! `pk-gsync auth | status | run [--dry-run] [--watch [--interval 60]] | undo <run_id>`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::time::Duration;

use clap::{Parser, Subcommand};
use pk_core::{Env, py, text::head};
use serde_json::{Map, Value, json};

use crate::auth::{load_creds, run_auth, token_info};
use crate::config::Config;
use crate::drive::GoogleDrive;
use crate::error::{GsError, Result};
use crate::memos::{MemoStore, processor_running, progress_line};
use crate::punchable::{Hosted, Local, WorkerAuth};
use crate::rules::Rules;
use crate::services::Services;
use crate::store::{Store, read_jsonl};
use crate::sync::Sync;

/// The arguments.
#[derive(Debug, Parser)]
#[command(name = "pk-gsync", about = "Google Docs <-> Punchable sync (the Rust gsync)")]
pub struct Args {
    /// What to do.
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// The commands, named as the Python's.
#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Sign in (once): Punchable's "Sign in with Google" via Keycloak.
    Auth {
        /// Fallback: a separate Desktop-app Google client (not in pk-gsync).
        #[arg(long)]
        desktop: bool,
    },
    /// What is set up and what is missing.
    Status,
    /// One sync pass.
    Run {
        /// Read only: plan the changes, write nothing to Drive, Punchable or the state.
        #[arg(long)]
        dry_run: bool,
        /// Keep running.
        #[arg(long)]
        watch: bool,
        /// Seconds between passes with --watch (at least 15).
        #[arg(long, default_value_t = 60)]
        interval: i64,
    },
    /// Undo one run's Punchable changes.
    Undo {
        /// The run id (`status` and `kb/gsync/runs.jsonl` list them).
        run_id: String,
    },
}

/// One line of `status` and `run`: the run id, the mode, the counts and the first errors.
pub fn summary(rec: &Value) -> String {
    let mut counts: Vec<(String, String)> = rec
        .get("counts")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), py::str(v))).collect())
        .unwrap_or_default();
    counts.sort();
    let c = if counts.is_empty() {
        "nothing to do".to_string()
    } else {
        counts
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let errors = rec.get("errors").and_then(Value::as_array).cloned().unwrap_or_default();
    let run_id = rec.get("run_id").map_or_else(|| "-".to_string(), py::str);
    let dry = if py::truthy(rec.get("dry_run")) { " DRY RUN" } else { "" };
    let mut line = format!(
        "{run_id} [{}{dry}] {c}",
        py::str(rec.get("mode").unwrap_or(&Value::Null))
    );
    if !errors.is_empty() {
        let first: Vec<String> = errors
            .iter()
            .take(3)
            .map(|e| head(&e.get("error").map_or_else(String::new, py::str), 160))
            .collect();
        let _ = write!(line, "; {} error(s): {}", errors.len(), first.join("; "));
    }
    line
}

fn no_auth(msg: &str) -> Value {
    json!({"mode": "no_auth", "errors": [{"where": "auth", "error": msg}], "actions": []})
}

/// `run_once`: one sync pass. Never fails for Drive or API trouble: problems are in the record's `errors` / `mode`.
pub fn run_once(dry_run: bool, cfg: &Config, env: &Env, svc: &Services, rules: &Rules) -> Result<Value> {
    let creds = match load_creds(cfg, svc, true) {
        Ok(Some(c)) => c,
        Ok(None) => return Ok(no_auth("no token: run `python -m gsync auth`")),
        Err(e @ GsError::Exit(_)) => return Err(e),
        Err(e) => return Ok(no_auth(&e.describe())),
    };
    if !cfg.online_first {
        return Err(GsError::Exit(
            "pk-gsync runs online-first only: set PK_ONLINE_FIRST=1 (the local-API mode needs the Python API, which is being retired)".into(),
        ));
    }
    let auth = WorkerAuth::from_env(env)?;
    let mut pk = Hosted::new(cfg, svc, auth, "import:gsync", dry_run);
    if !dry_run {
        let _ = pk.pull_state(); // online-first on a fresh PC: gsync's state comes back from the database
    }
    let drive = GoogleDrive::new(creds, svc, rules, dry_run);
    let mut sync = Sync::new(cfg, svc, rules, drive, pk, dry_run)?;
    let mut rec = sync.run()?;
    if !dry_run {
        let force = py::truthy(rec.get("actions")) || py::truthy(rec.get("errors"));
        if let Err(e) = sync.pk.push_state(&[], force) {
            // online-first: gsync's state (map, memos, runs) is posted to the database
            let msg = head(&e.describe(), 300);
            match rec.get_mut("errors").and_then(Value::as_array_mut) {
                Some(a) => a.push(json!({"where": "push_state", "error": msg})),
                None => rec["errors"] = json!([{"where": "push_state", "error": msg}]),
            }
        }
    }
    Ok(rec)
}

fn status(cfg: &Config, svc: &Services, out: &mut dyn Write) -> Result<i32> {
    let mut missing: Vec<String> = Vec::new();
    let _ = writeln!(out, "gsync status");
    let _ = writeln!(out, "  libraries          ok");
    let t = token_info(cfg);
    if t.desktop_fallback {
        let _ = writeln!(
            out,
            "  auth mode          desktop fallback ({})",
            cfg.token_file().display()
        );
    } else {
        let g = if t.google_env_keys {
            "ok"
        } else if t.google_env {
            "INCOMPLETE"
        } else {
            "MISSING"
        };
        let _ = writeln!(out, "  google-oauth.env   {g}  {}", cfg.google_env().display());
        if !t.google_env_keys {
            missing.push("SETUP.md steps 1-5: Google Cloud project + Web client; put GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET in that file".into());
        }
        let _ = writeln!(
            out,
            "  Keycloak sign-in   {}  {}",
            if t.keycloak_signed_in { "ok" } else { "MISSING" },
            cfg.kc_token_file().display()
        );
        if !t.keycloak_signed_in {
            missing.push("SETUP.md steps 6-9 (realm on the server, link Google), then: python -m gsync auth".into());
        }
    }
    if t.keycloak_signed_in || t.desktop_fallback {
        match load_creds(cfg, svc, true) {
            Ok(_) => {
                let _ = writeln!(out, "  Google token       ok (refreshed just now)");
            }
            Err(e @ GsError::Exit(_)) => return Err(e),
            Err(e) => {
                // messages carry the fix, never a token
                let _ = writeln!(out, "  Google token       FAILED: {e}");
                missing.push(
                    "if this fails about every 7 days, the consent screen is still 'Testing': SETUP.md step 3c".into(),
                );
            }
        }
    }
    let up = Local::new(cfg).health();
    let _ = writeln!(
        out,
        "  Punchable API      {}  {}",
        if up { "ok" } else { "NOT RUNNING" },
        cfg.api
    );
    if !up {
        missing.push("start the local API: python api-dev/server.py".into());
    }
    let st = Store::open(&cfg.state())?;
    let m = &st.m;
    let docs = m["docs"].as_object().cloned().unwrap_or_default();
    // `{dict(origins)}` prints a dict repr whose keys are the origins; None prints bare
    let repr = origin_repr(&docs);
    let len = |k: &str| m.get(k).and_then(Value::as_object).map_or(0, Map::len);
    let _ = writeln!(out, "  map                {}", cfg.state().join("map.json").display());
    let boot = m
        .get("bootstrapped_at")
        .filter(|v| py::truthy(Some(v)))
        .map_or_else(|| "not yet (the first run bootstraps)".to_string(), py::str);
    let _ = writeln!(out, "    bootstrapped     {boot}");
    let hidden = docs.values().filter(|e| py::truthy(e.get("hidden"))).count();
    let _ = writeln!(out, "    docs             {repr}; hidden {hidden}");
    let _ = writeln!(
        out,
        "    legal excluded   {} (IDs only; never exported)",
        len("excluded")
    );
    let _ = writeln!(
        out,
        "    not comedy       {}; pending {}",
        len("skipped"),
        len("pending")
    );
    let _ = writeln!(out, "    boards           {}", len("boards"));
    let runs = read_jsonl(&cfg.state().join("runs.jsonl"));
    let _ = writeln!(
        out,
        "  last run           {}",
        runs.last().map_or_else(|| "never".to_string(), summary)
    );
    let memos = MemoStore::new(&cfg.memos_state(), svc).load();
    let _ = writeln!(
        out,
        "  voice memos        {}{}",
        progress_line(&memos),
        if processor_running(cfg) {
            " (transcribing now)"
        } else {
            ""
        }
    );
    let conf = read_jsonl(&cfg.state().join("conflicts.jsonl"));
    if !conf.is_empty() {
        let _ = writeln!(
            out,
            "  conflicts          {} (comedy/kb/gsync/conflicts.jsonl); latest:",
            conf.len()
        );
        for c in &conf[conf.len().saturating_sub(5)..] {
            let f = |k: &str| py::str(c.get(k).unwrap_or(&Value::Null));
            let _ = writeln!(out, "    {}  {}  {}", f("ts"), f("kind"), f("title"));
        }
    }
    if !missing.is_empty() {
        let _ = writeln!(out, "\nTo do:");
        for (n, x) in missing.iter().enumerate() {
            let _ = writeln!(out, "  {}. {x}", n + 1);
        }
        return Ok(1);
    }
    let _ = writeln!(out, "\nReady.");
    Ok(0)
}

/// `dict(Counter(origin of each doc))` printed as Python prints a dict.
fn origin_repr(docs: &Map<String, Value>) -> String {
    let mut order: Vec<String> = Vec::new();
    let mut n: BTreeMap<String, i64> = BTreeMap::new();
    for e in docs.values() {
        let k = match e.get("origin") {
            Some(Value::String(s)) => format!("'{s}'"),
            _ => "None".to_string(),
        };
        if !n.contains_key(&k) {
            order.push(k.clone());
        }
        *n.entry(k).or_insert(0) += 1;
    }
    let body: Vec<String> = order.iter().map(|k| format!("{k}: {}", n[k])).collect();
    format!("{{{}}}", body.join(", "))
}

fn undo(cfg: &Config, run_id: &str, out: &mut dyn Write) -> Result<i32> {
    let runs: Vec<Value> = read_jsonl(&cfg.state().join("runs.jsonl"))
        .into_iter()
        .filter(|r| r.get("run_id").and_then(Value::as_str) == Some(run_id))
        .collect();
    let Some(last) = runs.last() else {
        let _ = writeln!(out, "no run {run_id} in runs.jsonl");
        return Ok(1);
    };
    let mut idx: Vec<i64> = last
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|a| a.get("indexes").and_then(Value::as_array).cloned().unwrap_or_default())
        .filter_map(|i| i.as_i64())
        .collect();
    idx.sort_unstable_by(|a, b| b.cmp(a));
    idx.dedup();
    let pk = Local::new(cfg);
    let mut done = 0;
    for i in &idx {
        match pk.undo(&[*i]) {
            Ok(_) => done += 1,
            Err(e @ GsError::Api { .. }) => {
                let _ = writeln!(out, "  line {i}: {e}");
            }
            Err(e) => return Err(e),
        }
    }
    let _ = writeln!(
        out,
        "undid {done} of {} Punchable change(s) from {run_id}. Google Docs were not changed.",
        idx.len()
    );
    Ok(0)
}

/// Run the command line. Output goes to `out`; an error the Python would end the program with goes to `err` and
/// exit code 1.
pub fn run(args: &[String], env: &Env, svc: &Services, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let parsed = match Args::try_parse_from(std::iter::once("pk-gsync".to_string()).chain(args.iter().cloned())) {
        Ok(a) => a,
        Err(e) => {
            if e.use_stderr() {
                let _ = write!(err, "{e}");
            } else {
                let _ = write!(out, "{e}");
            }
            return e.exit_code();
        }
    };
    let cfg = Config::from_env(env);
    let outcome: Result<i32> = (|| match parsed.cmd {
        Cmd::Auth { desktop } => {
            let path = run_auth(&cfg, svc, desktop, out)?;
            let _ = writeln!(out, "signed in; token saved (owner-only): {path}");
            Ok(0)
        }
        Cmd::Status => status(&cfg, svc, out),
        Cmd::Undo { run_id } => undo(&cfg, &run_id, out),
        Cmd::Run {
            dry_run,
            watch,
            interval,
        } => {
            let rules = Rules::load(&cfg.legal_rules_path, cfg.owner_email.as_deref());
            loop {
                let rec = run_once(dry_run, &cfg, env, svc, &rules)?;
                let _ = writeln!(out, "{}", summary(&rec));
                let _ = out.flush();
                if dry_run {
                    for x in rec
                        .get("actions")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .take(200)
                    {
                        let _ = writeln!(out, "   {}", py::dumps(x));
                    }
                }
                if !watch {
                    let failed = rec
                        .get("errors")
                        .and_then(Value::as_array)
                        .is_some_and(|a| !a.is_empty());
                    return Ok(if failed { 2 } else { 0 });
                }
                (svc.sleep)(Duration::from_secs(interval.max(15) as u64));
            }
        }
    })();
    match outcome {
        Ok(code) => code,
        Err(GsError::Exit(msg)) => {
            let _ = writeln!(err, "{msg}");
            1
        }
        Err(e) => {
            let _ = writeln!(err, "{}", e.describe());
            1
        }
    }
}
