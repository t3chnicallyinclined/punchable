//! The Google Docs side of a merge (`gsync/merge.py`): for every merge the comic made in the app, once, ONE
//! consolidated Google Doc in the Punchable folder (the main joke's title, then every original's words verbatim, each
//! under its date and source doc), and the source docs MOVED into "Merged (archive)". Moving changes a doc's folder
//! only: its text and name are never edited, nothing is trashed or deleted. A legal doc is never touched. An undone
//! merge moves each doc back and releases the consolidated doc's claim; it is never trashed.
//!
//! State lives in `map.json` `merges` {`merge_id`: {state, `doc_id`, moved:{doc:{from:[parents], pending?}}, left, ...}}.
//! Every move is recorded (pending) before it is made, so a crash between the two can always be undone.

use std::collections::BTreeSet;

use pk_core::{py, text::head};
use serde_json::{Map, Value, json};

use crate::error::{GsError, Result};
use crate::sync::Sync;
use crate::units::h;

const ARCHIVE_NAME: &str = "Merged (archive)";
const NOT_MOVED_ORIGINS: [&str; 5] = ["board", "ai_draft", "mixed", "merged", "merged_undone"];

fn sget<'v>(v: &'v Value, k: &str) -> Option<&'v str> {
    v.get(k).and_then(Value::as_str)
}

fn pstr(v: &Value, k: &str) -> String {
    py::str(v.get(k).unwrap_or(&Value::Null))
}

fn list_of(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(Value::as_array)
        .map(|a| a.iter().map(py::str).collect())
        .unwrap_or_default()
}

/// The consolidated doc: his words only, verbatim, dated. Catalog summaries (AI-written) are never written.
pub fn merge_text(rec: &Value) -> String {
    let snap = rec
        .get("snapshot")
        .filter(|v| py::truthy(Some(v)))
        .cloned()
        .unwrap_or(json!({}));
    let main = pstr(rec, "main");
    let mut order = vec![main.clone()];
    order.extend(list_of(rec, "others").into_iter().filter(|j| *j != main));
    let first = snap
        .get(&main)
        .filter(|v| py::truthy(Some(v)))
        .cloned()
        .unwrap_or(json!({}));
    let title = first
        .get("title")
        .filter(|v| py::truthy(Some(v)))
        .map_or_else(|| main.clone(), py::str);
    let created = rec.get("created_at").and_then(Value::as_str).unwrap_or_default();
    let mut lines: Vec<String> = vec![
        title,
        String::new(),
        format!(
            "MERGED BY PUNCHABLE on {} from {} versions of this bit. Every line below is yours, verbatim, under its date and the doc it came from. The original docs are in the \"{ARCHIVE_NAME}\" folder, unchanged.",
            head(created, 10),
            order.len()
        ),
    ];
    for (n, j) in order.iter().enumerate() {
        let s = snap
            .get(j)
            .filter(|v| py::truthy(Some(v)))
            .cloned()
            .unwrap_or(json!({}));
        let shown = s
            .get("title")
            .filter(|v| py::truthy(Some(v)))
            .map_or_else(|| j.clone(), py::str);
        let head_line = format!("{}{shown}", if n == 0 { "MAIN: " } else { "" });
        lines.extend([
            String::new(),
            format!("{} {head_line} {}", "=".repeat(8), "=".repeat(8)),
        ]);
        let mut wrote = false;
        for u in s.get("units").and_then(Value::as_array).into_iter().flatten() {
            let src = match u.get("doc_title").filter(|v| py::truthy(Some(v))) {
                Some(t) => format!(" \u{b7} from \"{}\"", py::str(t)),
                None => String::new(),
            };
            let date = u
                .get("date")
                .filter(|v| py::truthy(Some(v)))
                .map_or_else(|| "undated".to_string(), py::str);
            let text = u
                .get("text")
                .filter(|v| py::truthy(Some(v)))
                .map(py::str)
                .unwrap_or_default();
            lines.extend([String::new(), format!("[{date}{src}]"), text]);
            wrote = true;
        }
        if py::truthy(s.get("summary_is_his")) && py::truthy(s.get("summary")) {
            let when = ["added_at", "first_seen"]
                .into_iter()
                .find_map(|k| s.get(k).filter(|v| py::truthy(Some(v))).map(py::str))
                .unwrap_or_else(|| "undated".into());
            lines.extend([
                String::new(),
                format!("[{when} \u{b7} typed in Punchable]"),
                pstr(&s, "summary"),
            ]);
            wrote = true;
        }
        if !wrote {
            lines.extend([
                String::new(),
                "(no written text of yours: this one is known from tape only)".into(),
            ]);
        }
    }
    lines.join("\n") + "\n"
}

