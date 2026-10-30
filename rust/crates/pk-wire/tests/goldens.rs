//! The differential against the Python it mirrors: `rust/tools/gen_wire_goldens.py` drives the Python API's worker
//! routes and app-facing routes over a synthetic knowledge base and writes each body; these tests read each golden
//! into its struct, write it back and compare as parsed JSON. A field the struct drops, renames, retypes, or turns
//! from `null` into absent shows up here (the pk-kb pattern; the loader is the same).

use std::path::PathBuf;

use pk_wire::api::{ApiEvent, EdgeView, JokesPage, Review, Sets, UploadMeta, Written};
use pk_wire::creative::{
    AskAnswer, BitHistory, Board, BoardView, Boards, Bridge, ClipsView, Connections, ConnectionsStatus, GeoJSON, Job,
    Nights, PunchUp, Queued, TodayFeed, VenuesView, WSView, WorkingSets,
};
use pk_wire::simple::{Build, DismissResponse, Facts, JokeMap, Related, Riff, SavedIdea};
use pk_wire::worker::creative::{CreativeLines, EventRequest, EventResponse, MergeRecord};
use pk_wire::worker::files::{FileContent, FilesListing, FilesPost, FilesPostResponse};
use pk_wire::worker::kb::{
    DecisionRequest, DecisionResponse, DecisionsPage, IdeaRequest, IdeaResponse, JokePage, UndoRequest,
};
use pk_wire::worker::voice::{VoiceProfile, VoiceProfileApp, VoicePut};
use pk_wire::worker::{
    AskChatResult, Claim, ClaimResponse, EmbedResult, ErrorBody, JobResult, MemolinkResult, ProcessSetResult,
    ResultResponse, SetTextResult, State,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn golden(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(format!("{name}.json"));
    serde_json::from_str(
        &std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_wire_goldens.py)", p.display())),
    )
    .expect("golden is JSON")
}

/// A readable diff on failure: the first path where two JSON values differ.
fn first_diff(rust: &Value, python: &Value, path: &str) -> Option<String> {
    match (rust, python) {
        (Value::Object(ro), Value::Object(po)) => {
            for key in ro.keys().chain(po.keys().filter(|k| !ro.contains_key(*k))) {
                match (ro.get(key), po.get(key)) {
                    (Some(rv), Some(pv)) => {
                        if let Some(d) = first_diff(rv, pv, &format!("{path}.{key}")) {
                            return Some(d);
                        }
                    }
                    (Some(rv), None) => return Some(format!("{path}.{key}: rust has {rv}, python has nothing")),
                    (None, Some(pv)) => return Some(format!("{path}.{key}: rust has nothing, python has {pv}")),
                    (None, None) => {}
                }
            }
            None
        }
        (Value::Array(ra), Value::Array(pa)) => {
            if ra.len() != pa.len() {
                return Some(format!("{path}: rust has {} items, python has {}", ra.len(), pa.len()));
            }
            ra.iter()
                .zip(pa)
                .enumerate()
                .find_map(|(i, (rv, pv))| first_diff(rv, pv, &format!("{path}[{i}]")))
        }
        (Value::Number(rn), Value::Number(pn)) => {
            let (fr, fp) = (rn.as_f64().unwrap_or(f64::NAN), pn.as_f64().unwrap_or(f64::NAN));
            ((fr - fp).abs() > 1e-9).then(|| format!("{path}: rust {rn}, python {pn}"))
        }
        _ => (rust != python).then(|| format!("{path}: rust {rust}, python {python}")),
    }
}

/// The golden read into `T` and written back must equal the golden as parsed values.
fn roundtrip<T: Serialize + DeserializeOwned>(name: &str) -> T {
    let python = golden(name);
    let typed: T = serde_json::from_value(python.clone()).unwrap_or_else(|e| {
        panic!(
            "{name}: the golden does not read into {}: {e}",
            std::any::type_name::<T>()
        )
    });
    let rust = json!(typed);
    if let Some(d) = first_diff(&rust, &python, name) {
        panic!(
            "{name} written back by {} differs from the Python golden at {d}",
            std::any::type_name::<T>()
        );
    }
    typed
}

/// A value inside a golden (a job's `result`, a snapshot's `items`) read into `T` and written back.
fn roundtrip_value<T: Serialize + DeserializeOwned>(what: &str, python: &Value) -> T {
    let typed: T = serde_json::from_value(python.clone())
        .unwrap_or_else(|e| panic!("{what}: does not read into {}: {e}", std::any::type_name::<T>()));
    let rust = json!(typed);
    if let Some(d) = first_diff(&rust, python, what) {
        panic!("{what} written back by {} differs at {d}", std::any::type_name::<T>());
    }
    typed
}

// ---------------------------------------------------------------- the worker protocol

