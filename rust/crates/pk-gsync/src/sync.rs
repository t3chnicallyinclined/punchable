//! The sync engine (`gsync/sync.py`): bootstrap (first run) / pull (Changes API) / push (app jokes) / boards. Ported
//! line for line: the same requests in the same order, the same state, the same words in the run record.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use pk_core::{py, text::head};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::config::{Config, DOC_MIME, FOLDER_MIME};
use crate::drive::GoogleDrive;
use crate::error::{GsError, Result};
use crate::punchable::{GDOCS, Hosted, read_text};
use crate::rules::Rules;
use crate::services::Services;
use crate::store::Store;
use crate::units::{h, mirror_name, mirror_text, norm, split_units};

static FM_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(\w+): (.*)$").expect("regex"));

/// `datetime.fromisoformat(s.replace("Z", "+00:00"))`.
pub fn parse_ts(s: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    let Some(s) = s.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    DateTime::parse_from_rfc3339(&s.replace('Z', "+00:00"))
        .map(|t| Some(t.with_timezone(&Utc)))
        .map_err(|_| GsError::other("ValueError", format!("Invalid isoformat string: '{s}'")))
}

fn untitled(name: Option<&str>) -> bool {
    let n = name.unwrap_or_default();
    py::strip(n).is_empty() || py::strip(n).to_lowercase().starts_with("untitled document")
}

/// The string at `key`, or `None` when it is missing, null or not a string.
fn sget<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key).and_then(Value::as_str)
}

/// `v.get(key)` as `str()` prints it (`None` for a missing key).
fn pstr(v: &Value, key: &str) -> String {
    py::str(v.get(key).unwrap_or(&Value::Null))
}

/// Python truthiness of `v.get(key)`.
fn tr(v: &Value, key: &str) -> bool {
    py::truthy(v.get(key))
}

fn same_version(meta: &Value, e: &Value) -> bool {
    pstr(meta, "version") == py::str(&e["version"])
}

/// One sync pass.
pub struct Sync<'a> {
    /// The settings.
    pub cfg: &'a Config,
    /// The outside world.
    pub svc: &'a Services,
    /// The legal rules.
    pub rules: &'a Rules,
    /// Drive.
    pub drive: GoogleDrive<'a>,
    /// Punchable.
    pub pk: Hosted<'a>,
    /// A dry run: reads only, nothing written to Drive, Punchable or the state.
    pub dry: bool,
    /// The files under `kb/gsync`; `store.m` is the map.
    pub store: Store,
    /// The run record being built.
    pub rec: Value,
}

impl<'a> Sync<'a> {
    /// A pass over `drive` and `pk`.
    pub fn new(
        cfg: &'a Config,
        svc: &'a Services,
        rules: &'a Rules,
        drive: GoogleDrive<'a>,
        pk: Hosted<'a>,
        dry: bool,
    ) -> Result<Self> {
        let mut store = Store::open(&cfg.state())?;
        for k in ["docs", "boards", "excluded", "skipped", "pending", "folders"] {
            if !store.m.as_object().is_some_and(|o| o.contains_key(k)) {
                store.m[k] = json!({});
            }
        }
        Ok(Self {
            cfg,
            svc,
            rules,
            drive,
            pk,
            dry,
            store,
            rec: json!({}),
        })
    }

    // ---- bookkeeping --------------------------------------------------------------------------------------------
    /// Record an action in the run: `{"op": op, ...kv}` with the null values left out, and count it.
    pub fn act(&mut self, op: &str, kv: &[(&str, Value)]) {
        let mut a = Map::new();
        a.insert("op".into(), json!(op));
        for (k, v) in kv {
            if !v.is_null() {
                a.insert((*k).to_string(), v.clone());
            }
        }
        self.rec["actions"]
            .as_array_mut()
            .expect("actions")
            .push(Value::Object(a));
        let n = self.rec["counts"][op].as_i64().unwrap_or(0) + 1;
        self.rec["counts"][op] = json!(n);
    }

    /// Add an entry to the run's `errors`.
    pub fn error(&mut self, e: Value) {
        self.rec["errors"].as_array_mut().expect("errors").push(e);
    }

    fn conflict(&mut self, kw: &[(&str, Value)]) -> Result<()> {
        let get = |k: &str| kw.iter().find(|(n, _)| *n == k).map_or(Value::Null, |(_, v)| v.clone());
        if !self.dry {
            let mut rec = Map::new();
            rec.insert("ts".into(), json!(self.svc.clock.now_iso()));
            rec.insert("run_id".into(), self.rec["run_id"].clone());
            for (k, v) in kw {
                rec.insert((*k).to_string(), v.clone());
            }
            self.store.log("conflicts.jsonl", &Value::Object(rec))?;
        }
        self.act(
            "conflict",
            &[("doc", get("doc_id")), ("joke", get("joke_id")), ("kind", get("kind"))],
        );
        Ok(())
    }

    /// Write `map.json` unless this is a dry run.
    pub fn save(&self) -> Result<()> {
        if self.dry {
            return Ok(());
        }
        self.store.save()
    }

    fn now_iso(&self) -> String {
        self.svc.clock.now_iso()
    }

    fn owner_edit(&self, mut fields: Vec<(&str, Value)>) -> Value {
        let mut m = Map::new();
        m.insert("ts".into(), json!(self.now_iso()));
        m.insert("by".into(), json!(self.cfg.owner_id));
        m.insert("source".into(), json!("gdocs"));
        for (k, v) in fields.drain(..) {
            m.insert(k.to_string(), v);
        }
        Value::Object(m)
    }

    /// `m["docs"][fid]` updated with the pairs, as `dict.update` does it.
    fn upd(&mut self, fid: &str, pairs: Vec<(&str, Value)>) {
        let e = &mut self.store.m["docs"][fid];
        if !e.is_object() {
            *e = json!({});
        }
        for (k, v) in pairs {
            e[k] = v;
        }
    }

    fn pending_pop(&mut self, fid: &str) {
        if let Some(p) = self.store.m["pending"].as_object_mut() {
            p.shift_remove(fid);
        }
    }

    // ---- entry point --------------------------------------------------------------------------------------------
    /// Run one pass and return the record. Never fails for Drive or API trouble: problems are in `errors` / `mode`.
    pub fn run(&mut self) -> Result<Value> {
        let t0 = self.svc.now();
        self.rec = json!({"run_id": format!("gs-{}", t0.format("%Y%m%d-%H%M%S")), "ts": self.now_iso(),
                          "dry_run": self.dry, "mode": null, "actions": [], "counts": {}, "errors": []});
        let outcome = self.run_inner();
        match outcome {
            Ok(true) => return self.finish(),
            Ok(false) => {}
            Err(GsError::ApiDown(e)) => self.error(json!({"where": "api", "error": format!("api_down: {e}")})),
            Err(e @ GsError::Exit(_)) => return Err(e),
            Err(e) => self.error(json!({"where": "run", "error": e.describe(),
                                        "trace": "(no traceback: pk-gsync)"})),
        }
        self.finish()
    }

