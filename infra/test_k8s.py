"""The k8s manifests parse and agree with each other (infra/k8s/*.yaml, rendered with sample values; no cluster).

  * every manifest renders (render.sh's placeholders filled with the example config) and parses as YAML;
  * each ServiceMonitor in 60-observability.yaml selects a Service that exists, in the right namespace, with a port of
    the name the endpoint scrapes, and carries `release: prometheus` (the label this cluster's Prometheus matches);
  * the PrometheusRule carries the same label, every alert has severity + service=punchable and runbook text, and
    every pk_* metric an expression names is one the code emits (api-dev/metrics.py, worker/metrics.py);
  * the Bugsink manifest (70-bugsink.yaml) parses, pins its image, mounts its PVC, and its NetworkPolicy admits exactly
    the ingress controller, the API pods and the worker pods;
  * the metrics ports are admitted from the observability namespace and from nowhere else;
  * the Grafana dashboard is valid JSON and only queries metrics that exist (or kube-state-metrics / cAdvisor families).

    python -m pytest infra/test_k8s.py -q
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
K8S = ROOT / "infra" / "k8s"
SAMPLE = {"__PK_DOMAIN__": "example.com", "__PK_AUTH_HOST__": "auth.example.com", "__PK_MAIL_FROM__": "no-reply@example.com",
          "__PK_NS__": "punchable", "__PK_KC_NS__": "keycloak", "__PK_REALM__": "punchable", "__PK_ORIGIN_IP__": "203.0.113.10",
          "__PK_OWNER_ID__": "owner", "__PK_COMIC_NAME__": "the comic", "__PK_COMIC_FIRST__": "the comic",
          "__IMAGE_REF__": "localhost:32000/punchable-x@sha256:" + "0" * 64,
          "__IMAGE_REF_RS__": "localhost:32000/punchable-api-rs@sha256:" + "1" * 64, "__PK_WORKER_API__": "http://punchable-api"}

sys.path.insert(0, str(ROOT / "api-dev"))
sys.path.insert(0, str(ROOT))


def render(path: Path) -> str:
    s = path.read_text(encoding="utf-8")
    for k, v in SAMPLE.items():
        s = s.replace(k, v)
    assert not re.search(r"__[A-Z][A-Z0-9_]*__", s), f"{path.name}: unrendered placeholder"
    return s


def docs(name: str) -> list[dict]:
    return [d for d in yaml.safe_load_all(render(K8S / name)) if d]


def all_docs() -> list[dict]:
    out = []
    for p in sorted(K8S.glob("*.yaml")):
        out += docs(p.name)
    return out


def by_kind(kind: str, ds=None) -> list[dict]:
    return [d for d in (ds if ds is not None else all_docs()) if d.get("kind") == kind]


@pytest.mark.parametrize("name", sorted(p.name for p in K8S.glob("*.yaml")))
def test_every_manifest_renders_and_parses(name):
    for d in docs(name):
        assert d.get("apiVersion") and d.get("kind") and d["metadata"]["name"], (name, d.get("kind"))


# ------------------------------------------------------------------------------------------ ServiceMonitors


def emitted_metrics() -> set[str]:
    import metrics as M
    import worker.metrics as WX
    names = set()
    for reg in (M.registry, WX.registry):
        for line in reg.render().splitlines():
            if line.startswith("# TYPE "):
                names.add(line.split()[2])
    return names


def test_servicemonitors_select_real_services_with_the_named_port():
    ds = all_docs()
    services = {(s["metadata"]["namespace"], s["metadata"]["name"]): s for s in by_kind("Service", ds)}
    sms = by_kind("ServiceMonitor", ds)
    assert {s["metadata"]["name"] for s in sms} == {"punchable-api", "punchable-worker"}
    for sm in sms:
        assert sm["metadata"]["labels"].get("release") == "prometheus", sm["metadata"]["name"]
        sel = sm["spec"]["selector"]["matchLabels"]
        nss = sm["spec"]["namespaceSelector"]["matchNames"]
        matched = [s for (ns, _), s in services.items() if ns in nss and all((s["metadata"].get("labels") or {}).get(k) == v for k, v in sel.items())]
        assert matched, f"{sm['metadata']['name']}: no Service carries {sel} in {nss}"
        for ep in sm["spec"]["endpoints"]:
            assert ep["path"] == "/metrics"
            for s in matched:
                assert ep["port"] in {p.get("name") for p in s["spec"]["ports"]}, (sm["metadata"]["name"], s["metadata"]["name"], ep["port"])
        # jobLabel=app -> the job name the alerts match (up{job="punchable-api"|"punchable-worker"})
        assert sm["spec"]["jobLabel"] == "app"
        assert {(s["metadata"]["labels"] or {}).get("app") for s in matched} == {sm["metadata"]["name"]}
    # the metrics ports really are the ones the pods listen on
    api = by_kind("Deployment", docs("30-api.yaml"))[0]["spec"]["template"]["spec"]["containers"][0]
    assert api["ports"][0]["containerPort"] == 8080
    worker = by_kind("Deployment", docs("35-worker.yaml"))[0]["spec"]["template"]["spec"]["containers"][0]
    assert {"name": "metrics", "containerPort": 9100} in worker["ports"]
    assert {"name": "PK_METRICS_PORT", "value": "9100"} in worker["env"]


def test_prometheusrule_alerts_are_routable_and_built_on_emitted_metrics():
    [pr] = by_kind("PrometheusRule")
    assert pr["metadata"]["labels"].get("release") == "prometheus"
    emitted = emitted_metrics()
    names = []
    for g in pr["spec"]["groups"]:
        for r in g["rules"]:
            names.append(r["alert"])
            assert r["labels"].get("service") == "punchable", r["alert"]
            assert r["labels"].get("severity") in ("critical", "warning", "info"), r["alert"]
            assert r["annotations"].get("summary") and r["annotations"].get("description"), r["alert"]
            assert len(r["annotations"].get("runbook", "")) > 40, f"{r['alert']}: runbook text"
            for m in re.findall(r"\bpk_[a-z_]+", r["expr"]):
                base = re.sub(r"_(bucket|count|sum)$", "", m)
                assert base in emitted, f"{r['alert']} names {m}, which nothing emits"
    for want in ("PunchableApiDown", "PunchableApiHighErrorRate", "PunchableApiHighLatency", "PunchableWorkerHeartbeatStale",
                 "PunchableAiBudgetPaused", "PunchableJobsWaitingForTranscriber", "PunchableSurrealDbRestarted",
                 "PunchableBackupFailed", "PunchableBackupMissed", "PunchableCertificateExpiringSoon", "PunchableApiRsDown"):
        assert want in names, want


def test_pk_api_is_scraped_and_alerted_beside_the_python():
    """T11 round 1: the scrape config in force (apply_prometheus.py, the cluster has no ServiceMonitor CRD) keeps the
    Python's pod label AND pk-api's, as separate jobs, so one `up` never hides the other; both have a down alert and a
    Grafana tile. The Python's job and alert are untouched (it is still live)."""
    sys.path.insert(0, str(ROOT / "infra" / "observability"))
    try:
        import apply_prometheus as ap
    finally:
        sys.path.pop(0)
    jobs = {j["job_name"]: j for j in yaml.safe_load(ap.JOBS)}
    assert set(jobs) == {"punchable-api", "punchable-api-rs", "punchable-worker"}
    # merging adds each MISSING job: a live config made before pk-api had the other two (2026-11-07)
    head, blocks = ap.job_blocks()
    old = ("global: {}\nscrape_configs:\n  - job_name: 'other'\n    static_configs: [{targets: ['x:1']}]\n"
           + head + "\n" + blocks["punchable-api"] + "\n" + blocks["punchable-worker"] + "\n")
    merged, changed = ap.merged_prometheus_yml(old)
    got = [j["job_name"] for j in yaml.safe_load(merged)["scrape_configs"]]
    assert sorted(got) == ["other", "punchable-api", "punchable-api-rs", "punchable-worker"], got
    assert changed[0] == "scrape jobs punchable-api-rs", changed
    again, changed2 = ap.merged_prometheus_yml(merged)
    assert again == merged and not any(c.startswith("scrape jobs") for c in changed2)
    fresh, _ = ap.merged_prometheus_yml("scrape_configs:\n  - job_name: 'other'\n    static_configs: [{targets: ['x:1']}]\n")
    assert sorted(j["job_name"] for j in yaml.safe_load(fresh)["scrape_configs"]) == sorted(["other", *jobs])
    def keep(job):
        [r] = [r for r in jobs[job]["relabel_configs"] if r.get("action") == "keep"]
        return r["regex"]
    def port(job):
        [r] = [r for r in jobs[job]["relabel_configs"] if r.get("target_label") == "__address__"]
        return r["replacement"]
    assert (keep("punchable-api"), port("punchable-api")) == ("punchable-api", "${1}:8080")
    assert (keep("punchable-api-rs"), port("punchable-api-rs")) == ("punchable-api-rs", "${1}:8080")
    assert (keep("punchable-worker"), port("punchable-worker")) == ("punchable-worker", "${1}:9100")
    # the keep regexes are anchored by Prometheus: no job's regex matches another's pod label
    labels = {"punchable-api", "punchable-api-rs", "punchable-worker"}
    for job in jobs:
        assert {l for l in labels if re.fullmatch(keep(job), l)} == {keep(job)}, job
    [pr] = by_kind("PrometheusRule")
    rules = {r["alert"]: r for g in pr["spec"]["groups"] for r in g["rules"]}
    assert rules["PunchableApiRsDown"]["expr"] == 'up{job="punchable-api-rs"} == 0'
    assert rules["PunchableApiRsDown"]["for"] == "2m" and rules["PunchableApiRsDown"]["labels"]["severity"] == "critical"
    assert rules["PunchableApiDown"]["expr"] == 'up{job="punchable-api"} == 0'
    dash = json.loads((ROOT / "infra" / "grafana" / "punchable.json").read_text(encoding="utf-8"))
    exprs = [t["expr"] for p in dash["panels"] for t in p.get("targets", [])]
    assert 'max(up{job="punchable-api-rs"}) or vector(0)' in exprs and 'max(up{job="punchable-api"}) or vector(0)' in exprs
    stats = [p["gridPos"] for p in dash["panels"] if p["gridPos"]["y"] == 1]
    assert sum(g["w"] for g in stats) == 24 and len({g["x"] for g in stats}) == len(stats)


