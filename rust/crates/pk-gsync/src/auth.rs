//! Google credentials for gsync, from Punchable's "Sign in with Google" (Keycloak `IdP` `google`, `storeToken` on)
//! (`gsync/auth.py`).
//!
//! One time: `pk-gsync auth` = OAuth device flow against Keycloak client `pk-gsync` (he signs in with Google in the
//! browser). gsync keeps an OFFLINE Keycloak refresh token (rotated on every use) in
//! `~/.punchable/keycloak_gsync_token.json`.
//! Each run: refresh the cached Google token against Google (client id and secret from `google-oauth.env`). Only
//! when there is no cached Google refresh token, or Google refuses it, gsync asks Keycloak for the stored one
//! (`GET <realm>/broker/google/token` with a pk-gsync access token). Keycloak is touched at least daily so its
//! offline session never idles out.
//!
//! Every token file is owner-only; no token or secret is ever printed: the files are read and written by reference
//! and the only text that leaves this module is the fix to apply.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use pk_core::py;
use serde_json::{Map, Value, json};

use crate::config::Config;
use crate::error::{GsError, Result};
use crate::http::{Http, Response, urlencode};
use crate::services::Services;

/// Keycloak is asked at least this often (seconds).
pub const KC_DAILY_S: f64 = 86400.0;
const REFRESH_GRANT: &str = "refresh_token";

/// Write a file that holds a secret: locked down before the secret goes in, replaced atomically.
pub fn write_secret(svc: &Services, p: &Path, text: &str) -> Result<()> {
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, "")?;
    (svc.restrict)(&tmp);
    fs::write(&tmp, text)?;
    fs::rename(&tmp, p)?;
    (svc.restrict)(p);
    Ok(())
}

/// The `KEY=value` lines of an env file (`read_env`).
pub fn read_env(p: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Ok(text) = fs::read_to_string(p) {
        for ln in py::splitlines(&text) {
            if let Some((k, v)) = ln.split_once('=')
                && !ln.trim_start_matches(py::isspace).starts_with('#')
            {
                out.insert(
                    py::strip(k).to_string(),
                    py::strip(v).trim_matches('"').trim_matches('\'').to_string(),
                );
            }
        }
    }
    out
}

fn urlopen_error(msg: &str) -> GsError {
    GsError::other("URLError", format!("<urlopen error {msg}>"))
}

/// `_post`: a form POST; the status and the JSON answer (an unreadable error body is `{}`).
fn post_form(http: &Http, url: &str, data: &[(&str, &str)]) -> Result<(u16, Value)> {
    let r = http
        .send(
            "POST",
            url,
            &[("Content-Type", "application/x-www-form-urlencoded".to_string())],
            Some(urlencode(data).into_bytes()),
        )
        .map_err(|e| urlopen_error(&e.message))?;
    match serde_json::from_slice::<Value>(&r.body) {
        Ok(v) => Ok((r.status, v)),
        Err(_) if r.status >= 400 => Ok((r.status, json!({}))),
        Err(e) => Err(GsError::other("JSONDecodeError", e.to_string())),
    }
}

fn save_kc(cfg: &Config, svc: &Services, t: &Value) -> Result<()> {
    let Some(rt) = t.get("refresh_token").filter(|v| py::truthy(Some(v))) else {
        return Err(GsError::Exit(
            "Keycloak returned no refresh token (offline_access missing?)".into(),
        ));
    };
    let rec = json!({"refresh_token": rt, "issuer": cfg.auth_url, "client_id": cfg.kc_client,
                     "saved_at": svc.now_secs() as i64});
    write_secret(svc, &cfg.kc_token_file(), &py::dumps(&rec))
}

fn realm_base(cfg: &Config) -> String {
    cfg.auth_url.trim_end_matches('/').to_string() + "/protocol/openid-connect"
}

fn get_str(v: &Value, k: &str) -> String {
    py::str(v.get(k).unwrap_or(&Value::Null))
}

