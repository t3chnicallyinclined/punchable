//! The claim / run / result loop (`worker/__main__.py` `main` and `run_one`, `worker/online.py` `run_job`).
//!
//! Each turn: touch the heartbeat; wait for a job slot (a semaphore, `PK_WORKER_JOBS`, 1 by default); check the
//! day's budget (over it, the claim says why and takes nothing); `POST /api/worker/claim`; nothing queued → sleep
//! `--idle`; a job → its own task in a `JoinSet` holding the slot. A job fetches his voice profile, runs its kind,
//! posts the files it made (legal-checked, `db/srcpaths.py` paths only), then its result. A loop error (the claim,
//! the token) is logged, reported and backed off 5 s doubling to 300 s.
//!
//! Stopping (SIGTERM): no new claim; a running job is dropped at its next await and handed back with
//! `{ok: false, retry: true}` so the queue keeps it; the loop waits for the hand-backs inside the pod's 60 s grace.
//! The claim request itself is never cancelled half way (a job the API handed out must be run or handed back).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use pk_creative::corpus::{self, Corpus};
use pk_llm::{Budget, Router};
use pk_obs::WorkerMetrics;
use pk_wire::worker::files::{FilePut, FilesPost};
use pk_wire::worker::{Claim, JobResult};
use reqwest::Method;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use crate::api::{ApiClient, ApiError, quote};
use crate::config::WorkerConfig;
use crate::error::JobError;
use crate::heartbeat;
use crate::kinds::{self, Ctx, Job, Kind, OutFile, Pages, TRANSCRIBER_KINDS};
use crate::legal::{Legal, worker_may_write};
use crate::mirror::Mirror;
use crate::voice::VoiceCache;

/// The first back-off after a loop error, and where it resets.
pub const BACKOFF_MIN: Duration = Duration::from_secs(5);
/// The longest back-off.
pub const BACKOFF_MAX: Duration = Duration::from_secs(300);
/// What a job stopped by SIGTERM is handed back with.
pub const STOPPED: &str = "the worker was stopped; the job goes back to the queue";
/// How long the loop waits for running jobs to hand back after a stop (the pod's grace is 60 s).
pub const DRAIN: Duration = Duration::from_secs(50);
/// A hand-back post's own limit.
const HANDBACK_TIMEOUT: Duration = Duration::from_secs(10);
/// `POST_CHUNK`: files per post, by UTF-8 bytes.
pub const POST_CHUNK: usize = 12 * 1024 * 1024;
/// The handback words for a transcriber kind.
pub const NO_AUDIO: &str = "this worker has no audio pipeline: transcription happens on the phone";

/// What stops a turn of the loop (as opposed to a job failing, which is the job's result).
#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    /// The claim or the token.
    #[error(transparent)]
    Api(#[from] ApiError),
    /// The API handed out a kind this worker does not run; it went back to the queue.
    #[error("{0}")]
    HandedBack(String),
}

/// The parts a [`Worker`] is built from.
#[derive(Debug)]
pub struct Parts {
    /// The settings.
    pub cfg: WorkerConfig,
    /// The API.
    pub api: ApiClient,
    /// The model router (owns the one charging budget).
    pub router: Router,
    /// The same ledger, read for the claim's pause and the spend gauges.
    pub budget: Arc<Budget>,
    /// The registry `/metrics` serves.
    pub metrics: Arc<WorkerMetrics>,
    /// The court-material net.
    pub legal: Legal,
    /// The fact-check page fetcher.
    pub pages: Pages,
    /// The bge-small model, when the image has it (the embedding kinds need it).
    pub embedder: Option<crate::kinds::Model>,
}

/// The worker. Shared by the loop and its job tasks behind an `Arc`.
#[derive(Debug)]
pub struct Worker {
    cfg: WorkerConfig,
    api: ApiClient,
    router: Router,
    budget: Arc<Budget>,
    metrics: Arc<WorkerMetrics>,
    legal: Legal,
    pages: Pages,
    embedder: Option<crate::kinds::Model>,
    voice: VoiceCache,
    mirror: Mirror,
    model_line: Option<String>,
    counted: Mutex<usize>,
    /// Jobs started and not finished. A job decrements it (through `Running`, so a panic does too) before its slot
    /// is released, so the `--once` loop, which claims only after taking a slot, reads 0 only when no job can still
    /// post.
    running: AtomicUsize,
}