#[test]
fn claims_and_jobs_round_trip() {
    let claim = roundtrip::<Claim>("worker_claim");
    assert!(claim.hosted && claim.paused.is_none());
    assert!(roundtrip::<Claim>("worker_claim_paused").paused.is_some());
    for kind in [
        "riff",
        "facts",
        "ask",
        "bridge",
        "punchup",
        "process_set",
        "set_text",
        "ask_chat",
        "embed",
        "memolink",
    ] {
        let r = roundtrip::<ClaimResponse>(&format!("worker_claimed_{kind}"));
        assert_eq!(json!(r.job.kind)["kind"], kind, "the typed kind is the wire's");
        assert_eq!(r.job.status, pk_wire::worker::JobStatus::Claimed);
    }
    roundtrip::<ErrorBody>("worker_error");
    roundtrip::<ErrorBody>("worker_error_job_file");
    roundtrip::<ErrorBody>("worker_error_result_conflict");
}

#[test]
fn results_round_trip() {
    roundtrip::<JobResult>("worker_result_retry");
    roundtrip::<ResultResponse>("worker_result_retry_response");
    roundtrip::<ResultResponse>("worker_result_response");
    // the result object of each kind is the kind's own shape
    let r = roundtrip::<JobResult>("worker_result_riff");
    roundtrip_value::<Riff>("worker_result_riff.result", r.result.as_ref().unwrap());
    let r = roundtrip::<JobResult>("worker_result_facts");
    roundtrip_value::<Facts>("worker_result_facts.result", r.result.as_ref().unwrap());
    let r = roundtrip::<JobResult>("worker_result_ask");
    roundtrip_value::<AskAnswer>("worker_result_ask.result", r.result.as_ref().unwrap());
    let r = roundtrip::<JobResult>("worker_result_bridge");
    roundtrip_value::<Bridge>("worker_result_bridge.result", r.result.as_ref().unwrap());
    let r = roundtrip::<JobResult>("worker_result_punchup");
    roundtrip_value::<PunchUp>("worker_result_punchup.result", r.result.as_ref().unwrap());
    let r = roundtrip::<JobResult>("worker_result_process_set");
    roundtrip_value::<ProcessSetResult>("worker_result_process_set.result", r.result.as_ref().unwrap());
    let r = roundtrip::<JobResult>("worker_result_set_text");
    roundtrip_value::<SetTextResult>("worker_result_set_text.result", r.result.as_ref().unwrap());
    let r = roundtrip::<JobResult>("worker_result_ask_chat");
    roundtrip_value::<AskChatResult>("worker_result_ask_chat.result", r.result.as_ref().unwrap());
    let r = roundtrip::<JobResult>("worker_result_embed");
    roundtrip_value::<EmbedResult>("worker_result_embed.result", r.result.as_ref().unwrap());
    let r = roundtrip::<JobResult>("worker_result_memolink");
    roundtrip_value::<MemolinkResult>("worker_result_memolink.result", r.result.as_ref().unwrap());
}

#[test]
fn state_files_and_kb_round_trip() {
    roundtrip::<State>("worker_state");
    roundtrip::<FilesListing>("worker_files");
    roundtrip::<FileContent>("worker_file_content");
    roundtrip::<FilesPost>("worker_files_post");
    roundtrip::<FilesPostResponse>("worker_files_post_response");
    roundtrip::<ErrorBody>("worker_error_files_refused");
    roundtrip::<IdeaRequest>("worker_ideas");
    let r = roundtrip::<IdeaResponse>("worker_ideas_response");
    assert_eq!(r.attach.as_ref().map(|a| a.decision.as_str()), Some("new"));
    let r = roundtrip::<IdeaResponse>("worker_ideas_response_attached");
    assert!(r.attach.as_ref().and_then(|a| a.record.as_ref()).is_some());
    roundtrip::<DecisionRequest>("worker_decision");
    roundtrip::<DecisionResponse>("worker_decision_response");
    roundtrip::<UndoRequest>("worker_undo");
    let u = roundtrip::<DecisionResponse>("worker_undo_response");
    assert!(u.decisions[0].undo_of.is_some());
    roundtrip::<ErrorBody>("worker_error_actor");
    roundtrip::<JokePage>("worker_joke");
    roundtrip::<DecisionsPage>("worker_decisions");
    roundtrip::<DecisionsPage>("worker_decisions_after");
}

#[test]
fn creative_lines_events_and_voice_round_trip() {
    roundtrip::<CreativeLines>("worker_creative");
    let snap = roundtrip::<CreativeLines>("worker_creative_snapshot");
    for (id, item) in snap.items.as_ref().expect("items") {
        roundtrip_value::<Board>(&format!("worker_creative_snapshot.items.{id}"), item);
    }
    let merges = roundtrip::<CreativeLines>("worker_creative_merges");
    for (id, item) in merges.items.as_ref().expect("items") {
        roundtrip_value::<MergeRecord>(&format!("worker_creative_merges.items.{id}"), item);
    }
    roundtrip::<EventRequest>("worker_events");
    assert!(roundtrip::<EventResponse>("worker_events_response").record.is_some());
    assert_eq!(
        roundtrip::<EventResponse>("worker_events_response_already").already,
        Some(true)
    );
    assert!(roundtrip::<VoiceProfile>("worker_voice_empty").text.is_none());
    roundtrip::<VoicePut>("worker_voice_put");
    assert!(
        roundtrip::<VoiceProfileApp>("worker_voice_put_response")
            .previous
            .is_none()
    );
    assert!(
        roundtrip::<VoiceProfileApp>("worker_voice_put_response_previous")
            .previous
            .is_some()
    );
    roundtrip::<VoiceProfile>("worker_voice");
    roundtrip::<VoiceProfileApp>("api_voice");
}

