//! The `/api/worker/*` protocol: what the worker sends and what the API answers, route by route.
//!
//! | Route | Request | Response |
//! |---|---|---|
//! | `POST claim` | [`Claim`] | 200 [`ClaimResponse`], 204 nothing to do (or paused), 400 [`ErrorBody`] |
//! | `POST jobs/<id>/result` | [`JobResult`] | [`ResultResponse`]; 404 / 409 [`ErrorBody`] |
//! | `GET jobs/<id>/file` | | the staged upload's bytes; 404 [`ErrorBody`]. PC-only, deleted at T11 |
//! | `GET state` | | [`State`] |
//! | `GET files[?prefix=]` | | [`files::FilesListing`] |
//! | `GET files/content?path=` | | [`files::FileContent`]; 404 [`ErrorBody`] |
//! | `POST files` | [`files::FilesPost`] | [`files::FilesPostResponse`]; 403 / 413 / 422 [`ErrorBody`] |
//! | `GET decisions?after=` | | [`kb::DecisionsPage`] |
//! | `POST ideas` | [`kb::IdeaRequest`] | [`kb::IdeaResponse`]; 422 [`ErrorBody`] |
//! | `POST decision` | [`kb::DecisionRequest`] | [`crate::api::Written`] |
//! | `POST undo` | [`kb::UndoRequest`] | [`crate::api::Written`] |
//! | `GET jokes/<id>` | | [`kb::JokePage`]; 404 [`ErrorBody`]. PC-only (gsync), deleted at T11 |
//! | `GET creative?store=[&snapshot=1]` | | [`creative::CreativeLines`]; 400 [`ErrorBody`] |
//! | `POST events` | [`creative::EventRequest`] | [`creative::EventResponse`] |
//! | `GET profile/voice` | | [`voice::VoiceProfile`] |
//! | `PUT profile/voice` | [`voice::VoicePut`] | [`voice::VoiceProfileApp`] (the app's shape, see there) |
//!
//! Every refusal on every route is [`ErrorBody`] with the status the Python picked.

pub mod creative;
pub mod files;
pub mod kb;
pub mod voice;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `POST /api/worker/claim`: who is asking and for what (`worker/__main__.py` `claim_body`). `kinds` is a list,
/// or `null` when the PC worker runs with no `--kinds` (the API then hands out any kind). `model` and `paused`
/// are sent only when set; a `paused` claim records the heartbeat and is answered 204 (nothing is handed out).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Claim {
    /// The job kinds this worker runs (a subset of the API's `JOB_KINDS`); a hosted worker must list them and
    /// never a transcriber kind.
    pub kinds: Option<Vec<String>>,
    /// The worker's name (the pod, `COMPUTERNAME` or "home"), at most 40 characters.
    pub worker: String,
    /// The server's worker (the Anthropic API, no audio) rather than a development PC.
    pub hosted: bool,
    /// What the app shows under "AI for drafts": "Anthropic API · <models>".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Why no job is taken right now (the day's AI budget is spent), at most 300 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<String>,
}

/// The 200 answer to a claim: the job just handed out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ClaimResponse {
    /// The job record, now `claimed` by this worker.
    pub job: Job,
}

/// A job record as the API's job store holds it (`creative_api.py` `job_apply`): every field is always present.
/// `kind` and `input` are one typed pair ([`JobKind`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Job {
    /// `job:<kind>-<12 hex>`.
    pub id: String,
    /// The kind and its input, typed per kind.
    #[serde(flatten)]
    pub kind: JobKind,
    /// The de-duplication key (`riff|<hash>`, `bridge|a|b`, `upload|<file>`, ...); `null` for a job queued without one.
    pub key: Option<String>,
    /// Where the job is in its life.
    pub status: JobStatus,
    /// When it was queued.
    pub created_at: String,
    /// Who queued it: the owner, or the import that queued a follow-up (`import:upload`).
    pub by: Option<String>,
    /// Claims so far; a third failed attempt ends the job.
    pub attempts: u32,
    /// When the current claim was made.
    pub claimed_at: Option<String>,
    /// The worker that claimed it.
    pub worker: Option<String>,
    /// When it finished (done or failed for good).
    pub done_at: Option<String>,
    /// The worker's result object once done.
    pub result: Option<Value>,
    /// The last failure's message (kept on a retry).
    pub error: Option<String>,
    /// `"transcriber"` while a transcriber kind is queued (nothing on the server runs it); `null` otherwise.
    pub waiting_for: Option<String>,
}

