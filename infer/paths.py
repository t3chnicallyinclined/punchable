"""Locations. COMEDY_DIR overrides the private-material root."""
import os

HERE = os.path.dirname(os.path.abspath(__file__))
DESK = os.path.dirname(HERE)
COMEDY = os.environ.get("COMEDY_DIR", os.path.expanduser("~/punchable-data"))
SETS = os.path.join(COMEDY, "corpus", "sets")
UNITS = os.path.join(COMEDY, "corpus", "gdocs_units.json")
KB_V0 = os.path.join(COMEDY, "kb", "v0")
GRAPH = os.path.join(KB_V0, "graph.json")
INFERRED = os.path.join(KB_V0, "inferred.json")
V1_BATCHES = os.path.join(COMEDY, "kb", "v1", "batches")
# Judgments on your own sets are your material: kept in the data folder (PK_INFER_JUDGE_DIR to move them).
JUDGE = os.environ.get("PK_INFER_JUDGE_DIR") or os.path.join(COMEDY, "kb", "infer_judge")
_CACHE_ROOT = os.environ.get("PK_CACHE_DIR")  # caches outside the repo: <PK_CACHE_DIR>/infer and /kb
CACHE = os.path.join(_CACHE_ROOT, "infer") if _CACHE_ROOT else os.path.join(HERE, "cache")
KB_VECTORS = os.path.join(_CACHE_ROOT, "kb") if _CACHE_ROOT else os.path.join(DESK, "tools", "kb", "cache")  # read-only reuse
SPIKE = os.path.join(DESK, "spikes", "01-vector-graph")  # transformers.js install (read-only)


def set_dirs():
    return sorted(d for d in os.listdir(SETS)
                  if os.path.isdir(os.path.join(SETS, d)) and os.path.exists(os.path.join(SETS, d, "transcript.json")))


def set_id(set_dir):
    # graph uses set:<YYYY-MM-DD>_<ytid>; analysis.json uses the dir name "<date>_<ytid>"
    return f"set:{set_dir[:4]}-{set_dir[4:6]}-{set_dir[6:8]}_{set_dir.split('_', 1)[1]}"
