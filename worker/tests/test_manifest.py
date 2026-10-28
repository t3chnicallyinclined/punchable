"""The hosted worker's Kubernetes manifest and NetworkPolicy (infra/k8s), rendered the way infra/render.sh renders them
and parsed with PyYAML (no kubectl here). What a reviewer would check by eye: one replica, Recreate, no GPU, the two
Secrets, the in-cluster API, a liveness probe on the heartbeat, read-only root, the worker's egress and the API's
ingress from it, the Dockerfile's dockerignore in step with its COPY lines.

    python -m pytest worker/tests/test_manifest.py -q
"""
from __future__ import annotations

import re
from pathlib import Path

import pytest

yaml = pytest.importorskip("yaml")
DESK = Path(__file__).resolve().parents[2]
K8S = DESK / "infra" / "k8s"
FILL = {"__PK_DOMAIN__": "example.com", "__PK_AUTH_HOST__": "auth.example.com", "__PK_MAIL_FROM__": "x@example.com",
        "__PK_NS__": "punchable", "__PK_KC_NS__": "keycloak", "__PK_REALM__": "punchable", "__PK_ORIGIN_IP__": "203.0.113.10",
        "__PK_OWNER_ID__": "owner", "__PK_COMIC_NAME__": "the comic", "__PK_COMIC_FIRST__": "the comic",
        "__IMAGE_REF__": "localhost:32000/punchable-worker@sha256:" + "0" * 64}


def render(name: str) -> list[dict]:
    s = (K8S / name).read_text(encoding="utf-8")
    for k, v in FILL.items():
        s = s.replace(k, v)
    assert not re.search(r"__(PK_[A-Z_]+|IMAGE_REF)__", s), "a placeholder render.sh does not know"
    return [d for d in yaml.safe_load_all(s) if d]


def test_worker_deployment():
    dep, svc = render("35-worker.yaml")
    assert dep["kind"] == "Deployment" and dep["metadata"] == {"name": "punchable-worker", "namespace": "punchable"}
    # the metrics Service (the ServiceMonitor in 60-observability.yaml selects app=punchable-worker, port `metrics`)
    assert svc["kind"] == "Service" and svc["metadata"]["labels"] == {"app": "punchable-worker"}
    assert svc["spec"] == {"selector": {"app": "punchable-worker"}, "ports": [{"name": "metrics", "port": 9100, "targetPort": 9100}]}
    spec = dep["spec"]
    assert spec["replicas"] == 1 and spec["strategy"] == {"type": "Recreate"}
    pod = spec["template"]["spec"]
    assert spec["template"]["metadata"]["labels"] == {"app": "punchable-worker"}
    assert pod["automountServiceAccountToken"] is False
    assert pod["securityContext"]["runAsNonRoot"] is True and pod["securityContext"]["runAsUser"] == 10001
    [c] = pod["containers"]
    env = {e["name"]: e.get("value") for e in c["env"]}
    assert c["ports"] == [{"name": "metrics", "containerPort": 9100}] and env["PK_METRICS_PORT"] == "9100"  # the only listener
    assert env["PK_API"] == "http://punchable-api"            # the Service (port 80 -> 8080), in-cluster
    assert env["PK_HOSTED"] == "1" and env["PK_ONLINE_FIRST"] == "1"
    assert env["ENGINE_CONFIG"].endswith("config.hosted.toml")
    assert env["PK_TOKEN_URL"].startswith("https://auth.example.com/realms/punchable/")
    assert float(env["PK_AI_DAILY_USD"]) == 2.0
    assert env["PK_HEARTBEAT_FILE"] == "/tmp/pk-worker/heartbeat"
    assert {e["name"] for e in c["env"] if "valueFrom" in e} == {"PK_WORKER_NAME"}
    refs = {s["secretRef"]["name"]: s["secretRef"].get("optional", False) for s in c["envFrom"]}
    # the two it cannot run without are required; the crash-report DSN is the API's Secret and optional (unset = off)
    assert refs == {"punchable-worker": False, "punchable-anthropic": False, "punchable-sentry": True}
    assert "ANTHROPIC" not in str(c["env"])  # the key is never an inline value
    sc = c["securityContext"]
    assert sc["readOnlyRootFilesystem"] is True and sc["allowPrivilegeEscalation"] is False and sc["capabilities"] == {"drop": ["ALL"]}
    assert [p["containerPort"] for p in c["ports"]] == [9100]  # the metrics port is the ONLY listener (worker/metrics.py)
    assert "nvidia.com/gpu" not in str(c["resources"]) and c["resources"]["limits"]["memory"] == "1Gi"
    lp = c["livenessProbe"]
    assert "heartbeat" in " ".join(lp["exec"]["command"]) and lp["periodSeconds"] == 60 and lp["failureThreshold"] == 3
    assert "readinessProbe" not in c  # not a Service backend
    assert [v["name"] for v in pod["volumes"]] == ["tmp"] and pod["volumes"][0]["emptyDir"]["sizeLimit"] == "2Gi"
    assert c["volumeMounts"] == [{"name": "tmp", "mountPath": "/tmp"}]