/// The job kinds and the input each carries, as the API queues them (`creative_api.py` routes). The wire is
/// `{"kind": "<kind>", "input": {...}}` inside the job record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "input", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum JobKind {
    /// A bridge line between two bits (`POST /api/boards/<id>/bridge`, or a set's `mysets` route).
    Bridge(BridgeInput),
    /// Punch-up drafts for a bit (`POST .../punchup`).
    Punchup(PunchupInput),
    /// The older one-shot Ask (`POST /api/ask`).
    Ask(AskInput),
    /// An upload staged in the inbox, waiting for a transcriber. PC-only, deleted at T11.
    ProcessSet(ProcessSetInput),
    /// A `YouTube` set to fetch and analyse. PC-only, deleted at T11. No golden: the public repo's commit guard
    /// refuses any watch URL, so the input and result are typed from `worker/execute.py` alone.
    ImportYoutube(ImportYoutubeInput),
    /// Checkable facts for an idea (`POST /api/facts`).
    Facts(FactsInput),
    /// Riff lines for an idea (`POST /api/riff`).
    Riff(RiffInput),
    /// One turn of a conversation (`POST /api/ask/chats`): the messages the API already built.
    AskChat(AskChatInput),
    /// Vectors for new material; queued after an import with an empty input.
    Embed(EmbedInput),
    /// The memo-to-bit linking; queued after a memo or upload import.
    Memolink(MemolinkInput),
    /// A set transcribed on the phone (`POST /api/sets/transcript`).
    SetText(SetTextInput),
}

/// Where a job is: `queued` → `claimed` → `done` | `failed` (a retried failure goes back to `queued`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum JobStatus {
    /// Waiting for a worker.
    Queued,
    /// Handed to a worker.
    Claimed,
    /// Finished with a result.
    Done,
    /// Finished with an error, no more retries.
    Failed,
}

/// The `bridge` input: the two bits, and the board (`board`) or the set (`set`) the request came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BridgeInput {
    /// The bit before the bridge.
    pub from: String,
    /// The bit after it.
    pub to: String,
    /// The storyboard id (the boards route).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
    /// The working set id (the `mysets` route).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
}

/// The `punchup` input: the working set record the locks are read from (or, for a bit outside any working set, a
/// stand-in `{id: null, title: null, current: 0, locks: {...}}`), and the bit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PunchupInput {
    /// The working set, whole, or the stand-in: open because it is the store's record.
    pub working_set: Value,
    /// The bit to punch up.
    pub joke_id: String,
}

/// The `ask` input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AskInput {
    /// His question, whitespace-normalised.
    pub question: String,
}

/// The `process_set` input: the staged file's name in the inbox and the upload's `meta.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ProcessSetInput {
    /// `<date>_punchable-<kind>_<time>_<name>.<ext>`, fetched from `GET jobs/<id>/file`.
    pub file: String,
    /// What the app said about the recording.
    pub meta: UploadRecord,
}

/// An upload's `meta.json` as `server.py` writes it next to the staged file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UploadRecord {
    /// `set`, `note` or `upload`.
    pub kind: String,
    /// The file name on the phone.
    pub original_name: Option<String>,
    /// `X-Title`.
    pub title: Option<String>,
    /// When the API received it.
    pub received_at: String,
    /// The file's size.
    pub bytes: u64,
    /// `X-Duration-S`.
    pub duration_s: Option<f64>,
    /// `X-Marks-Ms`: the moments he marked while recording.
    pub marks_ms: Vec<i64>,
    /// Always `"queued"`.
    pub status: String,
    /// `X-Recorded-At`.
    pub recorded_at: Option<String>,
    /// `X-Venue`.
    pub venue: Option<String>,
    /// `X-Lat` / `X-Lng`.
    pub gps: Option<Gps>,
    /// `X-Show-Group`.
    pub show_group: Option<String>,
    /// `X-Align-Working-Set`.
    pub align_working_set: Option<String>,
}

