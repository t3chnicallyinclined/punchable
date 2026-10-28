# Observability

What the hosted Punchable measures, what wakes the owner, where to look, and how crashes from the phone, the web app
and the API are collected. Everything here mirrors the conventions of the other app on the same MicroK8s box
(Prometheus + Grafana + the shared Alertmanager in namespace `observability`, the `release: prometheus` label, alerts
labelled `severity` + `service`), so a Punchable alert travels the same path as every other alert on that box.
Nothing in a metric label, a log line or a crash report is the comic's material: labels are route patterns, job kinds,
model ids and status codes; crash reports are scrubbed before they leave (below).

## What is measured

**API** (`GET /metrics` on the API pod, port 8080; `api-dev/metrics.py`). Open, no token, reachable only in-cluster:
the public Ingress forwards `/api` and `/`, never `/metrics`, and the NetworkPolicy admits only the `observability`
namespace to the port.

| Metric | Labels | What |
|---|---|---|
| `pk_http_requests_total` | method, route, status | every request; `route` is a **pattern** (`/api/joke/{id}`), never a path with an id or a query |
| `pk_http_request_duration_seconds` | method, route | latency histogram |
| `pk_http_requests_in_flight` | | requests being served now (`/api/live` and the scrape itself excluded) |
| `pk_live_clients` | | open `/api/live` WebSockets (phones and tabs in front) |
| `pk_jobs_queued` | kind | queued jobs by kind, read from the job store at scrape time |
| `pk_jobs_waiting_transcriber` / `..._oldest_seconds` | | queued jobs no server worker runs (transcription is on the phone), and the oldest one's age |
| `pk_db_query_duration_seconds` | | every SurrealDB round trip (`db/sdb.py Rpc.query`, the one choke point) |
| `pk_build_info` | version, sha | 1; the image's git sha (`PK_BUILD_SHA`, set by `deploy.sh build`) |

**Worker** (`GET /metrics` on the worker pod, port 9100; `worker/metrics.py`; the only thing the worker listens on).

| Metric | Labels | What |
|---|---|---|
| `pk_worker_jobs_claimed_total` / `_done_total` / `_failed_total` | kind (+ retry) | the job loop |
| `pk_worker_job_duration_seconds` | kind | wall time per job |
| `pk_ai_spend_usd_today`, `pk_ai_budget_usd`, `pk_ai_paused` | | today's USD vs `PK_AI_DAILY_USD` and whether the worker is refusing jobs (`engine/budget.py`) |
| `pk_ai_calls_total`, `pk_ai_tokens_total` | model (+ direction: input, output, cache_read, cache_write) | from the SDK's usage numbers on every charge |
| `pk_ai_spend_usd_total` | model, task | USD since the process started |
| `pk_worker_heartbeat_age_seconds` | | seconds since the poll loop last ticked |
| `pk_worker_build_info` | version, sha | |

Everything else on the dashboard comes from the cluster's own exporters: kube-state-metrics (restarts, the backup
CronJob's last success, failed Jobs, Bugsink's available replicas) and cAdvisor (pod memory).

Every metric is capped at 200 distinct label sets (`engine/prom.py`); past that, new label values collapse into one
`other` series. The tests (`api-dev/tests/test_metrics.py`) assert that 500 different joke ids produce one route
series and that no label value on the page is an id.

## Alerts

`infra/k8s/60-observability.yaml` (PrometheusRule `punchable-alerts`, every rule labelled `service: punchable` with a
`severity`). Each alert's `runbook` annotation is in the alert itself; this is the short list.