/// `model_line()`: what the app shows under "AI for drafts" ("Anthropic API · <models>").
pub fn model_line(cfg: &pk_llm::Config) -> Option<String> {
    let mut models: Vec<String> = Vec::new();
    for t in ["creative_riff", "ask_chat", "merge_judge"] {
        let Ok(tc) = cfg.task(t) else { continue };
        for (name, m) in &tc.models {
            let anthropic = cfg.providers().get(name).is_some_and(|p| p.kind == "anthropic");
            if anthropic && !models.contains(m) {
                models.push(m.clone());
            }
        }
    }
    if !models.is_empty() {
        return Some(format!("Anthropic API · {}", models.join(", ")));
    }
    cfg.task("default").ok().and_then(|t| t.providers.first().cloned())
}

enum Claimed {
    Job(Job),
    Paused,
    Nothing,
}

impl Worker {
    /// A worker over its parts.
    pub fn new(p: Parts) -> Arc<Self> {
        let budget = Arc::clone(&p.budget);
        p.metrics.attach_budget(move || {
            let s = budget.status();
            Some(pk_obs::BudgetStatus {
                daily_usd: s.daily_usd,
                spent_usd: s.spent_usd,
                over: s.over,
            })
        });
        Arc::new(Self {
            model_line: model_line(p.router.config()),
            cfg: p.cfg,
            api: p.api,
            router: p.router,
            budget: p.budget,
            metrics: p.metrics,
            legal: p.legal,
            pages: p.pages,
            embedder: p.embedder,
            voice: VoiceCache::new(),
            mirror: Mirror::new(),
            counted: Mutex::new(0),
            running: AtomicUsize::new(0),
        })
    }

    /// The settings.
    pub fn config(&self) -> &WorkerConfig {
        &self.cfg
    }

    /// What the claim says the worker writes with.
    pub fn model(&self) -> Option<&str> {
        self.model_line.as_deref()
    }

    async fn heartbeat(&self) {
        self.metrics.heartbeat();
        let path = self.cfg.heartbeat.clone();
        match tokio::task::spawn_blocking(move || heartbeat::touch(&path)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("heartbeat: {e}"),
            Err(e) => tracing::warn!("heartbeat: {e}"),
        }
    }

    async fn paused(&self) -> Option<String> {
        let b = Arc::clone(&self.budget);
        match tokio::task::spawn_blocking(move || (b.remaining() <= 0.0).then(|| b.reason())).await {
            Ok(p) => p,
            Err(e) => {
                // the backend checks the cap again before every call, so a claim here cannot overspend
                tracing::warn!("budget check: {e}; claiming anyway");
                None
            }
        }
    }

    async fn claim(&self) -> Result<Claimed, ApiError> {
        let paused = self.paused().await;
        let body = Claim {
            kinds: Some(self.cfg.kinds.iter().map(|k| k.name().to_string()).collect()),
            worker: self.cfg.worker_name.clone(),
            hosted: true,
            model: self.model_line.clone(),
            paused: paused.as_ref().map(|p| p.chars().take(300).collect()),
        };
        let answer = self.api.call(Method::POST, "/api/worker/claim", Some(&body)).await?;
        if paused.is_some() {
            if answer.is_some() {
                // the API answers a paused claim with 204; the Python ignores anything else, and so does this
                tracing::warn!("the API handed out a job to a paused claim; not taken");
            }
            return Ok(Claimed::Paused);
        }
        let Some(mut answer) = answer else {
            return Ok(Claimed::Nothing);
        };
        let job = answer
            .get_mut("job")
            .map(Value::take)
            .ok_or_else(|| ApiError::Decode("the claim answer has no job".into()))?;
        let job: Job = serde_json::from_value(job).map_err(|e| ApiError::Decode(format!("the claimed job: {e}")))?;
        Ok(Claimed::Job(job))
    }

    async fn post_result(&self, job_id: &str, body: &JobResult) -> Result<(), ApiError> {
        let path = format!("/api/worker/jobs/{}/result", quote(job_id));
        self.api.call(Method::POST, &path, Some(body)).await.map(drop)
    }