/// A point the phone reported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Gps {
    /// Latitude.
    pub lat: f64,
    /// Longitude.
    pub lon: f64,
}

/// The `import_youtube` input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ImportYoutubeInput {
    /// The canonical watch URL.
    pub url: String,
    /// The 11-character video id.
    pub video_id: String,
}

/// The `facts` input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FactsInput {
    /// The idea to check, trimmed.
    pub text: String,
}

/// The `riff` input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RiffInput {
    /// His idea.
    pub idea: String,
    /// His lines so far (blank ones dropped).
    pub lines: Vec<String>,
}

/// The `ask_chat` input: the prompt the API built (`creative/askchat.py` `prompt`), ready for the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AskChatInput {
    /// System, user and assistant turns.
    pub messages: Vec<ChatMessage>,
}

/// One message of a model prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ChatMessage {
    /// `system`, `user` or `assistant`.
    pub role: String,
    /// The text.
    pub content: String,
}

/// The `embed` input: always `{}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct EmbedInput {}

/// The `memolink` input: `{}` from the API; a hand-queued job may name its `steps`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct MemolinkInput {
    /// A subset of `embed`, `judge`, `triage`, `attach`; all four when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<String>>,
}

/// The `set_text` input: a set transcribed on the phone (`creative_api.py` `set_transcript`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SetTextInput {
    /// The phone's segments, 1 to 5000.
    pub segments: Vec<TranscriptSegment>,
    /// The set's title, if given.
    pub title: Option<String>,
    /// When it was recorded (ISO), if given.
    pub recorded_at: Option<String>,
    /// The room, if given.
    pub venue: Option<String>,
    /// The recording's length in seconds, if given.
    pub duration_s: Option<f64>,
    /// When the API received it.
    pub received_at: String,
    /// The phone's transcription engine, or `"phone"`.
    pub engine: String,
}

/// One segment the phone sent: `start` and `end` (seconds) are kept only when it sent them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TranscriptSegment {
    /// The words.
    pub text: String,
    /// Start, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    /// End, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
}

/// `POST /api/worker/jobs/<id>/result`: `{ok: true, result}` or `{ok: false, error, retry}`
/// (`worker/__main__.py` `run_one`). The result object's shape is the kind's: `creative::Riff`, `simple::Facts`,
/// `creative::AskAnswer`, `creative::Bridge`, `creative::PunchUp`, and `ProcessSetResult`, `SetTextResult`,
/// `AskChatResult`, `EmbedResult`, `MemolinkResult`, `ImportYoutubeResult` here. `error` is cut at 1,500
/// characters by the worker and 2,000 by the API; `retry` puts the job back in the queue while attempts remain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct JobResult {
    /// Done, or failed.
    pub ok: bool,
    /// The kind's result object (must be an object: a 400 otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// `"<Type>: <message>"`, or the budget reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Back to the queue (a timeout, an IO error, the budget), or failed for good.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<bool>,
}

/// The answer to a result post: the job's brief after the change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ResultResponse {
    /// Always `true` (a refusal is an [`ErrorBody`]).
    pub ok: bool,
    /// The job as the app sees it.
    pub job: JobBrief,
}

/// A job as the app sees it (`creative_api.py` `job_brief`): the 202 bodies, the result post's answer and
/// `/api/jobs/<id>` (which adds `result`, see `creative::Job`). `waiting_for` and `note` are present only while
/// the job waits for a transcriber; the full [`Job`] record always carries `waiting_for`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct JobBrief {
    /// `job:<kind>-<12 hex>`.
    pub id: String,
    /// The kind's name.
    pub kind: String,
    /// Where the job is.
    pub status: JobStatus,
    /// When it was queued.
    pub created_at: Option<String>,
    /// When it finished.
    pub done_at: Option<String>,
    /// Claims so far.
    pub attempts: u32,
    /// The last failure's message.
    pub error: Option<String>,
    /// `"transcriber"` while nothing on the server can run it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<String>,
    /// The sentence the app shows under the worker line while it waits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `GET /api/worker/state`: what the worker checks first (`ingest_api.py`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct State {
    /// Decision lines replayed.
    pub decisions: u64,
    /// `meta:state.files_rev` on `SurrealDB`; `null` on the files backend.
    pub files_rev: Option<i64>,
    /// `"db"` hosted (srcfile rows), `"disk"` locally.
    pub files_mode: String,
    /// `"surreal"` or `"files"`.
    pub backend: String,
    /// `meta:state.online_first`: set at the cutover.
    pub online_first: bool,
    /// The first 12 hex of the legal rules' fingerprint.
    pub legal_rules: String,
}

