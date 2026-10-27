-- Punchable on SurrealDB 3.3.4: the schema. Applied by db/load.py (and safe to re-apply).
--
-- Rules (docs/CONTEXT.md "Storage decision, revised 2026-10-26"; docs/SURREALDB_AUDIT.md):
-- * The database is STRICT (an unknown table is an error) and every table is SCHEMAFULL (an unknown field is
--   an error). load.py defines the database with STRICT before this file runs.
-- * Tables are defined with IF NOT EXISTS, never OVERWRITE: `DEFINE TABLE OVERWRITE` without its CHANGEFEED clause
--   silently drops the feed (trap T6). A table-level change therefore needs an explicit migration (see db/DB.md).
--   Fields, indexes, functions and events use OVERWRITE, which keeps the feed.
-- * decisions.jsonl stays the canonical history. The `decision` table is its replay; VERSION is not trusted.
-- * Values keep the KB files' own types so the API's responses are identical to the files backend:
--   dates stay ISO strings (they sort correctly as text), and fields accept NULL because the comic's undo lines set
--   fields to null. NONE (absent) and NULL (null) are kept distinct.
-- * Private replay markers from api-dev (`_by_tristan`, `_edited`, `_last`, `_ai`, `_ai_node`, `_ai_kind`,
--   `_ai_hit_sets`) are stored as `x_<name>`. `ord` keeps the files backend's insertion order.
--   DB-only fields (never part of a KB node): ord, working, working_sets, embedding, run_id.

-- ===================================================================== nodes
DEFINE TABLE IF NOT EXISTS joke SCHEMAFULL CHANGEFEED 30d INCLUDE ORIGINAL;
DEFINE FIELD OVERWRITE ord                 ON joke TYPE number;
DEFINE FIELD OVERWRITE title               ON joke TYPE string;
DEFINE FIELD OVERWRITE stage               ON joke TYPE string ASSERT $value IN ['idea', 'to_try', 'testing', 'working', 'retired'];
DEFINE FIELD OVERWRITE summary             ON joke TYPE option<string | null>;
DEFINE FIELD OVERWRITE first_seen          ON joke TYPE option<string | null>;
DEFINE FIELD OVERWRITE last_seen           ON joke TYPE option<string | null>;
DEFINE FIELD OVERWRITE stage_reason        ON joke TYPE option<string | null>;
DEFINE FIELD OVERWRITE performances        ON joke TYPE option<int | null>;
DEFINE FIELD OVERWRITE unit_count          ON joke TYPE option<int | null>;
DEFINE FIELD OVERWRITE first_seen_on_stage ON joke TYPE option<bool | null>;
DEFINE FIELD OVERWRITE aliases             ON joke TYPE option<array | null>;
DEFINE FIELD OVERWRITE years               ON joke TYPE option<array | null>;
DEFINE FIELD OVERWRITE versions_written    ON joke TYPE option<int | null>;
DEFINE FIELD OVERWRITE versions            ON joke TYPE option<int | null>;
DEFINE FIELD OVERWRITE lead_unit           ON joke TYPE option<string | null>;
DEFINE FIELD OVERWRITE quality             ON joke TYPE option<object | null> FLEXIBLE;
DEFINE FIELD OVERWRITE catalog_confidence  ON joke TYPE option<number | null>;
DEFINE FIELD OVERWRITE anatomy             ON joke TYPE option<object | null> FLEXIBLE;
DEFINE FIELD OVERWRITE flags               ON joke TYPE option<array | null>;
DEFINE FIELD OVERWRITE merged_from         ON joke TYPE option<array | null>;
DEFINE FIELD OVERWRITE hidden              ON joke TYPE option<bool | null>;
DEFINE FIELD OVERWRITE hidden_reason       ON joke TYPE option<string | null>;
DEFINE FIELD OVERWRITE from                ON joke TYPE option<object | null> FLEXIBLE;   -- "Save as joke" source
DEFINE FIELD OVERWRITE status              ON joke TYPE option<string | null>;            -- a node confirm/reject
DEFINE FIELD OVERWRITE x_ai                ON joke TYPE option<object | null> FLEXIBLE;   -- inferred stage (AI layer)
DEFINE FIELD OVERWRITE x_ai_hit_sets       ON joke TYPE option<int | null>;
DEFINE FIELD OVERWRITE x_ai_node           ON joke TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_by_tristan        ON joke TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last              ON joke TYPE option<int | null>;
-- Working is DERIVED: written only by fn::recompute_working (events below), never by a loader or the API.
DEFINE FIELD OVERWRITE working             ON joke TYPE bool DEFAULT false;
DEFINE FIELD OVERWRITE working_sets        ON joke TYPE array<record<set>> DEFAULT [];
DEFINE INDEX OVERWRITE joke_ord   ON joke FIELDS ord;
DEFINE INDEX OVERWRITE joke_stage ON joke FIELDS stage;