def test_api_down_rule_matches_two_minutes_and_latency_ten():
    [pr] = by_kind("PrometheusRule")
    rules = {r["alert"]: r for g in pr["spec"]["groups"] for r in g["rules"]}
    assert rules["PunchableApiDown"]["for"] == "2m" and rules["PunchableApiDown"]["labels"]["severity"] == "critical"
    assert rules["PunchableApiHighErrorRate"]["for"] == "5m" and "0.05" in rules["PunchableApiHighErrorRate"]["expr"]
    assert rules["PunchableApiHighLatency"]["for"] == "10m" and "0.95" in rules["PunchableApiHighLatency"]["expr"]
    assert rules["PunchableWorkerHeartbeatStale"]["expr"].strip() == "pk_worker_heartbeat_age_seconds > 600"
    assert "86400" in rules["PunchableJobsWaitingForTranscriber"]["expr"]
    assert "14 * 86400" in rules["PunchableCertificateExpiringSoon"]["expr"]


# ---------------------------------------------------------------------------------------- NetworkPolicies


def policy(name: str) -> dict:
    [p] = [d for d in by_kind("NetworkPolicy", docs("10-networkpolicy.yaml")) if d["metadata"]["name"] == name]
    return p


def ingress_peers(p: dict) -> list[tuple[dict, list[int]]]:
    out = []
    for rule in p["spec"].get("ingress") or []:
        ports = [x["port"] for x in rule.get("ports", [])]
        for peer in rule.get("from", []):
            out.append((peer, ports))
    return out


