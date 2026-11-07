//! Punchable (`gsync/punchable.py`): writes go through the hosted API's worker routes (`/api/worker/ideas`,
//! `/decision`, `/undo`, `/jokes/<id>`, `/decisions`, `/creative`, `/files`) with the worker's Keycloak token, each
//! stamped with its actor: `import:gsync` for Google Docs, `import:memo` for voice memos (never the comic's own id:
//! these are imports, not his taps). Nothing is read from local files; gsync's own state (`kb/gsync/*`) is posted
//! back as srcfile rows.
//!
//! [`Local`] is the plain client of the local API, kept for `status` and `undo` only, which use it in the Python.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use pk_core::{py, text::head};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::config::{Config, IDEA_MAX};
use crate::error::{GsError, Result};
use crate::http::{Http, quote_safe, urlencode};
use crate::services::Services;
use crate::store::write_atomic;

/// The reason prefix on every decision gsync writes: decision lines have no `source` field.
pub const GDOCS: &str = "[gdocs]";
/// The worker's `User-Agent`.
pub const USER_AGENT: &str = "punchable-worker/1";
const STATE_FILES: [&str; 5] = ["map.json", "memos.json", "units.json", "edits.jsonl", "conflicts.jsonl"];
/// `runs.jsonl` grows every minute: posted at most every 10 minutes, or when a run did something.
const RUNS_EVERY_S: f64 = 600.0;

/// A text file read the way Python's `read_text` reads it: universal newlines.
pub fn read_text(p: &Path) -> std::io::Result<String> {
    let t = fs::read_to_string(p)?;
    Ok(if t.contains('\r') {
        t.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        t
    })
}

/// Where the worker client's secret and endpoints come from (`worker.__main__.secrets()`).
#[derive(Clone)]
pub struct WorkerAuth {
    /// The API root.
    pub api: String,
    /// The token endpoint.
    pub token_url: String,
    /// The Keycloak client id.
    pub client_id: String,
    secret: String,
}

impl std::fmt::Debug for WorkerAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WorkerAuth({} as {}, secret [redacted])", self.api, self.client_id)
    }
}

impl WorkerAuth {
    /// From the environment: `PK_API`, `PK_TOKEN_URL`, `PK_WORKER_CLIENT_ID`, `PK_WORKER_SECRET` (the secrets file
    /// under the process environment, which [`pk_core::Env`] already merges).
    pub fn from_env(env: &pk_core::Env) -> Result<Self> {
        let Some(secret) = env.opt("PK_WORKER_SECRET") else {
            return Err(GsError::Exit(format!(
                "no PK_WORKER_SECRET in {} (see worker/README.md)",
                env.place()
            )));
        };
        Ok(Self {
            api: env
                .opt("PK_API")
                .unwrap_or("https://punchable.io")
                .trim_end_matches('/')
                .to_string(),
            token_url: env
                .opt("PK_TOKEN_URL")
                .unwrap_or("https://auth.punchable.io/realms/punchable/protocol/openid-connect/token")
                .to_string(),
            client_id: env.opt("PK_WORKER_CLIENT_ID").unwrap_or("pk-worker").to_string(),
            secret: secret.to_string(),
        })
    }
}

/// How a worker call failed (`worker/__main__.py` `Client.call`).
#[derive(Debug)]
pub enum CallError {
    /// The API answered an error status.
    Status {
        /// The status.
        code: u16,
        /// The first 300 bytes of the body.
        body: String,
    },
    /// Keycloak refused the client credentials.
    Runtime(String),
    /// No answer.
    Net(String),
    /// The answer was not JSON.
    Decode(String),
}

/// The worker's client: JSON over HTTP to `/api/worker/*` with a Keycloak client-credentials token.
pub struct Client {
    http: Http,
    auth: WorkerAuth,
    tok: Option<(String, f64)>,
}

impl Client {
    /// A client for `auth`.
    pub fn new(auth: WorkerAuth) -> Self {
        Self {
            http: Http::new(Duration::from_secs(120)),
            auth,
            tok: None,
        }
    }

