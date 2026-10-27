#!/usr/bin/env python3
"""WORKSTREAM P3 acceptance tests against a THROWAWAY Keycloak (test-realm.sh starts it). Stdlib only.

usage: test_realm.py <base http url> <admin user env> <admin pass env> <expected frontend url> <domain>
Exit 0 only if every check passes. Never run against production: it creates a test user with a password.
"""
import base64, hashlib, html, http.cookiejar, json, os, re, secrets, sys, urllib.error, urllib.parse, urllib.request

BASE, EXPECT_FRONT = sys.argv[1].rstrip("/"), sys.argv[4]
AU, AP = os.environ[sys.argv[2]], os.environ[sys.argv[3]]
REALM = "punchable"
fails = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (("  -- " + detail) if detail and not ok else ""))
    if not ok:
        fails.append(name)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None


def req(url, data=None, headers=None, method=None, opener=None):
    h = dict(headers or {})
    body = urllib.parse.urlencode(data).encode() if isinstance(data, dict) else data
    r = urllib.request.Request(url, data=body, headers=h, method=method)
    try:
        resp = (opener or urllib.request.build_opener(NoRedirect)).open(r, timeout=30)
        return resp.status, dict(resp.headers), resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read().decode("utf-8", "replace")


def admin_token():
    s, _, b = req(f"{BASE}/realms/master/protocol/openid-connect/token",
                  {"grant_type": "password", "client_id": "admin-cli", "username": AU, "password": AP})
    return json.loads(b)["access_token"]


def admin(method, path, body=None):
    h = {"Authorization": "Bearer " + admin_token(), "Content-Type": "application/json"}
    return req(f"{BASE}/admin/realms/{REALM}{path}", json.dumps(body).encode() if body is not None else None, h, method)


def jwt(t):
    p = t.split(".")[1]
    return json.loads(base64.urlsafe_b64decode(p + "=" * (-len(p) % 4)))


# 1. As imported: the issuer is the public frontend URL, settings match the identity plan.
s, _, b = req(f"{BASE}/realms/{REALM}/.well-known/openid-configuration")
iss = json.loads(b)["issuer"] if s == 200 else None
check("issuer is the public auth host", iss == f"{EXPECT_FRONT}/realms/{REALM}", f"got {iss}")
s, _, b = admin("GET", "")
rs = json.loads(b)
check("accessTokenLifespan == 300", rs.get("accessTokenLifespan") == 300, str(rs.get("accessTokenLifespan")))
check("refresh rotation on (revokeRefreshToken, maxReuse 0)", rs.get("revokeRefreshToken") is True and rs.get("refreshTokenMaxReuse") == 0)
check("self-registration off", rs.get("registrationAllowed") is False)
check("verifyEmail on", rs.get("verifyEmail") is True)
# "Sign in with Google" (gsync/GSYNC.md): exactly one IdP, tokens stored, refresh token requested, link-only first login.
s, _, b = admin("GET", "/identity-provider/instances")
idps = json.loads(b) if s == 200 else []
g = idps[0] if len(idps) == 1 else {}
gc = g.get("config") or {}
check("exactly one identity provider: google", [i.get("alias") for i in idps] == ["google"], str([i.get("alias") for i in idps]))
check("google IdP: storeToken + addReadTokenRoleOnCreate", g.get("storeToken") is True and g.get("addReadTokenRoleOnCreate") is True)
check("google IdP: offlineAccess + prompt=consent (refresh token every sign-in)",
      gc.get("offlineAccess") == "true" and gc.get("prompt") == "consent", str({k: gc.get(k) for k in ("offlineAccess", "prompt")}))
check("google IdP: Drive scope requested", "https://www.googleapis.com/auth/drive" in (gc.get("defaultScope") or "").split())
check("google IdP: trustEmail off (verifyEmail not bypassed)", g.get("trustEmail") is False)
check("google IdP: client secret masked by the admin API, never echoed", gc.get("clientSecret") in ("**********", None)
      and "throwaway-not-a-secret" not in json.dumps(g))
