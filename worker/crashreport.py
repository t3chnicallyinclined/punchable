"""Crash reports from the worker: the API's crash.py + scrub.py (api-dev/, copied into the hosted image by
infra/worker/Dockerfile) under the release name punchable-worker@<version>+<sha>. The same Secret as the API
(punchable-sentry, SENTRY_DSN, optional in infra/k8s/35-worker.yaml); unset = every call here is a no-op.

What a worker event carries: the exception type, a bounded message with quoted literals blanked, the stack (file,
function, line), the release, and the tags `kind` (the job kind) and `retry`. Never a job id, a prompt, a draft or
the comic's words: api-dev/scrub.py runs on every event (api-dev/tests/test_metrics.py proves the rule).
"""
from __future__ import annotations

import sys
from pathlib import Path

API_DEV = Path(__file__).resolve().parent.parent / "api-dev"
if str(API_DEV) not in sys.path:
    sys.path.insert(0, str(API_DEV))
import crash as _C  # noqa: E402  (api-dev/crash.py)

SERVICE = "punchable-worker"


def init(log=print) -> bool:
    """On when SENTRY_DSN is set; never raises."""
    return _C.init(log=log, service=SERVICE)


def enabled() -> bool:
    return _C.enabled()


def release() -> str:
    return _C.release(SERVICE)


def report(exc: BaseException, **tags):
    """One failed job or loop error. `tags` are closed-set values only (kind, retry)."""
    _C.report(exc, tags=tags or None)


def flush(timeout: float = 2.0):
    _C.flush(timeout)
