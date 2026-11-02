//! `memolink` (`worker/online.py` `memolink_job`): voice memos linked to his bits. The steps, each optional (all
//! four by default): `embed` (vectors for new summary-only ideas, then the notebook read again with them), `judge`
//! (the merge judge on the candidate pairs that involve a memo idea, then the second look on borderline answers;
//! cached in `kb/creative/merge_judge.json`), `triage` (each memo sorted once: material, a performed set, a
//! conversation, a note; cached in `kb/creative/suggest_judge.json`), `attach` (the clear ones posted as `ai:attach`
//! events, undoable); and the count of same-idea groups that include a memo. A model failure leaves its pairs or
//! memos unjudged, never guessed. Court text never reaches a model.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use pk_creative::corpus::Corpus;
use pk_creative::memolink::{
    self as ml, JUDGE_FILE, SUMMARY_FILE, TRIAGE_FILE, TRIAGE_SCHEMA, TRIAGE_SYSTEM, attach_event, clear_attaches,
    memo_jokes, memos, suggestions,
};
use pk_creative::merge::{
    Index, JUDGE_BATCH, JUDGE_SCHEMA, JUDGE_SYSTEM, MergeState, SECOND_BATCH, judge_candidates, judge_finish,
    judge_user, second_finish, second_todo, second_user,
};
use pk_creative::{Store, for_store};
use pk_llm::ChatMessage;
use pk_wire::worker::MemolinkInput;
use serde_json::{Map, Value, json};

use super::embed::embed_summaries;
use super::{Ctx, Done, OutFile, Runner};
use crate::error::JobError;
use crate::pyjson::dumps_indent1;

/// The `memolink` kind.
#[derive(Debug)]
pub struct Memolink;

/// One model call for a step; a failure is logged and leaves its items unanswered (the Python's `continue`).
async fn ask(ctx: &Ctx<'_>, task: &str, system: &str, user: String, schema: &Value) -> Option<(Value, String)> {
    let msgs = [ChatMessage::system(system), ChatMessage::user(user)];
    match ctx
        .router
        .complete(task, &msgs, Some(schema), Some("creative"), None)
        .await
    {
        Ok(r) => Some((r.data.unwrap_or(Value::Null), r.model)),
        Err(e) => {
            tracing::warn!(task, error = %crate::error::llm_type(&e), "a batch went unanswered");
            None
        }
    }
}

fn read_doc(ctx: &Ctx<'_>, path: &str) -> Option<Value> {
    ctx.files
        .and_then(|f| f.get(path))
        .and_then(|t| serde_json::from_str(t).ok())
}

