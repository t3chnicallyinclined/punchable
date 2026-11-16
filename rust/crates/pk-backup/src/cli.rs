//! The commands, in the order and with the words of `backup.py`:
//!
//! ```text
//! pk-backup run                     export -> gzip -> check -> PUT -> HEAD -> (Sunday UTC: weekly copy) -> prune
//!                                   then, when BUGSINK_DB_PATH is set: the same for Bugsink's SQLite
//! pk-backup list                    the backups under the prefix, oldest first
//! pk-backup fetch <key> <out.gz>    download one, check its length and gzip CRC
//! pk-backup restore-check <file.gz> the same checks `run` makes before an upload, on a local file
//! pk-backup export <out.gz>         export + checks to a local file, no upload (needs no R2_* settings)
//! ```
//!
//! Settings are environment variables only ([`crate::Vars`]); see `infra/backup/backup.py` for the list. Exit status:
//! 0 ok, 1 any failure (one line on stderr, token-like strings redacted), 2 a command line that is none of these.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Utc, Weekday};

use crate::dump::{DumpInfo, ExportConfig, check_dump};
use crate::error::{BackupError, Result, fail};
use crate::redact::redact;
use crate::retention::{SQLITE_EXT, SURQL_EXT, select_prune};
use crate::s3::{ObjectStore, S3};
use crate::settings::{Vars, bugsink_prefix, fullmatch, prefix};
use crate::sqlite;

const USAGE: &str = "usage: pk-backup run | list | fetch <key> <out.gz> | restore-check <file.gz> | export <out.gz>";

/// What a run needs besides the settings.
pub struct Ctx<'a> {
    /// The environment snapshot.
    pub vars: &'a Vars,
    /// Extra trusted root certificates (PEM), from `SSL_CERT_FILE` in the binary.
    pub roots: &'a [Vec<u8>],
    /// Where the log goes (stdout).
    pub out: &'a mut dyn Write,
    /// The wait base for retried requests; `None` is the job's 2 s.
    pub retry_unit: Option<Duration>,
}

impl Ctx<'_> {
    fn log(&mut self, msg: &str) {
        let line = redact(msg, self.vars);
        let _ = writeln!(self.out, "{line}");
        let _ = self.out.flush();
    }

    fn s3(&self) -> Result<S3> {
        let mut s = S3::from_vars(self.vars, self.roots)?;
        if let Some(u) = self.retry_unit {
            s.retry_unit = u;
        }
        Ok(s)
    }
}

/// Run one command line (`args` without the program name). Returns the exit status.
pub fn run(args: &[String], ctx: &mut Ctx<'_>, err: &mut dyn Write) -> i32 {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let outcome = match a.as_slice() {
        ["run"] => cmd_run(ctx),
        ["list"] => cmd_list(ctx),
        ["fetch", key, out] => cmd_fetch(ctx, key, Path::new(out)),
        ["restore-check", path] => cmd_restore_check(ctx, Path::new(path)),
        ["export", out] => cmd_export(ctx, Path::new(out)),
        _ => {
            let _ = writeln!(err, "{USAGE}");
            return 2;
        }
    };
    match outcome {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "{}", redact(&format!("backup: FAIL {e}"), ctx.vars));
            1
        }
    }
}

/// `want_weekly(now)`: `BACKUP_WEEKLY` is `auto` (Sunday UTC), `always` or `never`.
pub fn want_weekly(v: &Vars, now: DateTime<Utc>) -> Result<bool> {
    match v.get_or("BACKUP_WEEKLY", "auto") {
        "always" => Ok(true),
        "never" => Ok(false),
        "auto" => Ok(now.weekday() == Weekday::Sun),
        _ => fail("BACKUP_WEEKLY must be auto, always or never"),
    }
}

fn stamp(now: DateTime<Utc>) -> String {
    now.format("%Y%m%dT%H%M%SZ").to_string()
}

fn tmp_in(v: &Vars, prefix: &str, suffix: &str) -> Result<tempfile::NamedTempFile> {
    let dir = v.get("BACKUP_TMP").map_or_else(std::env::temp_dir, PathBuf::from);
    tempfile::Builder::new()
        .prefix(prefix)
        .suffix(suffix)
        .tempfile_in(dir)
        .map_err(|e| BackupError(format!("{e}")))
}

fn size_text(got: Option<u64>) -> String {
    got.map_or_else(
        || "None".to_string(),
        |n| if n == u64::MAX { "-1".into() } else { n.to_string() },
    )
}

fn secs(t0: Instant) -> String {
    format!("{:.1}", t0.elapsed().as_secs_f64())
}