def test_metrics_ports_are_admitted_from_observability_only():
    for name, port in (("punchable-api", 8080), ("punchable-worker", 9100)):
        p = policy(name)
        obs = [(peer, ports) for peer, ports in ingress_peers(p)
               if (peer.get("namespaceSelector") or {}).get("matchLabels", {}).get("kubernetes.io/metadata.name") == "observability"]
        assert obs and all(port in ports for _, ports in obs), name
        for peer, ports in ingress_peers(p):
            if "namespaceSelector" in peer:
                assert peer["namespaceSelector"]["matchLabels"]["kubernetes.io/metadata.name"] == "observability", (name, peer)
    # the worker's only ingress is the metrics port; nothing else may reach it
    assert {pt for _, ports in ingress_peers(policy("punchable-worker")) for pt in ports} == {9100}


def test_api_and_worker_may_reach_bugsink_and_bugsink_reaches_nothing():
    for name in ("punchable-api", "punchable-worker"):
        eg = policy(name)["spec"]["egress"]
        assert any(any((peer.get("podSelector") or {}).get("matchLabels", {}).get("app") == "punchable-bugsink" for peer in r.get("to", []))
                   and [x["port"] for x in r["ports"]] == [8000] for r in eg), name
    b = policy("punchable-bugsink")
    assert b["spec"]["policyTypes"] == ["Ingress", "Egress"] and not b["spec"].get("egress")   # DNS comes from dns-egress
    peers = ingress_peers(b)
    assert {pt for _, ports in peers for pt in ports} == {8000}
    apps = {(peer.get("podSelector") or {}).get("matchLabels", {}).get("app") for peer, _ in peers if "podSelector" in peer}
    assert apps == {"punchable-api", "punchable-worker"}
    ips = [peer["ipBlock"]["cidr"] for peer, _ in peers if "ipBlock" in peer]
    assert ips == ["203.0.113.10/32"]   # the ingress controller (host network), rendered from PK_ORIGIN_IP