check("google IdP: first login is the link-only flow", g.get("firstBrokerLoginFlowAlias") == "pk-google-link-only")
s, _, b = admin("GET", "/authentication/flows/pk-google-link-only/executions")
ex = json.loads(b) if s == 200 else []
prov = [(x.get("providerId"), x.get("requirement")) for x in ex]
check("link-only flow: detect-existing (REQUIRED) then re-authenticate (REQUIRED), no user creation",
      prov == [("idp-detect-existing-broker-user", "REQUIRED"), ("idp-username-password-form", "REQUIRED")], str(prov))
s, _, b = admin("GET", "/roles/default-roles-punchable/composites")
dr = json.loads(b) if s == 200 else []
check("default roles include broker/read-token", any(r.get("name") == "read-token" and r.get("clientRole") for r in dr),
      str([r.get("name") for r in dr]))
s, _, b = admin("GET", "/users?max=5")
check("realm import created no users (service accounts aside)",
      s == 200 and [u for u in json.loads(b) if not u.get("username", "").startswith("service-account-")] == [], b[:200])
clients = {c["clientId"]: c for c in json.loads(admin("GET", "/clients")[2])}
for cid in ("pk-web", "pk-mobile"):
    c = clients.get(cid, {})
    check(f"{cid} is public + PKCE S256 + no password grant/implicit",
          c.get("publicClient") is True and c.get("attributes", {}).get("pkce.code.challenge.method") == "S256"
          and not c.get("directAccessGrantsEnabled") and not c.get("implicitFlowEnabled"))
check("pk-api has no login flow", not any(clients.get("pk-api", {}).get(k) for k in
      ("standardFlowEnabled", "implicitFlowEnabled", "directAccessGrantsEnabled", "serviceAccountsEnabled")))
web_redirect = clients["pk-web"]["redirectUris"]
check("pk-web redirect is exact (one URI, no wildcard)", len(web_redirect) == 1 and "*" not in web_redirect[0], str(web_redirect))

# Point the THROWAWAY at itself so the browser flow can run over http. (Production keeps the https frontend.)
admin("PUT", "", {"attributes": {"frontendUrl": BASE}})
user, pw = "flowtest@example.invalid", secrets.token_urlsafe(18) + "aA1!"
admin("POST", "/users", {"username": user, "email": user, "enabled": True, "emailVerified": True,
                         "credentials": [{"type": "password", "value": pw, "temporary": False}]})

AUTH = f"{BASE}/realms/{REALM}/protocol/openid-connect/auth"
TOKEN = f"{BASE}/realms/{REALM}/protocol/openid-connect/token"


def auth_url(client, redirect, challenge=True, verifier=None):
    q = {"client_id": client, "redirect_uri": redirect, "response_type": "code", "scope": "openid", "state": "s1"}
    if challenge:
        q.update(code_challenge=base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode(),
                 code_challenge_method="S256")
    return AUTH + "?" + urllib.parse.urlencode(q)


# 2. A request without code_challenge is refused.
DOMAIN = sys.argv[5] if len(sys.argv) > 5 else "punchable.io"
WEB_CB = f"https://{DOMAIN}/auth/callback"
s, h, b = req(auth_url("pk-web", WEB_CB, challenge=False))
loc = h.get("Location", "")
check("no code_challenge => refused", "error=" in loc and "code_challenge" in urllib.parse.unquote(loc), f"{s} {loc[:160]}")

# 3. A redirect one character off is refused (no redirect issued at all).
v = secrets.token_urlsafe(48)
for bad in (WEB_CB + "s", WEB_CB[:-1], WEB_CB.replace("https://", "https://x.")):
    s, h, b = req(auth_url("pk-web", bad, verifier=v))
    check(f"pk-web refuses redirect {bad!r}", s == 400 and "Location" not in h, f"{s} {h.get('Location','')[:80]}")