    fn failed(error: String, retry: bool) -> JobResult {
        JobResult {
            ok: false,
            result: None,
            error: Some(error),
            retry: Some(retry),
        }
    }

    /// Count the model calls made since the last count into the `pk_ai_*` metrics (charged calls only, as the
    /// Python's budget listener).
    fn count_calls(&self) {
        // the cursor first, then the snapshot: two jobs ending together cannot count one call twice
        let mut seen = self.counted.lock().unwrap_or_else(PoisonError::into_inner);
        let calls = self.router.calls();
        for rec in calls.get(*seen..).unwrap_or_default() {
            if let Some(usd) = rec.usd {
                let usage = pk_obs::Usage {
                    input_tokens: rec.tokens_in,
                    output_tokens: rec.tokens_out,
                    cache_read_input_tokens: rec.cache_read.unwrap_or(0),
                    cache_creation_input_tokens: rec.cache_write.unwrap_or(0),
                };
                self.metrics.on_charge(Some(&rec.model), &usage, usd, Some(&rec.task));
            }
        }
        *seen = calls.len();
    }

    /// `Workspace.push`: post the files (court text dropped here, never sent), batched by `POST_CHUNK`; returns the
    /// paths the API wrote and how many were refused.
    async fn push(&self, files: &[OutFile], actor: &str) -> Result<(Vec<String>, usize), ApiError> {
        let mut written = Vec::new();
        let mut refused = 0;
        let mut batch: Vec<FilePut> = Vec::new();
        let mut size = 0;
        let mut sorted: Vec<&OutFile> = files.iter().filter(|f| worker_may_write(&f.path)).collect();
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        for f in sorted {
            if self.legal.is_legal(&f.text) {
                refused += 1;
                continue;
            }
            let b = f.text.len();
            if size + b > POST_CHUNK && !batch.is_empty() {
                written.extend(self.post_files(std::mem::take(&mut batch), actor).await?);
                size = 0;
            }
            batch.push(FilePut {
                path: f.path.clone(),
                text: f.text.clone(),
                sha256: Some(hex::encode(Sha256::digest(f.text.as_bytes()))),
            });
            size += b;
        }
        if !batch.is_empty() {
            written.extend(self.post_files(batch, actor).await?);
        }
        Ok((written, refused))
    }

    async fn post_files(&self, files: Vec<FilePut>, actor: &str) -> Result<Vec<String>, ApiError> {
        let body = FilesPost {
            files,
            remove: Vec::new(),
            actor: actor.to_string(),
        };
        let r = self.api.call(Method::POST, "/api/worker/files", Some(&body)).await?;
        Ok(r.and_then(|v| v.get("written").cloned())
            .and_then(|w| serde_json::from_value(w).ok())
            .unwrap_or_default())
    }

    /// `run_job`: the voice profile, the kind, the files, and the `online` block of the result.
    async fn execute(&self, kind: Kind, job: &Job) -> Result<Value, JobError> {
        // the notebook first, then the profile: the Python's `run_job` order
        let (corpus, files, refresh) = if kind.reads_notebook() {
            let snap = self.mirror.refresh(&self.api).await?;
            let owner = self.cfg.owner_id.clone();
            let refresh = snap.refresh.clone();
            let files = snap.files.clone(); // path → Arc'd text: the texts are shared, not copied
            let built = tokio::task::spawn_blocking(move || {
                corpus::Parts::from_files(&snap.files, snap.decisions, snap.vectors, &owner).map(Corpus::new)
            })
            .await
            .map_err(|e| JobError::Value(format!("building the notebook: {e}")))??;
            (Some(Arc::new(built)), Some(files), refresh)
        } else {
            (None, None, Value::Null)
        };
        let voice = self.voice.fetch(&self.api).await;
        let ctx = Ctx {
            corpus: corpus.as_ref(),
            files: files.as_ref(),
            embedder: self.embedder.as_ref(),
            api: &self.api,
            router: &self.router,
            voice: &voice,
            names: &self.cfg.names,
            legal: &self.legal,
            pages: &self.pages,
        };
        let done = kinds::dispatch(kind, ctx, job.input.clone()).await?;
        let (pushed, refused) = self.push(&done.files, kind.actor()).await?;
        let mut result = done.result;
        // `refresh` is null for a kind that does not read the notebook: nothing was mirrored for it
        result.insert(
            "online".into(),
            json!({"refresh": refresh, "pushed": pushed, "legal_refused": refused, "media_deleted": 0,
                   "voice": voice.status()}),
        );
        Ok(Value::Object(result))
    }