/// `kc_device_login`: the device flow against Keycloak. Returns the token file's path.
pub fn kc_device_login(cfg: &Config, svc: &Services, out: &mut dyn Write) -> Result<String> {
    let http = Http::new(Duration::from_secs(30));
    let base = realm_base(cfg);
    let (s, d) = post_form(
        &http,
        &format!("{base}/auth/device"),
        &[("client_id", &cfg.kc_client), ("scope", "openid offline_access")],
    )?;
    if s != 200 {
        return Err(GsError::Exit(format!(
            "Keycloak refused the device login ({s} {}); is the realm applied? (SETUP.md)",
            get_str(&d, "error")
        )));
    }
    let url = d
        .get("verification_uri_complete")
        .filter(|v| py::truthy(Some(v)))
        .or_else(|| d.get("verification_uri"))
        .map(py::str)
        .ok_or_else(|| GsError::other("KeyError", "'verification_uri'"))?;
    let user_code = d
        .get("user_code")
        .map(py::str)
        .ok_or_else(|| GsError::other("KeyError", "'user_code'"))?;
    let _ = writeln!(
        out,
        "Sign in at {url}\n  code: {user_code}  (choose 'Sign in with Google')"
    );
    (svc.opener)(&url);
    let as_int = |k: &str, default: i64| {
        d.get(k)
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
            .unwrap_or(default)
    };
    let mut interval = as_int("interval", 5);
    let deadline = svc.now_secs() + as_int("expires_in", 600) as f64;
    let device_code = d
        .get("device_code")
        .map(py::str)
        .ok_or_else(|| GsError::other("KeyError", "'device_code'"))?;
    while svc.now_secs() < deadline {
        (svc.sleep)(Duration::from_secs(interval.max(0) as u64));
        let (s, t) = post_form(
            &http,
            &format!("{base}/token"),
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &device_code),
                ("client_id", &cfg.kc_client),
            ],
        )?;
        if s == 200 {
            save_kc(cfg, svc, &t)?;
            return Ok(cfg.kc_token_file().to_string_lossy().into_owned());
        }
        match t.get("error").and_then(Value::as_str) {
            Some("slow_down") => interval += 5,
            Some("authorization_pending") => {}
            _ => {
                return Err(GsError::Exit(format!(
                    "sign-in failed: {} {}",
                    get_str(&t, "error"),
                    t.get("error_description").map_or_else(String::new, py::str)
                )));
            }
        }
    }
    Err(GsError::Exit(
        "sign-in timed out; run `python -m gsync auth` again".into(),
    ))
}

/// A fresh pk-gsync access token. Rotation is on, so the new refresh token is saved before anything else.
pub fn kc_access_token(cfg: &Config, svc: &Services) -> Result<String> {
    let file = cfg.kc_token_file();
    if !file.exists() {
        return Err(GsError::AuthMissing("not signed in: run `python -m gsync auth`".into()));
    }
    let kc: Value = serde_json::from_str(&fs::read_to_string(&file)?)?;
    let rt = kc
        .get("refresh_token")
        .map(py::str)
        .ok_or_else(|| GsError::other("KeyError", "'refresh_token'"))?;
    let http = Http::new(Duration::from_secs(30));
    let (s, t) = post_form(
        &http,
        &format!("{}/token", realm_base(cfg)),
        &[
            ("grant_type", REFRESH_GRANT),
            ("client_id", &cfg.kc_client),
            ("refresh_token", &rt),
        ],
    )?;
    if s != 200 {
        return Err(GsError::AuthMissing(format!(
            "Keycloak sign-in expired ({}): run `python -m gsync auth`",
            get_str(&t, "error")
        )));
    }
    save_kc(cfg, svc, &t)?;
    t.get("access_token")
        .map(py::str)
        .ok_or_else(|| GsError::other("KeyError", "'access_token'"))
}