# ------------------------------------------------------------------------------------------------ Bugsink


def test_bugsink_manifest_is_one_small_pinned_pod_on_its_pvc():
    ds = docs("70-bugsink.yaml")
    assert {d["kind"] for d in ds} == {"PersistentVolumeClaim", "Service", "Deployment", "Ingress"}
    [dep] = by_kind("Deployment", ds)
    assert dep["spec"]["replicas"] == 1 and dep["spec"]["strategy"] == {"type": "Recreate"}
    [c] = dep["spec"]["template"]["spec"]["containers"]
    assert re.fullmatch(r"bugsink/bugsink:\d+\.\d+\.\d+|bugsink/bugsink@sha256:[0-9a-f]{64}", c["image"]), c["image"]
    assert c["resources"]["requests"]["memory"] == "128Mi" and c["resources"]["limits"]["memory"] == "512Mi"
    env = {e["name"]: e for e in c["env"]}
    assert env["BASE_URL"]["value"] == "https://bugs.example.com" and env["BEHIND_HTTPS_PROXY"]["value"] == "True"
    assert env["DATABASE_PATH"]["value"] == "/data/db.sqlite3" and env["PHONEHOME"]["value"] == "False"
    assert env["SECRET_KEY"]["valueFrom"]["secretKeyRef"] == {"name": "punchable-bugsink", "key": "SECRET_KEY"}
    assert env["CREATE_SUPERUSER"]["valueFrom"]["secretKeyRef"]["optional"] is True
    mounts = {m["mountPath"]: m["name"] for m in c["volumeMounts"]}
    vols = {v["name"]: v for v in dep["spec"]["template"]["spec"]["volumes"]}
    assert vols[mounts["/data"]]["persistentVolumeClaim"]["claimName"] == "punchable-bugsink-data"
    [pvc] = by_kind("PersistentVolumeClaim", ds)
    assert pvc["metadata"]["name"] == "punchable-bugsink-data" and pvc["spec"]["resources"]["requests"]["storage"] == "2Gi"
    sc = dep["spec"]["template"]["spec"]["securityContext"]
    assert sc["runAsNonRoot"] is True and sc["runAsUser"] == 14237 and sc["fsGroup"] == 14237
    [ing] = by_kind("Ingress", ds)
    assert ing["spec"]["rules"][0]["host"] == "bugs.example.com" and ing["spec"]["tls"][0]["hosts"] == ["bugs.example.com"]
    assert ing["metadata"]["annotations"]["cert-manager.io/cluster-issuer"] == "letsencrypt"
    # the probes send the real Host (Django's ALLOWED_HOSTS comes from BASE_URL)
    assert c["readinessProbe"]["httpGet"]["httpHeaders"] == [{"name": "Host", "value": "bugs.example.com"}]
    # the API and the worker read their DSN from the optional Secret the sentry-secret step makes (one Secret, two
    # releases: punchable-api@..., punchable-worker@...)
    for f in ("30-api.yaml", "35-worker.yaml"):
        c = by_kind("Deployment", docs(f))[0]["spec"]["template"]["spec"]["containers"][0]
        assert {"secretRef": {"name": "punchable-sentry", "optional": True}} in c["envFrom"], f