    async fn job_task(self: Arc<Self>, job: Job, kind: Kind, stop: CancellationToken, slot: OwnedSemaphorePermit) {
        let running = Running(&self.running);
        let span = tracing::info_span!("job", job_id = %job.id, kind = %job.kind);
        async {
            tracing::info!("job claimed");
            let timer = self.metrics.job(Some(&job.kind));
            let exec = self.execute(kind, &job);
            tokio::pin!(exec);
            let outcome = tokio::select! {
                biased;
                r = &mut exec => Some(r),
                () = stop.cancelled() => None,
            };
            // stopping: a job about to finish gets a short grace, then it goes back to the queue
            let outcome = match outcome {
                Some(r) => Some(r),
                None => tokio::time::timeout(self.cfg.stop_grace, &mut exec).await.ok(),
            };
            self.count_calls();
            match outcome {
                Some(Ok(result)) => {
                    let status = result.get("status").and_then(Value::as_str).unwrap_or("ok").to_string();
                    let body = JobResult {
                        ok: true,
                        result: Some(result),
                        error: None,
                        retry: None,
                    };
                    match self.post_result(&job.id, &body).await {
                        Ok(()) => {
                            timer.done();
                            tracing::info!(status = %status, "job done");
                        }
                        // the Python's path: the post raised inside the job's try, so the job fails with that error
                        Err(e) => self.fail(&job, kind, timer, &JobError::Api(e)).await,
                    }
                }
                Some(Err(e)) => self.fail(&job, kind, timer, &e).await,
                None => {
                    timer.failed(true);
                    let handback = Self::failed(STOPPED.into(), true);
                    match tokio::time::timeout(HANDBACK_TIMEOUT, self.post_result(&job.id, &handback)).await {
                        Ok(Ok(())) => tracing::info!("job handed back: the worker is stopping"),
                        Ok(Err(e)) => result_not_posted(&e),
                        Err(_) => tracing::warn!("job hand-back timed out; the API's stale-claim sweep re-queues it"),
                    }
                }
            }
            self.heartbeat().await;
        }
        .instrument(span)
        .await;
        drop(running);
        drop(slot);
    }

    /// A job that failed: counted, reported (never the budget), and its failure posted with `retry` as the Python
    /// decides it.
    async fn fail(&self, job: &Job, kind: Kind, timer: pk_obs::JobTimer<'_>, e: &JobError) {
        let retry = e.retry();
        tracing::warn!(error = %e.py_type(), retry, "job failed");
        timer.failed(retry);
        if !e.is_budget() {
            pk_obs::crash::report(e, None, None, &[("kind", kind.name()), ("retry", bool_tag(retry))]);
        }
        if let Err(e) = self.post_result(&job.id, &Self::failed(e.result_text(), retry)).await {
            result_not_posted(&e);
        }
    }

    /// A job of a kind this worker does not run: a transcriber kind or one not ported yet goes back to the queue
    /// (the API should not have handed it out), an unknown kind fails as the Python's `unknown job kind`.
    async fn refuse(&self, job: &Job) -> Result<(), LoopError> {
        let timer = self.metrics.job(Some(&job.kind));
        let known = Kind::parse(&job.kind);
        if TRANSCRIBER_KINDS.contains(&job.kind.as_str()) || known.is_some() {
            let words = if known.is_some() {
                format!("this worker does not run {} yet", job.kind)
            } else {
                NO_AUDIO.to_string()
            };
            self.post_result(&job.id, &Self::failed(words, true)).await?;
            timer.failed(true);
            return Err(LoopError::HandedBack(format!(
                "claimed a {} job while hosted; handed back",
                job.kind
            )));
        }
        timer.failed(false);
        let e = JobError::Value(format!("unknown job kind {}", job.kind));
        pk_obs::crash::report(&e, None, None, &[("kind", "other"), ("retry", "false")]);
        self.post_result(&job.id, &Self::failed(e.result_text(), false)).await?;
        Ok(())
    }