DEFINE TABLE IF NOT EXISTS unit SCHEMAFULL;
DEFINE FIELD OVERWRITE ord           ON unit TYPE number;
DEFINE FIELD OVERWRITE text          ON unit TYPE string;
DEFINE FIELD OVERWRITE kind          ON unit TYPE option<string | null>;
DEFINE FIELD OVERWRITE source        ON unit TYPE option<string | null>;
DEFINE FIELD OVERWRITE doc_id        ON unit TYPE option<string | null>;
DEFINE FIELD OVERWRITE doc_title     ON unit TYPE option<string | null>;
DEFINE FIELD OVERWRITE date          ON unit TYPE option<string | null>;
DEFINE FIELD OVERWRITE locator       ON unit TYPE option<object | null> FLEXIBLE;
DEFINE FIELD OVERWRITE anatomy       ON unit TYPE option<object | null> FLEXIBLE;
DEFINE FIELD OVERWRITE quality       ON unit TYPE option<object | null> FLEXIBLE;
DEFINE FIELD OVERWRITE bit           ON unit TYPE option<object | null> FLEXIBLE;
DEFINE FIELD OVERWRITE catalog_batch ON unit TYPE option<int | string | null>;
DEFINE FIELD OVERWRITE catalog_kind  ON unit TYPE option<string | null>;
DEFINE FIELD OVERWRITE duplicates    ON unit TYPE option<array | null>;
DEFINE FIELD OVERWRITE flags         ON unit TYPE option<array | null>;
DEFINE FIELD OVERWRITE hidden        ON unit TYPE option<bool | null>;
DEFINE FIELD OVERWRITE hidden_reason ON unit TYPE option<string | null>;
DEFINE FIELD OVERWRITE status        ON unit TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_ai_node     ON unit TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_by_tristan  ON unit TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last        ON unit TYPE option<int | null>;
-- bge-small-en-v1.5 (384-d, L2-normalised) from tools/kb/cache; for search and candidate finding only.
DEFINE FIELD OVERWRITE embedding     ON unit TYPE option<array<float, 384>>;
DEFINE INDEX OVERWRITE unit_ord ON unit FIELDS ord;
-- Snowball stemming: "dating" also finds "date".
DEFINE ANALYZER OVERWRITE punch_en TOKENIZERS class FILTERS lowercase, ascii, snowball(english);
DEFINE INDEX OVERWRITE unit_ft  ON unit  FIELDS text  FULLTEXT ANALYZER punch_en BM25 HIGHLIGHTS;
DEFINE INDEX OVERWRITE joke_ft  ON joke  FIELDS title FULLTEXT ANALYZER punch_en BM25 HIGHLIGHTS;
-- HNSW lives in RAM and is rebuilt on restart (>= 3.3.1 required). Since 3.3.0 a KNN query's WHERE runs inside
-- the HNSW scan (EXPLAIN: KnnScan ... predicate), so the hidden/live filter never leaves fewer than k results.
DEFINE INDEX OVERWRITE unit_vec ON unit  FIELDS embedding HNSW DIMENSION 384 DIST COSINE TYPE F32 EFC 150 M 12;

DEFINE TABLE IF NOT EXISTS set SCHEMAFULL CHANGEFEED 30d INCLUDE ORIGINAL;
DEFINE FIELD OVERWRITE ord          ON set TYPE number;
DEFINE FIELD OVERWRITE title        ON set TYPE option<string | null>;
DEFINE FIELD OVERWRITE date         ON set TYPE option<string | null>;
DEFINE FIELD OVERWRITE venue        ON set TYPE option<string | null>;
DEFINE FIELD OVERWRITE duration_s   ON set TYPE option<number | null>;
DEFINE FIELD OVERWRITE status       ON set TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_ai_node    ON set TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON set TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON set TYPE option<int | null>;