/// Every refusal, on every route: `{"error": "<plain words>"}` with the HTTP status the Python picked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ErrorBody {
    /// The message the app may show.
    pub error: String,
}

/// The `process_set` result (`worker/execute.py` `run`): the inbox report's kept keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ProcessSetResult {
    /// Always `"processed"`.
    pub status: String,
    /// `status`, `kind`, `set`, `set_id`, `stats`, `totals`, `date`, `venue`, `error`: whichever the pipeline wrote.
    pub report: Map<String, Value>,
}

/// The `import_youtube` result (`worker/execute.py` `import_youtube`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ImportYoutubeResult {
    /// Always `"imported"`.
    pub status: String,
    /// The set folder the fetch wrote.
    pub set_dir: String,
    /// The video id.
    pub video_id: String,
    /// The pipeline steps and how long each took.
    pub steps: Vec<Value>,
}

/// The `set_text` result (`worker/execute.py` `analyze_transcript` plus `worker/online.py` `set_text_job`'s `ideas`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SetTextResult {
    /// `"analyzed_text"`, or `"skipped_legal"` (then nothing else).
    pub status: String,
    /// Always `"set"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// `<date>_ph<sha9>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_dir: Option<String>,
    /// Units the split step found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<u64>,
    /// Of which funny.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funny_units: Option<u64>,
    /// `{status: "unavailable", why, timestamps}`: no recording was here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<Map<String, Value>>,
    /// The split step's run id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// The ideas posted for the funny units: `{unit, joke_id, attach}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ideas: Option<Vec<SetTextIdea>>,
}

/// One idea the `set_text` job posted through `POST /api/worker/ideas`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SetTextIdea {
    /// The unit in the analysis.
    pub unit: String,
    /// The joke the API created.
    pub joke_id: Option<String>,
    /// The auto-attach decision: `attach`, `ambiguous` or `new`.
    pub attach: Option<String>,
}

/// The `ask_chat` result (`creative/askchat.py` `run_job`): the model's answer, gated by the API when it reads it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AskChatResult {
    /// Always `"answered"`.
    pub status: String,
    /// `{found, answer, citations: [{source, quote}]}`, the model's own schema (empty when it gave nothing).
    pub data: Map<String, Value>,
    /// The provider that answered.
    pub provider: Option<String>,
    /// The model that answered.
    pub model: Option<String>,
}

/// The `embed` result (`worker/online.py` `embed_job`): `status` plus `creative/memolink.py` `embed_summaries`' counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct EmbedResult {
    /// Always `"embedded"`.
    pub status: String,
    /// Jokes that only have a summary to embed.
    pub summary_only: u64,
    /// Vectors written this run.
    pub embedded: u64,
    /// Vectors dropped.
    pub removed: u64,
    /// Vectors in the file now.
    pub have: u64,
    /// The vector file written (a path under the workspace).
    pub file: String,
}

/// The `memolink` result (`worker/online.py` `memolink_job`): one key per step run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct MemolinkResult {
    /// Always `"linked"`.
    pub status: String,
    /// `embed_summaries`' counts, when the step ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed: Option<Map<String, Value>>,
    /// The judge's counts, when the step ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge: Option<Map<String, Value>>,
    /// The triage's counts, when the step ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<Map<String, Value>>,
    /// The attach event ids posted this run, when the step ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached: Option<Vec<String>>,
    /// Same-idea groups after the run.
    pub same_idea_groups: u64,
}
