"""Keycloak token checks (api-dev/auth.py) and their use on every route. Locally generated keys only.

    python -m pytest api-dev/tests -q

Unit: the verifier on its own (fake clock, fake JWKS). HTTP: a real API process (files backend, TEMP copies of the
data) with PK_AUTH=1 and a local JWKS server: an expired token, a wrong audience, a wrong issuer, a tampered
signature and a missing token are each 401 on a real route; a good token is 200; worker and app tokens are kept to
their own routes; /api/health stays open; a token in the URL is ignored.
"""
from __future__ import annotations

import json
import subprocess
import sys
import time
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE.parents[1] / "db" / "tests"))
import auth as A  # noqa: E402
from jwt_kit import AUD, ISS, JWKSServer, Key, auth_env, token, worker_token  # noqa: E402

KEY = Key("k1", seed=1)
KEY2 = Key("k2", seed=2)
OTHER = Key("k1", seed=3)  # same kid, different key: a forged signature


class FakeJWKS:
    def __init__(self, keys):
        self.keys, self.calls = keys, 0

    def __call__(self, url):
        self.calls += 1
        return {"keys": [k.jwk() for k in self.keys]}


def verifier(keys=(KEY,), now=None):
    clock = [now or time.time()]
    fetch = FakeJWKS(list(keys))
    v = A.Verifier(ISS, AUD, A.JWKS("http://jwks.invalid/certs", fetch=fetch, clock=lambda: clock[0]), clock=lambda: clock[0])
    return v, fetch, clock


def deny(v, t, why):
    with pytest.raises(A.AuthError) as e:
        v.verify(t)
    assert why in str(e.value), str(e.value)


# ---------------------------------------------------------------- unit
def test_good_token():
    v, _, _ = verifier()
    c = v.verify(token(KEY))
    assert c["iss"] == ISS and AUD in c["aud"]


def test_audience_as_string():
    v, _, _ = verifier()
    assert v.verify(token(KEY, aud=AUD))["aud"] == AUD


def test_expired_and_skew():
    v, _, clock = verifier()
    now = clock[0]
    deny(v, token(KEY, now=now - 400), "expired")          # exp = now - 100: past the 60 s skew
    v.verify(token(KEY, now=now - 330))                    # exp = now - 30: inside the skew
    deny(v, token(KEY, now=now, exp=int(now) - 61), "expired")


def test_not_yet_valid_and_future_iat():
    v, _, clock = verifier()
    deny(v, token(KEY, nbf=int(clock[0]) + 120), "not yet valid")
    deny(v, token(KEY, now=clock[0] + 120), "future")
    v.verify(token(KEY, nbf=int(clock[0]) + 30))


def test_wrong_audience_issuer():
    v, _, _ = verifier()
    deny(v, token(KEY, aud=["account"]), "audience")
    deny(v, token(KEY, aud=None), "audience")
    deny(v, token(KEY, iss="https://auth.punchable.io/realms/master"), "issuer")
    deny(v, token(KEY, iss=ISS + "/"), "issuer")


def test_tampered_signature_and_payload():
    v, _, _ = verifier()
    t = token(KEY)
    h, p, s = t.split(".")
    body = json.loads(A.b64d(p))
    body["sub"] = "someone-else"
    deny(v, f"{h}.{A.b64e(json.dumps(body).encode())}.{s}", "signature")   # payload changed, old signature
    sig = bytearray(A.b64d(s))
    sig[-1] ^= 1
    deny(v, f"{h}.{p}.{A.b64e(bytes(sig))}", "signature")                  # one bit of the signature
    deny(v, token(OTHER), "signature")                                     # right kid, wrong key


def test_alg_confusion():
    v, _, _ = verifier()
    t = token(KEY)
    h, p, s = t.split(".")
    none = A.b64e(json.dumps({"alg": "none", "kid": "k1"}).encode())
    deny(v, f"{none}.{p}.", "algorithm")
    hs = A.b64e(json.dumps({"alg": "HS256", "kid": "k1"}).encode())
    deny(v, f"{hs}.{p}.{s}", "algorithm")
    nokid = A.b64e(json.dumps({"alg": "RS256"}).encode())
    deny(v, f"{nokid}.{p}.{s}", "key id")
    deny(v, "not.a.jwt!", "malformed")
    deny(v, "abc", "malformed")


def test_refresh_and_id_tokens_refused():
    v, _, _ = verifier()
    deny(v, token(KEY, typ="Refresh"), "access token")
    deny(v, token(KEY, typ="ID"), "access token")


