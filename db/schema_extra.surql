-- Punchable: tables added for hosting (2026-10-26). Additive, idempotent: db/load.py applies it after schema.surql,
-- and the API applies it at startup, so an existing database needs no reload.
-- Same rules as schema.surql: SCHEMAFULL, IF NOT EXISTS on tables (OVERWRITE would drop a change feed, trap T6).

-- the comic's creative writes (storyboards, working sets, venue pins) and the draft/job queue, one row per event.
-- `line` is the exact JSON line of the store's jsonl (creative/store.py), so a replay from the database is byte-equal
-- to a replay from the file. Append-only: rows are never updated or deleted by the API.
DEFINE TABLE IF NOT EXISTS creative_event SCHEMAFULL CHANGEFEED 30d INCLUDE ORIGINAL
    PERMISSIONS FOR select, create FULL, FOR update, delete NONE;
DEFINE FIELD OVERWRITE store ON creative_event TYPE string;      -- storyboards | workingsets | venues | jobs
DEFINE FIELD OVERWRITE seq   ON creative_event TYPE int;         -- 0-based line number in <store>.jsonl
DEFINE FIELD OVERWRITE line  ON creative_event TYPE string;      -- the event, exactly as written to the jsonl
DEFINE FIELD OVERWRITE h     ON creative_event TYPE string;      -- sha256 of `line` (dedupe across machines)
DEFINE FIELD OVERWRITE op    ON creative_event TYPE option<string | null>;
DEFINE FIELD OVERWRITE item  ON creative_event TYPE option<string | null>;
DEFINE FIELD OVERWRITE at    ON creative_event TYPE option<string | null>;
DEFINE INDEX OVERWRITE creative_event_store_seq ON creative_event FIELDS store, seq UNIQUE;

-- Text/JSON inputs the hosted API needs as files (graph.json, id_map, inferred/insights, every set's analysis, meta,
-- transcript and clips, KB_REPORT, geocode cache). Written only by db/push_remote.py; the hosted API writes them
-- under /tmp at startup. Never audio.
DEFINE TABLE IF NOT EXISTS srcfile SCHEMAFULL;
DEFINE FIELD OVERWRITE path   ON srcfile TYPE string;            -- relative to the comedy folder, forward slashes
DEFINE FIELD OVERWRITE sha256 ON srcfile TYPE string;
DEFINE FIELD OVERWRITE bytes  ON srcfile TYPE int;
DEFINE FIELD OVERWRITE text   ON srcfile TYPE string;
DEFINE INDEX OVERWRITE srcfile_path ON srcfile FIELDS path UNIQUE;

-- the comic's own set edits (docs/API_CONTRACT.md "Requested by app"; kb.py NODE_EDIT_FIELDS["set"]): the show date,
-- and "audio is bad" (the set drops out of trends). `venue` is already a set field. Written only by a decision.
DEFINE FIELD OVERWRITE show_date        ON set TYPE option<string | null>;
DEFINE FIELD OVERWRITE audio_unreliable ON set TYPE option<bool | null>;

-- the comic's short name for a joke (what he writes on a paper set list; kb.py NODE_EDIT_FIELDS["joke"]).
-- Written only by a decision; null = the default cut of the title (creative/notebook.py short_name_of).
DEFINE FIELD OVERWRITE short_name ON joke TYPE option<string | null>;

-- Schema bookkeeping (db/load.py apply_schema): the sha256 of the schema files last applied. An unchanged schema is
-- not re-applied, because DEFINE INDEX ... OVERWRITE rebuilds every index from scratch even when nothing changed.
DEFINE FIELD OVERWRITE schema_sha ON meta TYPE option<string>;
DEFINE FIELD OVERWRITE extra_sha  ON meta TYPE option<string>;

-- Online-first (2026-10-27): files_rev moves whenever the API writes a srcfile at runtime (POST /api/worker/files), so
-- every API pod refreshes its cache; online_first is set once at the cutover (db/push_remote.py cutover) and makes
-- push_remote.py refuse any later bundle load (the database is the source of truth from then on).
DEFINE FIELD OVERWRITE files_rev    ON meta TYPE option<int>;
DEFINE FIELD OVERWRITE online_first ON meta TYPE option<object> FLEXIBLE;
DEFINE FIELD OVERWRITE cutover_log  ON meta TYPE option<array<object>> FLEXIBLE;

-- Search over everything (2026-10-28, api-dev/search_api.py). A typed idea lives in joke.summary (no unit) and a
-- taped set moment in performance.text; neither had an index or a vector, so word search missed the jots and the
-- tapes were invisible. Full-text (the same analyzer as unit.text) and a bge-small vector on both, like unit.text.
-- The API's sweeper fills `embedding` and records the sha256 of the text it embedded in `embedded_sha`, so an edited
-- text is embedded again and an unchanged one never is. HNSW indexes live in RAM and rebuild on restart.
DEFINE FIELD OVERWRITE embedding    ON joke        TYPE option<array<float, 384>>;
DEFINE FIELD OVERWRITE embedded_sha ON joke        TYPE option<string>;
DEFINE FIELD OVERWRITE embedding    ON performance TYPE option<array<float, 384>>;
DEFINE FIELD OVERWRITE embedded_sha ON performance TYPE option<string>;
DEFINE INDEX OVERWRITE joke_sum_ft  ON joke        FIELDS summary FULLTEXT ANALYZER punch_en BM25 HIGHLIGHTS;
DEFINE INDEX OVERWRITE perf_ft      ON performance FIELDS text    FULLTEXT ANALYZER punch_en BM25 HIGHLIGHTS;
DEFINE INDEX OVERWRITE joke_vec     ON joke        FIELDS embedding HNSW DIMENSION 384 DIST COSINE TYPE F32 EFC 150 M 12;
DEFINE INDEX OVERWRITE perf_vec     ON performance FIELDS embedding HNSW DIMENSION 384 DIST COSINE TYPE F32 EFC 150 M 12;
