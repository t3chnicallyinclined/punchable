//! `pk-gsync`: the scheduled task's program. Same command line as `python -m gsync` (see the library docs).

use std::io::{stderr, stdout};

use pk_core::Env;
use pk_gsync::cli;
use pk_gsync::services::Services;

fn main() {
    let env = Env::from_process();
    let svc = Services::system(env.opt("USERNAME").map(str::to_string));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = cli::run(&args, &env, &svc, &mut stdout().lock(), &mut stderr().lock());
    std::process::exit(code);
}