    /// Run until stopped (or, with `--once`, until nothing is queued). `Err` is the loop error that ended a
    /// `--once` run.
    pub async fn run(self: &Arc<Self>, stop: CancellationToken) -> Result<(), LoopError> {
        tracing::info!(
            api = %self.cfg.api,
            auth = self.cfg.creds.is_some(),
            kinds = ?self.cfg.kinds.iter().map(|k| k.name()).collect::<Vec<_>>(),
            jobs = self.cfg.jobs,
            "worker up"
        );
        let slots = Arc::new(Semaphore::new(self.cfg.jobs));
        let mut jobs: JoinSet<()> = JoinSet::new();
        let mut backoff = BACKOFF_MIN;
        self.heartbeat().await;
        let outcome = loop {
            while let Some(done) = jobs.try_join_next() {
                if let Err(e) = done {
                    tracing::error!("a job task ended abnormally: {e}");
                }
            }
            let slot = tokio::select! {
                biased;
                () = stop.cancelled() => break Ok(()),
                s = Arc::clone(&slots).acquire_owned() => match s {
                    Ok(s) => s,
                    Err(_) => break Ok(()),
                },
            };
            self.heartbeat().await;
            let turn = match self.claim().await {
                Ok(Claimed::Job(job)) => {
                    if let Some(kind) = Kind::parse(&job.kind).filter(|k| self.cfg.kinds.contains(k)) {
                        self.running.fetch_add(1, Ordering::SeqCst);
                        jobs.spawn(Arc::clone(self).job_task(job, kind, stop.child_token(), slot));
                        Ok(true)
                    } else {
                        drop(slot);
                        self.refuse(&job).await.map(|()| true)
                    }
                }
                Ok(Claimed::Paused | Claimed::Nothing) => {
                    drop(slot);
                    Ok(false)
                }
                Err(e) => {
                    drop(slot);
                    Err(LoopError::from(e))
                }
            };
            match turn {
                Ok(busy) => {
                    backoff = BACKOFF_MIN;
                    if busy {
                        continue;
                    }
                    if self.cfg.once {
                        if self.running.load(Ordering::SeqCst) == 0 {
                            break Ok(());
                        }
                        while let Some(done) = jobs.join_next().await {
                            if let Err(e) = done {
                                tracing::error!("a job task ended abnormally: {e}");
                            }
                        }
                        continue;
                    }
                    if sleep_or_stop(self.cfg.idle, &stop).await {
                        break Ok(());
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "loop error");
                    pk_obs::crash::report(&e, None, None, &[]);
                    if self.cfg.once {
                        break Err(e);
                    }
                    if sleep_or_stop(backoff, &stop).await {
                        break Ok(());
                    }
                    backoff = (backoff * 2).min(BACKOFF_MAX);
                }
            }
        };
        let drain = async {
            while let Some(done) = jobs.join_next().await {
                if let Err(e) = done {
                    tracing::error!("a job task ended abnormally: {e}");
                }
            }
        };
        if tokio::time::timeout(DRAIN, drain).await.is_err() {
            tracing::warn!("jobs still running after the drain; aborted");
            jobs.abort_all();
        }
        outcome
    }
}

/// Decrements the running-jobs count when a job task ends, by return or by panic.
struct Running<'a>(&'a AtomicUsize);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn result_not_posted(e: &ApiError) {
    tracing::error!(error = %e, "the job's result was not posted; the API's stale-claim sweep re-queues it");
    pk_obs::crash::report(e, None, None, &[]);
}

fn bool_tag(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

/// Sleep, or wake early on a stop; `true` when stopped.
async fn sleep_or_stop(d: Duration, stop: &CancellationToken) -> bool {
    tokio::select! {
        biased;
        () = stop.cancelled() => true,
        () = tokio::time::sleep(d) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_line_is_the_hosted_models() {
        assert_eq!(
            model_line(&pk_llm::Config::hosted()).as_deref(),
            Some("Anthropic API · claude-opus-5-5, claude-sonnet-5-5, claude-haiku-5-5")
        );
    }
}
