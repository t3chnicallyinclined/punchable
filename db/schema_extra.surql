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

-- Schema bookkeeping (db/load.py apply_schema): the sha256 of the schema files last applied. An unchanged schema is
-- not re-applied, because DEFINE INDEX ... OVERWRITE rebuilds every index from scratch even when nothing changed.
DEFINE FIELD OVERWRITE schema_sha ON meta TYPE option<string>;
DEFINE FIELD OVERWRITE extra_sha  ON meta TYPE option<string>;
