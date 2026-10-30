"""Test-only token kit: a locally generated RSA keypair, a JWKS, and RS256 tokens. No real Keycloak, no real tokens."""
from __future__ import annotations

import hashlib
import json
import random
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from auth import SHA256_PREFIX, b64e  # noqa: E402

ISS = "https://auth.punchable.io/realms/punchable"
AUD = "pk-api"
_SMALL = [p for p in range(3, 2000, 2) if all(p % q for q in range(3, int(p ** 0.5) + 1, 2))]


def _probable_prime(n: int, rng: random.Random) -> bool:
    if any(n % p == 0 for p in _SMALL):
        return n in _SMALL
    d, r = n - 1, 0
    while d % 2 == 0:
        d //= 2
        r += 1
    for _ in range(24):
        a = rng.randrange(2, n - 2)
        x = pow(a, d, n)
        if x in (1, n - 1):
            continue
        for _ in range(r - 1):
            x = pow(x, 2, n)
            if x == n - 1:
                break
        else:
            return False
    return True


def _prime(bits: int, rng: random.Random) -> int:
    while True:
        c = rng.getrandbits(bits) | (1 << (bits - 1)) | (1 << (bits - 2)) | 1
        if _probable_prime(c, rng):
            return c


class Key:
    def __init__(self, kid: str, bits: int = 2048, seed: int | None = None):
        rng = random.Random(seed)
        e = 65537
        while True:
            p, q = _prime(bits // 2, rng), _prime(bits // 2, rng)
            phi = (p - 1) * (q - 1)
            if p != q and phi % e and (p * q).bit_length() == bits:
                break
        self.kid, self.n, self.e, self.d = kid, p * q, e, pow(e, -1, phi)

    def jwk(self) -> dict:
        nb = self.n.to_bytes((self.n.bit_length() + 7) // 8, "big")
        return {"kty": "RSA", "kid": self.kid, "use": "sig", "alg": "RS256", "n": b64e(nb), "e": b64e(self.e.to_bytes(3, "big"))}

    def sign(self, msg: bytes) -> bytes:
        k = (self.n.bit_length() + 7) // 8
        t = SHA256_PREFIX + hashlib.sha256(msg).digest()
        em = b"\x00\x01" + b"\xff" * (k - len(t) - 3) + b"\x00" + t
        return pow(int.from_bytes(em, "big"), self.d, self.n).to_bytes(k, "big")


def token(key: Key, now: float | None = None, header: dict | None = None, **claims) -> str:
    now = time.time() if now is None else now
    h = {"alg": "RS256", "typ": "JWT", "kid": key.kid, **(header or {})}
    c = {"iss": ISS, "aud": [AUD, "account"], "sub": "u-owner", "azp": "pk-web", "typ": "Bearer",
         "iat": int(now), "exp": int(now) + 300, **claims}
    c = {k: v for k, v in c.items() if v is not None}
    si = b64e(json.dumps(h).encode()) + "." + b64e(json.dumps(c).encode())
    return si + "." + b64e(key.sign(si.encode()))


def worker_token(key: Key, **claims) -> str:
    return token(key, azp="pk-worker", sub="service-account-pk-worker", realm_access={"roles": ["worker"]}, **claims)


class JWKSServer:
    """Serves {"keys": [...]} at http://127.0.0.1:<port>/certs; `keys` can be swapped (rotation)."""

    def __init__(self, keys: list[Key]):
        self.keys = keys
        self.hits = 0
        outer = self

        class H(BaseHTTPRequestHandler):
            def do_GET(self):
                outer.hits += 1
                b = json.dumps({"keys": [k.jwk() for k in outer.keys]}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(b)))
                self.end_headers()
                self.wfile.write(b)

            def log_message(self, *a):
                pass

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.url = f"http://127.0.0.1:{self.srv.server_address[1]}/certs"
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()

    def close(self):
        self.srv.shutdown()


def auth_env(jwks_url: str) -> dict:
    return {"PK_AUTH": "1", "OIDC_ISSUER": ISS, "OIDC_AUDIENCE": AUD, "OIDC_JWKS_URL": jwks_url}