    /// `Ok(true)` when the pass stopped early (API down).
    fn run_inner(&mut self) -> Result<bool> {
        if !self.pk.health() {
            self.rec["mode"] = json!("api_down");
            return Ok(true);
        }
        self.adopt();
        if self.store.m["changes_token"].is_null() {
            self.rec["mode"] = json!("bootstrap");
            self.bootstrap()?;
        } else {
            self.rec["mode"] = json!("pull");
            self.pull()?;
        }
        self.push_jokes()?;
        self.push_boards()?;
        crate::merge::push_merges(self)?;
        crate::memos::pull_memos(self)?;
        Ok(false)
    }

    fn finish(&mut self) -> Result<Value> {
        self.rec["drive"] = self.drive.stats.to_value();
        self.rec["api_calls"] = json!(self.pk.calls);
        self.rec["ts_end"] = json!(self.now_iso());
        self.save()?;
        if !self.dry {
            self.store.log("runs.jsonl", &self.rec)?;
        }
        Ok(self.rec.clone())
    }

    // ---- adoption -----------------------------------------------------------------------------------------------
    fn adopt(&mut self) {
        let cfg = self.cfg;
        if cfg.scopes_mode == "narrow" {
            if !self.store.m.as_object().is_some_and(|o| o.contains_key("folder_id")) {
                self.store.m["folder_id"] = Value::Null;
            }
        } else {
            if !self.store.m.as_object().is_some_and(|o| o.contains_key("folder_id")) {
                self.store.m["folder_id"] = json!(cfg.folder_id);
            }
            for (sb, did) in &cfg.board_docs {
                if self.store.m["boards"].get(sb).is_none() {
                    self.store.m["boards"][sb] = json!({"doc_id": did, "hash": null, "version": null, "adopted": true});
                    self.store.m["docs"][did] = json!({"origin": "board", "board": sb});
                    self.act("adopt", &[("doc", json!(did)), ("kind", json!("board"))]);
                }
            }
        }
        for (did, title) in &cfg.ai_draft_docs {
            if self.store.m["docs"].get(did).is_none() {
                self.store.m["docs"][did] = json!({"origin": "ai_draft", "source": "ai_draft", "contains_drafts": true,
                                                   "title": title, "joke_id": null, "version": null, "doc_hash": null});
                self.act("adopt", &[("doc", json!(did)), ("kind", json!("ai_draft"))]);
            }
        }
        for (did, jid) in &cfg.mixed_docs {
            if self.store.m["docs"].get(did).is_none() {
                self.store.m["docs"][did] = json!({"origin": "mixed", "source": "mixed", "contains_drafts": true,
                                                   "joke_id": jid, "version": null, "doc_hash": null, "app_hash": null});
                self.act(
                    "adopt",
                    &[("doc", json!(did)), ("joke", json!(jid)), ("kind", json!("mixed"))],
                );
            }
        }
        for (did, jid) in &cfg.adopted_joke_docs {
            if self.store.m["docs"].get(did).is_none() {
                self.store.m["docs"][did] = json!({"origin": "app", "adopted": true, "joke_id": jid, "version": null,
                                                   "doc_hash": null, "app_hash": null});
                self.act(
                    "adopt",
                    &[("doc", json!(did)), ("joke", json!(jid)), ("kind", json!("joke"))],
                );
            }
        }
        let boards: Vec<Value> = self.store.m["boards"]
            .as_object()
            .map(|o| o.values().cloned().collect())
            .unwrap_or_default();
        for b in boards {
            if tr(&b, "doc_id") && (cfg.scopes_mode != "narrow" || !tr(&b, "adopted")) {
                self.drive.allow(sget(&b, "doc_id").unwrap_or_default());
            }
        }
        let apps: Vec<String> = self.store.m["docs"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter(|(_, e)| e["origin"] == "app")
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        for did in apps {
            self.drive.allow(&did);
        }
    }

    // ---- bootstrap ----------------------------------------------------------------------------------------------
    fn seed_corpus(&mut self) -> Result<usize> {
        let dj = self.pk.doc_jokes()?;
        let dir = self.cfg.corpus_gdocs();
        let mut n = 0;
        if dir.exists() {
            let mut files: Vec<_> = std::fs::read_dir(&dir)?
                .filter_map(std::result::Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "md"))
                .collect();
            files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
            for f in files {
                let text = read_text(&f)?;
                let head_text = text.split("\n---").next().unwrap_or_default();
                let mut fm: BTreeMap<String, String> = BTreeMap::new();
                for c in FM_LINE.captures_iter(head_text) {
                    fm.insert(c[1].to_string(), c[2].to_string());
                }
                let Some(did) = fm.get("doc_id").filter(|d| !d.is_empty()) else {
                    continue;
                };
                if self.store.m["docs"].get(did).is_some() {
                    continue;
                }
                let name = f
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.store.m["docs"][did] = json!({
                    "origin": "corpus", "jokes": dj.get(did).cloned().unwrap_or_default(),
                    "modified": fm.get("modified"),
                    "title": fm.get("title").map_or("", |t| t.trim_matches('"')),
                    "file": format!("corpus/gdocs/{name}"), "version": null, "doc_hash": null});
                n += 1;
            }
        }
        Ok(n)
    }

    fn bootstrap(&mut self) -> Result<()> {
        let token = self.drive.start_token()?; // before listing, so nothing changed during the listing is missed
        let n = self.seed_corpus()?;
        self.act("seed_corpus", &[("n", json!(n))]);
        let listed = self.drive.list_docs()?;
        let ids: BTreeSet<String> = listed
            .iter()
            .filter_map(|x| sget(x, "id").map(str::to_string))
            .collect();
        for meta in &listed {
            self.safe(meta, true)?;
        }
        let entries: Vec<(String, Value)> = self.store.m["docs"]
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        for (did, e) in entries {
            let origin = sget(&e, "origin");
            if matches!(origin, Some("corpus" | "gdocs"))
                && !ids.contains(&did)
                && !tr(&e, "hidden")
                && !tr(&e, "excluded")
            {
                self.gone(&did)?;
            }
        }
        self.store.m["bootstrapped_at"] = json!(self.now_iso());
        if !self.dry && self.rec["errors"].as_array().is_some_and(Vec::is_empty) {
            self.store.m["changes_token"] = json!(token);
        }
        Ok(())
    }