def test_unknown_kid_refetches_once_rate_limited():
    v, fetch, clock = verifier(keys=(KEY,))
    v.verify(token(KEY))
    assert fetch.calls == 1
    v.verify(token(KEY))
    assert fetch.calls == 1                         # cached
    fetch.keys.append(KEY2)                         # Keycloak rotates in a new key
    clock[0] += 31
    v.verify(token(KEY2, now=clock[0]))             # unknown kid -> one refetch -> accepted
    assert fetch.calls == 2
    deny(v, token(Key("k9", seed=9), now=clock[0]), "unknown signing key")
    deny(v, token(Key("k8", seed=8), now=clock[0]), "unknown signing key")
    assert fetch.calls == 2                         # junk kids within 30 s: no more fetches


def test_cross_check_with_cryptography():
    """Our verifier accepts what a standard RSA library signs, and rejects a standard signature over other bytes."""
    crypto = pytest.importorskip("cryptography.hazmat.primitives.asymmetric.rsa")
    from cryptography.hazmat.primitives import hashes
    from cryptography.hazmat.primitives.asymmetric import padding
    pk = crypto.generate_private_key(public_exponent=65537, key_size=2048)
    nums = pk.public_key().public_numbers()
    msg = b"header.payload"
    sig = pk.sign(msg, padding.PKCS1v15(), hashes.SHA256())
    assert A.rs256_verify(nums.n, nums.e, msg, sig)
    assert not A.rs256_verify(nums.n, nums.e, b"header.payloaD", sig)
    sig2 = KEY.sign(msg)  # and the library verifies ours
    from cryptography.hazmat.primitives.asymmetric.rsa import RSAPublicNumbers
    RSAPublicNumbers(KEY.e, KEY.n).public_key().verify(sig2, msg, padding.PKCS1v15(), hashes.SHA256())


def test_roles_and_bearer_parsing():
    assert A.is_worker({"realm_access": {"roles": ["worker"]}})
    assert not A.is_worker({"realm_access": {"roles": "worker"}})
    assert not A.is_worker({})
    assert A.bearer("Bearer abc") == "abc" and A.bearer("bearer abc") == "abc"
    assert A.bearer("Basic abc") is None and A.bearer("Bearer ") is None and A.bearer(None) is None


# ---------------------------------------------------------------- HTTP, a real API process
@pytest.fixture(scope="module")
def api():
    from common import Api, TempKB
    jw = JWKSServer([KEY])
    t = TempKB("auth")
    a = Api(t, "files", env=auth_env(jw.url))
    yield a, jw
    a.stop()
    jw.close()
    t.cleanup()


def H(t):
    return {"Authorization": "Bearer " + t}


def test_http_the_five_401s(api):
    a, _ = api
    now = time.time()
    cases = {
        "missing": {},
        "expired": H(token(KEY, now=now - 3600)),
        "wrong audience": H(token(KEY, aud=["account"])),
        "wrong issuer": H(token(KEY, iss="https://evil.example/realms/punchable")),
        "tampered signature": H(token(OTHER)),
    }
    for path in ("/api/meta", "/api/boards", "/api/today"):
        for name, hdr in cases.items():
            st, body, headers = a.get(path, raw=True, headers=hdr)
            assert st == 401, (path, name, st, body[:200])
            assert headers.get("WWW-Authenticate", "").startswith("Bearer"), (path, name)
    st, _ = a.post("/api/decision", {"target": "node", "action": "confirm", "id": "joke:x"})
    assert st == 401
    st, _ = a.post("/api/boards/sb:x", {"title": "x"}, method="PATCH")
    assert st == 401


def test_http_good_token(api):
    a, _ = api
    st, body = a.get("/api/meta", headers=H(token(KEY)))
    assert st == 200 and body["counts"]["jokes"] > 0
    st, body = a.get("/api/boards", headers=H(token(KEY)))
    assert st == 200 and "boards" in body


def test_http_health_open_but_quiet(api):
    a, _ = api
    st, body = a.get("/api/health")
    assert st == 200 and body["ok"] is True and "graph" not in body and "inbox" not in body
    st, body = a.get("/api/health", headers=H(token(KEY)))
    assert st == 200 and "graph" in body


def test_http_token_in_url_is_ignored(api):
    a, _ = api
    st, _ = a.get("/api/meta?access_token=" + token(KEY))
    assert st == 401


