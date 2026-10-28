"""Build the strategic-merge patch that mounts the punchable login theme on the shared Keycloak Deployment.

    python3 infra/keycloak/theme_patch.py <deployment.json> <patch-out.json>

Mirrors how the other products' themes are mounted there: a ConfigMap volume `punchable-configmap` at /punchable-src
in the `setup-theme` init container, an emptyDir `punchable-theme` the init container fills (/punchable) and the
keycloak container mounts at /opt/keycloak/themes/punchable, and four copy lines appended to the init container's
one `sh -c` script. Idempotent: prints "unchanged" and writes nothing when the Deployment already has all of it.
Exit 0 = patch written, 3 = unchanged, 1 = the Deployment doesn't look like the one this was written for.
"""
import json
import sys

COPY = ("mkdir -p /punchable/login/resources/css /punchable/login/messages\n"
        "cp /punchable-src/*.ftl /punchable-src/theme.properties /punchable/login/\n"
        "cp /punchable-src/messages_en.properties /punchable/login/messages/\n"
        "cp /punchable-src/login.css /punchable/login/resources/css/login.css\n")


def build(d: dict) -> dict | None:
    spec = d["spec"]["template"]["spec"]
    changed = False
    vols = spec.setdefault("volumes", [])
    names = {v["name"] for v in vols}
    if "punchable-configmap" not in names:
        vols.append({"name": "punchable-configmap", "configMap": {"name": "keycloak-theme-punchable"}})
        changed = True
    if "punchable-theme" not in names:
        vols.append({"name": "punchable-theme", "emptyDir": {}})
        changed = True
    ic = next((c for c in spec.get("initContainers", []) if c["name"] == "setup-theme"), None)
    if ic is None:
        raise SystemExit("!! no `setup-theme` init container on deploy/keycloak; look at the Deployment by hand")
    mounts = {m["name"] for m in ic.setdefault("volumeMounts", [])}
    if "punchable-configmap" not in mounts:
        ic["volumeMounts"].append({"name": "punchable-configmap", "mountPath": "/punchable-src"})
        changed = True
    if "punchable-theme" not in mounts:
        ic["volumeMounts"].append({"name": "punchable-theme", "mountPath": "/punchable"})
        changed = True
    for key in ("command", "args"):
        lst = ic.get(key) or []
        for i, a in enumerate(lst):
            if "cp /" in a:
                if "/punchable-src" not in a:
                    lst[i] = a.rstrip("\n") + "\n" + COPY
                    changed = True
                break
        else:
            continue
        break
    else:
        raise SystemExit("!! setup-theme has no `sh -c` copy script to append to; look at the Deployment by hand")
    mc = next(c for c in spec["containers"] if c["name"] == "keycloak")
    if "punchable-theme" not in {m["name"] for m in mc.setdefault("volumeMounts", [])}:
        mc["volumeMounts"].append({"name": "punchable-theme", "mountPath": "/opt/keycloak/themes/punchable"})
        changed = True
    if not changed:
        return None
    return {"spec": {"template": {"spec": {"volumes": vols, "initContainers": spec["initContainers"],
                                           "containers": spec["containers"]}}}}


if __name__ == "__main__":
    src, out = sys.argv[1], sys.argv[2]
    patch = build(json.load(open(src)))
    if patch is None:
        print("unchanged: deploy/keycloak already mounts the punchable theme")
        sys.exit(3)
    json.dump(patch, open(out, "w"))
    print(f"patch written: {out}")
