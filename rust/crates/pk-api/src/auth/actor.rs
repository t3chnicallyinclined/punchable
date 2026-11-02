//! Who a write is by (`server.py` `actor`): an app token, or a local app without auth, is the comic (`OWNER_ID`). A
//! worker token must name an `import:<source>` or `ai:<what>` actor in the body; a loopback dev API without auth may
//! name one too. Anything else is refused with the Python's text. The writes that call it land at T10; the rule is
//! here now so the auth module is whole and tested in one place.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use super::Principal;
use crate::error::ApiError;

/// `ACTOR_RE`: `^(import|ai):[a-z][a-z0-9_]{1,30}$` under `re.match`, whose `$` also matches before one final
/// newline.
static ACTOR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A(?:import|ai):[a-z][a-z0-9_]{1,30}\n?\z").unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});

/// Whether `s` is an import or ai actor id.
pub fn is_actor(s: &str) -> bool {
    ACTOR_RE.is_match(s)
}

/// The actor of a write: `auth_on` = the server checks tokens; `principal` = the caller (auth on); `body` = the
/// request body when it is an object; `owner_id` = `PK_OWNER_ID`.
pub fn for_write(
    auth_on: bool,
    principal: Option<&Principal>,
    body: Option<&Map<String, Value>>,
    owner_id: &str,
) -> Result<String, ApiError> {
    let want = body.and_then(|b| b.get("actor"));
    if auth_on && principal.is_some_and(Principal::is_worker) {
        return match want.and_then(Value::as_str) {
            Some(s) if is_actor(s) => Ok(s.to_string()),
            _ => Err(ApiError::bad_request(
                "a worker write must name its actor: import:<source> or ai:<what>",
            )),
        };
    }
    if let Some(w) = want.filter(|w| !w.is_null())
        && !auth_on
    {
        return match w.as_str() {
            Some(s) if is_actor(s) => Ok(s.to_string()),
            _ => Err(ApiError::bad_request("actor must be import:<source> or ai:<what>")),
        };
    }
    Ok(owner_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn worker() -> Principal {
        Principal {
            claims: json!({"realm_access": {"roles": ["worker"]}})
                .as_object()
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn app() -> Principal {
        Principal {
            claims: json!({"sub": "u"}).as_object().cloned().unwrap_or_default(),
        }
    }

    fn body(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(o) => o,
            _ => Map::new(),
        }
    }

    #[test]
    fn the_actor_regex_is_re_match() {
        assert!(is_actor("import:gsync"));
        assert!(is_actor("ai:riff"));
        assert!(is_actor("ai:riff\n"));
        assert!(!is_actor("ai:riff\n\n"));
        assert!(!is_actor("ai:r"));
        assert!(!is_actor("ai:Riff"));
        assert!(!is_actor("owner"));
        assert!(!is_actor(&format!("ai:{}", "a".repeat(32))));
        assert!(is_actor(&format!("ai:{}", "a".repeat(31))));
    }

    #[test]
    fn an_app_token_is_always_the_owner() {
        let b = body(json!({"actor": "ai:riff"}));
        assert_eq!(for_write(true, Some(&app()), Some(&b), "owner").unwrap(), "owner");
        assert_eq!(for_write(true, Some(&app()), None, "owner").unwrap(), "owner");
    }

    #[test]
    fn a_worker_must_name_an_actor() {
        let b = body(json!({"actor": "import:memo"}));
        assert_eq!(
            for_write(true, Some(&worker()), Some(&b), "owner").unwrap(),
            "import:memo"
        );
        let e = for_write(true, Some(&worker()), None, "owner").unwrap_err();
        assert_eq!(
            e.msg,
            "a worker write must name its actor: import:<source> or ai:<what>"
        );
        assert_eq!(e.status, 400);
        let b = body(json!({"actor": "owner"}));
        assert!(for_write(true, Some(&worker()), Some(&b), "owner").is_err());
    }

    #[test]
    fn a_dev_server_without_auth_may_name_one() {
        let b = body(json!({"actor": "ai:merge"}));
        assert_eq!(for_write(false, None, Some(&b), "owner").unwrap(), "ai:merge");
        let b = body(json!({"actor": 3}));
        assert_eq!(
            for_write(false, None, Some(&b), "owner").unwrap_err().msg,
            "actor must be import:<source> or ai:<what>"
        );
        let b = body(json!({"actor": null}));
        assert_eq!(for_write(false, None, Some(&b), "owner").unwrap(), "owner");
    }
}
