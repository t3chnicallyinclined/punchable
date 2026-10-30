# contracts/: the rules two languages must agree on

Each file here is a small set of JSON cases (or a vocabulary) that **both** sides of Punchable must pass: the server
(Python today, Rust after T11; `docs/RUST_WORKSTREAM.md`) and the app (TypeScript, `app/`). A rule lives in one place
(the "single home" table in `docs/ARCHITECTURE.md` section 4); the other side either stops computing it, or, where
it must keep a copy for a reason the table names, is pinned to the same cases here.

Nothing private: synthetic venue names and invented text only, like every golden under `rust/crates/*/tests/goldens`.

| File | Rule | Home | Pinned by |
|---|---|---|---|
| `venue_short.json` | the short room name on a list row, `{name, guessed}`, no year | the app's `venueShort` (`laughs.ts`, under `app/src/rules/`) today; `creative/data.py venue_label` sends it as `venue_short` + `venue_guessed`; `pk-creative::display` after T11 | `creative/tests/test_contracts.py`; the app's node tests |
| `ratings.json` | the rating vocabulary with its order (`killed` > `worked` > `meh` > `died`) and the notebook's word for each | `creative/data.py RATING_ORDER`, `creative/notebook.py RATING_WORD`; `pk-kb::model::Rating` after T11 | `creative/tests/test_contracts.py`; the app's `laughs.ts` button list |
| `say_fillers.json` | the leading filler words the say classifier drops | `api-dev/say_api.py FILLERS`; the app's `sayRules.ts` keeps its copy | `api-dev/tests/test_contracts.py`; the app's node tests |
| `sensitive_keys.json` | field names a crash report never carries out (+ the hint words) | `api-dev/scrub.py` (the API and the worker); the app's `crash.ts` keeps its copy | `api-dev/tests/test_contracts.py`; the app's `crash.test.mjs` |

How the files are made and read:

- The Python tests read each file and assert the Python side matches (a changed server rule fails here first).
- The app's node tests (`app/tests/*.test.mjs`) read the same files, so a server rule that moves fails the app's
  contract test: that is the red arm of T16.
- `venue_short.json`'s cases were produced by running the app's own `venueShort` under node over inputs chosen to hit
  every branch (empty, `unknown`, `maybe …`, `likely`, `(show)`, `Comedy Club` / `Comedy Show` tails, `Comix at X`,
  apostrophes, commas, inner whitespace). Regenerate the same way if the app's rule changes on purpose.
- After T11, `cargo xtask contracts` writes these files from the Rust crates and the app's tests read them; until
  then they are written by hand from the Python and checked by the Python tests.

A change to a rule changes its file in the same commit, with both sides' tests green.