def test_backup_cronjob_also_copies_the_bugsink_database():
    [cj] = by_kind("CronJob", docs("60-backup.yaml"))
    pod = cj["spec"]["jobTemplate"]["spec"]["template"]["spec"]
    [c] = pod["containers"]
    env = {e["name"]: e.get("value") for e in c["env"]}
    assert env.get("BUGSINK_DB_PATH") == "/bugsink/db.sqlite3"
    mounts = {m["mountPath"]: m for m in c["volumeMounts"]}
    assert mounts["/bugsink"]["readOnly"] is False   # sqlite3's backup API needs the -wal/-shm files writable
    vols = {v["name"]: v for v in pod["volumes"]}
    assert vols[mounts["/bugsink"]["name"]]["persistentVolumeClaim"]["claimName"] == "punchable-bugsink-data"
    # the reader CREATES -shm/-wal (mode 644) when absent: it must be Bugsink's own uid or Bugsink loses write access
    [bs] = by_kind("Deployment", docs("70-bugsink.yaml"))
    bsc = bs["spec"]["template"]["spec"]["securityContext"]
    sc = pod["securityContext"]
    assert (sc["runAsUser"], sc["runAsGroup"], sc["fsGroup"]) == (bsc["runAsUser"], bsc["runAsGroup"], bsc["fsGroup"])


# --------------------------------------------------------------------------------------------- dashboard


def test_grafana_dashboard_is_valid_and_queries_only_metrics_that_exist():
    d = json.loads((ROOT / "infra" / "grafana" / "punchable.json").read_text(encoding="utf-8"))
    assert d["uid"] == "punchable-main" and "punchable" in d["tags"]
    emitted = emitted_metrics()
    known_families = ("kube_", "container_memory_", "ALERTS", "up")
    rows = [p["title"] for p in d["panels"] if p["type"] == "row"]
    assert rows == ["Overview", "Jobs", "AI spend", "Data"]
    ids = [p["id"] for p in d["panels"]]
    assert len(ids) == len(set(ids))
    for p in d["panels"]:
        for t in p.get("targets", []):
            for m in re.findall(r"\b([a-zA-Z_:][a-zA-Z0-9_:]*)\s*(?:\{|\[|\))", t["expr"]):
                if m.startswith("pk_"):
                    assert re.sub(r"_(bucket|count|sum)$", "", m) in emitted, (p["title"], m)
            assert any(k in t["expr"] for k in ("pk_",) + known_families), (p["title"], t["expr"])
        if p["type"] != "row":
            assert p.get("datasource") == "Prometheus", p["title"]