    // ---- pull ---------------------------------------------------------------------------------------------------
    fn pull(&mut self) -> Result<()> {
        let token = py::str(&self.store.m["changes_token"]);
        let (changes, new_token) = self.drive.changes(&token)?;
        let mut last: Vec<(String, Value)> = Vec::new();
        for c in changes {
            let id = pstr(&c, "fileId");
            match last.iter_mut().find(|(k, _)| *k == id) {
                Some(slot) => slot.1 = c,
                None => last.push((id, c)),
            }
        }
        for (fid, c) in &last {
            let f = c
                .get("file")
                .filter(|v| py::truthy(Some(v)))
                .cloned()
                .unwrap_or(json!({}));
            if tr(c, "removed") || tr(&f, "trashed") {
                if self.store.m["docs"].get(fid).is_some() {
                    self.gone(fid)?;
                }
                self.pending_pop(fid);
                continue;
            }
            if f["mimeType"] == FOLDER_MIME {
                let parents = f
                    .get("parents")
                    .filter(|v| py::truthy(Some(v)))
                    .cloned()
                    .unwrap_or(json!([]));
                self.store.m["folders"][fid] = parents;
                continue;
            }
            if f["mimeType"] == DOC_MIME {
                self.safe(&f, false)?;
            }
        }
        let pending: Vec<String> = self.store.m["pending"]
            .as_object()
            .map(|o| {
                o.keys()
                    .filter(|k| !last.iter().any(|(l, _)| l == *k))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for fid in pending {
            let meta = self.drive.get(&fid)?;
            match meta {
                Some(m) if !tr(&m, "trashed") => self.safe(&m, false)?,
                _ => {
                    self.pending_pop(&fid);
                    if self.store.m["docs"].get(&fid).is_some() {
                        self.gone(&fid)?;
                    }
                }
            }
        }
        if !self.dry {
            self.store.m["changes_token"] = json!(new_token);
        }
        Ok(())
    }

    /// One bad doc never stops the run; it is retried next run.
    fn safe(&mut self, meta: &Value, bootstrap: bool) -> Result<()> {
        match self.handle(meta, bootstrap) {
            Ok(()) => Ok(()),
            Err(e @ (GsError::ApiDown(_) | GsError::Exit(_))) => Err(e),
            Err(e) => {
                let id = pstr(meta, "id");
                self.store.m["pending"][&id] = json!("error");
                self.error(json!({"doc": id, "error": e.describe()}));
                Ok(())
            }
        }
    }

    /// Every folder above the doc, nearest cache first (the Python's stack order, so the same Drive reads happen).
    pub fn ancestors(&mut self, meta: &Value) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        let mut todo: Vec<String> = meta
            .get("parents")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(py::str).collect())
            .unwrap_or_default();
        while let Some(p) = todo.pop() {
            if !seen.insert(p.clone()) {
                continue;
            }
            out.push(p.clone());
            if self.store.m["folders"].get(&p).is_none() {
                let fm = self.drive.get(&p)?;
                let parents = fm
                    .as_ref()
                    .and_then(|f| f.get("parents"))
                    .filter(|v| py::truthy(Some(v)))
                    .cloned()
                    .unwrap_or(json!([]));
                self.store.m["folders"][&p] = parents;
            }
            if let Some(a) = self.store.m["folders"][&p].as_array() {
                todo.extend(a.iter().map(py::str));
            }
        }
        Ok(out)
    }

    fn excluded(&self) -> Map<String, Value> {
        self.store.m["excluded"].as_object().cloned().unwrap_or_default()
    }

    fn handle(&mut self, meta: &Value, bootstrap: bool) -> Result<()> {
        let fid = pstr(meta, "id");
        // 1. legal: metadata only, before anything is exported
        let anc = self.ancestors(meta)?;
        if let Some(why) = self.rules.legal_reason(meta, &anc, &self.excluded()) {
            let m = &mut self.store.m;
            if m["excluded"].get(&fid).is_none() {
                m["excluded"][&fid] = json!(why);
            }
            if m["docs"].get(&fid).is_some() {
                m["docs"][&fid]["excluded"] = json!(true);
            }
            self.pending_pop(&fid);
            let reason = why.split(':').next().unwrap_or_default().to_string();
            self.act("skip_legal", &[("doc", json!(fid)), ("why", json!(reason))]);
            return Ok(());
        }
        let e = self.store.m["docs"].get(&fid).filter(|v| py::truthy(Some(v))).cloned();
        let origin = e.as_ref().and_then(|e| sget(e, "origin")).map(str::to_string);
        let origin = origin.as_deref();
        // 2. ours
        if origin == Some("board") || meta["appProperties"]["punchable_kind"] == "board" {
            return Ok(());
        }
        match origin {
            Some("ai_draft") => return self.ai_draft(meta),
            Some("mixed") => return self.mixed(meta),
            Some("merged" | "merged_undone") => return self.merged_doc(meta),
            _ => {}
        }
        if let Some(e) = &e {
            if tr(e, "excluded") || tr(e, "detached") {
                return Ok(());
            }
            if origin == Some("app") && e["version"].is_null() {
                // adopted by id (made through the connector, no appProperties): today's state is the baseline
                let jid = sget(e, "joke_id").filter(|j| !j.is_empty());
                let cur = match jid {
                    Some(j) => self.pk.joke(j)?,
                    None => None,
                };
                let doc_hash = h(&self.drive.export(&fid, "text/plain")?);
                let app_hash = cur
                    .as_ref()
                    .map_or(Value::Null, |c| json!(h(sget(c, "summary").unwrap_or_default())));
                self.upd(
                    &fid,
                    vec![
                        ("version", meta.get("version").cloned().unwrap_or(Value::Null)),
                        ("modified", meta.get("modifiedTime").cloned().unwrap_or(Value::Null)),
                        ("title", meta.get("name").cloned().unwrap_or(Value::Null)),
                        ("doc_hash", json!(doc_hash)),
                        ("app_hash", app_hash),
                    ],
                );
                self.pending_pop(&fid);
                self.act(
                    "baseline",
                    &[
                        ("doc", json!(fid)),
                        ("joke", e.get("joke_id").cloned().unwrap_or(Value::Null)),
                        ("kind", json!("joke")),
                    ],
                );
                return Ok(());
            }
            if !e["version"].is_null() && same_version(meta, e) {
                return Ok(()); // our own write, or nothing new: the loop guard
            }
            if origin == Some("corpus")
                && e["version"].is_null()
                && sget(meta, "modifiedTime").unwrap_or_default() <= sget(e, "modified").unwrap_or_default()
            {
                // unchanged since the ingest: remember its version, export nothing
                self.store.m["docs"][&fid]["version"] = meta.get("version").cloned().unwrap_or(Value::Null);
                return Ok(());
            }
        } else if bootstrap && self.store.m["skipped"].get(&fid).is_some() {
            return Ok(());
        }
        // 3. settling
        let mod_t = parse_ts(sget(meta, "modifiedTime"))?;
        if let Some(t) = mod_t
            && ((self.svc.now() - t).num_milliseconds() as f64 / 1000.0) < self.cfg.settle_s as f64
        {
            self.store.m["pending"][&fid] = json!("settling");
            self.act("settling", &[("doc", json!(fid))]);
            return Ok(());
        }
        // 4. comedy triage (new docs only; a mapped doc is already his material)
        if e.is_none() {
            let t = self.rules.triage(meta);
            if t != "comedy" {
                self.store.m["skipped"][&fid] = json!(t);
                self.pending_pop(&fid);
                self.act("skip_not_comedy", &[("doc", json!(fid))]);
                return Ok(());
            }
        }
        // 5. export, then the content net
        let plain = norm(&self.drive.export(&fid, "text/plain")?);
        if self.rules.content_is_legal(&plain) {
            self.store.m["excluded"][&fid] = json!("content");
            if e.is_some() {
                self.store.m["docs"][&fid]["excluded"] = json!(true);
            }
            self.pending_pop(&fid);
            self.act("skip_legal", &[("doc", json!(fid)), ("why", json!("content"))]);
            return Ok(());
        }
        if e.is_none() && plain.is_empty() && untitled(sget(meta, "name")) {
            self.pending_pop(&fid);
            self.act("wait_empty", &[("doc", json!(fid))]); // it comes back through the Changes feed once he types
            return Ok(());
        }
        let md = self
            .drive
            .export(&fid, "text/markdown")?
            .replace('\u{feff}', "")
            .replace("\r\n", "\n");
        if e.is_some() {
            self.edited(meta, &plain, &md)?;
        } else {
            self.new_doc(meta, &plain, &md)?;
        }
        self.pending_pop(&fid);
        Ok(())
    }

