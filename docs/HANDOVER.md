# Punchable: start here (handover, 2026-10-28)

Read this first in a new session. It says what is live, what learns, what is open, and the traps that cost time
today. It is public-safe: no secrets, no server names, no material. Private operating state (deploy log with image
digests and rollbacks, token notes) lives in the owner's private operations repo (its `infra/` deploy log); secrets and private
settings live in `~/.punchable/` and are never printed or committed.

## What Punchable is

One comic's joke-writing app. It learns **his** voice from **his** material only (notes, Google Docs, taped sets) and
every AI line that claims to be his is checked word for word against his notes first. Personal use first; the hosted
copy at punchable.io is his. Product direction: `docs/UX_2026-10.md`. iPhone design rules: `docs/design/HIG.md`.
Engineering rules (platform splits, CI gates): `docs/ENGINEERING.md`. Phone operating manual: `docs/IOS.md`.

## Live right now

| Surface | State |
|---|---|
| Web | punchable.io, main at `e4c5057` (plus this doc). Tabs Tonight · Bits · Sets. |
| iPhone | TestFlight **0.1.0 (9)**, built from `e4c5057`, auto-submitted. Builds 4-6 crashed (self-import, see ENGINEERING.md §1); 7, 8, 9 are good. |
| API + hosted worker | Python API behind Keycloak; the worker runs Claude (Opus 5.5 riff / punch-up / set analysis, Sonnet 5.5 ask / bridge, Haiku 5.5 merge / triage) with a daily budget. No home PC in the loop. |
| Database | Hosted SurrealDB is the single source of truth (online-first since 2026-10-27). Nightly backup Job to R2 with a bucket-scoped token; the Bugsink database rides along. |
| Crash reporting | Self-hosted Bugsink at bugs.punchable.io. API wired and proven end to end. App DSN is an EAS environment variable (`EXPO_PUBLIC_SENTRY_DSN`, pulled by each `eas.json` profile's `environment`). **Worker not wired yet.** |
| Metrics | Prometheus scrape of API + worker, Grafana dashboard, 13 alert rules. **Two alerts can never fire** (backup missed, certificate expiring): the cluster has no kube-state-metrics and cert-manager is not scraped. Fix needs the owner's go (shared namespace). |
| DNS | `infra/dns/cf_dns.py plan` / `apply` with a token scoped to this one zone (DNS Write + Zone Read). Records: apex, auth, bugs. |

## What the app does today (phone + web)

- **Tonight**: tonight's set list as one bar, "N to sort" inbox, three questions to ask the material, a system search
  field ("Ask your material", typed only). "+" opens the create sheet. No input bar in the tab bar (owner decision).
- **Bits**: every joke with versions and stage history; the **galaxy map** (`/map`: clusters by likeness, edge kinds by
  colour and dash, status by shape, shuffle / centre / find / legend isolate); write + riff; merge same-idea notes.
- **Sets**: set lists (boards) with timing, gaps, bridges; the **stage** screen (`/stage/[id]`) with the set recorder.
- **Recorder** (sets only; jots and questions are text): 48 kHz stereo AAC .m4a, keep-awake while taping, real dBFS
  meter on Record and Stage, a phone call pauses the clock ("Paused by a call", 60-pt Resume), a media-services reset
  keeps the partial file, built-in mic preferred over a Bluetooth headset mic. Logic in `app/src/lib/recorderLogic.ts`
  (32 unit checks). Measurement mode / AGC off needs a config plugin and is not done.
- **Settings › Voice profile**: his hand-written profile (edit, save, undo) and **"What it has learned"**.

## What learns (built 2026-10-28)

Before today the voice profile was a static document and nothing he did with drafts reached a prompt. Now:

| Signal | Where it is recorded | How it reaches the model |
|---|---|---|
| Riff / punch-up line he keeps ("Use as mine", including one he edited first) | `profile:voice.learned.kept_lines` (cap 25) via the `draft: true` flag on the save | second cached system block after the profile, writing tasks only |
| Line he dismisses | `.dropped_lines` (cap 25) via `POST /api/profile/voice/learn` | same block ("lines he passed over") |
| Laugh marks from tapes | `.laugh_lines` (cap 15) | same block; **empty until audio processing runs on the hosted worker** |
| Locks on a set | board state | already in the set-analysis prompt |

Code: `creative/learned.py`, `creative/voice.py` (`learned_block`, `cacheable_learned`), `api-dev/profile_api.py`,
`app/src/lib/simple.ts` (`saveIdea`, `dismissDraft`), `app/src/app/(tabs)/(tonight)/learned.tsx`. The block is only
cached as a prefix together with the profile (alone it is under the 512-token cache minimum).

## Standing owner decisions (do not re-open)

- Phone + frontier models only. The home PC / GPU worker is retired, never proposed.
- TestFlight-only distribution; email-only sign-in on the phone; Google sign-in on the web only.
- Jots and questions are typed. Voice is for recording SETS only.
- HIG first on iPhone: standard, familiar, one-handed in the dark. Dark default, light must work.
- No review step is ever required; AI inferences show as the current state with an AI badge and he edits.
- No Sentry SaaS: crash reports go to the self-hosted Bugsink.
- Nothing private in this public repo (material, hostnames beyond punchable.io / auth / bugs, secrets, other projects).
  The commit guard blocks it; fix the content, never bypass.
- Court documents are never ingested; the legal rules run on every ingest and request path.

## How to work (the short version)

```bash
# gates before any commit (all must be clean)
python -m pytest creative engine gsync worker infer/tests tools/audio/tests api-dev/tests -q
cd app && npx tsc --noEmit && npm run test:unit && npx expo-doctor
# ship (from the repo root; each step logs to the private deploy log with a rollback line)
bash infra/ship.sh build api && bash infra/ship.sh rollout api      # same for worker, web
# phone
cd app && EAS_NO_VCS=1 npx eas-cli build --profile production --platform ios --non-interactive --auto-submit
EXPO_PUBLIC_API_URL=https://punchable.io/api EXPO_PUBLIC_AUTH=1 npx expo start --dev-client --tunnel
```

Commit with the owner's git identity (the one on the existing commits, passed with `git -c user.name=… -c user.email=… -c core.autocrlf=false commit`),
stage paths explicitly (never `git add -A`), message ending with a `Co-Authored-By:` line for the agent that helped.

## Traps that cost time (each happened at least once)

| Trap | Rule |
|---|---|
| `*.native.tsx` importing its own basename | Base + shim + `.native`; never import `./<own name>` (ENGINEERING.md §1; static tests pin it). |
| Worktree with an `app/node_modules` junction | Delete the junction FIRST (PowerShell `(Get-Item <path> -Force).Delete()`), then `git worktree remove`, then `git branch -d`. Both `Remove-Item -Recurse` and `git worktree remove --force` follow the junction into the real `node_modules` (restore: `npm ci`). |
| `ship.sh rollout <svc>` on the same image digest | Does not restart pods. After a Secret change run `kubectl rollout restart deploy/<name>`. |
| A Job that opens Bugsink's SQLite as another uid | Creates `-shm`/`-wal` files Bugsink cannot write ("readonly database"). The backup Job runs as Bugsink's uid; `infra/test_k8s.py` pins it. |
| `npx eas …` | "could not determine executable": use `npx eas-cli …` with `EAS_NO_VCS=1`. |
| The DNS tool's plan vs its docstring | A record must be in `WANT` in `cf_dns.py`, not only in the comment. |
| Hand-listed modules in `api/Dockerfile` | Glob the directory and keep the build-time import-all check. |
| Shared Keycloak rolling update | Cannot surge on this node: a theme change is a short outage for every realm. Needs the owner's go. |
| Never on the server | `docker run` (restarts the whole cluster), `ship.sh test`, touching other namespaces beyond the agreed Keycloak theme mount and the observability ConfigMaps. |

## Open, in order

1. **Owner checks on build 9**: tape a short set from Record, clap near the phone and watch the meter; riff a bit, keep
   one line and dismiss another, then Settings › Voice profile › "What it has learned" should list both; Bits › Map.
2. **"go ksm"**: deploy kube-state-metrics and a cert-manager scrape in the shared observability namespace so the two
   dead alerts work (config reload, no app downtime).
3. Worker crash reporting: `SENTRY_DSN` into `infra/k8s/35-worker.yaml` from the same Secret as the API.
4. Laugh detection on the hosted worker (audio processing returns `laugh_lines`; today the set's laugh map is frozen).
5. Measurement mode / AGC off (expo-audio config plugin), only if a real tape sounds pumped against a plain Voice Memo.
6. Source maps for app crashes (EAS secret `SENTRY_AUTH_TOKEN`, drop `SENTRY_DISABLE_AUTO_UPLOAD`).
7. Remove `holdToTalk.native.ts` (jots are typed now).
8. Move the Google Docs sync off the PC (last PC dependency).
9. `docs/design/ARCHITECTURE.md` performance items (fonts, cold start, storage on iOS); prompt-audit items 6 / 8 / 9.
10. Pin the Bugsink image by digest.
11. Riff eval: five BEFORE/AFTER pairs await the owner's judgment (private folder).