fn cmd_run(ctx: &mut Ctx<'_>) -> Result<()> {
    let pfx = prefix(ctx.vars)?;
    let keep_d = ctx.vars.int("BACKUP_KEEP_DAILY", 14, 1)?;
    let keep_w = ctx.vars.int("BACKUP_KEEP_WEEKLY", 8, 1)?;
    let ns = ctx.vars.env("SURREAL_NS", Some("punchable"))?;
    let db = ctx.vars.env("SURREAL_DB", Some("main"))?;
    if !fullmatch("[a-z0-9_]+", &ns) || !fullmatch("[a-z0-9_]+", &db) {
        return fail("SURREAL_NS / SURREAL_DB must be [a-z0-9_]+");
    }
    let s3 = ctx.s3()?;
    want_weekly(ctx.vars, Utc::now())?; // validate before the export
    let export = ExportConfig::from_vars(ctx.vars, ctx.roots)?;
    run_database(ctx, &s3, &export, Utc::now(), &pfx, (keep_d, keep_w))
}

/// The `SurrealDB` half of `run`, and Bugsink's after it, against any store.
pub fn run_database(
    ctx: &mut Ctx<'_>,
    store: &dyn ObjectStore,
    export: &ExportConfig,
    now: DateTime<Utc>,
    pfx: &str,
    keep: (i64, i64),
) -> Result<()> {
    let (ns, db) = (export.ns().to_string(), export.db().to_string());
    let name = format!("{ns}-{db}-{}.surql.gz", stamp(now));
    let daily = format!("{pfx}daily/{name}");
    {
        let tmp = tmp_in(ctx.vars, "surreal-export-", ".surql.gz")?;
        let t0 = Instant::now();
        let n = export.export_to_gzip(tmp.path())?;
        let info = check_dump(tmp.path(), None, ctx.vars)?;
        if info.bytes_sql != n {
            return fail("gzip re-read length differs from the export length");
        }
        ctx.log(&format!(
            "export ok: {ns}/{db} {n} bytes -> {} gz, {} tables, {}s",
            info.bytes_gz,
            info.tables,
            secs(t0)
        ));
        store.put_file(&daily, tmp.path(), &info.sha256)?;
        let got = store.head(&daily)?;
        if got != Some(info.bytes_gz) {
            return fail(format!(
                "HEAD {daily}: size {}, expected {}",
                size_text(got),
                info.bytes_gz
            ));
        }
        ctx.log(&format!("uploaded {daily} ({} bytes, verified)", info.bytes_gz));
        if want_weekly(ctx.vars, now)? {
            let weekly = format!("{pfx}weekly/{name}");
            store.copy(&daily, &weekly)?;
            let got = store.head(&weekly)?;
            if got != Some(info.bytes_gz) {
                return fail(format!(
                    "HEAD {weekly}: size {}, expected {}",
                    size_text(got),
                    info.bytes_gz
                ));
            }
            ctx.log(&format!("weekly copy {weekly} (verified)"));
        }
    }
    // prune: only reached after a verified upload
    let keys: Vec<String> = store.list(pfx)?.into_iter().map(|(k, _)| k).collect();
    let (kept, delete) = select_prune(&keys, pfx, &format!("{ns}-{db}"), keep.0, keep.1, SURQL_EXT)?;
    if !kept.contains(&daily) {
        return fail(format!(
            "prune refused: the new backup {daily} is not among the newest {} (clock skew?)",
            keep.0
        ));
    }
    for k in &delete {
        store.delete(k)?;
    }
    ctx.log(&format!(
        "retention: kept {} ({} daily / {} weekly max), deleted {}",
        kept.len(),
        keep.0,
        keep.1,
        delete.len()
    ));
    // the second file: Bugsink's crash-report database. After the SurrealDB backup on purpose: a Bugsink problem can
    // never cost the night's database backup, and its failure still fails the Job (the alert fires).
    if ctx.vars.truthy("BUGSINK_DB_PATH") {
        run_sqlite(ctx, store, now, keep)?;
    }
    Ok(())
}