DEFINE TABLE IF NOT EXISTS performance SCHEMAFULL CHANGEFEED 30d INCLUDE ORIGINAL;
DEFINE FIELD OVERWRITE ord          ON performance TYPE number;
DEFINE FIELD OVERWRITE set          ON performance TYPE record<set>;
DEFINE FIELD OVERWRITE start_ms     ON performance TYPE int;
DEFINE FIELD OVERWRITE end_ms       ON performance TYPE option<int | null>;
DEFINE FIELD OVERWRITE text         ON performance TYPE option<string | null>;
DEFINE FIELD OVERWRITE kind         ON performance TYPE option<string | null>;   -- bit | crowd_work | banter
DEFINE FIELD OVERWRITE name         ON performance TYPE option<string | null>;
DEFINE FIELD OVERWRITE status       ON performance TYPE option<string | null>;
DEFINE FIELD OVERWRITE source       ON performance TYPE option<string | null>;
DEFINE FIELD OVERWRITE ai_rating    ON performance TYPE option<string | null> ASSERT $value IN [NONE, NULL, 'killed', 'worked', 'meh', 'died'];
DEFINE FIELD OVERWRITE rating_basis ON performance TYPE option<string | null>;
DEFINE FIELD OVERWRITE laughs       ON performance TYPE option<object | null> FLEXIBLE;
-- the comic's own rating (a judgement on the measurements; the laughs stay measurements)
DEFINE FIELD OVERWRITE rating       ON performance TYPE option<string | null> ASSERT $value IN [NONE, NULL, 'killed', 'worked', 'meh', 'died'];
DEFINE FIELD OVERWRITE rating_auto  ON performance TYPE option<string | null>;
DEFINE FIELD OVERWRITE max_bars     ON performance TYPE option<int | null>;
DEFINE FIELD OVERWRITE x_ai_node    ON performance TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_ai_kind    ON performance TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON performance TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON performance TYPE option<int | null>;
DEFINE INDEX OVERWRITE performance_set ON performance FIELDS set, start_ms;

DEFINE TABLE IF NOT EXISTS label SCHEMAFULL CHANGEFEED 30d INCLUDE ORIGINAL;
DEFINE FIELD OVERWRITE ord          ON label TYPE number;
DEFINE FIELD OVERWRITE kind         ON label TYPE string ASSERT $value IN ['device', 'structure', 'form', 'attitude', 'theory', 'topic', 'persona', 'custom'];
DEFINE FIELD OVERWRITE name         ON label TYPE string;
DEFINE FIELD OVERWRITE definition   ON label TYPE option<string | null>;
DEFINE FIELD OVERWRITE merged_into  ON label TYPE option<string | null>;
DEFINE FIELD OVERWRITE status       ON label TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_ai_node    ON label TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON label TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON label TYPE option<int | null>;

-- Tonight's list (`list:tonight`), created by the app through a node `add` decision.
DEFINE TABLE IF NOT EXISTS list SCHEMAFULL CHANGEFEED 30d INCLUDE ORIGINAL;
DEFINE FIELD OVERWRITE ord          ON list TYPE number;
DEFINE FIELD OVERWRITE name         ON list TYPE option<string | null>;
DEFINE FIELD OVERWRITE items        ON list TYPE option<array | null>;
DEFINE FIELD OVERWRITE status       ON list TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON list TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON list TYPE option<int | null>;

-- Laughs are MEASUREMENTS, not claims: no status, never confirmed. Id [set, start_ms, laugh id], so the laughs
-- of one set (or one span of it) are a key-range scan: laugh:[$set, $a]..[$set, $b].
DEFINE TABLE IF NOT EXISTS laugh SCHEMAFULL;
DEFINE FIELD OVERWRITE ord        ON laugh TYPE number;
DEFINE FIELD OVERWRITE set        ON laugh TYPE record<set>;
DEFINE FIELD OVERWRITE lid        ON laugh TYPE option<string | null>;
DEFINE FIELD OVERWRITE start_ms   ON laugh TYPE int;
DEFINE FIELD OVERWRITE end_ms     ON laugh TYPE option<int | null>;
DEFINE FIELD OVERWRITE strength   ON laugh TYPE option<number | null>;
DEFINE FIELD OVERWRITE bars       ON laugh TYPE option<int | null>;
DEFINE FIELD OVERWRITE kind       ON laugh TYPE option<string | null>;
DEFINE FIELD OVERWRITE confidence ON laugh TYPE option<number | null>;
DEFINE FIELD OVERWRITE segment    ON laugh TYPE option<string | null>;
DEFINE FIELD OVERWRITE detail     ON laugh TYPE any;
DEFINE FIELD OVERWRITE extra      ON laugh TYPE option<object> FLEXIBLE;   -- any other key of the laugh row

-- One per set: the laughs_auto.json header (detector, run_id, room baseline, params).
DEFINE TABLE IF NOT EXISTS laugh_run SCHEMAFULL;
DEFINE FIELD OVERWRITE set     ON laugh_run TYPE record<set>;
DEFINE FIELD OVERWRITE set_dir ON laugh_run TYPE string;
DEFINE FIELD OVERWRITE doc     ON laugh_run TYPE object FLEXIBLE;