    fn token(&mut self, svc: &Services) -> std::result::Result<String, CallError> {
        if let Some((t, exp)) = &self.tok
            && svc.now_secs() < exp - 30.0
        {
            return Ok(t.clone());
        }
        let form = urlencode(&[
            ("grant_type", "client_credentials"),
            ("client_id", &self.auth.client_id),
            ("client_secret", &self.auth.secret),
        ]);
        let r = self
            .http
            .send(
                "POST",
                &self.auth.token_url,
                &[
                    ("Content-Type", "application/x-www-form-urlencoded".to_string()),
                    ("User-Agent", USER_AGENT.to_string()),
                ],
                Some(form.into_bytes()),
            )
            .map_err(|e| CallError::Net(format!("<urlopen error {}>", e.message)))?;
        if r.status >= 400 {
            return Err(CallError::Runtime(format!(
                "Keycloak refused the worker's client credentials (HTTP {})",
                r.status
            )));
        }
        let d: Value = serde_json::from_slice(&r.body).map_err(|e| CallError::Decode(e.to_string()))?;
        let access = d
            .get("access_token")
            .map(py::str)
            .ok_or_else(|| CallError::Decode("KeyError: 'access_token'".into()))?;
        let exp = svc.now_secs() + d.get("expires_in").and_then(Value::as_f64).unwrap_or(60.0);
        self.tok = Some((access.clone(), exp));
        Ok(access)
    }

    /// One call; 204 is `null`.
    pub fn call(
        &mut self,
        svc: &Services,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> std::result::Result<Value, CallError> {
        let tok = self.token(svc)?;
        let mut headers = vec![
            ("User-Agent", USER_AGENT.to_string()),
            ("Accept", "application/json".to_string()),
            ("Authorization", format!("Bearer {tok}")),
        ];
        let data = body.map(|b| serde_json::to_vec(b).unwrap_or_default());
        if data.is_some() {
            headers.push(("Content-Type", "application/json".to_string()));
        }
        let r = self
            .http
            .send(method, &format!("{}{path}", self.auth.api), &headers, data)
            .map_err(|e| CallError::Net(format!("<urlopen error {}>", e.message)))?;
        if r.status >= 400 {
            if r.status == 401 {
                self.tok = None;
            }
            let cut = &r.body[..r.body.len().min(300)];
            return Err(CallError::Status {
                code: r.status,
                body: String::from_utf8_lossy(cut).into_owned(),
            });
        }
        if r.status == 204 {
            return Ok(Value::Null);
        }
        if r.body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&r.body).map_err(|e| CallError::Decode(e.to_string()))
    }
}

/// The hosted Punchable client gsync runs against.
pub struct Hosted<'a> {
    cl: Client,
    cfg: &'a Config,
    svc: &'a Services,
    actor: String,
    /// Requests made through [`Hosted::req`] (the record's `api_calls`).
    pub calls: u64,
    graph: (Option<String>, Option<Value>),
    pushed: BTreeMap<String, String>,
    runs_at: f64,
    read_only: bool,
}

impl<'a> Hosted<'a> {
    /// A client acting as `actor` (`import:gsync`, `import:memo`). `read_only` is added in the port: a read-only client
    /// refuses every request but a GET before it is made. A dry run is built that way, so it cannot write to Punchable
    /// even if a code path forgot to check.
    pub fn new(cfg: &'a Config, svc: &'a Services, auth: WorkerAuth, actor: &str, read_only: bool) -> Self {
        Self {
            cl: Client::new(auth),
            cfg,
            svc,
            actor: actor.to_string(),
            calls: 0,
            graph: (None, None),
            pushed: BTreeMap::new(),
            runs_at: 0.0,
            read_only,
        }
    }

    /// `_req`: the local route names mapped to the worker routes, the errors mapped to gsync's.
    pub fn req(&mut self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        if self.read_only && method != "GET" {
            return Err(GsError::WriteBlocked(format!(
                "a dry run never writes to Punchable: {method} {path}"
            )));
        }
        self.calls += 1;
        let mut path = path.to_string();
        if let Some(rest) = path.strip_prefix("/api/jokes/") {
            path = format!("/api/worker/jokes/{rest}");
        }
        path = match path.as_str() {
            "/api/ideas" => "/api/worker/ideas".into(),
            "/api/decision" => "/api/worker/decision".into(),
            "/api/undo" => "/api/worker/undo".into(),
            _ => path,
        };
        match self.cl.call(self.svc, method, &path, body) {
            Ok(v) => Ok(v),
            Err(CallError::Status { code, body }) => {
                let msg = match serde_json::from_str::<Value>(&body) {
                    Ok(Value::Object(m)) => match m.get("error").filter(|v| py::truthy(Some(v))) {
                        Some(e) => py::str(e),
                        None => body,
                    },
                    _ => body,
                };
                Err(GsError::Api { status: code, msg })
            }
            Err(CallError::Runtime(m) | CallError::Net(m)) => Err(GsError::ApiDown(m)),
            Err(CallError::Decode(m)) => Err(GsError::other("JSONDecodeError", m)),
        }
    }