s, h, b = req(auth_url("pk-mobile", WEB_CB, verifier=v))
check("pk-mobile refuses the web redirect", s == 400, str(s))
s, h, b = req(auth_url("pk-mobile", "punchable://auth/callback", verifier=v))
check("pk-mobile accepts punchable://auth/callback (login page)", s == 200 and 'action="' in b, str(s))


def login_code(client, redirect, verifier):
    # The throwaway runs on http; Keycloak's cookies are Secure. Send them anyway (TEST ONLY).
    pol = http.cookiejar.DefaultCookiePolicy()
    pol.return_ok_secure = lambda cookie, request: True
    jar = http.cookiejar.CookieJar(pol)
    op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar), NoRedirect)
    s, h, b = req(auth_url(client, redirect, verifier=verifier), opener=op)
    m = re.search(r'action="([^"]+)"', b)
    if not m:
        return None, f"no login form ({s})"
    s, h, b = req(html.unescape(m.group(1)), {"username": user, "password": pw, "credentialId": ""}, opener=op)
    loc = h.get("Location", "")
    if not loc:
        text = re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", b))
        return None, "%s %s" % (s, text[:300])
    q = urllib.parse.parse_qs(urllib.parse.urlparse(loc).query)
    return (q.get("code") or [None])[0], f"{s} {loc[:120]}"


# 4. Full PKCE flow: token has aud pk-api, exp-iat 300, SurrealDB claims; wrong verifier refused.
code, why = login_code("pk-web", WEB_CB, v)
check("pk-web login yields a code", bool(code), why)
s, _, b = req(TOKEN, {"grant_type": "authorization_code", "client_id": "pk-web", "code": code or "x",
                      "redirect_uri": WEB_CB, "code_verifier": "wrong-" + v})
check("REAL code + wrong code_verifier => invalid_grant", bool(code) and s == 400 and "invalid_grant" in b, f"{s} {b[:120]}")
code, why = login_code("pk-web", WEB_CB, v)
s, _, b = req(TOKEN, {"grant_type": "authorization_code", "client_id": "pk-web", "code": code or "x",
                      "redirect_uri": WEB_CB, "code_verifier": v})
tok = json.loads(b) if s == 200 else {}
check("code + verifier => tokens", s == 200, f"{s} {b[:160]}")
if tok:
    a = jwt(tok["access_token"])
    aud = a.get("aud") if isinstance(a.get("aud"), list) else [a.get("aud")]
    check("access token aud contains pk-api", "pk-api" in aud, str(a.get("aud")))
    check("access token exp-iat == 300", a["exp"] - a["iat"] == 300, str(a["exp"] - a["iat"]))
    check("access token has ns/db/ac for SurrealDB", (a.get("ns"), a.get("db"), a.get("ac")) == ("punchable", "main", "keycloak"))
    check("azp is pk-web", a.get("azp") == "pk-web")
    check("id token does not carry aud pk-api", "pk-api" not in str(jwt(tok["id_token"]).get("aud")))
    # 5. Refresh rotation: the old refresh token is dead after one use.
    s1, _, b1 = req(TOKEN, {"grant_type": "refresh_token", "client_id": "pk-web", "refresh_token": tok["refresh_token"]})
    check("refresh works once", s1 == 200, f"{s1} {b1[:120]}")
    s2, _, b2 = req(TOKEN, {"grant_type": "refresh_token", "client_id": "pk-web", "refresh_token": tok["refresh_token"]})
    check("reused refresh token => invalid_grant", s2 == 400 and "invalid_grant" in b2, f"{s2} {b2[:120]}")
    check("pk-web user token does not carry the worker role", "worker" not in (a.get("realm_access") or {}).get("roles", []))
    check("pk-web token carries no broker role", "broker" not in (a.get("resource_access") or {}))
    s_, _, b_ = req(f"{BASE}/realms/{REALM}/broker/google/token", headers={"Authorization": "Bearer " + tok["access_token"]})
    check("pk-web token cannot read the stored Google token (403)", s_ == 403, f"{s_} {b_[:120]}")
