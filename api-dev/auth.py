"""Keycloak bearer tokens for the Punchable API (standard library only, so the image needs no crypto wheel).

    PK_AUTH=1                      every route except /api/health needs a token (the hosted default)
    OIDC_ISSUER                    https://auth.punchable.io/realms/punchable  (exact match on `iss`)
    OIDC_AUDIENCE                  pk-api                                      (must be in `aud`)
    OIDC_JWKS_URL                  where the signing keys are (in the cluster: Keycloak's Service, no hairpin)

Checks, in order: three base64url parts; header `alg` is exactly RS256 (no `none`, no HS*, no key in the header);
`kid` names a key in the JWKS (an unknown kid refetches the JWKS once, at most every 30 s); the RSASSA-PKCS1-v1_5
SHA-256 signature, compared as the whole encoded block (no parsing of the padding, so no Bleichenbacher-style
forgery); `iss` exact; `aud` contains the audience; `exp`, `nbf`, `iat` with at most 60 s of clock skew; `typ`, when
present, is `Bearer` (a refresh or ID token is refused).
The token never appears in a URL: HTTP routes take `Authorization: Bearer`, and /api/live takes it in the first
WebSocket message.
"""
from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
import threading
import time
import urllib.request

LEEWAY_S = 60
MIN_REFETCH_S = 30
JWKS_TTL_S = 3600
# DER prefix of DigestInfo(SHA-256) for EMSA-PKCS1-v1_5 (RFC 8017 section 9.2, note 1)
SHA256_PREFIX = bytes.fromhex("3031300d060960864801650304020105000420")
WORKER_ROLE = "worker"


class AuthError(Exception):
    def __init__(self, msg: str, status: int = 401):
        super().__init__(msg)
        self.status = status


def b64d(s: str) -> bytes:
    if not isinstance(s, str) or not s or any(c not in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_" for c in s):
        raise AuthError("malformed token")
    return base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))


def b64e(b: bytes) -> str:
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def rs256_verify(n: int, e: int, signing_input: bytes, sig: bytes) -> bool:
    k = (n.bit_length() + 7) // 8
    if len(sig) != k:
        return False
    s = int.from_bytes(sig, "big")
    if s >= n:
        return False
    em = pow(s, e, n).to_bytes(k, "big")
    t = SHA256_PREFIX + hashlib.sha256(signing_input).digest()
    if k < len(t) + 11:
        return False
    expected = b"\x00\x01" + b"\xff" * (k - len(t) - 3) + b"\x00" + t
    return hmac.compare_digest(em, expected)


class JWKS:
    """Signing keys by kid, cached; an unknown kid refetches (rate-limited, so a flood of junk kids can't hammer
    Keycloak)."""

    def __init__(self, url: str, fetch=None, clock=time.time):
        self.url = url
        self._fetch = fetch or self._http
        self.clock = clock
        self.keys: dict[str, tuple[int, int]] = {}
        self.fetched_at = 0.0
        self.tried_at = float("-inf")
        self.fetches = 0
        self.lock = threading.Lock()

    def _http(self, url: str) -> dict:
        with urllib.request.urlopen(urllib.request.Request(url, headers={"Accept": "application/json"}), timeout=10) as r:
            return json.loads(r.read().decode("utf-8"))

    def refresh(self) -> None:
        doc = self._fetch(self.url)
        keys = {}
        for k in doc.get("keys", []) if isinstance(doc, dict) else []:
            if not isinstance(k, dict) or k.get("kty") != "RSA" or not isinstance(k.get("kid"), str):
                continue
            if k.get("use") not in (None, "sig") or k.get("alg") not in (None, "RS256"):
                continue
            try:
                n = int.from_bytes(b64d(k["n"]), "big")
                e = int.from_bytes(b64d(k["e"]), "big")
            except (AuthError, KeyError, ValueError):
                continue
            if n.bit_length() < 2048 or e < 3:
                continue
            keys[k["kid"]] = (n, e)
        self.keys = keys
        self.fetched_at = self.clock()
        self.fetches += 1

    def get(self, kid: str) -> tuple[int, int] | None:
        with self.lock:
            now = self.clock()
            stale = now - self.fetched_at > JWKS_TTL_S
            if (kid not in self.keys or stale) and now - self.tried_at >= MIN_REFETCH_S:
                self.tried_at = now
                try:
                    self.refresh()
                except Exception:  # noqa: BLE001  keep the cached keys if Keycloak is unreachable
                    pass
            return self.keys.get(kid)


class Verifier:
    def __init__(self, issuer: str, audience: str, jwks: JWKS, clock=time.time, leeway: int = LEEWAY_S):
        self.issuer, self.audience, self.jwks, self.clock, self.leeway = issuer, audience, jwks, clock, leeway

    def verify(self, token: str) -> dict:
        if not isinstance(token, str) or token.count(".") != 2 or len(token) > 16384:
            raise AuthError("malformed token")
        h64, p64, s64 = token.split(".")
        try:
            header = json.loads(b64d(h64))
            claims = json.loads(b64d(p64))
        except ValueError:
            raise AuthError("malformed token")
        if not isinstance(header, dict) or not isinstance(claims, dict):
            raise AuthError("malformed token")
        if header.get("alg") != "RS256":
            raise AuthError("unsupported token algorithm")
        kid = header.get("kid")
        if not isinstance(kid, str):
            raise AuthError("token has no key id")
        key = self.jwks.get(kid)
        if key is None:
            raise AuthError("unknown signing key")
        if not rs256_verify(key[0], key[1], f"{h64}.{p64}".encode("ascii"), b64d(s64)):
            raise AuthError("bad signature")
        if claims.get("iss") != self.issuer:
            raise AuthError("wrong issuer")
        aud = claims.get("aud")
        auds = aud if isinstance(aud, list) else [aud]
        if self.audience not in auds:
            raise AuthError("wrong audience")
        now = self.clock()
        exp = claims.get("exp")
        if not isinstance(exp, (int, float)) or isinstance(exp, bool) or now > exp + self.leeway:
            raise AuthError("token expired")
        nbf = claims.get("nbf")
        if isinstance(nbf, (int, float)) and now + self.leeway < nbf:
            raise AuthError("token not yet valid")
        iat = claims.get("iat")
        if isinstance(iat, (int, float)) and iat > now + self.leeway:
            raise AuthError("token issued in the future")
        if "typ" in claims and claims["typ"] != "Bearer":
            raise AuthError("not an access token")
        return claims


def roles(claims: dict) -> set[str]:
    ra = claims.get("realm_access")
    rs = ra.get("roles") if isinstance(ra, dict) else None
    return {r for r in rs if isinstance(r, str)} if isinstance(rs, list) else set()


def is_worker(claims: dict) -> bool:
    return WORKER_ROLE in roles(claims)


def bearer(header_value: str | None) -> str | None:
    if not header_value:
        return None
    parts = header_value.strip().split(" ", 1)
    if len(parts) != 2 or parts[0].lower() != "bearer" or not parts[1].strip():
        return None
    return parts[1].strip()


def from_env() -> Verifier | None:
    """The verifier when PK_AUTH=1, else None (local dev: open, loopback only)."""
    if os.environ.get("PK_AUTH", "0") != "1":
        return None
    iss = os.environ.get("OIDC_ISSUER")
    aud = os.environ.get("OIDC_AUDIENCE", "pk-api")
    if not iss:
        raise SystemExit("PK_AUTH=1 needs OIDC_ISSUER")
    url = os.environ.get("OIDC_JWKS_URL") or iss.rstrip("/") + "/protocol/openid-connect/certs"
    return Verifier(iss, aud, JWKS(url))
