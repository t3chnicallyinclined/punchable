-- Punchable: the usage meter (SPEC 2026-11-17 §13, phase 1). pk-api's own tables: the Python never reads or writes
-- them, so this file is NOT part of the schema shas (schema.surql + schema_extra.surql) and does not move the uploads'
-- 503 gate. pk-api applies it after it connects (pk_store::Store::apply_usage_schema), every time it starts:
-- additive and idempotent (tables and indexes IF NOT EXISTS, which never rebuilds an index; fields OVERWRITE, which
-- is a definition write only). dbload deletes a named list of tables (db/load.py DATA_TABLES) and never these. The
-- nightly backup exports every table, these included.
--
-- Privacy (owner rule): counts, names of kinds and models, and money. Never a prompt, an answer, a line of his
-- material or a title. `account` is the owner id until per-user data exists (phase 2: the Keycloak `sub`'s
-- account). `job` is the queue's job id (`job:<kind>-<12 hex>`): it names the job row in the same database, which holds
-- that job's input, so a cost can be traced to its job; it is never in the report, the export, a metric or a log.

-- One row per model call the worker's budget charged: the record id is the router's call id, so a batch the worker
-- posts twice is stored once.
DEFINE TABLE IF NOT EXISTS usage_call SCHEMAFULL;
DEFINE FIELD OVERWRITE account     ON usage_call TYPE string;
DEFINE FIELD OVERWRITE at          ON usage_call TYPE string;         -- when the call ended, ISO UTC
DEFINE FIELD OVERWRITE day         ON usage_call TYPE string;         -- YYYY-MM-DD of `at`, UTC
DEFINE FIELD OVERWRITE job         ON usage_call TYPE option<string>;
DEFINE FIELD OVERWRITE kind        ON usage_call TYPE string;         -- the job kind, or 'other'
DEFINE FIELD OVERWRITE task        ON usage_call TYPE string;         -- the router task
DEFINE FIELD OVERWRITE model       ON usage_call TYPE string;
DEFINE FIELD OVERWRITE tokens_in   ON usage_call TYPE int;
DEFINE FIELD OVERWRITE tokens_out  ON usage_call TYPE int;
DEFINE FIELD OVERWRITE cache_read  ON usage_call TYPE int;
DEFINE FIELD OVERWRITE cache_write ON usage_call TYPE int;
DEFINE FIELD OVERWRITE usd         ON usage_call TYPE float;          -- priced by pk-api from pk_llm::budget::PRICES
DEFINE FIELD OVERWRITE priced      ON usage_call TYPE string;         -- the day that price table was read
DEFINE INDEX IF NOT EXISTS usage_call_day ON usage_call FIELDS account, day;

-- One row per recording or transcript the server received: how long and how big, nothing it said.
DEFINE TABLE IF NOT EXISTS usage_media SCHEMAFULL;
DEFINE FIELD OVERWRITE account ON usage_media TYPE string;
DEFINE FIELD OVERWRITE at      ON usage_media TYPE string;
DEFINE FIELD OVERWRITE day     ON usage_media TYPE string;
DEFINE FIELD OVERWRITE kind    ON usage_media TYPE string;            -- 'transcript' | 'upload'
DEFINE FIELD OVERWRITE seconds ON usage_media TYPE float;             -- 0 when the sender did not say
DEFINE FIELD OVERWRITE bytes   ON usage_media TYPE int;
DEFINE INDEX IF NOT EXISTS usage_media_day ON usage_media FIELDS account, day;

-- The daily rollup, one row per (account, day): the activity counters pk-api adds up in memory and flushes once a
-- minute (+=, so two pk-api processes add, never overwrite), and the AI totals recomputed from usage_call (=, so a
-- rollup run twice is the same rollup).
DEFINE TABLE IF NOT EXISTS usage_day SCHEMAFULL;
DEFINE FIELD OVERWRITE account    ON usage_day TYPE string;
DEFINE FIELD OVERWRITE day        ON usage_day TYPE string;
DEFINE FIELD OVERWRITE requests   ON usage_day TYPE int DEFAULT 0;    -- app requests answered (not the worker's)
DEFINE FIELD OVERWRITE writes     ON usage_day TYPE int DEFAULT 0;    -- of those, writes (POST, PATCH, PUT, DELETE)
DEFINE FIELD OVERWRITE bits       ON usage_day TYPE int DEFAULT 0;    -- bits and versions saved, jots
DEFINE FIELD OVERWRITE redlines   ON usage_day TYPE int DEFAULT 0;    -- Redline rounds sent
DEFINE FIELD OVERWRITE drafts     ON usage_day TYPE int DEFAULT 0;    -- AI jobs asked for (riff, punch-up, rant, ask, ...)
DEFINE FIELD OVERWRITE sessions   ON usage_day TYPE int DEFAULT 0;    -- /api/live sockets opened (the app coming forward)
DEFINE FIELD OVERWRITE ai_usd     ON usage_day TYPE float DEFAULT 0;
DEFINE FIELD OVERWRITE ai_calls   ON usage_day TYPE int DEFAULT 0;
DEFINE FIELD OVERWRITE ai         ON usage_day TYPE array<object> FLEXIBLE DEFAULT [];   -- per kind and model
DEFINE FIELD OVERWRITE rolled     ON usage_day TYPE option<string>;   -- when the AI totals were last recomputed
DEFINE FIELD OVERWRITE storage    ON usage_day TYPE option<object> FLEXIBLE;             -- rows and bytes, sampled
DEFINE INDEX IF NOT EXISTS usage_day_day ON usage_day FIELDS account, day;