fn source_docs(sync: &Sync<'_>, rec: &Value) -> Vec<String> {
    let snap = rec
        .get("snapshot")
        .filter(|v| py::truthy(Some(v)))
        .cloned()
        .unwrap_or(json!({}));
    let main = pstr(rec, "main");
    let mut all = vec![main.clone()];
    all.extend(list_of(rec, "others"));
    let group: BTreeSet<&String> = all.iter().collect();
    let mut out: Vec<String> = Vec::new();
    for j in &all {
        for u in snap
            .get(j)
            .and_then(|s| s.get("units"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if py::truthy(u.get("doc_id")) {
                out.push(py::str(&u["doc_id"]));
            }
        }
    }
    if let Some(docs) = sync.store.m["docs"].as_object() {
        for (did, e) in docs {
            let in_group = e
                .get("joke_id")
                .and_then(Value::as_str)
                .is_some_and(|j| group.iter().any(|g| g.as_str() == j));
            let origin = sget(e, "origin").unwrap_or_default();
            if in_group && !NOT_MOVED_ORIGINS.contains(&origin) {
                out.push(did.clone());
            }
        }
    }
    let mut seen = BTreeSet::new();
    out.retain(|d| seen.insert(d.clone())); // dict.fromkeys: first occurrence wins, order kept
    out
}

/// Merges the comic made in the app -> one consolidated doc + the old docs moved to the archive.
pub fn push_merges(s: &mut Sync<'_>) -> Result<()> {
    if !s.store.m["merges"].is_object() {
        s.store.m["merges"] = json!({});
    }
    let mut merges: Vec<(String, Value)> = s.pk.merges()?.into_iter().collect();
    merges.sort_by(|a, b| a.0.cmp(&b.0));
    for (mid, rec) in merges {
        if rec.get("kind") != Some(&json!("merge")) {
            continue;
        }
        let st = s.store.m["merges"].get(&mid).filter(|v| py::truthy(Some(v))).cloned();
        let status = sget(&rec, "status");
        let state = st.as_ref().and_then(|st| sget(st, "state"));
        let outcome = if status == Some("merged") && (st.is_none() || matches!(state, Some("applying" | "error"))) {
            apply(s, &mid, &rec)
        } else if status == Some("undone")
            && st.is_some()
            && matches!(state, Some("applied" | "applying" | "undoing" | "error"))
        {
            undo(s, &mid)
        } else {
            if status == Some("undone") && st.is_none() && !s.dry {
                s.store.m["merges"][&mid] =
                    json!({"state": "undone_before_applied", "undone_at": s.svc.clock.now_iso()});
            }
            Ok(())
        };
        match outcome {
            Ok(()) => {}
            Err(e @ (GsError::WriteBlocked(_) | GsError::Exit(_))) => return Err(e),
            Err(e) => {
                // one bad merge never stops the run; it is retried next run
                if s.store.m["merges"].get(&mid).is_some() && !s.dry {
                    s.store.m["merges"][&mid]["error"] = json!(head(&e.describe(), 300));
                }
                s.error(json!({"merge": mid, "error": e.describe()}));
            }
        }
        s.save()?;
    }
    Ok(())
}

fn archive(s: &mut Sync<'_>) -> Result<String> {
    let fid = s.store.m["archive_folder_id"]
        .as_str()
        .filter(|f| !f.is_empty())
        .map(str::to_string);
    if let Some(fid) = fid {
        let meta = s.drive.get(&fid)?;
        if meta.is_some_and(|m| !py::truthy(m.get("trashed"))) {
            return Ok(fid);
        }
    }
    let found = s.drive.find_by_prop("punchable_kind", "merge_archive")?; // crash guard: made but not recorded
    let folder = s.store.m["folder_id"].as_str().map(str::to_string);
    let meta = match found {
        Some(m) => m,
        None => s.drive.create_folder(
            ARCHIVE_NAME,
            folder.as_deref(),
            &json!({"punchable_kind": "merge_archive"}),
        )?,
    };
    let id = pstr(&meta, "id");
    s.store.m["archive_folder_id"] = json!(id);
    let parents = match meta.get("parents").filter(|v| py::truthy(Some(v))) {
        Some(p) => p.clone(),
        None => json!([folder]),
    };
    s.store.m["folders"][&id] = parents;
    s.save()?;
    s.act("merge_archive_folder", &[("doc", json!(id))]);
    Ok(id)
}

fn apply(s: &mut Sync<'_>, mid: &str, rec: &Value) -> Result<()> {
    let docs = source_docs(s, rec);
    if s.dry {
        s.act("merge_apply", &[("merge", json!(mid)), ("docs", json!(docs.len()))]);
        return Ok(());
    }
    if s.store.m["merges"].get(mid).is_none() {
        s.store.m["merges"][mid] = json!({"state": "applying", "moved": {}, "left": {}, "skipped_legal": []});
    }
    let main = pstr(rec, "main");
    {
        let st = &mut s.store.m["merges"][mid];
        st["state"] = json!("applying");
        st["main"] = json!(main);
        st["error"] = Value::Null;
    }
    let arch = archive(s)?;
    s.store.m["merges"][mid]["archive_folder"] = json!(arch);
    // 1. the one consolidated doc (found again by its tag after a crash, never made twice)
    if !py::truthy(s.store.m["merges"][mid].get("doc_id")) {
        let text = merge_text(rec);
        let snap_title = rec
            .get("snapshot")
            .and_then(|sn| sn.get(&main))
            .and_then(|m| m.get("title"))
            .filter(|v| py::truthy(Some(v)))
            .map_or_else(|| main.clone(), py::str);
        let found = s.drive.find_by_prop("punchable_merge", mid)?;
        let folder = s.store.m["folder_id"].as_str().map(str::to_string);
        let meta = match found {
            Some(m) => m,
            None => s.drive.create_doc(
                &format!("{snap_title} (merged)"),
                &text,
                folder.as_deref(),
                &json!({"punchable_kind": "merged", "punchable_merge": mid, "punchable_joke": main}),
            )?,
        };
        let id = pstr(&meta, "id");
        s.drive.allow(&id);
        {
            let st = &mut s.store.m["merges"][mid];
            st["doc_id"] = json!(id);
            st["doc_url"] = json!(format!("https://docs.google.com/document/d/{id}/edit"));
        }
        s.store.m["docs"][&id] = json!({"origin": "merged", "merge_id": mid, "merged_joke": main,
                                        "title": meta.get("name"), "version": meta.get("version"),
                                        "modified": meta.get("modifiedTime"), "doc_hash": h(&text)});
        s.save()?;
        s.act("merge_doc", &[("merge", json!(mid)), ("doc", json!(id))]);
    }
    // 2. move every source doc into the archive (recorded before each move)
    let mut group: BTreeSet<String> = list_of(rec, "others").into_iter().collect();
    group.insert(main.clone());
    let doc_jokes = s.pk.doc_jokes()?;
    for did in docs {
        let moved = &s.store.m["merges"][mid]["moved"];
        if moved.get(&did).is_some() && !py::truthy(moved[&did].get("pending")) {
            continue;
        }
        let meta = s.drive.get(&did)?;
        let Some(meta) = meta.filter(|m| !py::truthy(m.get("trashed"))) else {
            s.store.m["merges"][mid]["left"][&did] = json!("gone from Drive");
            continue;
        };
        let anc = s.ancestors(&meta)?;
        let excluded = s.store.m["excluded"].as_object().cloned().unwrap_or_else(Map::new);
        if excluded.contains_key(&did) || s.rules.legal_reason(&meta, &anc, &excluded).is_some() {
            let skipped = &mut s.store.m["merges"][mid]["skipped_legal"];
            if !skipped.as_array().is_some_and(|a| a.iter().any(|x| x == did.as_str())) {
                skipped
                    .as_array_mut()
                    .expect("skipped_legal is a list")
                    .push(json!(did)); // the ID only; never read, never moved
            }
            continue;
        }
        let parents = list_of(&meta, "parents");
        if parents.contains(&arch) {
            let st = &mut s.store.m["merges"][mid];
            if st["moved"].get(&did).is_none() {
                st["moved"][&did] = json!({"from": parents.iter().filter(|p| **p != arch).collect::<Vec<_>>()});
            }
            if let Some(m) = st["moved"][&did].as_object_mut() {
                m.shift_remove("pending");
            }
            continue;
        }
        let mut outside: Vec<&String> = doc_jokes
            .get(&did)
            .map(|v| v.iter().filter(|j| !group.contains(*j)).collect())
            .unwrap_or_default();
        outside.sort();
        outside.dedup();
        if !outside.is_empty() {
            s.store.m["merges"][mid]["left"][&did] = json!(format!(
                "also holds {} other joke{}; left in place",
                outside.len(),
                if outside.len() > 1 { "s" } else { "" }
            ));
            continue;
        }
        s.store.m["merges"][mid]["moved"][&did] = json!({"from": parents, "pending": true});
        s.save()?;
        let new = s.drive.move_doc(&did, &arch, &parents, &anc)?;
        if let Some(m) = s.store.m["merges"][mid]["moved"][&did].as_object_mut() {
            m.shift_remove("pending");
        }
        if s.store.m["docs"].get(&did).is_some() {
            // our own move must not come back through the Changes feed as an edit
            if let Some(v) = new.get("version") {
                s.store.m["docs"][&did]["version"] = v.clone();
            }
            s.store.m["docs"][&did]["archived_by_merge"] = json!(mid);
        }
        s.save()?;
        s.act("merge_move", &[("merge", json!(mid)), ("doc", json!(did))]);
    }
    let now = s.svc.clock.now_iso();
    let st = &mut s.store.m["merges"][mid];
    st["state"] = json!("applied");
    st["applied_at"] = json!(now);
    Ok(())
}

fn undo(s: &mut Sync<'_>, mid: &str) -> Result<()> {
    if s.dry {
        let n = s.store.m["merges"][mid]["moved"].as_object().map_or(0, Map::len);
        s.act("merge_undo", &[("merge", json!(mid)), ("docs", json!(n))]);
        return Ok(());
    }
    s.store.m["merges"][mid]["state"] = json!("undoing");
    let arch = {
        let st = &s.store.m["merges"][mid];
        let a = st
            .get("archive_folder")
            .filter(|v| py::truthy(Some(v)))
            .or_else(|| s.store.m.get("archive_folder_id"));
        a.and_then(Value::as_str).map(str::to_string)
    };
    if !s.store.m["merges"][mid]["moved_back"].is_object() {
        s.store.m["merges"][mid]["moved_back"] = json!({});
    }
    let mut moved: Vec<(String, Value)> = s.store.m["merges"][mid]["moved"]
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    moved.sort_by(|a, b| a.0.cmp(&b.0));
    let arch_s = arch.clone().unwrap_or_default();
    for (did, mv) in moved {
        if s.store.m["merges"][mid]["moved_back"].get(&did).is_some() {
            continue;
        }
        let meta = s.drive.get(&did)?;
        let Some(meta) = meta.filter(|m| !py::truthy(m.get("trashed"))) else {
            s.store.m["merges"][mid]["moved_back"][&did] = json!("gone from Drive");
            continue;
        };
        let parents = list_of(&meta, "parents");
        if arch.is_none() || !parents.contains(&arch_s) {
            s.store.m["merges"][mid]["moved_back"][&did] =
                json!("not in the archive any more (you moved it): left where it is");
            continue;
        }
        let mut dest: Vec<String> = list_of(&mv, "from").into_iter().filter(|p| *p != arch_s).collect();
        if dest.is_empty() {
            dest = vec![s.store.m["folder_id"].as_str().unwrap_or_default().to_string()];
        }
        let anc = s.ancestors(&meta)?;
        let mut new = s.drive.move_doc(&did, &dest[0], std::slice::from_ref(&arch_s), &anc)?;
        for extra in &dest[1..] {
            // a doc that lived in several folders goes back to all of them
            let anc = s.ancestors(&meta)?;
            new = s.drive.move_doc(&did, extra, &[], &anc)?;
        }
        s.store.m["merges"][mid]["moved_back"][&did] = json!("moved back");
        if s.store.m["docs"].get(&did).is_some() {
            if let Some(v) = new.get("version") {
                s.store.m["docs"][&did]["version"] = v.clone();
            }
            if let Some(o) = s.store.m["docs"][&did].as_object_mut() {
                o.shift_remove("archived_by_merge");
            }
        }
        s.save()?;
        s.act("merge_move_back", &[("merge", json!(mid)), ("doc", json!(did))]);
    }
    let did = s.store.m["merges"][mid]
        .get("doc_id")
        .and_then(Value::as_str)
        .filter(|d| !d.is_empty())
        .map(str::to_string);
    if let Some(did) = did
        && !py::truthy(s.store.m["merges"][mid].get("released"))
    {
        let meta = s.drive.get(&did)?;
        if let Some(meta) = meta.filter(|m| !py::truthy(m.get("trashed"))) {
            s.drive.allow(&did);
            let name = format!("(merge undone) {}", sget(&meta, "name").unwrap_or_default());
            let mut new = s.drive.set_props(
                &did,
                &json!({"punchable_kind": "merge_undone", "punchable_merge": null, "punchable_joke": null}),
                Some(py::strip(&name)),
            )?;
            let meta_parents = list_of(&meta, "parents");
            if let Some(a) = &arch
                && !a.is_empty()
                && !meta_parents.contains(a)
            {
                let anc = s.ancestors(&meta)?;
                new = s.drive.move_doc(&did, a, &meta_parents, &anc)?;
            }
            let mut e = s.store.m["docs"]
                .get(&did)
                .filter(|v| py::truthy(Some(v)))
                .cloned()
                .unwrap_or(json!({}));
            e["origin"] = json!("merged_undone");
            e["version"] = new.get("version").cloned().unwrap_or(Value::Null);
            e["hidden"] = json!(true);
            s.store.m["docs"][&did] = e;
        }
        s.store.m["merges"][mid]["released"] = json!(true);
        s.act("merge_release", &[("merge", json!(mid)), ("doc", json!(did))]);
    }
    let now = s.svc.clock.now_iso();
    let st = &mut s.store.m["merges"][mid];
    st["state"] = json!("undone");
    st["undone_at"] = json!(now);
    Ok(())
}