-- One per analysed set: the AI's reading (analysis.json) with each row's matched performance.
DEFINE TABLE IF NOT EXISTS analysis SCHEMAFULL;
DEFINE FIELD OVERWRITE ord          ON analysis TYPE number;
DEFINE FIELD OVERWRITE set          ON analysis TYPE record<set>;
DEFINE FIELD OVERWRITE meta         ON analysis TYPE object FLEXIBLE;    -- {source, path, run_id, generated_at, summary, set}
DEFINE FIELD OVERWRITE rows         ON analysis TYPE array<object> FLEXIBLE;              -- [{row: <analysis.json row>, perf: 'perf:...'}]

-- Load and replay state. meta:state.gen changes on every write, so a reader knows its snapshot is stale.
DEFINE TABLE IF NOT EXISTS meta SCHEMAFULL;
DEFINE FIELD OVERWRITE gen       ON meta TYPE option<int>;
DEFINE FIELD OVERWRITE ord_next  ON meta TYPE option<int>;
DEFINE FIELD OVERWRITE orphans   ON meta TYPE option<int>;
DEFINE FIELD OVERWRITE loading   ON meta TYPE option<bool>;
DEFINE FIELD OVERWRITE load_info ON meta TYPE option<object> FLEXIBLE;

-- ===================================================================== edges: one RELATION table per KB kind
-- status is the claim state. 'rejected' is what api-dev/kb.py writes for the comic's reject (bucket "rejected",
-- like ruled_out), so it must be allowed or a reject could never be saved.
-- Endpoints: has_label must point at a label. The other kinds accept any node table, because kb.py lets the comic
-- connect any two nodes (the KB contract types them by convention, not by rule).
DEFINE TABLE IF NOT EXISTS part_of      TYPE RELATION IN unit | joke OUT joke | unit | set | performance | label | list SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS variant_of   TYPE RELATION IN unit | joke | set | performance | label | list OUT unit | joke | set | performance | label | list SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS performed_as TYPE RELATION IN performance | joke OUT joke | performance | unit | set | label | list SCHEMAFULL CHANGEFEED 30d INCLUDE ORIGINAL;
DEFINE TABLE IF NOT EXISTS tag_of       TYPE RELATION IN unit | joke | set | performance | label | list OUT joke | unit | set | performance | label | list SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS same_shape   TYPE RELATION IN joke | unit | set | performance | label | list OUT joke | unit | set | performance | label | list SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS callback_to  TYPE RELATION IN joke | unit | set | performance | label | list OUT joke | unit | set | performance | label | list SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS same_premise TYPE RELATION IN joke | unit | set | performance | label | list OUT joke | unit | set | performance | label | list SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS has_label    TYPE RELATION IN joke | unit OUT label SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS segue        TYPE RELATION IN joke | unit | set | performance | label | list OUT joke | unit | set | performance | label | list SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS custom       TYPE RELATION IN joke | unit | set | performance | label | list OUT joke | unit | set | performance | label | list SCHEMAFULL;

-- the same fields on every edge table (written out per table: SurrealQL has no field templates)
DEFINE FIELD OVERWRITE ord          ON part_of TYPE number;
DEFINE FIELD OVERWRITE status       ON part_of TYPE string ASSERT $value IN ['proposed', 'inferred', 'confirmed', 'ruled_out', 'superseded', 'rejected'];
DEFINE FIELD OVERWRITE confidence   ON part_of TYPE option<number | null>;
DEFINE FIELD OVERWRITE reason       ON part_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE evidence     ON part_of TYPE option<array | null>;
DEFINE FIELD OVERWRITE source       ON part_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE run_id       ON part_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE custom_kind  ON part_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON part_of TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_edited     ON part_of TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON part_of TYPE option<int | null>;

DEFINE FIELD OVERWRITE ord          ON variant_of TYPE number;
DEFINE FIELD OVERWRITE status       ON variant_of TYPE string ASSERT $value IN ['proposed', 'inferred', 'confirmed', 'ruled_out', 'superseded', 'rejected'];
DEFINE FIELD OVERWRITE confidence   ON variant_of TYPE option<number | null>;
DEFINE FIELD OVERWRITE reason       ON variant_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE evidence     ON variant_of TYPE option<array | null>;
DEFINE FIELD OVERWRITE source       ON variant_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE run_id       ON variant_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE custom_kind  ON variant_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON variant_of TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_edited     ON variant_of TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON variant_of TYPE option<int | null>;