/// Bugsink's SQLite: online backup, gzip, integrity check, PUT under `BUGSINK_PREFIX`, the same retention.
pub fn run_sqlite(ctx: &mut Ctx<'_>, store: &dyn ObjectStore, now: DateTime<Utc>, keep: (i64, i64)) -> Result<()> {
    let src = ctx.vars.env("BUGSINK_DB_PATH", None)?;
    let pfx = bugsink_prefix(ctx.vars)?;
    let name = "bugsink-db";
    let key_name = format!("{name}-{}.sqlite3.gz", stamp(now));
    let daily = format!("{pfx}daily/{key_name}");
    {
        let tmp = tmp_in(ctx.vars, "bugsink-", ".sqlite3.gz")?;
        let t0 = Instant::now();
        let bytes_db = sqlite::backup_to_gzip(Path::new(&src), tmp.path())?;
        let info = sqlite::check_gz(tmp.path(), 8192)?;
        if info.bytes_db != bytes_db {
            return fail("gzip re-read length differs from the database copy");
        }
        ctx.log(&format!(
            "bugsink ok: {bytes_db} bytes -> {} gz, {} tables, integrity ok, {}s",
            info.bytes_gz,
            info.tables,
            secs(t0)
        ));
        store.put_file(&daily, tmp.path(), &info.sha256)?;
        if store.head(&daily)? != Some(info.bytes_gz) {
            return fail(format!("HEAD {daily}: size differs from the upload"));
        }
        ctx.log(&format!("uploaded {daily} ({} bytes, verified)", info.bytes_gz));
        if want_weekly(ctx.vars, now)? {
            let weekly = format!("{pfx}weekly/{key_name}");
            store.copy(&daily, &weekly)?;
            if store.head(&weekly)? != Some(info.bytes_gz) {
                return fail(format!("HEAD {weekly}: size differs from the copy"));
            }
            ctx.log(&format!("weekly copy {weekly} (verified)"));
        }
    }
    let keys: Vec<String> = store.list(&pfx)?.into_iter().map(|(k, _)| k).collect();
    let (kept, delete) = select_prune(&keys, &pfx, name, keep.0, keep.1, SQLITE_EXT)?;
    if !kept.contains(&daily) {
        return fail(format!(
            "prune refused: the new backup {daily} is not among the newest {} (clock skew?)",
            keep.0
        ));
    }
    for k in &delete {
        store.delete(k)?;
    }
    ctx.log(&format!(
        "bugsink retention: kept {}, deleted {}",
        kept.len(),
        delete.len()
    ));
    Ok(())
}

fn cmd_list(ctx: &mut Ctx<'_>) -> Result<()> {
    let pfx = prefix(ctx.vars)?;
    let mut items = ctx.s3()?.list(&pfx)?;
    items.sort();
    for (k, size) in items {
        // not redacted, as in the Python: the keys are the point
        let _ = writeln!(ctx.out, "{size:>12}  {k}");
    }
    Ok(())
}

fn cmd_fetch(ctx: &mut Ctx<'_>, key: &str, out: &Path) -> Result<()> {
    let pfx = prefix(ctx.vars)?;
    if !key.starts_with(&pfx) {
        return fail(format!("key must be under {pfx}"));
    }
    let s3 = ctx.s3()?;
    let Some(expected) = s3.head(key)? else {
        return fail(format!("{key}: not found"));
    };
    let mut part = out.as_os_str().to_os_string();
    part.push(".part");
    let part = PathBuf::from(part);
    let result = (|| -> Result<DumpInfo> {
        let mut f = File::create(&part).map_err(|e| BackupError(format!("{}: {e}", part.display())))?;
        let r = s3.get_into(key, &mut f, Duration::from_mins(15))?;
        drop(f);
        if r.status != 200 {
            return fail(format!("GET {key}: HTTP {}", r.status));
        }
        let n = r.streamed.unwrap_or(0);
        if n != expected {
            return fail(format!("GET {key}: got {n} bytes, expected {expected}"));
        }
        let info = check_dump(&part, None, ctx.vars)?;
        std::fs::rename(&part, out).map_err(|e| BackupError(format!("{}: {e}", out.display())))?;
        Ok(info)
    })();
    if part.exists() {
        let _ = std::fs::remove_file(&part);
    }
    let info = result?;
    ctx.log(&format!(
        "fetched {key} -> {}: {} gz, {} sql bytes, {} tables, sha256 {}",
        out.display(),
        info.bytes_gz,
        info.bytes_sql,
        info.tables,
        &info.sha256[..16]
    ));
    Ok(())
}

fn cmd_export(ctx: &mut Ctx<'_>, out: &Path) -> Result<()> {
    let export = ExportConfig::from_vars(ctx.vars, ctx.roots)?;
    let n = export.export_to_gzip(out)?;
    let info = match check_dump(out, None, ctx.vars) {
        Ok(i) => i,
        Err(e) => {
            let _ = std::fs::remove_file(out);
            return Err(e);
        }
    };
    ctx.log(&format!(
        "export ok: {n} bytes -> {} ({} gz, {} tables)",
        out.display(),
        info.bytes_gz,
        info.tables
    ));
    Ok(())
}

fn cmd_restore_check(ctx: &mut Ctx<'_>, path: &Path) -> Result<()> {
    let info = check_dump(path, None, ctx.vars)?;
    ctx.log(&format!(
        "ok: {} gz, {} sql bytes, {} tables, sha256 {}",
        info.bytes_gz,
        info.bytes_sql,
        info.tables,
        &info.sha256[..16]
    ));
    Ok(())
}
