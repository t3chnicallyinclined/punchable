//! Paths and constants (`gsync/config.py`). Settings come from the process environment and `secrets.env` through
//! [`pk_core::Env`] (the one reader); his own Drive ids live in a private file, `gsync_docs.json`, that is never in
//! the repo. Without it the sample ids below are used: they match nothing in a real Drive.

use std::path::{Path, PathBuf};

use pk_core::{Env, py};
use serde_json::Value;

/// The Google Docs mime type.
pub const DOC_MIME: &str = "application/vnd.google-apps.document";
/// The Drive folder mime type.
pub const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
/// The full Drive scope.
pub const SCOPES_FULL: [&str; 1] = ["https://www.googleapis.com/auth/drive"];
/// The narrow Drive scopes (`GSYNC_SCOPES=narrow`).
pub const SCOPES_NARROW: [&str; 2] = [
    "https://www.googleapis.com/auth/drive.readonly",
    "https://www.googleapis.com/auth/drive.file",
];
/// A doc modified this recently is parked and retried (he may still be typing).
pub const SETTLE_S: i64 = 120;
/// `POST /api/ideas` text limit.
pub const IDEA_MAX: usize = 4000;
/// His iPhone Voice Memos folder inside the Punchable folder.
pub const MEMO_FOLDER_NAME: &str = "Voice memos";
/// Never download or transcode when the disk would drop below this much free space.
pub const MEMO_RESERVE_MB: u64 = 600;
/// At most this much downloaded-but-not-yet-transcribed audio on disk at once.
pub const MEMO_STAGE_MB: u64 = 300;
/// Downloads per pass.
pub const MEMO_BATCH: usize = 25;

/// An ordered `{key: value}` table of ids (the Python dicts keep their file order).
pub type IdMap = Vec<(String, String)>;

/// The settings of one run.
#[derive(Debug, Clone)]
pub struct Config {
    /// The data folder.
    pub comedy: PathBuf,
    /// The local API (only `status` and `undo` use it, as in the Python).
    pub api: String,
    /// `~/.punchable`: tokens and private rule files, by reference.
    pub home: PathBuf,
    /// `full` or `narrow`.
    pub scopes_mode: String,
    /// The Punchable folder in his Drive.
    pub folder_id: String,
    /// Storyboard id -> board doc id.
    pub board_docs: IdMap,
    /// Doc id -> title.
    pub ai_draft_docs: IdMap,
    /// Doc id -> joke id.
    pub mixed_docs: IdMap,
    /// Doc id -> joke id.
    pub adopted_joke_docs: IdMap,
    /// Seconds a doc must rest before it is read.
    pub settle_s: i64,
    /// Voice memos on.
    pub memos_enabled: bool,
    /// See [`MEMO_RESERVE_MB`].
    pub memo_reserve_mb: u64,
    /// See [`MEMO_STAGE_MB`].
    pub memo_stage_mb: u64,
    /// See [`MEMO_BATCH`].
    pub memo_batch: usize,
    /// The Python that has faster-whisper (the memo processor).
    pub whisper_python: String,
    /// The realm.
    pub auth_url: String,
    /// The Keycloak client gsync signs in as.
    pub kc_client: String,
    /// `PK_ONLINE_FIRST=1`.
    pub online_first: bool,
    /// `legal_rules.json`, or `PUNCHABLE_LEGAL_RULES`.
    pub legal_rules_path: PathBuf,
    /// `PUNCHABLE_OWNER_EMAIL`.
    pub owner_email: Option<String>,
    /// The actor id on his own edits (`PK_OWNER_ID`).
    pub owner_id: String,
}

fn id_map(local: &Value, key: &str, default: &[(&str, &str)]) -> IdMap {
    match local.get(key) {
        Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), py::str(v))).collect(),
        _ => default.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    }
}