    /// Is the API there? Any failure is "no".
    pub fn health(&mut self) -> bool {
        match self.cl.call(self.svc, "GET", "/api/health", None) {
            Ok(r) => py::truthy(r.get("ok")) || py::truthy(r.get("loaded")),
            Err(_) => false,
        }
    }

    // ---- writes -------------------------------------------------------------------------------------------------
    /// A new idea; the id and the decision line indexes. A text over 4000 characters goes in as a node add.
    pub fn add_idea(&mut self, text: &str, title: Option<&str>, frm: &Value, reason: &str) -> Result<(String, Value)> {
        if py::len(text) <= IDEA_MAX {
            let body = json!({"text": text, "title": title.unwrap_or_default(), "from": frm, "reason": reason,
                              "actor": self.actor});
            let r = self.req("POST", "/api/ideas", Some(&body))?;
            return Ok((
                field(&r, "id")?,
                r.get("indexes").cloned().ok_or_else(|| key_error("indexes"))?,
            ));
        }
        let first_words: String = py::split(text).take(7).collect::<Vec<_>>().join(" ");
        let t = head(title.filter(|t| !t.is_empty()).unwrap_or(&first_words), 120);
        let t = if t.is_empty() { "New line".to_string() } else { t };
        let body = json!({"target": "node", "action": "add", "reason": reason, "actor": self.actor,
                          "after": {"type": "joke", "title": t, "stage": "idea", "summary": text}});
        let r = self.req("POST", "/api/decision", Some(&body))?;
        let id = r
            .get("decisions")
            .and_then(|d| d.get(0))
            .and_then(|d| d.get("id"))
            .map(py::str)
            .ok_or_else(|| key_error("decisions"))?;
        Ok((id, r.get("indexes").cloned().ok_or_else(|| key_error("indexes"))?))
    }

    /// Edit a joke; `None` when nothing changed.
    pub fn edit_joke(&mut self, jid: &str, after: &Value, reason: &str) -> Result<Option<Value>> {
        let body = json!({"target": "node", "action": "edit", "id": jid, "after": after, "reason": reason,
                          "actor": self.actor});
        match self.req("POST", "/api/decision", Some(&body)) {
            Ok(r) => Ok(Some(
                json!({"indexes": r.get("indexes").cloned().ok_or_else(|| key_error("indexes"))?}),
            )),
            Err(GsError::Api { status: 400, msg }) if msg.contains("nothing changed") => Ok(None),
            Err(e) => Err(e),
        }
    }