| Alert | Fires when | Severity | What to do |
|---|---|---|---|
| PunchableApiDown | `up{job="punchable-api"} == 0` for 2 m | critical | pods, logs, `ship.sh status`; a pod stuck not-ready is materializing the bundle; roll back to the previous digest |
| PunchableApiHighErrorRate | > 5 % 5xx for 5 m | critical | the "5xx by route" panel, the traceback in Bugsink (route tag) and the API log; every route failing at once = SurrealDB |
| PunchableApiHighLatency | p95 > 2 s for 10 m (uploads, audio excluded) | warning | "p95 by route" and "SurrealDB round trips"; memtable / memory |
| PunchableWorkerDown | worker not scrapeable 5 m | warning | usually a missing Secret at start; jobs stay queued |
| PunchableWorkerHeartbeatStale | heartbeat age > 10 m | warning | a hung job; the liveness probe restarts at 20 m; restart sooner by hand if needed |
| PunchableAiBudgetPaused | `pk_ai_paused == 1` for 15 m | warning | expected after a heavy day; raise `PK_AI_DAILY_USD` + `rollout worker` to continue today |
| PunchableJobsWaitingForTranscriber | oldest waiting job > 24 h, for 1 h | info | nothing broken on the server: the owner finishes it on the phone, or drops it |
| PunchableSurrealDbRestarted | any restart of `surrealdb-0` in 1 h | warning | `describe pod`: OOMKilled at 512 Mi → memtable; then confirm the last backup is recent |
| PunchableBackupFailed | a `punchable-backup*` Job failed, 10 m | critical | the Job's log (one redacted line): R2 token, a refused export, SurrealDB down; `ship.sh backup-now` |
| PunchableBackupMissed | no success for 36 h (VERIFY the metric, below) | critical | CronJob suspended? missed its deadline? `backup-now`, `backup-list` |
| PunchableCertificateExpiringSoon | < 14 days left (VERIFY, below) | warning | renewal failing: `describe certificate`, challenges, DNS, the acme-solver NetworkPolicy |
| PunchablePodCrashLooping | > 3 restarts in 1 h | warning | `describe pod`, `logs --previous` |
| PunchableBugsinkDown | no available replica 10 m | warning | its log; a PVC or SECRET_KEY problem; `ship.sh bugsink` |

**Where they go.** The same place as every other alert on the box: the shared Alertmanager in namespace
`observability` (routes on `severity`: critical repeats hourly, the rest every 4 h; inhibits on `service`), which
emails the owner, and the host's alert poller, which posts firing alerts to the owner's chat channel. No new
receiver, no new credential.

**Two rules to VERIFY on the live Prometheus before trusting them** (a rule on a metric nobody emits never fires and
looks exactly like a rule that is passing):

```
# at :9090/graph on the observability Prometheus
kube_cronjob_status_last_successful_time{namespace="punchable"}        # PunchableBackupMissed (kube-state-metrics >= 2.x)
certmanager_certificate_expiration_timestamp_seconds{namespace="punchable"}   # PunchableCertificateExpiringSoon (cert-manager scraped?)
```

If either returns nothing, that alert is decoration until the exporter is scraped; say so in the runbook rather
than leaving it looking green.

## Deploying it

```bash
infra/ship.sh build api && infra/ship.sh rollout api        # /metrics, the Sentry hook, PK_BUILD_SHA
infra/ship.sh build worker && infra/ship.sh rollout worker  # :9100/metrics, the metrics Service, the NetworkPolicy
infra/ship.sh observability                                 # ServiceMonitors + PrometheusRule, the dashboard ConfigMap
```