    fn mirror(&mut self, meta: &Value, md: &str) -> Result<Value> {
        if self.dry {
            return Ok(Value::Null);
        }
        let name = mirror_name(meta);
        let rel = self.store.put_mirror(&name, &mirror_text(meta, md))?;
        self.store.put_units(&pstr(meta, "id"), split_units(meta, md, &rel))?;
        Ok(json!(rel))
    }

    /// Crash guard: a gsync idea for this doc that reached the decision lines but not `map.json`.
    fn joke_for_doc(&mut self, fid: &str) -> Result<Option<String>> {
        let ds = self.pk.decisions()?;
        let undone: Vec<i64> = ds
            .iter()
            .filter_map(|d| d.get("undo_of").and_then(Value::as_i64))
            .collect();
        let needle = format!("({fid},");
        for (i, d) in ds.iter().enumerate() {
            let reason = d.get("reason").and_then(Value::as_str).unwrap_or_default();
            if d.get("action") == Some(&json!("add"))
                && !undone.contains(&i64::try_from(i).unwrap_or(i64::MAX))
                && reason.starts_with(GDOCS)
                && reason.contains(&needle)
            {
                return Ok(d.get("id").map(py::str));
            }
        }
        Ok(None)
    }

    fn new_doc(&mut self, meta: &Value, plain: &str, md: &str) -> Result<()> {
        let fid = pstr(meta, "id");
        let name = sget(meta, "name").unwrap_or_default().to_string();
        let text = if plain.is_empty() {
            name.clone()
        } else {
            plain.to_string()
        }; // an empty body: the title is the premise
        let title = if untitled(Some(&name)) {
            None
        } else {
            Some(head(&name, 120))
        };
        if self.dry {
            self.act("new", &[("doc", json!(fid)), ("title", json!(name))]);
            return Ok(());
        }
        let mut jid = self.joke_for_doc(&fid)?;
        let mut idx = Value::Null;
        if jid.is_none() {
            let frm = json!({"gdoc_id": fid, "title": name, "modified": meta.get("modifiedTime"),
                             "note": format!("Google Doc: {name}")});
            let reason = format!(
                "{GDOCS} new Google Doc '{name}' ({fid}, modified {})",
                pstr(meta, "modifiedTime")
            );
            let (id, indexes) = self.pk.add_idea(&text, title.as_deref(), &frm, &reason)?;
            jid = Some(id);
            idx = indexes;
        }
        let file = self.mirror(meta, md)?;
        self.store.m["docs"][&fid] = json!({"origin": "gdocs", "joke_id": jid, "title": name,
                                            "version": meta.get("version"), "modified": meta.get("modifiedTime"),
                                            "doc_hash": h(plain), "app_hash": h(&text), "file": file});
        self.save()?; // every write is recorded before the next one
        self.act(
            "new",
            &[
                ("doc", json!(fid)),
                ("joke", json!(jid)),
                ("indexes", idx),
                ("title", json!(name)),
            ],
        );
        Ok(())
    }

    fn version_modified(meta: &Value) -> Vec<(&'static str, Value)> {
        vec![
            ("version", meta.get("version").cloned().unwrap_or(Value::Null)),
            ("modified", meta.get("modifiedTime").cloned().unwrap_or(Value::Null)),
        ]
    }

