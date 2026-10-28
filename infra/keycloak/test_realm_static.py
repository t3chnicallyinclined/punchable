"""Static checks on realm-punchable.yaml and the repo (no Keycloak needed; runs anywhere):
    python -m pytest infra/keycloak/test_realm_static.py -q
- the google IdP is declared with storeToken and the link-only first login (auto-link to the existing account, never
  create a user), basic scopes only and no forced consent (2026-10-26: no Drive scope at sign-in, no "unverified app"
  screen on every login);
- its client id/secret are env placeholders, and no Google client secret is in any tracked file;
- the login theme is the realm's, sign-up is off, the two login clients use PKCE and their own redirect URIs;
- red: the same checks fail on a mutated realm and on a planted secret."""
from __future__ import annotations

import re
import subprocess
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
REALM = (HERE / "realm-punchable.yaml").read_text(encoding="utf-8")
# A real Google client secret starts GOCSPX-; any other GOOGLE_CLIENT_SECRET=<value> counts too, except the test
# realm's literal throwaway value and placeholders ($VAR, <paste>).
SECRET_RE = re.compile(r"GOCSPX-[A-Za-z0-9_-]{10,}|GOOGLE_CLIENT_SECRET\s*[=:]\s*[\"']?(?!throwaway-)[A-Za-z0-9_-]{20,}")
# Built at runtime so this file never contains a string the scan would flag.
PLANTED = "GOCSPX-" + "a" * 16


def idp_block(text: str) -> str:
    m = re.search(r"^identityProviders:\n((?:[ \-].*\n|\n)+?)(?=^\S)", text, re.M)
    assert m, "no identityProviders block"
    return m.group(1)


def flow_block(text: str) -> str:
    m = re.search(r"^authenticationFlows:\n((?:[ \-].*\n|\n)+?)(?=^\S)", text, re.M)
    assert m, "no authenticationFlows block"
    return m.group(1)


def check_realm(text: str) -> None:
    b = idp_block(text)
    assert re.search(r"^\s+- alias: google$", b, re.M), "no google IdP"
    assert re.search(r"^\s+storeToken: true$", b, re.M), "storeToken is not true"
    assert re.search(r"^\s+addReadTokenRoleOnCreate: true$", b, re.M)
    assert re.search(r"^\s+firstBrokerLoginFlowAlias: pk-google-link-only$", b, re.M)
    assert re.search(r'^\s+defaultScope: "openid email profile"$', b, re.M), "sign-in must ask Google for basic scopes only (no drive)"
    assert re.search(r'^\s+offlineAccess: "false"$', b, re.M), "no offline access at sign-in"
    assert not re.search(r"^\s+prompt:", b, re.M), "no forced consent prompt at sign-in"
    assert re.search(r'^\s+clientId: "\$\(env:GOOGLE_CLIENT_ID\)"$', b, re.M), "client id is not the env placeholder"
    assert re.search(r'^\s+clientSecret: "\$\(env:GOOGLE_CLIENT_SECRET\)"$', b, re.M), "client secret is not the env placeholder"
    f = flow_block(text)
    assert "alias: pk-google-link-only" in f, "the link-only first-broker flow is missing"
    assert "authenticator: idp-detect-existing-broker-user" in f and "authenticator: idp-auto-link" in f, "first login must detect the existing user and auto-link"
    assert "idp-create-user-if-unique" not in text, "a Google sign-in could create users"
    assert re.search(r"^registrationAllowed: false$", text, re.M), "sign-up must be off"
    assert re.search(r"^loginTheme: punchable$", text, re.M), "the realm uses the punchable login theme"
    assert re.search(r"^resetPasswordAllowed: true$", text, re.M) and re.search(r"^rememberMe: true$", text, re.M)
    for client, redirect in (("pk-web", '"https://$(env:PK_DOMAIN)/auth/callback"'), ("pk-mobile", '"punchable://auth/callback"')):
        # the client's lines up to the next `- clientId:` (comment lines between clients are included, harmlessly)
        m = re.search(r"^  - clientId: " + re.escape(client) + r"\n((?:(?!  - clientId: |^\S).*\n?)*)", text, re.M)
        assert m, f"client {client} missing"
        c = m.group(1)
        assert "publicClient: true" in c and "pkce.code.challenge.method: S256" in c, f"{client}: public client with PKCE S256"
        assert redirect in c, f"{client}: redirect {redirect}"
        assert "directAccessGrantsEnabled: false" in c, f"{client}: no password grant"
    assert not SECRET_RE.search(text)


def tracked_files() -> list[Path]:
    out = subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True).stdout.decode()
    return [ROOT / p for p in out.split("\0") if p]


def check_no_secret(files) -> None:
    for p in files:
        try:
            t = p.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        assert not SECRET_RE.search(t), f"a Google client secret is in {p.relative_to(ROOT) if p.is_relative_to(ROOT) else p}"


def test_realm_declares_google_idp():
    check_realm(REALM)


def test_no_google_client_secret_in_repo():
    check_no_secret(tracked_files())


@pytest.mark.parametrize("mutate", [
    lambda t: t.replace("    storeToken: true", "    storeToken: false"),
    lambda t: t.replace('clientSecret: "$(env:GOOGLE_CLIENT_SECRET)"', f'clientSecret: "{PLANTED}"'),
    lambda t: t.replace("authenticator: idp-auto-link", "authenticator: idp-create-user-if-unique"),
    lambda t: t.replace('defaultScope: "openid email profile"', 'defaultScope: "openid email profile https://www.googleapis.com/auth/drive"'),
    lambda t: t.replace('      offlineAccess: "false"\n', '      offlineAccess: "true"\n      prompt: "consent"\n'),
    lambda t: t.replace("registrationAllowed: false", "registrationAllowed: true"),
    lambda t: t.replace("loginTheme: punchable\n", ""),
    lambda t: t.replace("      pkce.code.challenge.method: S256\n      post.logout.redirect.uris: \"punchable://auth/logout\"", "      post.logout.redirect.uris: \"punchable://auth/logout\""),
    lambda t: t.replace('      - "punchable://auth/callback"', '      - "punchable://*"'),
], ids=["storeToken-off", "secret-inline", "creates-users", "drive-scope", "consent-offline", "signup-on", "no-theme", "mobile-no-pkce", "mobile-wildcard-redirect"])
def test_red_mutated_realm_fails(mutate):
    bad = mutate(REALM)
    assert bad != REALM
    with pytest.raises(AssertionError):
        check_realm(bad)


def test_red_planted_secret_is_found(tmp_path):
    p = tmp_path / "google-oauth.env"
    p.write_text(f"GOOGLE_CLIENT_ID=x.apps.googleusercontent.com\nGOOGLE_CLIENT_SECRET={PLANTED}\n")
    with pytest.raises(AssertionError):
        check_no_secret([p])