DEFINE FIELD OVERWRITE ord          ON performed_as TYPE number;
DEFINE FIELD OVERWRITE status       ON performed_as TYPE string ASSERT $value IN ['proposed', 'inferred', 'confirmed', 'ruled_out', 'superseded', 'rejected'];
DEFINE FIELD OVERWRITE confidence   ON performed_as TYPE option<number | null>;
DEFINE FIELD OVERWRITE reason       ON performed_as TYPE option<string | null>;
DEFINE FIELD OVERWRITE evidence     ON performed_as TYPE option<array | null>;
DEFINE FIELD OVERWRITE source       ON performed_as TYPE option<string | null>;
DEFINE FIELD OVERWRITE run_id       ON performed_as TYPE option<string | null>;
DEFINE FIELD OVERWRITE custom_kind  ON performed_as TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON performed_as TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_edited     ON performed_as TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON performed_as TYPE option<int | null>;

DEFINE FIELD OVERWRITE ord          ON tag_of TYPE number;
DEFINE FIELD OVERWRITE status       ON tag_of TYPE string ASSERT $value IN ['proposed', 'inferred', 'confirmed', 'ruled_out', 'superseded', 'rejected'];
DEFINE FIELD OVERWRITE confidence   ON tag_of TYPE option<number | null>;
DEFINE FIELD OVERWRITE reason       ON tag_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE evidence     ON tag_of TYPE option<array | null>;
DEFINE FIELD OVERWRITE source       ON tag_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE run_id       ON tag_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE custom_kind  ON tag_of TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON tag_of TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_edited     ON tag_of TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON tag_of TYPE option<int | null>;

DEFINE FIELD OVERWRITE ord          ON same_shape TYPE number;
DEFINE FIELD OVERWRITE status       ON same_shape TYPE string ASSERT $value IN ['proposed', 'inferred', 'confirmed', 'ruled_out', 'superseded', 'rejected'];
DEFINE FIELD OVERWRITE confidence   ON same_shape TYPE option<number | null>;
DEFINE FIELD OVERWRITE reason       ON same_shape TYPE option<string | null>;
DEFINE FIELD OVERWRITE evidence     ON same_shape TYPE option<array | null>;
DEFINE FIELD OVERWRITE source       ON same_shape TYPE option<string | null>;
DEFINE FIELD OVERWRITE run_id       ON same_shape TYPE option<string | null>;
DEFINE FIELD OVERWRITE custom_kind  ON same_shape TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON same_shape TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_edited     ON same_shape TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON same_shape TYPE option<int | null>;

DEFINE FIELD OVERWRITE ord          ON callback_to TYPE number;
DEFINE FIELD OVERWRITE status       ON callback_to TYPE string ASSERT $value IN ['proposed', 'inferred', 'confirmed', 'ruled_out', 'superseded', 'rejected'];
DEFINE FIELD OVERWRITE confidence   ON callback_to TYPE option<number | null>;
DEFINE FIELD OVERWRITE reason       ON callback_to TYPE option<string | null>;
DEFINE FIELD OVERWRITE evidence     ON callback_to TYPE option<array | null>;
DEFINE FIELD OVERWRITE source       ON callback_to TYPE option<string | null>;
DEFINE FIELD OVERWRITE run_id       ON callback_to TYPE option<string | null>;
DEFINE FIELD OVERWRITE custom_kind  ON callback_to TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON callback_to TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_edited     ON callback_to TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON callback_to TYPE option<int | null>;

DEFINE FIELD OVERWRITE ord          ON same_premise TYPE number;
DEFINE FIELD OVERWRITE status       ON same_premise TYPE string ASSERT $value IN ['proposed', 'inferred', 'confirmed', 'ruled_out', 'superseded', 'rejected'];
DEFINE FIELD OVERWRITE confidence   ON same_premise TYPE option<number | null>;
DEFINE FIELD OVERWRITE reason       ON same_premise TYPE option<string | null>;
DEFINE FIELD OVERWRITE evidence     ON same_premise TYPE option<array | null>;
DEFINE FIELD OVERWRITE source       ON same_premise TYPE option<string | null>;
DEFINE FIELD OVERWRITE run_id       ON same_premise TYPE option<string | null>;
DEFINE FIELD OVERWRITE custom_kind  ON same_premise TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON same_premise TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_edited     ON same_premise TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON same_premise TYPE option<int | null>;