/// Keycloak's stored Google token response (needs the broker `read-token` role in the access token).
pub fn broker_google_token(cfg: &Config, kc_token: &str) -> Result<Value> {
    let http = Http::new(Duration::from_secs(30));
    let url = cfg.auth_url.trim_end_matches('/').to_string() + "/broker/google/token";
    let r = http
        .send("GET", &url, &[("Authorization", format!("Bearer {kc_token}"))], None)
        .map_err(|e| urlopen_error(&e.message))?;
    if r.status >= 400 {
        let hint = match r.status {
            400 => "Google is not linked to your Punchable account (SETUP.md step 9)",
            403 => "this token may not read Google tokens (realm not applied?)",
            _ => "",
        };
        return Err(GsError::AuthMissing(format!(
            "Keycloak broker token: HTTP {}. {hint}",
            r.status
        )));
    }
    let body = r.text();
    match serde_json::from_str::<Value>(&body) {
        Ok(v) => Ok(v),
        // some providers store form-encoded responses
        Err(_) => Ok(Value::Object(
            form_urlencoded::parse(body.as_bytes())
                .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
                .collect::<Map<_, _>>(),
        )),
    }
}

fn google_client(cfg: &Config) -> Result<(String, String)> {
    let env = read_env(&cfg.google_env());
    match (env.get("GOOGLE_CLIENT_ID"), env.get("GOOGLE_CLIENT_SECRET")) {
        (Some(id), Some(sec)) if !id.is_empty() && !sec.is_empty() => Ok((id.clone(), sec.clone())),
        _ => Err(GsError::AuthMissing(format!(
            "missing GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET in {} (SETUP.md step 5)",
            cfg.google_env().display()
        ))),
    }
}

/// A Google OAuth credential (`google.oauth2.credentials.Credentials`): a refresh token that makes access tokens.
#[derive(Debug, Clone)]
pub struct Creds {
    /// The refresh token (a reference to the sign-in; never printed).
    refresh_token: String,
    client_id: String,
    client_secret: String,
    token_uri: String,
    scopes: Vec<String>,
    /// The current access token.
    pub token: Option<String>,
    /// When it lapses (seconds since the epoch).
    expiry: Option<f64>,
}

/// google-auth refreshes this long before the expiry.
const REFRESH_SKEW_S: f64 = 180.0;

impl Creds {
    /// Credentials for a refresh token and the client that owns it. No access token yet.
    pub fn new(refresh_token: &str, cid: &str, sec: &str, token_uri: &str, scopes: Vec<String>) -> Self {
        Self {
            refresh_token: refresh_token.to_string(),
            client_id: cid.to_string(),
            client_secret: sec.to_string(),
            token_uri: token_uri.to_string(),
            scopes,
            token: None,
            expiry: None,
        }
    }

    /// Whether the access token is there and not about to lapse.
    pub fn valid(&self, now: f64) -> bool {
        self.token.is_some() && self.expiry.is_none_or(|e| e - REFRESH_SKEW_S > now)
    }