`observability` applies `60-observability.yaml` when the cluster has the Prometheus Operator CRDs. If it does not (the
other app's live Prometheus on this box was a plain Deployment with a ConfigMap, despite its ServiceMonitors), the
step runs `infra/observability/apply_prometheus.py` instead, which puts the same two scrape jobs and the same rule
groups into `observability/prometheus-config` and tells you how to reload. Either way, verify on Prometheus:
targets `punchable-api` and `punchable-worker` are UP at `:9090/targets`, the rules show at `:9090/rules`.

## The dashboard

`infra/grafana/punchable.json` ("Punchable", uid `punchable-main`). `ship.sh observability` puts it in a ConfigMap
`grafana-dashboard-punchable` in namespace `observability` labelled `grafana_dashboard=1`, where this cluster's Grafana
picks dashboards up. **A changed dashboard needs `kubectl -n observability rollout restart deploy/grafana`**: Grafana
mounts each file with `subPath`, and Kubernetes never updates a subPath mount in place (the other app learned this the
hard way). Open it at the Grafana the estate already runs, folder of the sidecar's choosing, search "Punchable".

Rows: **Overview** (firing alerts, API/worker up, live clients, 5xx, build; requests and 5xx by route; p50/p95/p99;
p95 by route; in-flight and status mix; SurrealDB round trips), **Jobs** (queue depth by kind; waiting for a
transcriber and the oldest one's age; worker jobs/min by kind and outcome; job duration p95; heartbeat age), **AI
spend** (today vs budget as a gauge; spent/cap/paused; tokens/min by model and direction; USD/hour by model and task;
model calls/min), **Data** (SurrealDB restarts; age of the last successful backup; failed backup Jobs; Bugsink up; pod
memory vs limits; pod restarts).

The size of the last R2 object is not a Prometheus number (nothing on this box pushes from a CronJob):
`infra/ship.sh backup-list` prints it.

## Adding a metric

1. API: in `api-dev/metrics.py`, `registry.counter(...)` / `gauge(...)` / `histogram(...)`; increment where it happens,
   or give a gauge `collect=` to read it at scrape time. Worker: the same in `worker/metrics.py`.
2. Labels: a closed set (a kind, a model, a status, a route pattern). Never an id, a title, a file name, a query. If
   a value could be free text, do not make it a label.
3. Add it to `infra/grafana/punchable.json` and, if it should wake someone, a rule in `60-observability.yaml` with
   `service: punchable`, a `severity` and a `runbook`.
4. `python -m pytest api-dev/tests/test_metrics.py worker/tests/test_metrics.py infra/test_k8s.py -q`: the k8s test
   refuses a rule or a panel that names a `pk_*` metric nothing emits.
5. `ship.sh rollout api|worker`, `ship.sh observability`, restart Grafana.

## Crash reports (Bugsink)

Crashes and errors from the phone app, the web app and the API go to a self-hosted **Bugsink** at
`https://bugs.<domain>` (`infra/k8s/70-bugsink.yaml`: MIT, one container, SQLite on a 2 Gi PVC, 128 Mi requested).
It speaks the Sentry protocol, so the SDKs are the standard ones (`@sentry/react-native` with its Expo plugin on the
app, `sentry-sdk` on the API); only the DSN points at our box. No Sentry account, no data leaves the server.

### What is sent, and what never is

Both SDKs run every event through a scrubber before it leaves the device or the pod (`app/src/lib/crash.ts`,
`api-dev/scrub.py`):

* request bodies, query strings, cookies and all headers but an allow-list: removed;
* any field, at any depth, named `text`, `quote`, `transcript`, `query`, `title`, `name`, `venue`, `filename`,
  `token`, `password`, `cookie`, ... (and any key containing those words): replaced with `[scrubbed]`;
* breadcrumbs keep only category, level and, for API calls, the **route pattern** + method + status; console
  breadcrumbs are dropped entirely; stack-frame locals and source context are dropped;
* the user is an opaque id; the device/OS context is kept (that is what explains a crash).

What survives: the exception type, a bounded message with quoted literals blanked, the stack (file, function, line,
column), the release (`punchable@<version>+<build>` on the app, `punchable-api@<version>+<sha>` on the API), the
environment (`production` | `preview` | `dev`), and the API breadcrumbs.

Both scrubbers have a test that proves the rule is load-bearing by breaking it (`app/tests/crash.test.mjs`,
`api-dev/tests/test_metrics.py::test_the_scrubber_test_goes_red_when_the_rule_is_broken`). Run them before touching
either file:

```bash
cd app && node --experimental-strip-types --no-warnings tests/crash.test.mjs
python -m pytest api-dev/tests/test_metrics.py -q
```

### Setting it up, in order (the orchestrator runs these; the owner does the two UI steps)

1. **DNS**: `infra/ship.sh dns plan` shows the new `bugs.<domain>` A record; `infra/ship.sh dns apply` creates it.
2. **Secret**: `infra/ship.sh bugsink-secret`. The server generates `SECRET_KEY` (kept on every later run). If
   `~/.punchable/secrets.env` has a line `BUGSINK_SUPERUSER=you@example.com:<a long password>`, it becomes the first
   user (Bugsink uses it only while no user exists); otherwise create the user afterwards with
   `microk8s kubectl -n punchable exec -it deploy/punchable-bugsink -- bugsink-manage createsuperuser`.
3. **Deploy**: `infra/ship.sh bugsink` (PVC, Deployment, Service, Ingress; waits for the certificate). Pin the image by
   digest in `70-bugsink.yaml` after the first pull (`docker pull bugsink/bugsink:2.6.2` then
   `docker inspect --format '{{index .RepoDigests 0}}' bugsink/bugsink:2.6.2`; `docker pull` is safe on the box,
   `docker run` is not).
4. **Projects** (done 2026-10-28 from the Bugsink pod with `bugsink-manage shell`, no UI step needed): team
   `Punchable`, projects **`punchable-api`** (id 1) and **`punchable-app`** (id 2), the superuser a member of both.
   Each project page at `https://bugs.<domain>` shows its **DSN** (`https://<key>@bugs.<domain>/<id>`). A DSN only
   lets a client *submit* events; it is still kept out of the public repo so nobody can spam the store.
5. **API**: the `punchable-api` DSN is the `SENTRY_DSN=` line of `~/.punchable/secrets.env`; `infra/ship.sh sentry-secret`
   writes the Secret. A changed Secret needs a NEW pod: `rollout api` re-applies the same digest and changes nothing, so
   `kubectl rollout restart deploy/punchable-api -n punchable` (done by hand 2026-10-28). The API log then says
   `crash reporting on: punchable-api@...`; a `capture_message` from the pod showed up as issue 1 within seconds.
6. **App**: the `punchable-app` DSN is the `APP_SENTRY_DSN=` line of `~/.punchable/secrets.env` and an **EAS environment
   variable** `EXPO_PUBLIC_SENTRY_DSN` (scope project, visibility sensitive) in the `production`, `preview` and
   `development` environments (`eas env:list --environment production`). Each build profile in `app/eas.json` names
   its `environment`, which is how the variable reaches the build; nothing is written into the repo. Unset = off.
   For a local dev client, export `EXPO_PUBLIC_SENTRY_DSN=...` before `npx expo start`; the dev server is left without
   it on purpose so dev-build noise stays out of the store.
7. **Source maps** (so a TestFlight stack trace shows file and line instead of minified offsets). In the Bugsink UI,
   create an **auth token** (Settings / API tokens). Store it as an EAS secret, never in the repo:

   ```bash
   cd app && eas secret:create --scope project --name SENTRY_AUTH_TOKEN --value '<the token from Bugsink>' --type string
   ```

   Then remove `"SENTRY_DISABLE_AUTO_UPLOAD": "true"` from the `production` and `preview` profiles in `app/eas.json`.
   It is there so that a build never fails for want of a token: with it removed and the secret present, the Expo plugin
   (`@sentry/react-native/expo` in `app.json`, `url: https://bugs.<domain>`, organization `bugsinkhasnoorgs` because
   Bugsink has no organizations) uploads the build's bundle and source map, stamped with the Debug ID that
   `app/metro.config.js` adds, during the EAS build. Bugsink >= 2.0.14 accepts `sentry-cli sourcemaps upload`; the
   same upload by hand is

   ```bash
   SENTRY_AUTH_TOKEN=... npx sentry-cli --url https://bugs.<domain>/ sourcemaps --org bugsinkhasnoorgs --project punchable upload <dir with the bundle and .map>
   ```

   This path has not yet been exercised against this Bugsink: the first production build after the switch is the
   test. If the upload step fails, put `SENTRY_DISABLE_AUTO_UPLOAD` back and file it.
8. **Backups**: the nightly Job now also copies Bugsink's SQLite (sqlite3's online backup API, gzip, integrity check,
   `bugsink/daily|weekly/` in the same bucket, same 14 + 8 retention). `infra/ship.sh backup` re-applies the CronJob
   with the PVC mount; it refuses until the Bugsink PVC exists. Restore: `backup.py fetch bugsink/daily/<key> db.gz`,
   gunzip, copy over `/data/db.sqlite3` with the Deployment scaled to 0.

### Verifying a crash lands

From a phone build with the DSN set, Settings has nothing to tap on purpose; use a development build and throw from
the console (`throw new Error('test crash')`), or on the API `kubectl -n punchable exec deploy/punchable-api -- python -c "import crash; crash.init(); crash.report(RuntimeError('test')); crash.flush()"`.
The event appears in the Bugsink project within seconds, with no text field in it. If this had been in place on
2026-10-27, the TestFlight launch crash (a module resolving to itself) would have shown as a native crash with the
build number in `dist` the moment the first tester opened the app.