DEFINE FIELD OVERWRITE ord          ON has_label TYPE number;
DEFINE FIELD OVERWRITE status       ON has_label TYPE string ASSERT $value IN ['proposed', 'inferred', 'confirmed', 'ruled_out', 'superseded', 'rejected'];
DEFINE FIELD OVERWRITE confidence   ON has_label TYPE option<number | null>;
DEFINE FIELD OVERWRITE reason       ON has_label TYPE option<string | null>;
DEFINE FIELD OVERWRITE evidence     ON has_label TYPE option<array | null>;
DEFINE FIELD OVERWRITE source       ON has_label TYPE option<string | null>;
DEFINE FIELD OVERWRITE run_id       ON has_label TYPE option<string | null>;
DEFINE FIELD OVERWRITE custom_kind  ON has_label TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON has_label TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_edited     ON has_label TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON has_label TYPE option<int | null>;

DEFINE FIELD OVERWRITE ord          ON segue TYPE number;
DEFINE FIELD OVERWRITE status       ON segue TYPE string ASSERT $value IN ['proposed', 'inferred', 'confirmed', 'ruled_out', 'superseded', 'rejected'];
DEFINE FIELD OVERWRITE confidence   ON segue TYPE option<number | null>;
DEFINE FIELD OVERWRITE reason       ON segue TYPE option<string | null>;
DEFINE FIELD OVERWRITE evidence     ON segue TYPE option<array | null>;
DEFINE FIELD OVERWRITE source       ON segue TYPE option<string | null>;
DEFINE FIELD OVERWRITE run_id       ON segue TYPE option<string | null>;
DEFINE FIELD OVERWRITE custom_kind  ON segue TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON segue TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_edited     ON segue TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON segue TYPE option<int | null>;

DEFINE FIELD OVERWRITE ord          ON custom TYPE number;
DEFINE FIELD OVERWRITE status       ON custom TYPE string ASSERT $value IN ['proposed', 'inferred', 'confirmed', 'ruled_out', 'superseded', 'rejected'];
DEFINE FIELD OVERWRITE confidence   ON custom TYPE option<number | null>;
DEFINE FIELD OVERWRITE reason       ON custom TYPE option<string | null>;
DEFINE FIELD OVERWRITE evidence     ON custom TYPE option<array | null>;
DEFINE FIELD OVERWRITE source       ON custom TYPE option<string | null>;
DEFINE FIELD OVERWRITE run_id       ON custom TYPE option<string | null>;
DEFINE FIELD OVERWRITE custom_kind  ON custom TYPE option<string | null>;
DEFINE FIELD OVERWRITE x_by_tristan ON custom TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_edited     ON custom TYPE option<bool | null>;
DEFINE FIELD OVERWRITE x_last       ON custom TYPE option<int | null>;

DEFINE INDEX OVERWRITE performed_as_status ON performed_as FIELDS status;
DEFINE INDEX OVERWRITE has_label_status    ON has_label    FIELDS status;