# ----------------------------------------------------------------------------------------- deploy.sh steps


def test_deploy_and_ship_know_the_new_steps():
    dep = (ROOT / "infra" / "deploy.sh").read_text(encoding="utf-8")
    ship = (ROOT / "infra" / "ship.sh").read_text(encoding="utf-8")
    for step in ("observability)", "bugsink-secret)", "bugsink)", "sentry-secret)"):
        assert f"\n  {step}" in dep, step
    assert "SENTRY_DSN" in ship and "bugsink-secret" in ship
    assert "60-observability.yaml" in dep and "70-bugsink.yaml" in dep and "apply_prometheus.py" in dep
    assert "grafana_dashboard=1" in dep and "-n observability" in dep
    if subprocess.run(["bash", "--version"], capture_output=True).returncode == 0:
        for f in ("deploy.sh", "ship.sh"):
            r = subprocess.run(["bash", "-n", str(ROOT / "infra" / f)], capture_output=True, text=True)
            assert r.returncode == 0, (f, r.stderr)


# ---- infra/render.sh itself (run for real, with the example config) --------------------------------------------------
import shutil


def _render(tmp_path, file, *args, env=None):
    d = tmp_path / "infra"
    d.mkdir(exist_ok=True)
    shutil.copy(ROOT / "infra" / "render.sh", d / "render.sh")
    shutil.copy(ROOT / "infra" / "config.env.example", d / "config.env")
    e = {k: v for k, v in os.environ.items() if not k.startswith("PK_")}
    e.update(env or {})
    return subprocess.run(["bash", (d / "render.sh").as_posix(), Path(file).as_posix(), *args], capture_output=True, text=True, env=e)


PREPARED = ROOT / "infra" / "k8s" / "prepared" / "t10-api-rs.yaml"
RS_IMG = "localhost:32000/punchable-api-rs@sha256:" + "2" * 64


def test_render_fills_image_ref_rs_from_argument_or_env(tmp_path):
    r = _render(tmp_path, PREPARED, "", RS_IMG)
    assert r.returncode == 0, r.stderr
    assert f"image: {RS_IMG}" in r.stdout
    r = _render(tmp_path, PREPARED, env={"PK_IMAGE_REF_RS": RS_IMG})
    assert r.returncode == 0, r.stderr
    assert f"image: {RS_IMG}" in r.stdout


def test_render_refuses_a_leftover_image_ref_rs_red_arm(tmp_path):
    r = _render(tmp_path, PREPARED)
    assert r.returncode == 1 and "__IMAGE_REF_RS__" in r.stderr and r.stdout == ""


def test_render_refuses_any_unknown_placeholder(tmp_path):
    f = tmp_path / "x.yaml"
    f.write_text("a: __SOMETHING_NEW__\nb: __init__\n", encoding="utf-8")
    r = _render(tmp_path, f)
    assert r.returncode == 1 and "__SOMETHING_NEW__" in r.stderr and "__init__" not in r.stderr


def test_servicemonitor_is_off_unless_asked(tmp_path):
    off = _render(tmp_path, PREPARED, "", RS_IMG)
    on = _render(tmp_path, PREPARED, "", RS_IMG, env={"PK_SERVICEMONITOR": "1"})
    kinds = lambda out: [d["kind"] for d in yaml.safe_load_all(out) if d]
    assert "ServiceMonitor" not in kinds(off.stdout)
    assert kinds(on.stdout).count("ServiceMonitor") == 1
    assert len(kinds(off.stdout)) == len(kinds(on.stdout)) - 1


