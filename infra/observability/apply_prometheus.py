#!/usr/bin/env python3
"""Fallback for a cluster whose Prometheus is NOT run by the Prometheus Operator: install Punchable's scrape jobs and
alert rules into the plain Prometheus's ConfigMap in namespace `observability`, then reload it (no restart).

`infra/deploy.sh observability` runs this only when the ServiceMonitor CRD is absent; with the operator present,
infra/k8s/60-observability.yaml is applied instead and this script is never called. The same shape as the the other app
app's scripts/apply_prometheus_alerts.py on this box (read-only reference). Run ON the server:

    python3 infra/observability/apply_prometheus.py [--dry-run]

What it changes in ConfigMap observability/prometheus-config, idempotently:
  * data["prometheus.yml"]: a `punchable-api` and a `punchable-worker` scrape job (pod discovery in namespace punchable,
    kept by the `app` label, port 8080 / 9100) appended to scrape_configs if missing, and `punchable-alerts.yml` added
    to rule_files (the key is created if there is none);
  * data["punchable-alerts.yml"]: the rule groups of 60-observability.yaml's PrometheusRule, verbatim.
Nothing else in the ConfigMap moves (the other app's keys are left alone). It never touches the `alerting:` block: if there
is none, it says so, because then no rule anywhere on that Prometheus reaches a human.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.request

import yaml

NS, CM = "observability", "prometheus-config"
RULES_KEY = "punchable-alerts.yml"
HERE = os.path.dirname(os.path.abspath(__file__))
MANIFEST = os.path.join(HERE, "..", "k8s", "60-observability.yaml")
KUBECTL = os.environ.get("PK_KUBECTL", "microk8s kubectl").split()

JOBS = """
  # Punchable (infra/observability/apply_prometheus.py). The API serves /metrics on 8080, the worker on 9100; both
  # are reachable only from this namespace (punchable's NetworkPolicies admit `observability`).
  - job_name: 'punchable-api'
    kubernetes_sd_configs:
      - role: pod
        namespaces: { names: [punchable] }
    relabel_configs:
      - { source_labels: [__meta_kubernetes_pod_label_app], action: keep, regex: punchable-api }
      - { source_labels: [__meta_kubernetes_pod_ip], target_label: __address__, replacement: '${1}:8080' }
      - { source_labels: [__meta_kubernetes_namespace], target_label: namespace }
      - { source_labels: [__meta_kubernetes_pod_name], target_label: pod }
  - job_name: 'punchable-worker'
    kubernetes_sd_configs:
      - role: pod
        namespaces: { names: [punchable] }
    relabel_configs:
      - { source_labels: [__meta_kubernetes_pod_label_app], action: keep, regex: punchable-worker }
      - { source_labels: [__meta_kubernetes_pod_ip], target_label: __address__, replacement: '${1}:9100' }
      - { source_labels: [__meta_kubernetes_namespace], target_label: namespace }
      - { source_labels: [__meta_kubernetes_pod_name], target_label: pod }
"""


def kubectl(*args, inp: str | None = None) -> subprocess.CompletedProcess:
    return subprocess.run([*KUBECTL, *args], capture_output=True, text=True, input=inp)


def rule_groups() -> dict:
    with open(MANIFEST, encoding="utf-8") as fh:
        docs = [d for d in yaml.safe_load_all(fh) if d]
    rules = [d for d in docs if d.get("kind") == "PrometheusRule"]
    if len(rules) != 1:
        raise SystemExit(f"expected one PrometheusRule in {MANIFEST}, found {len(rules)}")
    return {"groups": rules[0]["spec"]["groups"]}


def merged_prometheus_yml(prom: str) -> tuple[str, list[str]]:
    changed = []
    if "job_name: 'punchable-api'" not in prom:
        if "scrape_configs:" not in prom:
            raise SystemExit("REFUSING: no scrape_configs in prometheus.yml")
        prom = prom.rstrip("\n") + "\n" + JOBS
        changed.append("scrape jobs punchable-api, punchable-worker")
    if RULES_KEY not in prom:
        if "rule_files:" in prom:
            # append to the existing list: `rule_files:` is a top-level key followed by `  - path` lines or a flow list
            lines = prom.split("\n")
            i = next(k for k, ln in enumerate(lines) if ln.startswith("rule_files:"))
            if lines[i].strip().endswith("]"):
                lines[i] = lines[i].rstrip()[:-1].rstrip(", ") + f", /etc/prometheus/{RULES_KEY}]"
            else:
                j = i + 1
                while j < len(lines) and lines[j].startswith("  "):
                    j += 1
                lines.insert(j, f"  - /etc/prometheus/{RULES_KEY}")
            prom = "\n".join(lines)
        else:
            prom = prom.replace("scrape_configs:", f"rule_files:\n  - /etc/prometheus/{RULES_KEY}\n\nscrape_configs:", 1)
        changed.append(f"rule_files += {RULES_KEY}")
    return prom, changed


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true", help="print what would change; touch nothing")
    a = ap.parse_args()
    r = kubectl("-n", NS, "get", "cm", CM, "-o", "json")
    if r.returncode:
        print(r.stderr, file=sys.stderr)
        return 1
    cm = json.loads(r.stdout)
    data = cm.get("data") or {}
    prom, changed = merged_prometheus_yml(data.get("prometheus.yml", ""))
    rules = yaml.safe_dump(rule_groups(), sort_keys=False, width=1000)
    if data.get(RULES_KEY) != rules:
        changed.append(f"{RULES_KEY} ({'updated' if RULES_KEY in data else 'created'})")
    if "alerting:" not in prom:
        print("WARNING: this prometheus.yml has no `alerting:` block: rules evaluate but reach no Alertmanager and no one.",
              file=sys.stderr)
    if not changed:
        print("prometheus-config already has punchable's jobs and rules; nothing to do")
        return 0
    print("changes:", "; ".join(changed))
    if a.dry_run:
        return 0
    patch = {"data": {"prometheus.yml": prom, RULES_KEY: rules}}
    r = kubectl("-n", NS, "patch", "cm", CM, "--type=merge", "-p", json.dumps(patch))
    if r.returncode:
        print(r.stderr, file=sys.stderr)
        return 1
    print("patched; the kubelet refreshes the mounted directory within about a minute. Then reload:")
    print(f"  {' '.join(KUBECTL)} -n {NS} exec deploy/prometheus -- wget -qO- --post-data='' http://localhost:9090/-/reload")
    print("Verify: targets punchable-api and punchable-worker are UP at :9090/targets and the rules show at :9090/rules.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