    /// Exchange the refresh token for an access token (`creds.refresh(Request())`).
    pub fn refresh(&mut self, svc: &Services) -> Result<()> {
        let http = Http::new(Duration::from_secs(120));
        let scope = self.scopes.join(" ");
        let mut form = vec![
            ("grant_type", REFRESH_GRANT),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("refresh_token", self.refresh_token.as_str()),
        ];
        if !self.scopes.is_empty() {
            form.push(("scope", scope.as_str()));
        }
        let r: Response = http
            .send(
                "POST",
                &self.token_uri,
                &[("Content-Type", "application/x-www-form-urlencoded".to_string())],
                Some(urlencode(&form).into_bytes()),
            )
            .map_err(|e| GsError::other("TransportError", e.message))?;
        let data = serde_json::from_slice::<Value>(&r.body).ok();
        if r.status != 200 {
            return Err(match data {
                Some(d @ Value::Object(_)) => {
                    let details = match d.get("error") {
                        Some(e) => format!(
                            "{}: {}",
                            py::str(e),
                            py::str(d.get("error_description").unwrap_or(&Value::Null))
                        ),
                        None => py::dumps(&d),
                    };
                    GsError::Refresh(format!("({}, {})", py::repr(&Value::String(details)), py::repr(&d)))
                }
                _ => GsError::Refresh(r.text()),
            });
        }
        let Some(d) = data.filter(|d| d.get("access_token").is_some()) else {
            return Err(GsError::Refresh(format!(
                "('No access token in response.', {})",
                data_repr(&r.body)
            )));
        };
        self.token = Some(py::str(&d["access_token"]));
        if let Some(rt) = d.get("refresh_token").and_then(Value::as_str) {
            self.refresh_token = rt.to_string();
        }
        self.expiry = d.get("expires_in").and_then(Value::as_f64).map(|s| svc.now_secs() + s);
        Ok(())
    }

    /// `creds.to_json()` for the Desktop fallback's token file.
    fn to_json(&self, svc: &Services) -> String {
        let mut m = Map::new();
        m.insert("token".into(), self.token.clone().map_or(Value::Null, Value::String));
        m.insert("refresh_token".into(), json!(self.refresh_token));
        m.insert("token_uri".into(), json!(self.token_uri));
        m.insert("client_id".into(), json!(self.client_id));
        m.insert("client_secret".into(), json!(self.client_secret));
        m.insert("scopes".into(), json!(self.scopes));
        m.insert("universe_domain".into(), json!("googleapis.com"));
        m.insert("account".into(), json!(""));
        if self.expiry.is_some() {
            // a naive UTC time, `isoformat() + "Z"`
            let t = svc.now()
                + chrono::Duration::milliseconds(((self.expiry.unwrap_or(0.0) - svc.now_secs()) * 1000.0) as i64);
            m.insert("expiry".into(), json!(t.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()));
        }
        m.retain(|_, v| !v.is_null());
        py::dumps(&Value::Object(m))
    }
}

fn data_repr(body: &[u8]) -> String {
    match serde_json::from_slice::<Value>(body) {
        Ok(v) => py::repr(&v),
        Err(_) => py::repr(&Value::String(String::from_utf8_lossy(body).into_owned())),
    }
}