    fn edited(&mut self, meta: &Value, plain: &str, md: &str) -> Result<()> {
        let fid = pstr(meta, "id");
        let name = sget(meta, "name").unwrap_or_default().to_string();
        let dh = h(plain);
        let e = self.store.m["docs"][&fid].clone();
        let origin = sget(&e, "origin").unwrap_or_default().to_string();
        if e["doc_hash"].is_null() && origin == "corpus" && tr(&e, "file") {
            let p = self.cfg.comedy.join(sget(&e, "file").unwrap_or_default());
            if p.exists() {
                let t = read_text(&p)?;
                let body = t.splitn(2, "\n---\n").last().unwrap_or_default();
                if norm(body) == norm(md) {
                    self.store.m["docs"][&fid]["doc_hash"] = json!(dh);
                }
            }
        }
        if self.store.m["docs"][&fid]["doc_hash"] == json!(dh) {
            self.upd(&fid, Self::version_modified(meta));
            return Ok(());
        }
        if origin == "corpus" {
            if !self.dry {
                let live = self.mirror(meta, md)?;
                self.store.m["docs"][&fid]["file_live"] = live.clone();
                let rec = self.owner_edit(vec![
                    ("doc_id", json!(fid)),
                    ("kind", json!("corpus_doc_edit")),
                    ("jokes", e.get("jokes").cloned().unwrap_or(Value::Null)),
                    ("modified", meta.get("modifiedTime").cloned().unwrap_or(Value::Null)),
                    ("file", live),
                ]);
                self.store.log("edits.jsonl", &rec)?;
                let mut up = Self::version_modified(meta);
                up.push(("doc_hash", json!(dh)));
                self.upd(&fid, up);
            }
            self.act(
                "edit_units",
                &[
                    ("doc", json!(fid)),
                    ("jokes", e.get("jokes").cloned().unwrap_or(Value::Null)),
                ],
            );
            return Ok(());
        }
        let jid = sget(&e, "joke_id").filter(|j| !j.is_empty()).map(str::to_string);
        let cur = match &jid {
            Some(j) => self.pk.joke(j)?,
            None => None,
        };
        if cur.is_none()
            && let Some(j) = &jid
            && self.merged_into(j)?.is_some()
        {
            // the joke was merged into another one (hidden): his edit is kept, verbatim, and the doc stays synced
            if !self.dry {
                let into = self.merged_into(j)?;
                let rec = self.owner_edit(vec![
                    ("doc_id", json!(fid)),
                    ("kind", json!("merged_member_doc_edit")),
                    ("joke_id", json!(j)),
                    ("merged_into", json!(into)),
                    ("title", json!(name)),
                    ("modified", meta.get("modifiedTime").cloned().unwrap_or(Value::Null)),
                    ("text", json!(plain)),
                ]);
                self.store.log("edits.jsonl", &rec)?;
                let mut up = Self::version_modified(meta);
                up.push(("doc_hash", json!(dh)));
                self.upd(&fid, up);
            }
            self.act("merged_member_edit", &[("doc", json!(fid)), ("joke", json!(j))]);
            return Ok(());
        }
        let Some(cur) = cur else {
            self.store.m["docs"][&fid]["detached"] = json!(true); // he undid or removed the joke; the doc stays his
            self.act(
                "detached",
                &[
                    ("doc", json!(fid)),
                    ("joke", e.get("joke_id").cloned().unwrap_or(Value::Null)),
                ],
            );
            return Ok(());
        };
        let jid = jid.unwrap_or_default();
        let new_text = if plain.is_empty() {
            name.clone()
        } else {
            plain.to_string()
        };
        let mut after = Map::new();
        after.insert("summary".into(), json!(new_text));
        let e_title = e.get("title").cloned().unwrap_or(Value::Null);
        if matches!(origin.as_str(), "gdocs" | "app")
            && !untitled(Some(&name))
            && e_title != json!(name)
            && json!(head(&name, 120)) != cur["title"]
        {
            after.insert("title".into(), json!(head(&name, 120)));
        }
        let cur_summary = sget(&cur, "summary").unwrap_or_default().to_string();
        if json!(h(&cur_summary)) != e["app_hash"] {
            self.conflict(&[
                ("kind", json!("both_edited")),
                ("doc_id", json!(fid)),
                ("joke_id", json!(jid)),
                ("title", cur["title"].clone()),
                ("app_text", json!(cur_summary)),
                ("doc_text", json!(new_text)),
                ("resolution", json!("the doc's text was applied (the doc is the source of truth); the app text is kept here verbatim, and undoing this run restores it")),
            ])?;
        }
        if self.dry {
            self.act("edit", &[("doc", json!(fid)), ("joke", json!(jid))]);
            return Ok(());
        }
        let reason = format!(
            "{GDOCS} edited in Google Docs '{name}' ({fid}, modified {})",
            pstr(meta, "modifiedTime")
        );
        let r = self.pk.edit_joke(&jid, &Value::Object(after), &reason)?;
        let file = self.mirror(meta, md)?;
        let mut up = Self::version_modified(meta);
        up.extend([
            ("doc_hash", json!(dh)),
            ("app_hash", json!(h(&new_text))),
            ("title", json!(name)),
            ("file", file),
        ]);
        self.upd(&fid, up);
        self.save()?;
        self.act(
            "edit",
            &[
                ("doc", json!(fid)),
                ("joke", json!(jid)),
                ("indexes", r.map_or(Value::Null, |r| r["indexes"].clone())),
            ],
        );
        Ok(())
    }

    fn settled(&self, meta: &Value) -> Result<bool> {
        let t = parse_ts(sget(meta, "modifiedTime"))?;
        Ok(t.is_none_or(|t| ((self.svc.now() - t).num_milliseconds() as f64 / 1000.0) >= self.cfg.settle_s as f64))
    }

    fn ai_draft(&mut self, meta: &Value) -> Result<()> {
        let fid = pstr(meta, "id");
        let e = self.store.m["docs"][&fid].clone();
        if e["version"].is_null() {
            self.upd(&fid, Self::version_modified(meta));
            self.act("baseline", &[("doc", json!(fid)), ("kind", json!("ai_draft"))]);
            return Ok(());
        }
        if same_version(meta, &e) {
            return Ok(());
        }
        if !self.settled(meta)? {
            self.store.m["pending"][&fid] = json!("settling");
            return Ok(());
        }
        let plain = norm(&self.drive.export(&fid, "text/plain")?);
        if json!(h(&plain)) != e["doc_hash"] && !self.dry {
            // his edit to a doc that holds AI drafts: stored as his edit, never as an idea or a his-words unit
            let rec = self.owner_edit(vec![
                ("doc_id", json!(fid)),
                ("kind", json!("ai_draft_doc_edit")),
                ("contains_drafts", json!(true)),
                ("title", meta.get("name").cloned().unwrap_or(Value::Null)),
                ("modified", meta.get("modifiedTime").cloned().unwrap_or(Value::Null)),
                ("text", json!(plain)),
            ]);
            self.store.log("edits.jsonl", &rec)?;
            self.act("ai_draft_edit", &[("doc", json!(fid))]);
        }
        if !self.dry {
            let mut up = Self::version_modified(meta);
            up.extend([("doc_hash", json!(h(&plain))), ("contains_drafts", json!(true))]);
            self.upd(&fid, up);
        }
        self.pending_pop(&fid);
        Ok(())
    }

    /// `^\s*([-*_])(\s*\1){2,}\s*$`: three or more of one rule character, spaces allowed between.
    fn is_rule(line: &str) -> bool {
        let mut chars = line.chars().filter(|c| !py::isspace(*c));
        let Some(first) = chars.next().filter(|c| matches!(c, '-' | '*' | '_')) else {
            return false;
        };
        let mut n = 1;
        for c in chars {
            if c != first {
                return false;
            }
            n += 1;
        }
        n >= 3
    }