def test_http_worker_and_app_routes_are_separate(api):
    a, _ = api
    st, _ = a.post("/api/worker/claim", {}, headers=H(token(KEY)))
    assert st == 403
    st, _ = a.get("/api/meta", headers=H(worker_token(KEY)))
    assert st == 403
    st, body, _ = a.get("/api/worker/decisions?after=0", raw=True, headers=H(worker_token(KEY)))
    assert st == 200 and json.loads(body)["total"] >= 0
    # his voice profile: the app's token reads and writes /api/profile/voice; the worker's token only its own copy
    st, body = a.post("/api/profile/voice", {"text": "# Voice

He talks fast."}, method="PUT", headers=H(token(KEY)))
    assert st == 200 and body["by"] == "owner" and body["text"].startswith("# Voice")
    st, body, _ = a.get("/api/worker/profile/voice", raw=True, headers=H(worker_token(KEY)))
    assert st == 200 and json.loads(body)["text"].startswith("# Voice") and "previous" not in json.loads(body)
    st, _, _ = a.get("/api/worker/profile/voice", raw=True, headers=H(token(KEY)))
    assert st == 403
    st, _ = a.post("/api/worker/profile/voice", {"text": "x", "actor": "import:profile"}, method="PUT", headers=H(token(KEY)))
    assert st == 403
    st, _ = a.post("/api/profile/voice", {"text": "x"}, method="PUT", headers=H(worker_token(KEY)))
    assert st == 403


NEW_ROUTES = [  # docs/API_CONTRACT.md "Requested by app (2026-10-26)"
    ("GET", "/api/settings", None), ("PATCH", "/api/settings", {"cellular": True}), ("GET", "/api/learned", None),
    ("GET", "/api/export", None), ("DELETE", "/api/me", {"confirm": "delete my material"}),
    ("POST", "/api/import/youtube", {"url": "https://youtu.be/dQw4w9WgXcQ"}),
    ("POST", "/api/jokes/joke:airport-bit/punchup", {}), ("POST", "/api/workingsets/ws:x/try", {"text": "x"}),
    ("PATCH", "/api/workingsets/ws:x/try/try:x", {"done": True}), ("GET", "/api/jokes?sort=strongest", None),
    ("GET", "/api/connections", None),
    ("GET", "/api/profile/voice", None), ("PUT", "/api/profile/voice", {"text": "# Voice

He talks fast."}),
]


def _call(a, method, path, body, headers):
    if method == "GET":
        st, _, _ = a.get(path, raw=True, headers=headers)
        return st
    return a.post(path, body, method=method, headers=headers)[0]


def test_http_new_routes_need_a_token(api):
    a, _ = api
    for method, path, body in NEW_ROUTES:
        assert _call(a, method, path, body, {}) == 401, (method, path)
        assert _call(a, method, path, body, H(token(OTHER))) == 401, (method, path)
        assert _call(a, method, path, body, H(worker_token(KEY))) == 403, (method, path)


def test_http_hosted_delete_only_records_a_request(api):
    a, _ = api
    root = Path(a.log.name).parent
    before = {p: p.stat().st_size for p in root.rglob("*") if p.is_file() and not p.name.startswith("api_")}
    st, body = a.post("/api/me", {}, method="DELETE", headers=H(token(KEY)))
    assert st == 400
    st, body = a.post("/api/me", {"confirm": "delete my material"}, method="DELETE", headers=H(token(KEY)))
    assert st == 202 and body["status"] == "requested" and body["deleted"] is False and body["would_delete"]
    after = {p: p.stat().st_size for p in root.rglob("*") if p.is_file() and not p.name.startswith("api_")}
    assert all(p in after for p in before), "a file was deleted"
    changed = sorted(p.name for p in after if before.get(p) != after[p])
    assert set(changed) <= {"account.jsonl", "account.json"}, changed  # only the request was recorded
    log = (root / "creative" / "account.jsonl").read_text(encoding="utf-8").splitlines()
    assert json.loads(log[-1])["op"] == "request_deletion"


def test_refuses_public_bind_without_auth():
    p = subprocess.run([sys.executable, str(HERE.parent / "server.py"), "--host", "0.0.0.0", "--port", "1", "--backend", "files"],
                       capture_output=True, text=True, timeout=60, env={k: v for k, v in __import__("os").environ.items() if k != "PK_AUTH"})
    assert p.returncode != 0 and "PK_AUTH" in (p.stderr + p.stdout)
