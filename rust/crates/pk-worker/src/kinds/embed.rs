//! `embed` (`creative/memolink.py` `embed_summaries`): a bge-small vector for every idea that has no units yet (a
//! voice memo, an app jot, a Google Doc), keyed `summary:<joke>` with the text's hash so an edited idea is embedded
//! again, written to `vectors/extra_summary.json` and posted (court text is never embedded). The same-idea matcher
//! then finds new notes by meaning.

use std::sync::Arc;

use pk_creative::corpus::Corpus;
use pk_creative::memolink::{SUMMARY_FILE, summary_doc, summary_only, to_embed, update_summary_vectors};
use pk_wire::worker::EmbedInput;
use serde_json::{Map, Value, json};

use super::{Ctx, Done, OutFile, Runner};
use crate::error::JobError;
use crate::pyjson::dumps_compact_ascii;

/// The `embed` kind.
#[derive(Debug)]
pub struct Embed;

impl Runner for Embed {
    type Input = EmbedInput;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad embed input: {e}"))
    }

    async fn run(ctx: Ctx<'_>, _input: EmbedInput) -> Result<Done, JobError> {
        let c = Arc::clone(ctx.corpus()?);
        let (mut result, file) = embed_summaries(&ctx, &c).await?;
        result.insert("status".into(), json!("embedded"));
        let mut done = Done::result(Value::Object(result));
        done.files.extend(file);
        Ok(done)
    }
}

/// `embed_summaries(c, vec_dir)`: the counts (`summary_only`, `embedded`, `removed`, `have`, `file`) and the
/// rewritten vector file when anything changed.
pub async fn embed_summaries(ctx: &Ctx<'_>, c: &Corpus) -> Result<(Map<String, Value>, Option<OutFile>), JobError> {
    let want = summary_only(c);
    let existing = ctx.files.and_then(|f| f.get(SUMMARY_FILE)).map(ToString::to_string);
    let mut doc = summary_doc(existing.as_deref());
    let todo = to_embed(&doc, &want, |t| ctx.legal.is_legal(t));
    let mut done_rows = Vec::with_capacity(todo.len());
    if !todo.is_empty() {
        let embedder = Arc::clone(
            &ctx.embedder
                .ok_or_else(|| {
                    JobError::Value("no bge-small model in this worker: embed runs only where the model is".into())
                })?
                .0,
        );
        let texts: Vec<String> = todo.iter().map(|(_, t)| t.clone()).collect();
        // the model is CPU work: off the async threads
        let vecs = tokio::task::spawn_blocking(move || {
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            embedder.embed(&refs, false)
        })
        .await
        .map_err(|e| JobError::Value(format!("embedding: {e}")))??;
        done_rows.extend(todo.iter().cloned().zip(vecs).map(|((j, t), v)| (j, t, v)));
    }
    let removed = update_summary_vectors(&mut doc, &want, &done_rows);
    let have = doc.get("rows").and_then(Value::as_object).map_or(0, Map::len);
    let file = (!done_rows.is_empty() || removed > 0).then(|| OutFile {
        path: SUMMARY_FILE.to_string(),
        text: dumps_compact_ascii(&Value::Object(doc)),
    });
    let counts = json!({"summary_only": want.len(), "embedded": done_rows.len(), "removed": removed, "have": have,
                        "file": SUMMARY_FILE});
    Ok((counts.as_object().cloned().unwrap_or_default(), file))
}
