//! `pk-backup`: the nightly backup job's program. Same command line as `python backup.py` (see the library docs).

use std::io::{stderr, stdout};
use std::panic::{AssertUnwindSafe, catch_unwind};

use pk_backup::Vars;
use pk_backup::cli::{Ctx, run};
use pk_backup::redact::redact;

fn main() {
    let vars = Vars::from_pairs(
        std::env::vars_os().map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned())),
    );
    // SSL_CERT_FILE adds trusted roots (a bundle of PEM certificates); an unreadable file adds none, as OpenSSL does
    let roots: Vec<Vec<u8>> = vars
        .get("SSL_CERT_FILE")
        .and_then(|p| std::fs::read(p).ok())
        .into_iter()
        .collect();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // one line, never a backtrace with request headers in it
    std::panic::set_hook(Box::new(|_| {}));
    let code = catch_unwind(AssertUnwindSafe(|| {
        let mut out = stdout().lock();
        let mut ctx = Ctx {
            vars: &vars,
            roots: &roots,
            out: &mut out,
            retry_unit: None,
        };
        run(&args, &mut ctx, &mut stderr().lock())
    }))
    .unwrap_or_else(|p| {
        let msg = p
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_else(|| "panic".to_string());
        eprintln!("{}", redact(&format!("backup: FAIL panic: {msg}"), &vars));
        1
    });
    std::process::exit(code);
}