def test_worker_api_url_is_one_setting_for_both_workers(tmp_path):
    for f in (K8S / "35-worker.yaml", ROOT / "infra" / "worker" / "rust" / "35-worker.yaml"):
        assert "__PK_WORKER_API__" in f.read_text(encoding="utf-8")
        for env, want in (({}, "http://punchable-api"), ({"PK_WORKER_API": "http://punchable-api-rs"}, "http://punchable-api-rs")):
            r = _render(tmp_path, f, "x@sha256:" + "0" * 64, env=env)
            assert r.returncode == 0, r.stderr
            dep = next(d for d in yaml.safe_load_all(r.stdout) if d and d["kind"] == "Deployment")
            envs = {e["name"]: e.get("value") for e in dep["spec"]["template"]["spec"]["containers"][0]["env"]}
            assert envs["PK_API"] == want


def test_prepared_policies_admit_the_worker_to_pk_api(tmp_path):
    r = _render(tmp_path, PREPARED, "", RS_IMG)
    ds = [d for d in yaml.safe_load_all(r.stdout) if d]
    pk = next(d for d in ds if d["kind"] == "NetworkPolicy" and d["metadata"]["name"] == "punchable-api-rs")
    peers = [p for rule in pk["spec"]["ingress"] for p in rule["from"]]
    assert {"podSelector": {"matchLabels": {"app": "punchable-worker"}}} in peers
    w = next(d for d in ds if d["metadata"]["name"] == "punchable-worker-to-api-rs")
    assert w["spec"]["egress"][0]["to"] == [{"podSelector": {"matchLabels": {"app": "punchable-api-rs"}}}]


def test_prepared_recordings_in_r2_read_their_token_from_a_secret_and_reach_only_the_public_443(tmp_path):
    """docs/adr/0015: pk-api's R2 settings are a Secret by reference (no value, no bucket name in the file), the pod does not
    start without it, the Python Deployment is not told to keep uploads in the database, the fetch of a recording is not
    buffered by nginx, and pk-api may reach R2 on 443 of the public internet only."""
    r = _render(tmp_path, PREPARED, "", RS_IMG)
    assert r.returncode == 0, r.stderr
    ds = [d for d in yaml.safe_load_all(r.stdout) if d]
    dep = next(d for d in ds if d["kind"] == "Deployment" and d["metadata"]["name"] == "punchable-api-rs")
    c = dep["spec"]["template"]["spec"]["containers"][0]
    env = {e["name"]: e.get("value") for e in c["env"]}
    assert env["PK_UPLOAD_R2"] == "1"
    assert not any(k.startswith("PK_REC_") or k == "PK_UPLOAD_DB" for k in env), "the bucket and the token are never in this file"
    sec = [e["secretRef"] for e in c["envFrom"] if e.get("secretRef", {}).get("name") == "punchable-r2-recordings"]
    assert sec and not sec[0].get("optional"), "the Secret is required: a rollout before it exists must stop"
    ing = {d["metadata"]["name"]: d for d in ds if d["kind"] == "Ingress"}
    paths = lambda n: [p["path"] for rule in ing[n]["spec"]["rules"] for p in rule["http"]["paths"]]  # noqa: E731
    fetch = "/api/worker/jobs/[^/]+/file$"
    assert fetch in paths("punchable-api-rs-file") and fetch not in paths("punchable-api-rs-ids")
    ann = ing["punchable-api-rs-file"]["metadata"]["annotations"]
    assert ann["nginx.ingress.kubernetes.io/proxy-buffering"] == "off"
    assert ing["punchable-api-rs-upload"]["metadata"]["annotations"]["nginx.ingress.kubernetes.io/proxy-body-size"] == "1g"
    pol = next(d for d in ds if d["kind"] == "NetworkPolicy" and d["metadata"]["name"] == "punchable-api-rs-r2")
    assert pol["spec"]["podSelector"] == {"matchLabels": {"app": "punchable-api-rs"}} and pol["spec"]["policyTypes"] == ["Egress"]
    (rule,) = pol["spec"]["egress"]
    assert rule["ports"] == [{"protocol": "TCP", "port": 443}]
    block = rule["to"][0]["ipBlock"]
    assert block["cidr"] == "0.0.0.0/0" and {"10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "169.254.0.0/16"} <= set(block["except"])