/// The Desktop fallback: a token file made by `from_authorized_user_file` (a separate Desktop-app client).
fn load_desktop(cfg: &Config, svc: &Services, refresh: bool) -> Result<Creds> {
    let info: Value = serde_json::from_str(&fs::read_to_string(cfg.token_file())?)?;
    let field = |k: &str| info.get(k).and_then(Value::as_str).map(str::to_string);
    let missing: Vec<&str> = ["client_secret", "refresh_token", "client_id"]
        .into_iter()
        .filter(|k| info.get(*k).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(GsError::other(
            "ValueError",
            format!(
                "Authorized user info was not in the expected format, missing fields {}.",
                missing.join(", ")
            ),
        ));
    }
    let mut creds = Creds::new(
        &field("refresh_token").unwrap_or_default(),
        &field("client_id").unwrap_or_default(),
        &field("client_secret").unwrap_or_default(),
        &svc.google_token_uri,
        cfg.scopes(),
    );
    creds.token = field("token");
    creds.expiry = field("expiry").and_then(|e| {
        chrono::NaiveDateTime::parse_from_str(e.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S%.f")
            .ok()
            .map(|t| t.and_utc().timestamp_millis() as f64 / 1000.0)
    });
    if refresh && !creds.valid(svc.now_secs()) && !creds.refresh_token.is_empty() {
        creds.refresh(svc)?;
        write_secret(svc, &cfg.token_file(), &creds.to_json(svc))?;
    }
    Ok(creds)
}

/// Google credentials or `None` (nothing set up). An error carries the fix when something is broken.
pub fn load_creds(cfg: &Config, svc: &Services, refresh: bool) -> Result<Option<Creds>> {
    if cfg.token_file().exists() && !cfg.kc_token_file().exists() {
        return load_desktop(cfg, svc, refresh).map(Some);
    }
    if !cfg.kc_token_file().exists() {
        return Ok(None);
    }
    let (cid, sec) = google_client(cfg)?;
    let cached: Value = if cfg.token_file().exists() {
        serde_json::from_str(&fs::read_to_string(cfg.token_file())?)?
    } else {
        json!({})
    };
    let checked = cached.get("kc_checked_at").and_then(Value::as_f64).unwrap_or(0.0);
    let kc_age = svc.now_secs() - checked;
    let cached_rt = cached
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if let Some(rt) = cached_rt
        && kc_age < KC_DAILY_S
    {
        let mut creds = Creds::new(rt, &cid, &sec, &svc.google_token_uri, Vec::new());
        match creds.refresh(svc) {
            Ok(()) => return Ok(Some(creds)),
            Err(GsError::Refresh(_)) => {} // revoked or expired: ask Keycloak for the token from his latest sign-in
            Err(e) => return Err(e),
        }
    }
    let stored = broker_google_token(cfg, &kc_access_token(cfg, svc)?)?;
    let rt = stored
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or(cached_rt)
        .map(str::to_string);
    let Some(rt) = rt else {
        return Err(GsError::AuthMissing(
            "Keycloak holds no Google refresh token: sign in with Google again (SETUP.md step 9)".into(),
        ));
    };
    let mut creds = Creds::new(&rt, &cid, &sec, &svc.google_token_uri, Vec::new());
    creds.refresh(svc)?; // a RefreshError here means a dead grant: SETUP.md step 3c
    write_secret(
        svc,
        &cfg.token_file(),
        &py::dumps(&json!({"refresh_token": rt, "source": "keycloak-broker", "kc_checked_at": svc.now_secs() as i64})),
    )?;
    Ok(Some(creds))
}

/// `pk-gsync auth`: sign in once. Returns the path of the token file that was written.
pub fn run_auth(cfg: &Config, svc: &Services, desktop: bool, out: &mut dyn Write) -> Result<String> {
    if desktop {
        return Err(GsError::Exit(
            "auth --desktop is not in pk-gsync (the fallback client flow); run the Python `python -m gsync auth --desktop` once, \
             then `run` here reads the token file it writes"
                .into(),
        ));
    }
    google_client(cfg)?; // fail early with the fix if the env file is missing
    let path = kc_device_login(cfg, svc, out)?;
    load_creds(cfg, svc, true)?; // proves the whole chain: Keycloak -> broker token -> Google refresh
    Ok(path)
}

/// What `status` shows: facts about the files, never their values.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
pub struct TokenInfo {
    /// `google-oauth.env` exists.
    pub google_env: bool,
    /// ... and has both keys.
    pub google_env_keys: bool,
    /// The Keycloak token file exists.
    pub keycloak_signed_in: bool,
    /// The Desktop fallback's token file is the one in use.
    pub desktop_fallback: bool,
}

/// `token_info`.
pub fn token_info(cfg: &Config) -> TokenInfo {
    let env = read_env(&cfg.google_env());
    let g: Value = if cfg.token_file().exists() {
        fs::read_to_string(cfg.token_file())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| json!({"_bad": true}))
    } else {
        json!({})
    };
    TokenInfo {
        google_env: cfg.google_env().exists(),
        google_env_keys: env.get("GOOGLE_CLIENT_ID").is_some_and(|s| !s.is_empty())
            && env.get("GOOGLE_CLIENT_SECRET").is_some_and(|s| !s.is_empty()),
        keycloak_signed_in: cfg.kc_token_file().exists(),
        desktop_fallback: py::truthy(Some(&g)) && g.get("client_id").is_some(),
    }
}