-- ===================================================================== decisions (the comic's own actions)
-- The replay of decisions.jsonl, line for line (seq = the line's index among valid lines, which is what the API's
-- undo addresses). Append-only for any non-root caller. The jsonl file stays the canonical history.
DEFINE TABLE IF NOT EXISTS decision SCHEMAFULL CHANGEFEED 30d INCLUDE ORIGINAL
    PERMISSIONS FOR select, create FULL, FOR update, delete NONE;
DEFINE FIELD OVERWRITE seq       ON decision TYPE int;
DEFINE FIELD OVERWRITE ts        ON decision TYPE option<string | null>;
DEFINE FIELD OVERWRITE target    ON decision TYPE option<string | null>;    -- edge | node | laugh
DEFINE FIELD OVERWRITE target_id ON decision TYPE option<string | null>;    -- the KB id ("e:...", "joke:...")
DEFINE FIELD OVERWRITE action    ON decision TYPE option<string | null>;    -- confirm | reject | edit | add | delete
DEFINE FIELD OVERWRITE before    ON decision TYPE option<object | null> FLEXIBLE;
DEFINE FIELD OVERWRITE after     ON decision TYPE option<object | null> FLEXIBLE;
DEFINE FIELD OVERWRITE reason    ON decision TYPE option<string | null>;
DEFINE FIELD OVERWRITE by        ON decision TYPE option<string | null>;
DEFINE FIELD OVERWRITE undo_of   ON decision TYPE option<int | null>;
DEFINE FIELD OVERWRITE extra     ON decision TYPE option<object> FLEXIBLE;    -- any other key of the line
DEFINE INDEX OVERWRITE decision_seq ON decision FIELDS seq UNIQUE;

-- ===================================================================== functions
-- "perf:x@1" / "label:topic:dating" -> record id (the table comes from the prefix; the key is the rest).
DEFINE FUNCTION OVERWRITE fn::rid($s: string) {
    LET $parts = string::split($s, ':');
    LET $p = $parts[0];
    LET $tb = IF $p = 'perf' { 'performance' } ELSE { $p };
    IF $tb NOTINSIDE ['joke', 'unit', 'set', 'performance', 'label', 'list'] OR array::len($parts) < 2 {
        THROW 'unknown node id (add its table to db/schema.surql): ' + $s;
    };
    RETURN type::record($tb, array::join(array::slice($parts, 1), ':'));
};

-- "e:<hash>" -> the edge record in whichever kind table holds it, or NONE.
DEFINE FUNCTION OVERWRITE fn::edge_find($eid: string) {
    LET $key = string::slice($eid, 2);
    RETURN array::first(array::filter(
        ['part_of', 'variant_of', 'performed_as', 'tag_of', 'same_shape', 'callback_to', 'same_premise',
         'has_label', 'segue', 'custom'].map(|$t| type::record($t, $key)),
        |$r| record::exists($r)));
};

DEFINE FUNCTION OVERWRITE fn::next_ord() {
    RETURN (UPDATE meta:state SET ord_next += 1 RETURN VALUE ord_next)[0] - 1;
};

-- A KB node dict (as in graph.json or a decision) -> record content.
DEFINE FUNCTION OVERWRITE fn::node_content($tb: string, $o: object) {
    LET $c = object::remove($o, ['id', 'type']);
    RETURN IF $tb = 'performance' AND type::is_string($c.set) { $c + { set: fn::rid($c.set) } } ELSE { $c };
};

-- A KB edge dict -> {table, content}. src/dst become in/out; kind picks the table.
DEFINE FUNCTION OVERWRITE fn::edge_content($o: object) {
    LET $c = object::remove($o, ['id', 'src', 'dst', 'kind']);
    LET $c2 = IF type::is_string($o.src) { $c + { in: fn::rid($o.src) } } ELSE { $c };
    RETURN IF type::is_string($o.dst) { $c2 + { out: fn::rid($o.dst) } } ELSE { $c2 };
};

DEFINE FUNCTION OVERWRITE fn::put_edge($key: string, $tb: string, $content: object) {
    IF $tb NOTINSIDE ['part_of', 'variant_of', 'performed_as', 'tag_of', 'same_shape', 'callback_to',
                      'same_premise', 'has_label', 'segue', 'custom'] {
        THROW 'unknown edge kind: ' + <string> $tb;
    };
    LET $r = INSERT RELATION [$content + { id: type::record($tb, $key) }] RETURN id;
    IF array::len($r) != 1 { THROW 'edge write returned nothing: ' + $key };
};

-- Working = confirmed performances, not crowd work or banter, that the comic rated killed or worked, in >= 3
-- distinct sets (api-dev AppIndex.joke_rooms). <-/-> both ways: a match may point either direction.
DEFINE FUNCTION OVERWRITE fn::working($j: record) {
    LET $ok = ['confirmed', 'edited'];
    LET $perfs = array::distinct(array::concat(
        $j<-(performed_as WHERE status IN $ok)<-performance,
        $j->(performed_as WHERE status IN $ok)->performance));
    LET $sets = array::distinct(array::filter(
        $perfs.map(|$p| IF $p.kind NOTINSIDE ['crowd_work', 'banter'] AND $p.rating INSIDE ['killed', 'worked'] { $p.set } ELSE { NONE }),
        |$s| $s != NONE));
    RETURN { working: array::len($sets) >= 3, hit_sets: $sets };
};

DEFINE FUNCTION OVERWRITE fn::recompute_working($j: any) {
    IF type::is_record($j) AND record::tb($j) = 'joke' AND record::exists($j) {
        LET $w = fn::working($j);
        UPDATE $j SET working = $w.working, working_sets = $w.hit_sets RETURN NONE;
    };
};

-- Replay one decision onto the graph, exactly like api-dev's kb.View._apply. Runs inside the decision's own
-- write (the event below), so the decision and its effect commit together or not at all.
DEFINE FUNCTION OVERWRITE fn::apply_decision($d: object) {
    UPDATE meta:state SET gen += 1 RETURN NONE;
    IF $d.target NOTINSIDE ['edge', 'node'] OR !type::is_string($d.target_id) OR $d.target_id = '' {
        RETURN 'ignored';
    };
    LET $after = IF type::is_object($d.after) { $d.after } ELSE { {} };
    LET $before = IF type::is_object($d.before) { $d.before } ELSE { {} };
    LET $edge = $d.target = 'edge';
    LET $cur = IF $edge { fn::edge_find($d.target_id) } ELSE { fn::rid($d.target_id) };
    LET $exists = $cur != NONE AND record::exists($cur);

    IF $d.action = 'add' {
        LET $ord = IF $exists { $cur.ord } ELSE { fn::next_ord() };
        IF $exists { DELETE $cur RETURN NONE; };
        IF $edge {
            LET $rec = IF 'status' INSIDE object::keys($after) { $after } ELSE { $after + { status: 'confirmed' } };
            fn::put_edge(string::slice($d.target_id, 2), $after.kind ?? 'custom', fn::edge_content($rec)
                + { ord: $ord, x_by_tristan: true, x_last: $d.seq, run_id: NONE });
        } ELSE {
            LET $r = fn::rid($d.target_id);
            LET $w = UPSERT $r CONTENT fn::node_content(record::tb($r), $after) + { ord: $ord, x_by_tristan: true, x_last: $d.seq } RETURN id;
            IF array::len($w) != 1 { THROW 'node add wrote nothing: ' + $d.target_id };
        };
        RETURN 'applied';
    };
    IF $d.action = 'delete' {
        IF $exists { DELETE $cur RETURN NONE; };
        RETURN 'applied';
    };
    IF $d.action NOTINSIDE ['confirm', 'reject', 'edit'] { RETURN 'ignored'; };

    -- confirm / reject / edit. A target the pipeline dropped is brought back from the decision's before-image.
    LET $target = IF $exists {
        $cur
    } ELSE IF $edge AND $before.src AND $before.dst {
        LET $key = string::slice($d.target_id, 2);
        fn::put_edge($key, $before.kind ?? 'custom', fn::edge_content($before) + { ord: fn::next_ord() });
        fn::edge_find($d.target_id)
    } ELSE IF !$edge AND $before.type {
        LET $r = fn::rid($d.target_id);
        UPSERT $r CONTENT fn::node_content(record::tb($r), $before) + { ord: fn::next_ord() } RETURN NONE;
        $r
    } ELSE {
        NONE
    };
    IF $target = NONE {
        UPDATE meta:state SET orphans += 1 RETURN NONE;
        RETURN 'orphan';
    };
    LET $patch = object::remove($after, ['id']);
    IF $edge {
        LET $flag = IF $d.undo_of != NONE { { x_edited: NONE } } ELSE IF $d.action = 'edit' { { x_edited: true } } ELSE { {} };   -- undo clears "edited"
        LET $moved = array::len(array::intersect(object::keys($patch), ['src', 'dst', 'kind'])) > 0;
        IF $moved {
            -- in/out can't be changed by UPDATE (it is silently ignored), and kind is the table: re-create the edge.
            LET $old = (SELECT * FROM ONLY $target);
            LET $base = IF $d.undo_of != NONE { object::remove($old, ['id', 'x_edited']) } ELSE { object::remove($old, ['id']) };
            LET $content = $base + fn::edge_content($patch) + $flag + { x_last: $d.seq };
            LET $tb = IF 'kind' INSIDE object::keys($patch) { $patch.kind } ELSE { record::tb($target) };
            DELETE $target RETURN NONE;
            fn::put_edge(string::slice($d.target_id, 2), $tb, $content);
        } ELSE {
            LET $w = UPDATE $target MERGE fn::edge_content($patch) + $flag + { x_last: $d.seq } RETURN id;
            IF array::len($w) != 1 { THROW 'edge update wrote nothing: ' + $d.target_id };
        };
    } ELSE {
        LET $w = UPDATE $target MERGE fn::node_content(record::tb($target), $patch) + { x_last: $d.seq } RETURN id;
        IF array::len($w) != 1 { THROW 'node update wrote nothing: ' + $d.target_id };
    };
    RETURN 'applied';
};

-- ===================================================================== events
-- A decision row is appended; the event applies it in the same transaction (an error rolls both back).
DEFINE EVENT OVERWRITE apply_decision ON decision WHEN $event = 'CREATE' THEN {
    fn::apply_decision($after);
};

-- Any change to a match's status or ends recomputes Working for the jokes at both ends.
DEFINE EVENT OVERWRITE working_on_match ON performed_as
    WHEN $event != 'UPDATE' OR $before.status != $after.status
    THEN {
        FOR $x IN array::distinct([$before.in, $before.out, $after.in, $after.out]) {
            fn::recompute_working($x);
        };
    };

-- the comic rating a performance (or marking it crowd work) recomputes Working for its jokes.
DEFINE EVENT OVERWRITE working_on_perf ON performance
    WHEN $event = 'UPDATE' AND ($before.rating != $after.rating OR $before.kind != $after.kind)
    THEN {
        FOR $j IN array::distinct(array::concat($after<-performed_as<-joke, $after->performed_as->joke)) {
            fn::recompute_working($j);
        };
    };