    fn mixed(&mut self, meta: &Value) -> Result<()> {
        // A Punchable-made doc: his verbatim words above the first horizontal rule, AI notes (ai_draft) below.
        // Only the part above the rule is his text; gsync never writes this doc.
        let fid = pstr(meta, "id");
        let e = self.store.m["docs"][&fid].clone();
        let jid = sget(&e, "joke_id").filter(|j| !j.is_empty()).map(str::to_string);
        let cur = match &jid {
            Some(j) => self.pk.joke(j)?,
            None => None,
        };
        if e["version"].is_null() {
            let mut up = Self::version_modified(meta);
            up.push((
                "app_hash",
                cur.as_ref()
                    .map_or(Value::Null, |c| json!(h(sget(c, "summary").unwrap_or_default()))),
            ));
            self.upd(&fid, up);
            self.act(
                "baseline",
                &[("doc", json!(fid)), ("joke", json!(jid)), ("kind", json!("mixed"))],
            );
            return Ok(());
        }
        if same_version(meta, &e) {
            return Ok(());
        }
        if !self.settled(meta)? {
            self.store.m["pending"][&fid] = json!("settling");
            return Ok(());
        }
        let md = self
            .drive
            .export(&fid, "text/markdown")?
            .replace('\u{feff}', "")
            .replace("\r\n", "\n");
        if self.rules.content_is_legal(&md) {
            self.store.m["excluded"][&fid] = json!("content");
            self.store.m["docs"][&fid]["excluded"] = json!(true);
            self.act("skip_legal", &[("doc", json!(fid)), ("why", json!("content"))]);
            return Ok(());
        }
        let lines: Vec<&str> = md.split('\n').collect();
        let cut = lines.iter().position(|ln| Self::is_rule(ln));
        self.pending_pop(&fid);
        let Some(cut) = cut else {
            // the rule is gone: can't tell his words from the notes, so nothing is applied
            if !self.dry {
                let rec = self.owner_edit(vec![
                    ("doc_id", json!(fid)),
                    ("kind", json!("mixed_doc_edit_unsplit")),
                    ("contains_drafts", json!(true)),
                    ("text", json!(md)),
                ]);
                self.store.log("edits.jsonl", &rec)?;
                self.upd(&fid, Self::version_modified(meta));
            }
            self.act("mixed_unsplit", &[("doc", json!(fid))]);
            return Ok(());
        };
        let his = norm(&lines[..cut].join("\n"));
        let cur_summary = cur.as_ref().map(|c| sget(c, "summary").unwrap_or_default().to_string());
        if cur_summary.is_none()
            || json!(h(&his)) == json!(h(cur_summary.as_deref().unwrap_or_default()))
            || json!(h(&his)) == e["doc_hash"]
        {
            if !self.dry {
                let mut up = Self::version_modified(meta);
                up.push(("doc_hash", json!(h(&his))));
                self.upd(&fid, up);
            }
            return Ok(());
        }
        let cur = cur.expect("checked above");
        let cur_summary = cur_summary.unwrap_or_default();
        if tr(&e, "app_hash") && json!(h(&cur_summary)) != e["app_hash"] {
            self.conflict(&[
                ("kind", json!("both_edited")),
                ("doc_id", json!(fid)),
                ("joke_id", json!(jid)),
                ("title", cur["title"].clone()),
                ("app_text", json!(cur_summary)),
                ("doc_text", json!(his)),
                (
                    "resolution",
                    json!("the doc's text (above the line) was applied; the app text is kept here"),
                ),
            ])?;
        }
        let jid = jid.unwrap_or_default();
        if self.dry {
            self.act("edit", &[("doc", json!(fid)), ("joke", json!(jid))]);
            return Ok(());
        }
        let reason = format!(
            "{GDOCS} edited in Google Docs (above the line) ({fid}, modified {})",
            pstr(meta, "modifiedTime")
        );
        let r = self.pk.edit_joke(&jid, &json!({"summary": his}), &reason)?;
        let rec = self.owner_edit(vec![
            ("doc_id", json!(fid)),
            ("kind", json!("mixed_doc_edit")),
            ("contains_drafts", json!(true)),
            ("joke_id", json!(jid)),
            ("modified", meta.get("modifiedTime").cloned().unwrap_or(Value::Null)),
        ]);
        self.store.log("edits.jsonl", &rec)?;
        let mut up = Self::version_modified(meta);
        up.extend([
            ("doc_hash", json!(h(&his))),
            ("app_hash", json!(h(&his))),
            ("contains_drafts", json!(true)),
        ]);
        self.upd(&fid, up);
        self.save()?;
        self.act(
            "edit",
            &[
                ("doc", json!(fid)),
                ("joke", json!(jid)),
                ("indexes", r.map_or(Value::Null, |r| r["indexes"].clone())),
            ],
        );
        Ok(())
    }

    fn merged_into(&mut self, jid: &str) -> Result<Option<String>> {
        for m in self.pk.merges()?.values() {
            if m.get("kind") == Some(&json!("merge"))
                && m.get("status") == Some(&json!("merged"))
                && m.get("others")
                    .and_then(Value::as_array)
                    .is_some_and(|o| o.iter().any(|x| x == jid))
            {
                return Ok(m.get("main").map(py::str));
            }
        }
        Ok(None)
    }

    fn merged_doc(&mut self, meta: &Value) -> Result<()> {
        // A consolidated doc gsync wrote for a merge. Our own write is skipped by version; his later edits are kept
        // verbatim in edits.jsonl (by: owner) and never turned into an idea.
        let fid = pstr(meta, "id");
        let e = self.store.m["docs"][&fid].clone();
        if same_version(meta, &e) {
            return Ok(());
        }
        if !self.settled(meta)? {
            self.store.m["pending"][&fid] = json!("settling");
            return Ok(());
        }
        let plain = norm(&self.drive.export(&fid, "text/plain")?);
        if json!(h(&plain)) != e["doc_hash"] && !self.dry {
            let rec = self.owner_edit(vec![
                ("doc_id", json!(fid)),
                ("kind", json!("merged_doc_edit")),
                ("merge_id", e.get("merge_id").cloned().unwrap_or(Value::Null)),
                ("joke_id", e.get("merged_joke").cloned().unwrap_or(Value::Null)),
                ("title", meta.get("name").cloned().unwrap_or(Value::Null)),
                ("modified", meta.get("modifiedTime").cloned().unwrap_or(Value::Null)),
                ("text", json!(plain)),
            ]);
            self.store.log("edits.jsonl", &rec)?;
            self.act(
                "merged_doc_edit",
                &[
                    ("doc", json!(fid)),
                    ("joke", e.get("merged_joke").cloned().unwrap_or(Value::Null)),
                ],
            );
        }
        if !self.dry {
            let mut up = Self::version_modified(meta);
            up.push(("doc_hash", json!(h(&plain))));
            self.upd(&fid, up);
        }
        self.pending_pop(&fid);
        Ok(())
    }

    /// A doc left Drive (trashed or removed).
    pub fn gone(&mut self, fid: &str) -> Result<()> {
        let e = self.store.m["docs"].get(fid).filter(|v| py::truthy(Some(v))).cloned();
        let Some(e) = e else { return Ok(()) };
        if tr(&e, "hidden") {
            return Ok(());
        }
        if e["origin"] == "board" {
            // he trashed a generated board doc: respect it, don't regenerate
            if let Some(sb) = sget(&e, "board")
                && let Some(b) = self.store.m["boards"].get_mut(sb)
            {
                b["doc_trashed"] = json!(true);
            }
            self.store.m["docs"][fid]["hidden"] = json!(true);
            self.act("gone", &[("doc", json!(fid)), ("kind", json!("board"))]);
            return Ok(());
        }
        let now = self.now_iso();
        self.upd(fid, vec![("hidden", json!(true)), ("gone_at", json!(now))]);
        let jid = sget(&e, "joke_id").filter(|j| !j.is_empty()).map(str::to_string);
        let mut idx = Value::Null;
        if let Some(j) = &jid
            && matches!(sget(&e, "origin"), Some("gdocs" | "app"))
            && !self.dry
        {
            let cur = self.pk.joke(j)?;
            if cur.as_ref().is_some_and(|c| c["stage"] == "idea") {
                let r = self.pk.edit_joke(
                    j,
                    &json!({"stage": "retired"}),
                    &format!("{GDOCS} Google Doc moved to trash ({fid}); hidden"),
                )?;
                idx = r.map_or(Value::Null, |r| r["indexes"].clone());
            }
        }
        self.save()?;
        self.act("gone", &[("doc", json!(fid)), ("joke", json!(jid)), ("indexes", idx)]);
        Ok(())
    }

