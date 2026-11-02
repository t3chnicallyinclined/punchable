//! The worker's in-process goldens from `rust/tools/gen_worker_goldens.py`: `creative/voice.py`'s blocks and
//! messages per task (named and unnamed, profile and learned present or not; the learned block is the second system
//! block of every writing prompt) and `creative/write.py`'s page text, `supported` and URL rule. Compared as parsed
//! JSON (ADR 0007).

use pk_llm::ChatMessage;
use pk_worker::kinds::facts::{URL_RE, page_text, supported};
use pk_worker::voice::{Names, Voice};
use serde_json::{Value, json};

fn golden(name: &str) -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(&p).expect("the golden: run rust/tools/gen_worker_goldens.py"))
        .expect("golden JSON")
}

#[test]
fn voice_blocks_and_messages_per_task() {
    let g = golden("voice.json");
    let cases = g["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 18);
    let mut learned_seen = 0;
    for c in cases {
        let env = &c["env"];
        let names = Names::new(env["PK_COMIC_NAME"].as_str(), env["PK_COMIC_FIRST"].as_str());
        assert_eq!(
            json!(names.material_owner()),
            c["names"]["material_owner"],
            "{}",
            c["doc"]
        );
        let v = Voice::from_answer(&c["voice"]);
        assert_eq!(
            serde_json::to_value(v.status()).unwrap(),
            c["status"],
            "{} status",
            c["doc"]
        );
        for (task, want) in c["tasks"].as_object().unwrap() {
            let at = format!("{} / {} / {task}", env, c["doc"]);
            assert_eq!(json!(v.system_block(task)), want["system_block"], "{at}: system_block");
            assert_eq!(
                json!(v.learned_block(task)),
                want["learned_block"],
                "{at}: learned_block"
            );
            assert_eq!(
                json!(v.cacheable_learned(task)),
                want["cacheable_learned"],
                "{at}: cacheable_learned"
            );
            let msgs = v.messages(task, "TASK LINE", [ChatMessage::user("USER TURN")]);
            assert_eq!(serde_json::to_value(&msgs).unwrap(), want["messages"], "{at}: messages");
            learned_seen += usize::from(!want["learned_block"].is_null());
        }
    }
    assert!(learned_seen > 0, "the golden exercises the learned block");
}

#[test]
fn page_text_supported_and_the_url_rule() {
    let g = golden("pages.json");
    for c in g["texts"].as_array().unwrap() {
        let html = c["html"].as_str().unwrap();
        if c["legacy_entities"] == json!(true) {
            // the allowed difference: entities without `;` stay as written here (html-escape); Python decodes them
            assert_ne!(
                json!(page_text(html)),
                c["text"],
                "{html}: the difference is gone; drop the allow-list"
            );
            assert_eq!(page_text(html), html.to_lowercase(), "{html}");
            continue;
        }
        assert_eq!(json!(page_text(html)), c["text"], "{html}");
    }
    assert!(g["missing"].is_null());
    for c in g["supported"].as_array().unwrap() {
        let page = c["page"].as_str().filter(|p| !p.is_empty());
        assert_eq!(json!(supported(c["fact"].as_str().unwrap(), page)), c["ok"], "{c}");
    }
    for c in g["url_ok"].as_array().unwrap() {
        assert_eq!(json!(URL_RE.is_match(c["url"].as_str().unwrap())), c["ok"], "{c}");
    }
}
