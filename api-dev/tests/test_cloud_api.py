"""The cloud's maths (api-dev/cloud_api.py): the projection, the clusters, their names. Pure numpy, no data.

    python -m pytest api-dev/tests/test_cloud_api.py -q
"""
from __future__ import annotations

import sys
from pathlib import Path

import numpy as np

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "api-dev"))
sys.path.insert(0, str(DESK))
import cloud_api as CL  # noqa: E402


def blobs(seed=3):
    """Three tight groups of unit vectors in 384-d, far apart."""
    rng = np.random.default_rng(seed)
    centres = rng.normal(size=(3, CL.DIM if hasattr(CL, "DIM") else 384))
    rows, labels = [], []
    for c, mu in enumerate(centres):
        for _ in range(20):
            v = mu + rng.normal(scale=0.15, size=mu.shape)
            rows.append(v / np.linalg.norm(v)); labels.append(c)
    return np.asarray(rows, np.float32), labels


def test_projection_is_three_axes_in_minus_one_to_one_and_stable():
    X, _ = blobs()
    P = CL.project(X, 3)
    assert P.shape == (60, 3) and P.min() >= -1 and P.max() <= 1
    assert np.allclose(P, CL.project(X, 3))                    # the same data lands the same way
    assert CL.project(X[:1], 3).shape == (1, 3) and CL.project(np.zeros((0, 384)), 3).shape == (0, 3)
    # the three groups are apart in the projection, not only in 384-d
    a, b, c = P[:20].mean(0), P[20:40].mean(0), P[40:].mean(0)
    assert min(np.linalg.norm(a - b), np.linalg.norm(b - c), np.linalg.norm(a - c)) > 0.5


def test_kmeans_recovers_the_groups_and_is_deterministic():
    X, truth = blobs()
    labels, centres = CL.kmeans(X, 3)
    assert centres.shape == (3, 384)
    # every group maps to one cluster id, and no two groups share one
    seen = {}
    for lab, t in zip(labels, truth):
        seen.setdefault(t, set()).add(int(lab))
    assert all(len(v) == 1 for v in seen.values()) and len({next(iter(v)) for v in seen.values()}) == 3
    assert np.array_equal(labels, CL.kmeans(X, 3)[0])
    assert CL.pick_k(1588) == 14 and CL.pick_k(60) == 6 and CL.pick_k(5) == 3 and CL.pick_k(1) == 1


def test_cluster_names_are_the_over_represented_title_words():
    names = CL.label_clusters({
        0: ["Parking meters at night", "The parking meter that talks", "Parking in the city", "Meters and tickets"],
        1: ["Dating apps are interviews", "An app for dating my cat", "Dating after forty", "Apps I deleted"],
        2: ["A bit about nothing"],
    })
    assert names[0].startswith("parking") and "meters" in names[0]
    assert "dating" in names[1] and "apps" in names[1]
    assert names[2] not in ("", "—") and "bit" not in names[2]   # a stop word never names a cluster
    assert CL.label_clusters({3: []})[3] == "—"