// ---------------------------------------------------------------- the app-facing shapes

#[test]
fn api_shapes_round_trip() {
    roundtrip::<Written>("api_written");
    roundtrip::<JokesPage>("api_jokes");
    roundtrip::<JokePage>("api_joke_detail");
    roundtrip::<Vec<EdgeView>>("api_edge_views");
    roundtrip::<Sets>("api_sets");
    let review = roundtrip::<Review>("api_review");
    assert!(review.steps.iter().any(|s| s.kind == pk_wire::api::StepKind::NewLine));
    roundtrip::<ApiEvent>("api_api_event");
    roundtrip::<UploadMeta>("api_upload_meta");
    roundtrip::<ErrorBody>("api_error");
}

#[test]
fn creative_shapes_round_trip() {
    roundtrip::<Connections>("creative_connections");
    roundtrip::<Boards>("creative_boards");
    roundtrip::<BoardView>("creative_board_view");
    roundtrip::<BoardView>("creative_board_view_read");
    assert_eq!(
        roundtrip::<BoardView>("creative_board_view_archived").board.archived,
        Some(true)
    );
    assert_eq!(
        roundtrip::<BoardView>("creative_board_view_unarchived").board.archived,
        Some(false)
    );
    let archived = roundtrip::<Boards>("creative_boards_archived");
    assert!(archived.archived && archived.boards.iter().all(|b| b.archived));
    assert!(
        roundtrip::<Boards>("creative_boards")
            .boards
            .iter()
            .any(|b| b.seed.is_some())
    );
    roundtrip::<ErrorBody>("creative_error_archived");
    roundtrip::<WorkingSets>("creative_workingsets");
    roundtrip::<WSView>("creative_ws_view");
    roundtrip::<Nights>("creative_nights");
    roundtrip::<BitHistory>("creative_bit_history");
    roundtrip::<BitHistory>("creative_bit_history_untaped");
    roundtrip::<VenuesView>("creative_venues");
    roundtrip::<GeoJSON>("creative_geojson");
    roundtrip::<TodayFeed>("creative_today");
    roundtrip::<ClipsView>("creative_clips");
    roundtrip::<ClipsView>("creative_clips_none");
    roundtrip::<ConnectionsStatus>("creative_connections_status");
}

#[test]
fn jobs_and_drafts_round_trip() {
    assert!(matches!(
        roundtrip::<Queued<AskAnswer>>("creative_queued"),
        Queued::Queued(_)
    ));
    assert!(matches!(
        roundtrip::<Queued<Bridge>>("creative_queued_bridge"),
        Queued::Queued(_)
    ));
    let w = roundtrip::<Job>("creative_job_waiting");
    assert_eq!(w.brief.waiting_for.as_deref(), Some("transcriber"));
    assert!(roundtrip::<Job<Riff>>("creative_job_riff").result.is_some());
    assert!(roundtrip::<Job<Facts>>("creative_job_facts").result.is_some());
    assert!(roundtrip::<Job<AskAnswer>>("creative_job_ask").result.is_some());
    assert!(roundtrip::<Job<Bridge>>("creative_job_bridge").result.is_some());
    assert!(roundtrip::<Job<PunchUp>>("creative_job_punchup").result.is_some());
    assert!(
        matches!(roundtrip::<Queued<AskAnswer>>("creative_cached_ask"), Queued::Ready(a) if a.cached == Some(true))
    );
    assert!(matches!(
        roundtrip::<Queued<Bridge>>("creative_cached_bridge"),
        Queued::Ready(_)
    ));
    assert!(matches!(
        roundtrip::<Queued<PunchUp>>("creative_cached_punchup"),
        Queued::Ready(_)
    ));
    assert!(matches!(
        roundtrip::<Queued<Riff>>("simple_cached_riff"),
        Queued::Ready(_)
    ));
    assert!(matches!(
        roundtrip::<Queued<Facts>>("simple_cached_facts"),
        Queued::Ready(_)
    ));
}

#[test]
fn simple_shapes_round_trip() {
    roundtrip::<Related>("simple_related");
    roundtrip::<JokeMap>("simple_joke_map");
    let b = roundtrip::<Build>("simple_build");
    assert!(b.steps[0].transition_in.is_none() && b.steps[1].transition_in.is_some());
    roundtrip::<SavedIdea>("simple_save_idea");
    roundtrip::<DismissResponse>("simple_dismiss_response");
}
