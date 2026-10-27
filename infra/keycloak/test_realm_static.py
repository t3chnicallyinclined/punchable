"""Static checks on realm-punchable.yaml and the repo (no Keycloak needed; runs anywhere):
    python -m pytest infra/keycloak/test_realm_static.py -q
- the google IdP is declared with storeToken, a refresh token request and the link-only first login;
- its client id/secret are env placeholders, and no Google client secret is in any tracked file;
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


def idp_block(text: str) -> str:
    m = re.search(r"^identityProviders:\n((?:[ \-].*\n|\n)+?)(?=^\S)", text, re.M)
    assert m, "no identityProviders block"
    return m.group(1)


def check_realm(text: str) -> None:
    b = idp_block(text)
    assert re.search(r"^\s+- alias: google$", b, re.M), "no google IdP"
    assert re.search(r"^\s+storeToken: true$", b, re.M), "storeToken is not true"
    assert re.search(r"^\s+addReadTokenRoleOnCreate: true$", b, re.M)
    assert re.search(r'^\s+offlineAccess: "true"$', b, re.M) and re.search(r'^\s+prompt: "consent"$', b, re.M)
    assert re.search(r"^\s+trustEmail: false$", b, re.M)
    assert re.search(r"^\s+firstBrokerLoginFlowAlias: pk-google-link-only$", b, re.M)
    assert re.search(r'^\s+clientId: "\$\(env:GOOGLE_CLIENT_ID\)"$', b, re.M), "client id is not the env placeholder"
    assert re.search(r'^\s+clientSecret: "\$\(env:GOOGLE_CLIENT_SECRET\)"$', b, re.M), "client secret is not the env placeholder"
    assert "idp-create-user-if-unique" not in text, "a Google sign-in could create users"
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
    lambda t: t.replace('clientSecret: "$(env:GOOGLE_CLIENT_SECRET)"', 'clientSecret: "GOCSPX-abcdefghijklmnop"'),
    lambda t: t.replace("authenticator: idp-username-password-form", "authenticator: idp-create-user-if-unique"),
    lambda t: t.replace('    prompt: "consent"\n', ""),
], ids=["storeToken-off", "secret-inline", "creates-users", "no-consent"])
def test_red_mutated_realm_fails(mutate):
    bad = mutate(REALM)
    assert bad != REALM
    with pytest.raises(AssertionError):
        check_realm(bad)


def test_red_planted_secret_is_found(tmp_path):
    p = tmp_path / "google-oauth.env"
    p.write_text("GOOGLE_CLIENT_ID=x.apps.googleusercontent.com\nGOOGLE_CLIENT_SECRET=GOCSPX-abcdefghijklmnop\n")
    with pytest.raises(AssertionError):
        check_no_secret([p])
