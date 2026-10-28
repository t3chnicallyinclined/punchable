"""Crash reporting for the hosted API and the hosted worker: sentry-sdk pointed at the self-hosted Bugsink
(infra/k8s/70-bugsink.yaml). The worker uses it through worker/crashreport.py (this file and scrub.py are copied into
its image) with service "punchable-worker".

    SENTRY_DSN          from the Secret punchable-sentry (infra/ship.sh sentry-secret), read by both Deployments. Empty
                        or unset = off: the process runs exactly as before and `report()` is a no-op.
    SENTRY_ENVIRONMENT  production (default) | preview | dev
    PK_BUILD_SHA        the image's git sha (api/Dockerfile, infra/worker/Dockerfile) -> release
                        "<service>@<version>+<sha12>", service = punchable-api (default) | punchable-worker

Every event passes api-dev/scrub.py before it leaves (no request bodies, no text/quote/transcript/query fields,
no headers beyond an allow-list, no local variables). sentry-sdk is an optional import: the dev API and the tests
never need it.
"""
from __future__ import annotations

import os
import sys
import threading
from pathlib import Path

HERE = Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))
import scrub as SC  # noqa: E402

VERSION = "0.1.0"
SERVICE = "punchable-api"
_sdk = None
_lock = threading.Lock()


def release(service: str | None = None) -> str:
    return f"{service or SERVICE}@{VERSION}+{(os.environ.get('PK_BUILD_SHA') or 'dev')[:12]}"


def enabled() -> bool:
    return _sdk is not None


def init(dsn: str | None = None, log=print, service: str | None = None) -> bool:
    """Start the SDK when a DSN is set. Returns True when reporting is on. Never raises: an unreachable Bugsink or a
    missing package must not stop the API (or the worker). `service` names the release ("punchable-api" by default);
    it is passed through, never stored, so one process (the tests) can hold both names."""
    global _sdk
    dsn = (dsn if dsn is not None else os.environ.get("SENTRY_DSN", "")).strip()
    if not dsn:
        return False
    try:
        import sentry_sdk
        from sentry_sdk.integrations.threading import ThreadingIntegration
    except ImportError:
        log("crash reporting: SENTRY_DSN is set but sentry-sdk is not installed; off")
        return False
    try:
        sentry_sdk.init(
            dsn=dsn,
            release=release(service),
            environment=os.environ.get("SENTRY_ENVIRONMENT", "production"),
            before_send=SC.scrub_event,
            before_breadcrumb=lambda bc, hint: SC.scrub_breadcrumb(bc) or None,
            send_default_pii=False,
            include_local_variables=False,
            include_source_context=False,
            max_breadcrumbs=30,
            traces_sample_rate=0.0,
            profiles_sample_rate=0.0,
            attach_stacktrace=False,
            default_integrations=True,
            auto_enabling_integrations=False,
            integrations=[ThreadingIntegration(propagate_scope=True)],
            # the DSN host is the only place events go; nothing else is contacted
        )
    except Exception as e:  # noqa: BLE001
        log(f"crash reporting: init failed ({type(e).__name__}); off")
        return False
    with _lock:
        _sdk = sentry_sdk
    log(f"crash reporting on: {release(service)} -> {dsn.split('@')[-1].split('/')[0]}")
    return True


def report(exc: BaseException, route: str | None = None, status: int | None = None, tags: dict | None = None):
    """One handled exception (the API's 500 paths; a worker job that failed). `route` is a route pattern
    (api-dev/metrics.route_pattern), never the raw path. `tags` are short closed-set values (the worker's job kind
    and whether it was retried), never an id or text; scrub.py drops any tag whose name sounds private anyway."""
    sdk = _sdk
    if sdk is None:
        return
    try:
        with sdk.new_scope() as scope:
            if route:
                scope.set_tag("route", route)
            if status:
                scope.set_tag("status", str(status))
            for k, v in (tags or {}).items():
                scope.set_tag(str(k), str(v)[:64])
            sdk.capture_exception(exc)
    except Exception:  # noqa: BLE001  reporting must never add a second failure
        pass


def breadcrumb(route: str, method: str, status: int):
    sdk = _sdk
    if sdk is None:
        return
    try:
        sdk.add_breadcrumb(category="http", level="warning" if status >= 500 else "info",
                           data={"route": route, "method": method, "status": status})
    except Exception:  # noqa: BLE001
        pass


def flush(timeout: float = 2.0):
    sdk = _sdk
    if sdk is not None:
        try:
            sdk.flush(timeout=timeout)
        except Exception:  # noqa: BLE001
            pass