    // ---- push: app jokes -> docs --------------------------------------------------------------------------------
    fn push_jokes(&mut self) -> Result<()> {
        let mut by_joke: BTreeMap<String, String> = BTreeMap::new();
        if let Some(docs) = self.store.m["docs"].as_object() {
            for (did, e) in docs {
                if let Some(j) = sget(e, "joke_id").filter(|j| !j.is_empty()) {
                    by_joke.insert(j.to_string(), did.clone());
                }
            }
        }
        let parent = self.store.m["folder_id"].as_str().map(str::to_string);
        for jid in self.pk.app_jokes()? {
            let Some(cur) = self.pk.joke(&jid)? else { continue };
            if tr(&cur, "hidden") {
                continue;
            }
            let title = cur["title"].clone();
            let text = match cur.get("summary").filter(|v| py::truthy(Some(v))) {
                Some(s) => py::str(s),
                None => py::str(&title),
            };
            let Some(did) = by_joke.get(&jid).cloned() else {
                if self.dry {
                    self.act("push_new", &[("joke", json!(jid)), ("title", title)]);
                    continue;
                }
                let found = self.drive.find_by_prop("punchable_joke", &jid)?;
                let meta = match found {
                    Some(m) => m,
                    None => self.drive.create_doc(
                        &py::str(&title),
                        &text,
                        parent.as_deref(),
                        &json!({"punchable_kind": "joke", "punchable_joke": jid}),
                    )?,
                };
                let mid = pstr(&meta, "id");
                self.drive.allow(&mid);
                let doc_hash = h(&self.drive.export(&mid, "text/plain")?);
                self.store.m["docs"][&mid] = json!({"origin": "app", "joke_id": jid, "title": title,
                                                    "version": meta.get("version"), "modified": meta.get("modifiedTime"),
                                                    "doc_hash": doc_hash, "app_hash": h(&text)});
                self.save()?;
                self.act(
                    "push_new",
                    &[("joke", json!(jid)), ("doc", json!(mid)), ("title", title)],
                );
                continue;
            };
            let e = self.store.m["docs"][&did].clone();
            if e["version"].is_null() {
                continue; // adopted, not seen in Drive yet: baseline first, never a second doc
            }
            if tr(&e, "hidden") || tr(&e, "excluded") || tr(&e, "detached") || json!(h(&text)) == e["app_hash"] {
                continue;
            }
            if e["origin"] != "app" {
                self.own_doc(&did, &cur, &text)?;
                continue;
            }
            let meta = self.drive.get(&did)?;
            let Some(meta) = meta.filter(|m| !tr(m, "trashed")) else {
                continue;
            };
            if !same_version(&meta, &e) {
                let key = json!([h(&text), meta.get("version").cloned().unwrap_or(Value::Null)]);
                if e.get("conflict_key") != Some(&key) {
                    self.conflict(&[
                        ("kind", json!("doc_changed_since_sync")),
                        ("doc_id", json!(did)),
                        ("joke_id", json!(jid)),
                        ("title", title.clone()),
                        ("app_text", json!(text)),
                        (
                            "resolution",
                            json!("the doc was not overwritten; its text is pulled next run"),
                        ),
                    ])?;
                    self.store.m["docs"][&did]["conflict_key"] = key;
                }
                continue;
            }
            if self.dry {
                self.act("push_edit", &[("joke", json!(jid)), ("doc", json!(did))]);
                continue;
            }
            let new_name = if title == e["title"] {
                None
            } else {
                Some(py::str(&title))
            };
            let new = self.drive.update_doc(&did, &text, new_name.as_deref())?;
            let doc_hash = h(&self.drive.export(&did, "text/plain")?);
            self.upd(
                &did,
                vec![
                    ("version", new.get("version").cloned().unwrap_or(Value::Null)),
                    ("modified", new.get("modifiedTime").cloned().unwrap_or(Value::Null)),
                    ("app_hash", json!(h(&text))),
                    ("title", title),
                    ("doc_hash", json!(doc_hash)),
                ],
            );
            self.save()?;
            self.act("push_edit", &[("joke", json!(jid)), ("doc", json!(did))]);
        }
        // ideas that came from his own docs and were then edited in the app: noted, his doc is never written (v1)
        let entries: Vec<(String, Value)> = self.store.m["docs"]
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        for (did, e) in entries {
            // the entry as it is now (an earlier pass of this loop may have changed it)
            let e = self.store.m["docs"].get(&did).cloned().unwrap_or(e);
            if e["origin"] == "gdocs"
                && tr(&e, "joke_id")
                && !(tr(&e, "hidden") || tr(&e, "excluded") || tr(&e, "detached"))
            {
                let cur = self.pk.joke(sget(&e, "joke_id").unwrap_or_default())?;
                if let Some(cur) = cur {
                    let s = sget(&cur, "summary").unwrap_or_default().to_string();
                    if json!(h(&s)) != e["app_hash"] {
                        self.own_doc(&did, &cur, &s)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn own_doc(&mut self, did: &str, cur: &Value, text: &str) -> Result<()> {
        if self.store.m["docs"][did]["noted_app_hash"] == json!(h(text)) {
            return Ok(());
        }
        if self.store.m["docs"][did]["origin"] == "mixed" {
            self.conflict(&[
                ("kind", json!("mixed_doc_not_written")),
                ("doc_id", json!(did)),
                ("joke_id", cur["id"].clone()),
                ("title", cur["title"].clone()),
                ("app_text", json!(text)),
                ("resolution", json!("edited in the app; this doc mixes your words with Punchable notes, so it is never rewritten; copy the change into the doc above the line")),
            ])?;
        } else {
            self.conflict(&[
                ("kind", json!("own_doc_not_written")),
                ("doc_id", json!(did)),
                ("joke_id", cur["id"].clone()),
                ("title", cur["title"].clone()),
                ("app_text", json!(text)),
                (
                    "resolution",
                    json!("edited in the app; your own Google Doc is never written in v1"),
                ),
            ])?;
        }
        self.store.m["docs"][did]["noted_app_hash"] = json!(h(text));
        Ok(())
    }

    // ---- push: storyboards -> board docs ------------------------------------------------------------------------
    fn board_text(&mut self, b: &Value) -> Result<String> {
        let his: BTreeSet<String> = self.pk.app_jokes()?.into_iter().collect();
        let mut doc_of: BTreeMap<String, String> = BTreeMap::new();
        if let Some(docs) = self.store.m["docs"].as_object() {
            for (did, e) in docs {
                if tr(e, "joke_id") && !tr(e, "hidden") {
                    doc_of.insert(pstr(e, "joke_id"), did.clone());
                }
            }
        }
        let mut lines: Vec<String> = vec![
            format!("{} (board)", pstr(b, "title")),
            String::new(),
            "GENERATED BY PUNCHABLE from your storyboard. This doc is rewritten whenever the board changes, so edit the board in the app, not here. Every bit's text below is yours, verbatim; nothing here is AI-written.".into(),
            format!("Board last changed: {}", pstr(b, "updated_at")),
        ];
        if tr(b, "target_s") {
            let secs = b["target_s"]
                .as_f64()
                .or_else(|| b["target_s"].as_str().and_then(|s| s.trim().parse::<f64>().ok()))
                .unwrap_or(0.0) as i64;
            let n = b.get("joke_ids").and_then(Value::as_array).map_or(0, Vec::len);
            lines.push(format!("Target: {}:{:02} · {n} bits", secs / 60, secs % 60));
        }
        if tr(b, "notes") && sget(b, "by") == Some(self.cfg.owner_id.as_str()) {
            lines.extend([String::new(), "Your notes:".into(), pstr(b, "notes")]);
        }
        let ids: Vec<String> = b
            .get("joke_ids")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(py::str).collect())
            .unwrap_or_default();
        for (n, jid) in ids.iter().enumerate() {
            let j = self.pk.joke(jid)?;
            let (title, summary, lead) = match &j {
                Some(j) => (j["title"].clone(), j["summary"].clone(), j["lead_text"].clone()),
                None => (json!(jid), json!(""), Value::Null),
            };
            let body = if py::truthy(Some(&lead)) {
                Some(py::str(&lead))
            } else if his.contains(jid) || doc_of.contains_key(jid) {
                py::truthy(Some(&summary)).then(|| py::str(&summary))
            } else {
                None
            };
            let shown = if py::truthy(Some(&title)) {
                py::str(&title)
            } else {
                jid.clone()
            };
            lines.extend([String::new(), format!("{}. {shown}", n + 1)]);
            if let Some(b) = body {
                lines.push(b);
            }
            if let Some(d) = doc_of.get(jid) {
                lines.push(format!("Doc: https://docs.google.com/document/d/{d}/edit"));
            }
        }
        Ok(lines.join("\n") + "\n")
    }

    fn push_boards(&mut self) -> Result<()> {
        let parent = self.store.m["folder_id"].as_str().map(str::to_string);
        let mut boards: Vec<(String, Value)> = self.pk.boards()?.into_iter().collect();
        boards.sort_by(|a, b| a.0.cmp(&b.0));
        for (sb, b) in boards {
            if !(sget(&b, "by") == Some(self.cfg.owner_id.as_str()) || sget(&b, "source") == Some("his")) {
                continue; // untouched AI seed boards are drafts: never written to his Drive
            }
            let be = self.store.m["boards"].get(&sb).filter(|v| py::truthy(Some(v))).cloned();
            if tr(&b, "deleted") {
                if let Some(be) = &be
                    && tr(be, "doc_id")
                    && !tr(be, "trashed")
                    && !tr(be, "doc_trashed")
                    && !self.dry
                {
                    let doc = pstr(be, "doc_id");
                    self.drive.trash(&doc)?;
                    self.store.m["boards"][&sb]["trashed"] = json!(true);
                    self.act("board_trash", &[("board", json!(sb)), ("doc", json!(doc))]);
                }
                continue;
            }
            if be.as_ref().is_some_and(|be| tr(be, "doc_trashed")) {
                continue;
            }
            let text = self.board_text(&b)?;
            if be.as_ref().is_some_and(|be| be["hash"] == json!(h(&text))) {
                continue;
            }
            if self.dry {
                let doc = be
                    .as_ref()
                    .map_or(Value::Null, |be| be.get("doc_id").cloned().unwrap_or(Value::Null));
                self.act("board", &[("board", json!(sb)), ("doc", doc)]);
                continue;
            }
            let name = format!("{} (board)", pstr(&b, "title"));
            let mut new: Option<Value> = None;
            let mut be = be;
            if let Some(cur_be) = be.clone().filter(|be| tr(be, "doc_id")) {
                let doc_id = pstr(&cur_be, "doc_id");
                let meta = self.drive.get(&doc_id)?;
                let Some(meta) = meta.filter(|m| !tr(m, "trashed")) else {
                    self.store.m["boards"][&sb]["doc_trashed"] = json!(true);
                    self.act("gone", &[("doc", json!(doc_id)), ("kind", json!("board"))]);
                    continue;
                };
                if cur_be["version"].is_null() || pstr(&meta, "version") != py::str(&cur_be["version"]) {
                    let prior = norm(&self.drive.export(&doc_id, "text/plain")?);
                    if json!(h(&prior)) != cur_be["doc_hash"] {
                        let by = if tr(&cur_be, "version") {
                            self.cfg.owner_id.clone()
                        } else {
                            "adopted".into()
                        };
                        let mut rec = self.owner_edit(vec![
                            ("doc_id", json!(doc_id)),
                            ("kind", json!("board_doc_before_regen")),
                            ("board", json!(sb)),
                            ("text", json!(prior)),
                        ]);
                        rec["by"] = json!(by);
                        self.store.log("edits.jsonl", &rec)?;
                    }
                }
                match self.drive.update_doc(&doc_id, &text, Some(&name)) {
                    Ok(n) => new = Some(n),
                    Err(e @ (GsError::WriteBlocked(_) | GsError::Exit(_))) => return Err(e),
                    Err(ex) => {
                        // adopted doc not writable (narrow scopes): make our own
                        if !tr(&cur_be, "adopted") {
                            return Err(ex);
                        }
                        self.error(json!({"board": sb, "error": format!("adopted board doc not writable: {ex}")}));
                    }
                }
            }
            let new = if let Some(n) = new {
                n
            } else {
                let n = self.drive.create_doc(
                    &name,
                    &text,
                    parent.as_deref(),
                    &json!({"punchable_kind": "board", "punchable_board": sb}),
                )?;
                be = Some(json!({"doc_id": n["id"]}));
                self.store.m["docs"][&pstr(&n, "id")] = json!({"origin": "board", "board": sb});
                n
            };
            let mut be = be.expect("a board entry exists by now");
            let doc_id = pstr(&be, "doc_id");
            let doc_hash = h(&self.drive.export(&doc_id, "text/plain")?);
            be["hash"] = json!(h(&text));
            be["version"] = new.get("version").cloned().unwrap_or(Value::Null);
            be["doc_hash"] = json!(doc_hash);
            self.store.m["boards"][&sb] = be;
            self.save()?;
            self.act("board", &[("board", json!(sb)), ("doc", json!(doc_id))]);
        }
        Ok(())
    }
}