def test_network_policy_worker_and_api_ingress():
    docs = {d["metadata"]["name"]: d for d in render("10-networkpolicy.yaml") if d["kind"] == "NetworkPolicy"}
    w = docs["punchable-worker"]["spec"]
    assert w["podSelector"] == {"matchLabels": {"app": "punchable-worker"}}
    assert w["policyTypes"] == ["Ingress", "Egress"]
    # in: ONLY Prometheus (namespace observability) to the metrics port; nothing else listens, nothing else may reach it
    assert w["ingress"] == [{"from": [{"namespaceSelector": {"matchLabels": {"kubernetes.io/metadata.name": "observability"}}}],
                             "ports": [{"protocol": "TCP", "port": 9100}]}]
    to_api, to_bugsink, to_public = w["egress"]
    assert to_api["to"] == [{"podSelector": {"matchLabels": {"app": "punchable-api"}}}]
    assert to_api["ports"] == [{"protocol": "TCP", "port": 8080}]
    assert to_bugsink["to"] == [{"podSelector": {"matchLabels": {"app": "punchable-bugsink"}}}]   # crash reports, in-cluster
    assert to_bugsink["ports"] == [{"protocol": "TCP", "port": 8000}]
    ipb = to_public["to"][0]["ipBlock"]
    assert ipb["cidr"] == "0.0.0.0/0" and "10.0.0.0/8" in ipb["except"] and "192.168.0.0/16" in ipb["except"]
    assert to_public["ports"] == [{"protocol": "TCP", "port": 443}]
    assert not any("surrealdb" in str(r) for r in w["egress"])  # never the database: the API is its only path
    api = docs["punchable-api"]["spec"]
    froms = api["ingress"][0]["from"]
    assert {"podSelector": {"matchLabels": {"app": "punchable-worker"}}} in froms
    assert api["ingress"][0]["ports"] == [{"protocol": "TCP", "port": 8080}]
    assert "punchable-worker" not in str(docs["surrealdb"]["spec"]["ingress"])


