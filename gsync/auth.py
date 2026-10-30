"""Google credentials for gsync, from Punchable's "Sign in with Google" (Keycloak IdP `google`, storeToken on).

One time:  `python -m gsync auth` = OAuth device flow against Keycloak client pk-gsync (he signs in with Google in the
           browser). gsync keeps an OFFLINE Keycloak refresh token (rotated on every use) in
           %USERPROFILE%\\.punchable\\keycloak_gsync_token.json.
Each run:  refresh the cached Google token against Google (client id/secret from google-oauth.env). Only when there is
           no cached Google refresh token, or Google refuses it, gsync asks Keycloak for the stored one:
           GET <realm>/broker/google/token with a pk-gsync access token (resource_access.broker has read-token).
           Keycloak is touched at least daily so its offline session never idles out.
Fallback:  `python -m gsync auth --desktop` with a separate Desktop-app client (google_client.json), as before.
Every token file is owner-only (icacls); no token or secret is ever printed."""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import webbrowser
from pathlib import Path

from .config import Config

GOOGLE_TOKEN_URI = "https://oauth2.googleapis.com/token"
KC_DAILY_S = 86400


class AuthMissing(RuntimeError):
    pass


def restrict(p: Path) -> bool:
    """Owner-only: drop inherited ACEs, grant the current user full control. Windows only (icacls)."""
    if sys.platform != "win32":
        os.chmod(p, 0o600)
        return True
    user = os.environ.get("USERNAME") or os.getlogin()
    r = subprocess.run(["icacls", str(p), "/inheritance:r", "/grant:r", f"{user}:F"], capture_output=True, text=True)
    return r.returncode == 0


def write_secret(p: Path, text: str) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    tmp = p.with_suffix(".tmp")
    tmp.write_text("", encoding="utf-8")
    restrict(tmp)  # locked down before the secret goes in
    tmp.write_text(text, encoding="utf-8")
    os.replace(tmp, p)
    restrict(p)


def read_env(p: Path) -> dict:
    out = {}
    if p.exists():
        for ln in p.read_text(encoding="utf-8").splitlines():
            if "=" in ln and not ln.lstrip().startswith("#"):
                k, v = ln.split("=", 1)
                out[k.strip()] = v.strip().strip('"').strip("'")
    return out