# 7. The home worker: pk-worker, confidential, client credentials only; its token has aud pk-api + realm role worker.
w = clients.get("pk-worker", {})
check("pk-worker is confidential, service account on, no browser/password/implicit flow",
      w.get("publicClient") is False and w.get("serviceAccountsEnabled") is True and not w.get("standardFlowEnabled")
      and not w.get("directAccessGrantsEnabled") and not w.get("implicitFlowEnabled"), str({k: w.get(k) for k in
      ("publicClient", "serviceAccountsEnabled", "standardFlowEnabled", "directAccessGrantsEnabled")}))
if w:
    s_, _, b_ = admin("GET", f"/clients/{w['id']}/client-secret")
    wsec = json.loads(b_).get("value") if s_ == 200 else None
    check("pk-worker has a generated secret (none in the realm file)", bool(wsec) and len(wsec) >= 20)
    s_, _, b_ = req(TOKEN, {"grant_type": "client_credentials", "client_id": "pk-worker", "client_secret": wsec or "x"})
    check("pk-worker client credentials => token", s_ == 200, f"{s_} {b_[:160]}")
    if s_ == 200:
        wt = jwt(json.loads(b_)["access_token"])
        waud = wt.get("aud") if isinstance(wt.get("aud"), list) else [wt.get("aud")]
        check("worker token aud contains pk-api", "pk-api" in waud, str(wt.get("aud")))
        check("worker token has realm role worker", "worker" in (wt.get("realm_access") or {}).get("roles", []), str(wt.get("realm_access")))
        check("worker token azp is pk-worker", wt.get("azp") == "pk-worker")
    s_, _, b_ = req(TOKEN, {"grant_type": "client_credentials", "client_id": "pk-worker", "client_secret": "wrong-" + (wsec or "")})
    check("pk-worker with a wrong secret => refused", s_ in (400, 401), f"{s_}")
    s_, _, b_ = req(TOKEN, {"grant_type": "client_credentials", "client_id": "pk-web"})
    check("pk-web cannot use client credentials", s_ in (400, 401), f"{s_}")
# 8. gsync's client: public, device grant only, the only client whose tokens may carry broker/read-token.
gs = clients.get("pk-gsync", {})
check("pk-gsync: public, device grant on, no browser/password/implicit/service flow",
      gs.get("publicClient") is True and gs.get("attributes", {}).get("oauth2.device.authorization.grant.enabled") == "true"
      and not any(gs.get(k) for k in ("standardFlowEnabled", "implicitFlowEnabled", "directAccessGrantsEnabled",
                                         "serviceAccountsEnabled")) and gs.get("fullScopeAllowed") is False)
broker_id = next((c["id"] for c in clients.values() if c["clientId"] == "broker"), None)
for cid, want in (("pk-gsync", True), ("pk-web", False), ("pk-mobile", False), ("pk-worker", False)):
    s_, _, b_ = admin("GET", f"/clients/{clients.get(cid, {}).get('id')}/scope-mappings/clients/{broker_id}")
    has = s_ == 200 and any(r.get("name") == "read-token" for r in json.loads(b_))
    check(f"{cid} {'has' if want else 'has no'} broker/read-token scope mapping", has is want, f"{s_}")
s_, _, b_ = req(f"{BASE}/realms/{REALM}/protocol/openid-connect/auth/device", {"client_id": "pk-gsync", "scope": "openid offline_access"})
check("pk-gsync device authorization starts", s_ == 200 and "device_code" in b_, f"{s_} {b_[:120]}")
# 6. Password grant is off for the public clients.
s, _, b = req(TOKEN, {"grant_type": "password", "client_id": "pk-web", "username": user, "password": pw})
check("password grant refused", s in (400, 401) and ("unauthorized_client" in b or "invalid_client" in b), f"{s} {b[:120]}")

print(f"\n{len(fails)} failed" if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