def test_dockerfile_copies_are_in_the_dockerignore_and_config_is_hosted():
    df = (DESK / "infra" / "worker" / "Dockerfile").read_text(encoding="utf-8")
    ign = (DESK / "infra" / "worker" / "Dockerfile.dockerignore").read_text(encoding="utf-8").splitlines()
    allowed = [l[1:] for l in ign if l.startswith("!")]
    for m in re.finditer(r"^COPY (?!--from)(.+?) \S+$", df, re.M):
        for src in m.group(1).split():
            assert any(Path(src).match(a) or src == a for a in allowed), f"{src} is not in Dockerfile.dockerignore"
    code = "\n".join(l for l in df.splitlines() if not l.lstrip().startswith("#"))  # the comments may name what is absent
    assert "config.hosted.toml" in code and 'ENTRYPOINT ["python", "-m", "worker", "--hosted"]' in code
    assert "USER 10001:10001" in code and "whisper" not in code and "yt-dlp" not in code and "ffmpeg" not in code
    req = "\n".join(l for l in (DESK / "infra" / "worker" / "requirements.txt").read_text(encoding="utf-8").splitlines()
                    if l.strip() and not l.startswith("#"))
    assert re.search(r"^anthropic==", req, re.M) and "torch" not in req and "whisper" not in req
    # crash reports: the image carries the API's crash.py + scrub.py and the same sentry-sdk pin as the API image
    assert "COPY api-dev/crash.py api-dev/scrub.py api-dev/" in code and "worker.crashreport" in code
    api_req = (DESK / "api" / "requirements.txt").read_text(encoding="utf-8")
    [pin] = re.findall(r"^sentry-sdk==\S+", req, re.M)
    assert pin in api_req, pin


def _bash():
    import shutil
    return shutil.which("bash")


@pytest.mark.skipif(not _bash(), reason="needs bash for infra/render.sh")
def test_render_sh_fills_the_comic_name_with_a_safe_default_and_leaves_no_placeholder(tmp_path):
    """infra/render.sh with a config that names the comic, and with one that does not: both manifests render with no
    __PK_ placeholder; the name lands in PK_COMIC_NAME / PK_COMIC_FIRST; unset, both are "the comic" (the prompts then
    say "a stand-up comic's", never "stand-up comic the comic's")."""
    import os
    import shutil
    import subprocess
    infra = tmp_path / "infra"
    shutil.copytree(DESK / "infra" / "k8s", infra / "k8s")
    shutil.copy(DESK / "infra" / "render.sh", infra / "render.sh")
    base = ("PK_DOMAIN=example.com\nPK_AUTH_HOST=auth.example.com\nPK_MAIL_FROM=x@example.com\nPK_NS=punchable\n"
            "PK_KC_NS=keycloak\nPK_REALM=punchable\nPK_ORIGIN_IP=203.0.113.10\nPK_OWNER_ID=owner\n")

    def render(cfg: str, name: str) -> dict:
        (infra / "config.env").write_text(cfg, encoding="utf-8")
        r = subprocess.run([_bash(), str(infra / "render.sh"), str(infra / "k8s" / name), "localhost:32000/x@sha256:" + "0" * 64],
                           capture_output=True, text=True, encoding="utf-8", env={**os.environ, "PK_COMIC_NAME": "", "PK_COMIC_FIRST": ""})
        assert r.returncode == 0, r.stderr
        assert not re.search(r"__(PK_[A-Z_]+|IMAGE_REF)__", r.stdout), r.stdout[:300]
        docs = [d for d in yaml.safe_load_all(r.stdout) if d]
        dep = next(d for d in docs if d["kind"] == "Deployment")
        return {e["name"]: e.get("value") for e in dep["spec"]["template"]["spec"]["containers"][0]["env"]}
    for name in ("30-api.yaml", "35-worker.yaml"):
        env = render(base, name)
        assert env["PK_COMIC_NAME"] == "the comic" and env["PK_COMIC_FIRST"] == "the comic", name
        env = render(base + 'PK_COMIC_NAME="Pat Example"\n', name)  # bash sources the file: a space needs quotes
        assert env["PK_COMIC_NAME"] == "Pat Example" and env["PK_COMIC_FIRST"] == "Pat Example", "the first name defaults to the name"
        env = render(base + 'PK_COMIC_NAME="Pat Example"\nPK_COMIC_FIRST=Pat\n', name)
        assert env["PK_COMIC_NAME"] == "Pat Example" and env["PK_COMIC_FIRST"] == "Pat", name