impl Runner for Memolink {
    type Input = MemolinkInput;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad memolink input: {e}"))
    }

    #[allow(clippy::too_many_lines)]
    async fn run(ctx: Ctx<'_>, input: MemolinkInput) -> Result<Done, JobError> {
        let steps: Vec<String> = input
            .steps
            .unwrap_or_else(|| ["embed", "judge", "triage", "attach"].map(str::to_string).to_vec());
        let has = |s: &str| steps.iter().any(|x| x == s);
        let mut corpus: Arc<Corpus> = Arc::clone(ctx.corpus()?);
        let mut out = Map::new();
        out.insert("status".into(), json!("linked"));
        let mut files: Vec<OutFile> = Vec::new();
        if has("embed") {
            let (counts, file) = embed_summaries(&ctx, &corpus).await?;
            out.insert("embed".into(), Value::Object(counts));
            if let Some(f) = file {
                // the notebook read again with the new vectors (the Python reloads its Corpus from the folder)
                let mut texts: Vec<(String, Arc<str>)> = ctx
                    .files
                    .into_iter()
                    .flatten()
                    .filter(|(p, _)| p.starts_with("vectors/") && p.as_str() != SUMMARY_FILE)
                    .map(|(p, t)| (p.clone(), Arc::clone(t)))
                    .collect();
                texts.push((SUMMARY_FILE.to_string(), Arc::from(f.text.as_str())));
                let base = Arc::clone(&corpus);
                corpus = tokio::task::spawn_blocking(move || {
                    let mut c = (*base).clone();
                    c.vectors = Arc::new(crate::mirror::build_vectors(&texts));
                    Arc::new(c)
                })
                .await
                .map_err(|e| JobError::Value(format!("reloading the notebook: {e}")))?;
                files.push(f);
            }
        }
        let ms = memos(ctx.files.unwrap_or(&BTreeMap::new()));
        let mj = memo_jokes(&corpus, &ms);
        let c = Arc::clone(&corpus);
        // the index compares every unit pair: CPU work, built off the async threads once and owned by this job
        let (ix, all_candidates) = tokio::task::spawn_blocking(move || {
            let ix = Index::new(c);
            let fs = judge_candidates(&ix);
            (ix, fs)
        })
        .await
        .map_err(|e| JobError::Value(format!("the same-idea index: {e}")))?;

        let mut judgements: Map<String, Value> = read_doc(&ctx, JUDGE_FILE)
            .and_then(|d| d.get("pairs").and_then(Value::as_object).cloned())
            .unwrap_or_default();
        if has("judge") {
            let before = judgements.len();
            let fs: Vec<Value> = all_candidates
                .into_iter()
                .filter(|f| {
                    f["a"].as_str().is_some_and(|a| mj.contains_key(a))
                        || f["b"].as_str().is_some_and(|b| mj.contains_key(b))
                })
                .collect();
            for outer in fs.chunks(JUDGE_BATCH) {
                let todo: Vec<Value> = outer
                    .iter()
                    .filter(|f| !judgements.contains_key(&ix.judge_key(f)))
                    .cloned()
                    .collect();
                for batch in todo.chunks(JUDGE_BATCH) {
                    if let Some((data, model)) =
                        ask(&ctx, "merge_judge", JUDGE_SYSTEM, judge_user(&ix, batch), &JUDGE_SCHEMA).await
                    {
                        judge_finish(&ix, batch, Some(&data), &model, &mut judgements);
                    }
                }
            }
            let todo = second_todo(&corpus, &judgements);
            for batch in todo.chunks(SECOND_BATCH) {
                let user = second_user(&ix, &judgements, batch);
                if let Some((data, _)) = ask(&ctx, "merge_judge", JUDGE_SYSTEM, user, &JUDGE_SCHEMA).await {
                    second_finish(batch, Some(&data), &mut judgements);
                }
            }
            let doc = json!({"version": 1, "task": "merge_judge", "pairs": judgements});
            files.push(OutFile {
                path: JUDGE_FILE.to_string(),
                text: dumps_indent1(&doc),
            });
            out.insert(
                "judge".into(),
                json!({"memo_pairs": fs.len(), "newly_judged": judgements.len() - before, "cache": judgements.len()}),
            );
        }

        let mut triage_doc: Value = read_doc(&ctx, TRIAGE_FILE)
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({"version": 1, "task": "memo_triage", "items": {}}));
        if !triage_doc.get("items").is_some_and(Value::is_object) {
            triage_doc["items"] = json!({});
        }
        if has("triage") {
            let mut cache = triage_doc["items"].as_object().cloned().unwrap_or_default();
            let (batches, asked, skipped) = ml::triage_batches(&ms, &cache, |t| ctx.legal.is_legal(t));
            let (mut sorted, mut wrote) = (0, false);
            for b in &batches {
                if let Some((data, model)) = ask(
                    &ctx,
                    "memo_triage",
                    TRIAGE_SYSTEM,
                    ml::triage_user(&ms, b),
                    &TRIAGE_SCHEMA,
                )
                .await
                {
                    sorted += ml::triage_finish(&ms, b, Some(&data), &model, &mut cache);
                    // the Python writes the cache after an answered batch only (a failed one `continue`s past it)
                    triage_doc["items"] = Value::Object(cache.clone());
                    wrote = true;
                }
            }
            if wrote {
                files.push(OutFile {
                    path: TRIAGE_FILE.to_string(),
                    text: dumps_indent1(&triage_doc),
                });
            }
            out.insert(
                "triage".into(),
                json!({"asked": asked, "sorted": sorted, "legal_skipped": skipped, "cached": cache.len()}),
            );
        }

        let lines = ctx.creative.and_then(|c| c.get("merges")).cloned().unwrap_or_default();
        let items: Map<String, Value> = for_store("merges", ctx.owner)
            .and_then(|r| Store::replay(r, lines.iter()).ok())
            .map(|s| s.items().clone())
            .unwrap_or_default();
        let st = MergeState::from_items(&items);
        let recordings: HashSet<String> = ml::set_recordings(Some(&triage_doc));
        if has("attach") {
            let today = pk_core::time::today();
            let mut done = Vec::new();
            for x in clear_attaches(&ix, &corpus, &judgements, &st, &recordings, &mj) {
                let (aid, data) = attach_event(&x, &today);
                let body = json!({"store": "merges", "op": "attach", "id": aid, "data": data, "actor": "ai:attach"});
                let r = ctx
                    .api
                    .call(reqwest::Method::POST, "/api/worker/events", Some(&body))
                    .await?;
                if !r.as_ref().is_some_and(|r| pk_core::py::truthy(r.get("already"))) {
                    done.push(json!(aid));
                }
            }
            out.insert("attached".into(), Value::Array(done));
        }
        out.insert(
            "same_idea_groups".into(),
            json!(suggestions(&ix, &judgements, &st, &recordings, &mj).len()),
        );

        // only what changed is posted (the Python's push compares each file's hash)
        let unchanged = |f: &OutFile| {
            ctx.files
                .and_then(|m| m.get(&f.path))
                .is_some_and(|t| t.as_ref() == f.text)
        };
        files.retain(|f| !unchanged(f));
        let mut done = Done::result(Value::Object(out));
        done.files = files;
        Ok(done)
    }
}