impl Config {
    /// The settings from the environment (the home folder is the one the `Env` carries, so a test points it elsewhere).
    pub fn from_env(env: &Env) -> Self {
        let home_dir = env.home().map(Path::to_path_buf).unwrap_or_default();
        let home = env
            .path("PUNCHABLE_HOME")
            .unwrap_or_else(|| home_dir.join(".punchable"));
        let local = std::fs::read_to_string(home.join("gsync_docs.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .filter(Value::is_object)
            .unwrap_or(Value::Null);
        let online_first = env.opt("PK_ONLINE_FIRST") == Some("1");
        let comedy = if online_first {
            if let Some(ws) = env.path("PK_WORKSPACE") {
                ws.join("comedy")
            } else if let Some(c) = env.path("PK_CACHE_DIR") {
                c.join("worker").join("comedy")
            } else {
                home_dir.join(".punchable-worker").join("comedy")
            }
        } else {
            env.path("COMEDY_DIR")
                .unwrap_or_else(|| home_dir.join("punchable-data"))
        };
        Self {
            comedy,
            api: env.opt("PUNCHABLE_API").unwrap_or("http://127.0.0.1:8796").to_string(),
            legal_rules_path: env
                .path("PUNCHABLE_LEGAL_RULES")
                .unwrap_or_else(|| home.join("legal_rules.json")),
            home,
            scopes_mode: env.opt("GSYNC_SCOPES").unwrap_or("full").to_string(),
            folder_id: local
                .get("folder_id")
                .map_or_else(|| "sample-folder-punchable-0000000000".to_string(), py::str),
            board_docs: id_map(
                &local,
                "board_docs",
                &[("sb:robot-vacuum-104b6cc0", "sample-board-doc-robot-vacuum-000000000000")],
            ),
            ai_draft_docs: id_map(
                &local,
                "ai_draft_docs",
                &[(
                    "sample-ai-draft-doc-0000000000000000000000",
                    "AI explained step by step (DRAFT walk-through for the AI chunk)",
                )],
            ),
            mixed_docs: id_map(
                &local,
                "mixed_docs",
                &[(
                    "sample-mixed-doc-00000000000000000000000000",
                    "joke:good-data-the-soup-label",
                )],
            ),
            adopted_joke_docs: id_map(
                &local,
                "adopted_joke_docs",
                &[
                    (
                        "sample-adopted-doc-1-000000000000000000000",
                        "joke:robot-guesses-the-next-word",
                    ),
                    (
                        "sample-adopted-doc-2-000000000000000000000",
                        "joke:the-room-finishes-the-sentence",
                    ),
                ],
            ),
            settle_s: SETTLE_S,
            memos_enabled: env.opt("PK_GSYNC_MEMOS").unwrap_or("1") != "0",
            memo_reserve_mb: env.u64("PK_MEMO_RESERVE_MB").ok().flatten().unwrap_or(MEMO_RESERVE_MB),
            memo_stage_mb: MEMO_STAGE_MB,
            memo_batch: MEMO_BATCH,
            whisper_python: env.opt("PK_WHISPER_PY").unwrap_or("python").to_string(),
            auth_url: env
                .opt("PK_AUTH_URL")
                .unwrap_or("https://auth.punchable.io/realms/punchable")
                .to_string(),
            kc_client: "pk-gsync".to_string(),
            online_first,
            owner_email: env.opt("PUNCHABLE_OWNER_EMAIL").map(str::to_string),
            owner_id: env.opt("PK_OWNER_ID").unwrap_or("owner").to_string(),
        }
    }

    /// The Drive scopes of this mode.
    pub fn scopes(&self) -> Vec<String> {
        if self.scopes_mode == "narrow" {
            SCOPES_NARROW.iter().map(ToString::to_string).collect()
        } else {
            SCOPES_FULL.iter().map(ToString::to_string).collect()
        }
    }

    /// `<comedy>/kb/gsync`.
    pub fn state(&self) -> PathBuf {
        self.comedy.join("kb").join("gsync")
    }

    /// `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` (the same Web client Keycloak uses).
    pub fn google_env(&self) -> PathBuf {
        self.home.join("google-oauth.env")
    }

    /// pk-gsync's Keycloak offline refresh token.
    pub fn kc_token_file(&self) -> PathBuf {
        self.home.join("keycloak_gsync_token.json")
    }

    /// The cached Google token (or the Desktop fallback's).
    pub fn token_file(&self) -> PathBuf {
        self.home.join("google_token.json")
    }

    /// `<comedy>/corpus/gdocs`.
    pub fn corpus_gdocs(&self) -> PathBuf {
        self.comedy.join("corpus").join("gdocs")
    }

    /// `<state>/memos.json`.
    pub fn memos_state(&self) -> PathBuf {
        self.state().join("memos.json")
    }

    /// Downloads land here; he can also drop audio files here by hand.
    pub fn notes_inbox(&self) -> PathBuf {
        self.comedy.join("notes").join("inbox")
    }
}
