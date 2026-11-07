//! The S3 stand-in as a program, for the differentials (`rust/tools/diff_api_*.py`) and by hand. Built only with the
//! `diagnostics` feature, which no shipped image has. It prints `ready <endpoint>` on stdout once it listens.
//!
//! Settings come from the environment, never argv (a process list shows argv): `STANDIN_BUCKET`, `STANDIN_KEY_ID`,
//! `STANDIN_SECRET` (required), `STANDIN_PORT` (default: any free one), `STANDIN_SINK=1` (keep only sizes).

#[path = "../../tests/support/s3_standin.rs"]
mod s3_standin;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let need = |k: &str| std::env::var(k).map_err(|_| anyhow::anyhow!("{k} is not set"));
    let opts = s3_standin::Options {
        bucket: need("STANDIN_BUCKET")?,
        key_id: need("STANDIN_KEY_ID")?,
        secret: need("STANDIN_SECRET")?,
        sink: std::env::var("STANDIN_SINK").is_ok_and(|v| v == "1"),
        page: 1000,
    };
    let port = std::env::var("STANDIN_PORT").unwrap_or_else(|_| "0".into());
    let standin = s3_standin::Standin::start_on(&format!("127.0.0.1:{port}"), opts).await;
    println!("ready {}", standin.endpoint);
    std::future::pending::<()>().await;
    Ok(())
}
