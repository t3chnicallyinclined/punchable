"""The Keycloak -> broker -> Google chain, with every network call faked. No real token is used or printed."""
from __future__ import annotations

import json

import pytest

from gsync import auth
from gsync.config import Config


class FakeCreds:
    refreshed: list = []
    fail: set = set()

    def __init__(self, rt):
        self.refresh_token = rt

    def refresh(self, _req):
        from google.auth.exceptions import RefreshError
        if self.refresh_token in FakeCreds.fail:
            raise RefreshError("invalid_grant")
        FakeCreds.refreshed.append(self.refresh_token)


@pytest.fixture
def cfg(tmp_path, monkeypatch):
    c = Config(comedy=tmp_path / "c", home=tmp_path / "home", auth_url="https://kc.example/realms/punchable")
    c.home.mkdir()
    c.google_env.write_text("GOOGLE_CLIENT_ID=abc.apps.googleusercontent.com\nGOOGLE_CLIENT_SECRET=s3cr3t-value\n")
    calls = {"kc": 0, "broker": 0}

    def post(url, data, timeout=30):
        assert url.endswith("/protocol/openid-connect/token") and data["client_id"] == "pk-gsync"
        calls["kc"] += 1
        return 200, {"access_token": "KC-AT", "refresh_token": f"KC-RT-{calls['kc']}"}

    def broker(cfg_, tok):
        assert tok == "KC-AT"
        calls["broker"] += 1
        return {"access_token": "G-AT", "refresh_token": "G-RT-from-keycloak"}
    monkeypatch.setattr(auth, "_post", post)
    monkeypatch.setattr(auth, "broker_google_token", broker)
    monkeypatch.setattr(auth, "_creds", lambda cfg_, rt, cid, sec: FakeCreds(rt))
    monkeypatch.setattr(auth, "restrict", lambda p: True)
    FakeCreds.refreshed, FakeCreds.fail = [], set()
    return c, calls


def test_not_signed_in_is_none(cfg):
    c, _ = cfg
    assert auth.load_creds(c) is None


def test_chain_rotates_keycloak_token_and_caches_google(cfg):
    c, calls = cfg
    c.kc_token_file.write_text(json.dumps({"refresh_token": "KC-RT-0"}))
    cr = auth.load_creds(c)
    assert cr.refresh_token == "G-RT-from-keycloak" and calls == {"kc": 1, "broker": 1}
    assert json.loads(c.kc_token_file.read_text())["refresh_token"] == "KC-RT-1", "rotated refresh token not saved"
    assert json.loads(c.token_file.read_text())["refresh_token"] == "G-RT-from-keycloak"
    auth.load_creds(c)  # within a day: Google directly, Keycloak untouched
    assert calls == {"kc": 1, "broker": 1}


def test_revoked_google_token_falls_back_to_keycloak(cfg):
    c, calls = cfg
    c.kc_token_file.write_text(json.dumps({"refresh_token": "KC-RT-0"}))
    import time
    c.token_file.write_text(json.dumps({"refresh_token": "OLD", "kc_checked_at": int(time.time())}))
    FakeCreds.fail = {"OLD"}
    cr = auth.load_creds(c)
    assert cr.refresh_token == "G-RT-from-keycloak" and calls["broker"] == 1


def test_missing_env_names_the_fix(cfg):
    c, _ = cfg
    c.google_env.unlink()
    c.kc_token_file.write_text(json.dumps({"refresh_token": "KC-RT-0"}))
    with pytest.raises(auth.AuthMissing, match="google-oauth.env"):
        auth.load_creds(c)


def test_status_never_prints_secrets(cfg, capsys, monkeypatch):
    c, _ = cfg
    c.kc_token_file.write_text(json.dumps({"refresh_token": "KC-RT-0"}))
    from gsync import __main__ as m
    from gsync.punchable import Punchable
    monkeypatch.setattr(Punchable, "health", lambda self: True)
    m.cmd_status(c)
    out = capsys.readouterr().out
    for s in ("s3cr3t-value", "KC-RT", "G-RT", "KC-AT"):
        assert s not in out
    assert "Google token       ok" in out