def _post(url: str, data: dict, timeout: float = 30) -> tuple[int, dict]:
    req = urllib.request.Request(url, data=urllib.parse.urlencode(data).encode(), method="POST",
                                 headers={"Content-Type": "application/x-www-form-urlencoded"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read().decode())
        except ValueError:
            return e.code, {}


# ---- Keycloak (pk-gsync) -------------------------------------------------------------------------------------------
def kc_device_login(cfg: Config, opener=webbrowser.open, sleep=time.sleep) -> str:
    base = cfg.auth_url.rstrip("/") + "/protocol/openid-connect"
    s, d = _post(base + "/auth/device", {"client_id": cfg.kc_client, "scope": "openid offline_access"})
    if s != 200:
        raise SystemExit(f"Keycloak refused the device login ({s} {d.get('error')}); is the realm applied? (SETUP.md)")
    url = d.get("verification_uri_complete") or d["verification_uri"]
    print(f"Sign in at {url}\n  code: {d['user_code']}  (choose 'Sign in with Google')")
    opener(url)
    interval, deadline = int(d.get("interval", 5)), time.time() + int(d.get("expires_in", 600))
    while time.time() < deadline:
        sleep(interval)
        s, t = _post(base + "/token", {"grant_type": "urn:ietf:params:oauth:grant-type:device_code",
                                       "device_code": d["device_code"], "client_id": cfg.kc_client})
        if s == 200:
            _save_kc(cfg, t)
            return str(cfg.kc_token_file)
        if t.get("error") == "slow_down":
            interval += 5
        elif t.get("error") != "authorization_pending":
            raise SystemExit(f"sign-in failed: {t.get('error')} {t.get('error_description', '')}")
    raise SystemExit("sign-in timed out; run `python -m gsync auth` again")


def _save_kc(cfg: Config, t: dict) -> None:
    if not t.get("refresh_token"):
        raise SystemExit("Keycloak returned no refresh token (offline_access missing?)")
    write_secret(cfg.kc_token_file, json.dumps({"refresh_token": t["refresh_token"], "issuer": cfg.auth_url,
                                                "client_id": cfg.kc_client, "saved_at": int(time.time())}))


def kc_access_token(cfg: Config) -> str:
    """A fresh pk-gsync access token. Rotation is on, so the new refresh token is saved before anything else."""
    if not cfg.kc_token_file.exists():
        raise AuthMissing("not signed in: run `python -m gsync auth`")
    kc = json.loads(cfg.kc_token_file.read_text(encoding="utf-8"))
    s, t = _post(cfg.auth_url.rstrip("/") + "/protocol/openid-connect/token",
                 {"grant_type": "refresh_token", "client_id": cfg.kc_client, "refresh_token": kc["refresh_token"]})
    if s != 200:
        raise AuthMissing(f"Keycloak sign-in expired ({t.get('error')}): run `python -m gsync auth`")
    _save_kc(cfg, t)
    return t["access_token"]


def broker_google_token(cfg: Config, kc_token: str) -> dict:
    """Keycloak's stored Google token response (needs resource_access.broker.read-token in kc_token)."""
    req = urllib.request.Request(cfg.auth_url.rstrip("/") + "/broker/google/token",
                                 headers={"Authorization": "Bearer " + kc_token})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            body = r.read().decode()
    except urllib.error.HTTPError as e:
        hint = {400: "Google is not linked to your Punchable account (SETUP.md step 9)",
                403: "this token may not read Google tokens (realm not applied?)"}.get(e.code, "")
        raise AuthMissing(f"Keycloak broker token: HTTP {e.code}. {hint}") from None
    try:
        return json.loads(body)
    except ValueError:  # some providers store form-encoded responses
        return dict(urllib.parse.parse_qsl(body))


# ---- Google credentials ---------------------------------------------------------------------------------------------
def _google_client(cfg: Config) -> tuple[str, str]:
    env = read_env(cfg.google_env)
    cid, sec = env.get("GOOGLE_CLIENT_ID"), env.get("GOOGLE_CLIENT_SECRET")
    if not cid or not sec:
        raise AuthMissing(f"missing GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET in {cfg.google_env} (SETUP.md step 5)")
    return cid, sec


def _creds(cfg: Config, refresh_token: str, cid: str, sec: str):
    from google.oauth2.credentials import Credentials
    return Credentials(token=None, refresh_token=refresh_token, token_uri=GOOGLE_TOKEN_URI, client_id=cid,
                       client_secret=sec)


def load_creds(cfg: Config, refresh: bool = True):
    """Google credentials or None (nothing set up). Raises AuthMissing with the fix when something is broken."""
    if cfg.token_file.exists() and not cfg.kc_token_file.exists():  # the --desktop fallback
        return _load_desktop(cfg, refresh)
    if not cfg.kc_token_file.exists():
        return None
    from google.auth.exceptions import RefreshError
    from google.auth.transport.requests import Request
    cid, sec = _google_client(cfg)
    cached = json.loads(cfg.token_file.read_text(encoding="utf-8")) if cfg.token_file.exists() else {}
    kc_age = time.time() - (cached.get("kc_checked_at") or 0)
    rt = cached.get("refresh_token")
    if rt and kc_age < KC_DAILY_S:
        creds = _creds(cfg, rt, cid, sec)
        try:
            creds.refresh(Request())
            return creds
        except RefreshError:
            pass  # revoked or expired: ask Keycloak for the token from his latest Google sign-in
    stored = broker_google_token(cfg, kc_access_token(cfg))  # also keeps the Keycloak offline session alive
    rt = stored.get("refresh_token") or rt
    if not rt:
        raise AuthMissing("Keycloak holds no Google refresh token: sign in with Google again (SETUP.md step 9)")
    creds = _creds(cfg, rt, cid, sec)
    creds.refresh(Request())  # RefreshError here means a dead grant: SETUP.md step 3c (Testing = 7-day tokens)
    write_secret(cfg.token_file, json.dumps({"refresh_token": rt, "source": "keycloak-broker",
                                             "kc_checked_at": int(time.time())}))
    return creds


def _load_desktop(cfg: Config, refresh: bool):
    from google.auth.transport.requests import Request
    from google.oauth2.credentials import Credentials
    creds = Credentials.from_authorized_user_file(str(cfg.token_file), cfg.scopes)
    if refresh and not creds.valid and creds.refresh_token:
        creds.refresh(Request())
        write_secret(cfg.token_file, creds.to_json())
    return creds


def run_auth(cfg: Config, desktop: bool = False) -> str:
    if not desktop:
        _google_client(cfg)  # fail early with the fix if the env file is missing
        path = kc_device_login(cfg)
        load_creds(cfg)  # proves the whole chain: Keycloak -> broker token -> Google refresh
        return path
    if not cfg.client_file.exists():
        raise SystemExit(f"missing Desktop OAuth client file: {cfg.client_file} (SETUP.md, fallback)")
    from google_auth_oauthlib.flow import InstalledAppFlow
    restrict(cfg.client_file)
    flow = InstalledAppFlow.from_client_secrets_file(str(cfg.client_file), cfg.scopes)
    creds = flow.run_local_server(port=0, open_browser=True, access_type="offline", prompt="consent",
                                  authorization_prompt_message="Opening your browser to sign in to Google...",
                                  success_message="Punchable is connected to Google Drive. You can close this tab.")
    if not creds.refresh_token:
        raise SystemExit("Google returned no refresh token; run `python -m gsync auth --desktop` again")
    write_secret(cfg.token_file, creds.to_json())
    return str(cfg.token_file)


def token_info(cfg: Config) -> dict:
    """What `status` shows: facts about the files, never their values."""
    env = read_env(cfg.google_env)
    g = {}
    if cfg.token_file.exists():
        try:
            g = json.loads(cfg.token_file.read_text(encoding="utf-8"))
        except ValueError:
            g = {"_bad": True}
    return {"google_env": cfg.google_env.exists(),
            "google_env_keys": bool(env.get("GOOGLE_CLIENT_ID")) and bool(env.get("GOOGLE_CLIENT_SECRET")),
            "keycloak_signed_in": cfg.kc_token_file.exists(),
            "google_cached": bool(g.get("refresh_token")), "desktop_fallback": bool(g) and "client_id" in g}