    // ---- reads --------------------------------------------------------------------------------------------------
    /// A joke as the sync reads it, or `None` on a 404.
    pub fn joke(&mut self, jid: &str) -> Result<Option<Value>> {
        let d = match self.req("GET", &format!("/api/jokes/{}", quote_safe(jid, ":")), None) {
            Ok(d) => d,
            Err(GsError::Api { status: 404, .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        let j = d
            .get("joke")
            .filter(|v| py::truthy(Some(v)))
            .cloned()
            .unwrap_or(json!({}));
        let lead = j.get("lead_unit").filter(|v| py::truthy(Some(v)));
        let mut lead_text = Value::Null;
        for v in d.get("versions").and_then(Value::as_array).into_iter().flatten() {
            for u in v.get("units").and_then(Value::as_array).into_iter().flatten() {
                if let Some(l) = lead
                    && u.get("id") == Some(l)
                {
                    lead_text = u.get("text").cloned().unwrap_or(Value::Null);
                }
            }
        }
        let summary = match j.get("summary").filter(|v| py::truthy(Some(v))) {
            Some(s) => s.clone(),
            None => json!(""),
        };
        Ok(Some(
            json!({"id": jid, "title": j.get("title").cloned().unwrap_or(Value::Null), "summary": summary,
                       "stage": j.get("stage").cloned().unwrap_or(Value::Null), "lead_text": lead_text,
                       "yours": py::truthy(d.get("yours")), "hidden": py::truthy(j.get("hidden"))}),
        ))
    }

    /// Every decision line, page by page.
    pub fn decisions(&mut self) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut after = 0usize;
        loop {
            let page = self.req("GET", &format!("/api/worker/decisions?after={after}"), None)?;
            let lines = page.get("lines").and_then(Value::as_array).cloned().unwrap_or_default();
            for x in &lines {
                out.push(x.get("decision").cloned().ok_or_else(|| key_error("decision"))?);
            }
            after += lines.len();
            let total = page
                .get("total")
                .and_then(Value::as_u64)
                .ok_or_else(|| key_error("total"))? as usize;
            if lines.is_empty() || after >= total {
                return Ok(out);
            }
        }
    }

    /// Jokes the comic created in the app: live node adds of type joke, not written by gsync.
    pub fn app_jokes(&mut self) -> Result<Vec<String>> {
        let ds = self.decisions()?;
        let undone: Vec<i64> = ds
            .iter()
            .filter_map(|d| d.get("undo_of").and_then(Value::as_i64))
            .collect();
        let mut alive: Vec<(String, bool)> = Vec::new();
        for (i, d) in ds.iter().enumerate() {
            if d.get("target") != Some(&json!("node"))
                || undone.contains(&i64::try_from(i).unwrap_or(i64::MAX))
                || d.get("undo_of").is_some()
            {
                continue;
            }
            let reason = d.get("reason").and_then(Value::as_str).unwrap_or_default();
            let is_joke_add = d.get("action") == Some(&json!("add"))
                && d.get("after").and_then(|a| a.get("type")) == Some(&json!("joke"))
                && !reason.starts_with(GDOCS);
            if is_joke_add {
                let id = field(d, "id")?;
                match alive.iter_mut().find(|(k, _)| *k == id) {
                    Some(e) => e.1 = true,
                    None => alive.push((id, true)),
                }
            } else if d.get("action") == Some(&json!("delete")) {
                let id = d.get("id").and_then(Value::as_str).unwrap_or_default();
                if let Some(e) = alive.iter_mut().find(|(k, _)| k == id) {
                    e.1 = false;
                }
            }
        }
        Ok(alive.into_iter().filter(|(_, v)| *v).map(|(k, _)| k).collect())
    }

    fn snapshot(&mut self, store: &str) -> Result<Map<String, Value>> {
        let r = self.req("GET", &format!("/api/worker/creative?store={store}&snapshot=1"), None)?;
        Ok(r.get("items").and_then(Value::as_object).cloned().unwrap_or_default())
    }

    /// The comic's storyboards, as the API replays them.
    pub fn boards(&mut self) -> Result<Map<String, Value>> {
        self.snapshot("storyboards")
    }

    /// The comic's merges (and AI attaches), as the API replays them.
    pub fn merges(&mut self) -> Result<Map<String, Value>> {
        self.snapshot("merges")
    }

    /// `gdoc_id` -> joke ids, from the KB graph's units and `part_of` edges (most units first).
    pub fn doc_jokes(&mut self) -> Result<BTreeMap<String, Vec<String>>> {
        let listing = self.req("GET", "/api/worker/files?prefix=kb/", None)?;
        let mut files: BTreeMap<String, String> = BTreeMap::new();
        for f in listing.get("files").and_then(Value::as_array).into_iter().flatten() {
            files.insert(field(f, "path")?, field(f, "sha256")?);
        }
        let rel = if files.contains_key("kb/v1/graph.json") {
            "kb/v1/graph.json"
        } else {
            "kb/v0/graph.json"
        };
        let Some(sha) = files.get(rel) else {
            return Ok(BTreeMap::new());
        };
        if self.graph.0.as_deref() != Some(sha) {
            let d = self.req(
                "GET",
                &format!("/api/worker/files/content?path={}", quote_safe(rel, "/")),
                None,
            )?;
            let g: Value = serde_json::from_str(&field(&d, "text")?)?;
            self.graph = (Some(sha.clone()), Some(g));
        }
        let g = self.graph.1.clone().unwrap_or(Value::Null);
        let mut udoc: BTreeMap<String, String> = BTreeMap::new();
        for n in g.get("nodes").and_then(Value::as_array).into_iter().flatten() {
            if n.get("type") == Some(&json!("unit"))
                && let (Some(id), Some(doc)) = (
                    n.get("id").and_then(Value::as_str),
                    n.get("doc_id").filter(|v| py::truthy(Some(v))),
                )
            {
                udoc.insert(id.to_string(), py::str(doc));
            }
        }
        let mut count: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
        for e in g.get("edges").and_then(Value::as_array).into_iter().flatten() {
            let status = e.get("status").and_then(Value::as_str);
            if e.get("kind") == Some(&json!("part_of"))
                && let Some(doc) = e.get("src").and_then(Value::as_str).and_then(|s| udoc.get(s))
                && status != Some("rejected")
                && status != Some("ruled_out")
            {
                *count
                    .entry(doc.clone())
                    .or_default()
                    .entry(field(e, "dst")?)
                    .or_insert(0) += 1;
            }
        }
        Ok(count
            .into_iter()
            .map(|(d, c)| {
                let mut jokes: Vec<(&String, &i64)> = c.iter().collect();
                jokes.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
                (d.clone(), jokes.into_iter().map(|(j, _)| j.clone()).collect())
            })
            .collect())
    }

    /// A fresh PC: gsync's state files that are in the database but not here are downloaded (never overwritten: this
    /// PC is their only writer).
    pub fn pull_state(&mut self) -> Result<Vec<String>> {
        let mut got = Vec::new();
        let listing = self.req("GET", "/api/worker/files?prefix=kb/gsync/", None)?;
        for f in listing.get("files").and_then(Value::as_array).into_iter().flatten() {
            let path = field(f, "path")?;
            let p = self.cfg.comedy.join(&path);
            if !p.exists() {
                let d = self.req(
                    "GET",
                    &format!("/api/worker/files/content?path={}", quote_safe(&path, "/")),
                    None,
                )?;
                write_atomic(&p, &field(&d, "text")?)?;
                got.push(path.clone());
            }
            self.pushed.insert(path, field(f, "sha256")?);
        }
        Ok(got)
    }

    /// gsync's own state (`kb/gsync/*`) -> the database, when it changed. `paths`: extra data-folder files to post too.
    pub fn push_state(&mut self, paths: &[String], force_runs: bool) -> Result<Value> {
        let mut rels: Vec<String> = STATE_FILES.iter().map(|n| format!("kb/gsync/{n}")).collect();
        let now = self.svc.now_secs();
        let with_runs = force_runs || now - self.runs_at > RUNS_EVERY_S;
        if with_runs {
            rels.push("kb/gsync/runs.jsonl".into());
        }
        rels.extend(paths.iter().cloned());
        let mut files = Vec::new();
        for rel in &rels {
            let p = self.cfg.comedy.join(rel);
            if !p.is_file() {
                continue;
            }
            let text = read_text(&p)?;
            let h = hex::encode(Sha256::digest(text.as_bytes()));
            if self.pushed.get(rel) == Some(&h) {
                continue;
            }
            files.push(json!({"path": rel, "text": text, "sha256": h}));
        }
        if files.is_empty() {
            return Ok(json!({"written": []}));
        }
        let body = json!({"files": files, "actor": self.actor});
        let r = self.req("POST", "/api/worker/files", Some(&body))?;
        for f in &files {
            self.pushed.insert(py::str(&f["path"]), py::str(&f["sha256"]));
        }
        if with_runs {
            self.runs_at = self.svc.now_secs();
        }
        Ok(r)
    }

    /// `POST /api/ideas` with a body of the caller's own making (the voice memos).
    pub fn post(&mut self, path: &str, body: &Value) -> Result<Value> {
        self.req("POST", path, Some(body))
    }

    /// The actor stamped on writes.
    pub fn actor(&self) -> &str {
        &self.actor
    }
}

fn key_error(k: &str) -> GsError {
    GsError::other("KeyError", format!("'{k}'"))
}

fn field(v: &Value, k: &str) -> Result<String> {
    v.get(k).map(py::str).ok_or_else(|| key_error(k))
}

/// The plain client of the local API (`Punchable` in the Python), for `status` and `undo`.
pub struct Local {
    api: String,
    actor: String,
}

impl Local {
    /// A client of the API at `cfg.api`.
    pub fn new(cfg: &Config) -> Self {
        Self {
            api: cfg.api.clone(),
            actor: "import:gsync".into(),
        }
    }

    fn req(&self, method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value> {
        let http = Http::new(timeout);
        let data = body.map(|b| serde_json::to_vec(b).unwrap_or_default());
        let r = http
            .send(
                method,
                &format!("{}{path}", self.api),
                &[("Content-Type", "application/json".to_string())],
                data,
            )
            .map_err(|e| GsError::ApiDown(format!("<urlopen error {}>", e.message)))?;
        if r.status >= 400 {
            let msg = match serde_json::from_slice::<Value>(&r.body) {
                Ok(v) => v.get("error").map_or_else(|| "None".to_string(), py::str),
                Err(_) => format!("HTTP Error {}: {}", r.status, r.phrase),
            };
            return Err(GsError::Api { status: r.status, msg });
        }
        Ok(serde_json::from_slice(&r.body)?)
    }

    /// Is the local API running?
    pub fn health(&self) -> bool {
        match self.req("GET", "/api/health", None, Duration::from_secs(5)) {
            Ok(r) => py::truthy(r.get("ok")),
            Err(_) => false,
        }
    }

    /// Undo decision lines.
    pub fn undo(&self, indexes: &[i64]) -> Result<Value> {
        let body = json!({"indexes": indexes, "actor": self.actor});
        self.req("POST", "/api/undo", Some(&body), Duration::from_secs(30))
    }
}
