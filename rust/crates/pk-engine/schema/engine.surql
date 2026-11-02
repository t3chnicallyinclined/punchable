-- pk-engine's tables (docs/adr/0013-fork-the-upstream-engine.md). Applied ONLY by an explicit call
-- (`pk_store::Store::apply_engine_schema`): never on connect, never by a migration runner. On the hosted database it
-- goes through the same owner-approved path as db/schema_extra.surql (ADR 0009), after the offline proof, by his go.
-- His material (unit, joke, performance) is read through the tables and indexes db/schema.surql already defines;
-- nothing here changes them.
--
-- From upstream's schema 0005 (journal facts), 0006 (provenance) and 0022 (learning), adapted. Upstream guards
-- these rows with PERMISSIONS per member role; Punchable's services sign in as root or a database user, which
-- PERMISSIONS do not bind, so the same rules are held here by READONLY fields, ASSERTs, and events (an event runs for
-- every writer): every learning row is append-only, a config's id is the sha256 of its canonical JSON, a rollback
-- goes to a snapshot and to that snapshot's own config, every pointer move but an auto-rollback or the bootstrap
-- carries his statement, and the learning mode is raised only by a row his own signed-in session wrote (`signed`,
-- set by the API from the session, never from a request) with his statement. Lowering is open.
-- IF NOT EXISTS on tables (OVERWRITE would drop a changefeed), OVERWRITE on fields, indexes and events.

-- ---- lessons ----------------------------------------------------------------------------------------------------
DEFINE TABLE IF NOT EXISTS lesson SCHEMAFULL;
DEFINE FIELD OVERWRITE text      ON lesson TYPE string READONLY ASSERT string::len(string::trim($value)) > 0;
DEFINE FIELD OVERWRITE kind      ON lesson TYPE string READONLY ASSERT $value IN ['fact', 'decision', 'eliminated'];
-- the evidence rule's grade; internal, never served to the app (pk-engine edge.rs)
DEFINE FIELD OVERWRITE grade     ON lesson TYPE string READONLY ASSERT $value IN ['UNKNOWN', 'INFERRED', 'CONFIRMED'];
DEFINE FIELD OVERWRITE source    ON lesson TYPE string READONLY;
DEFINE FIELD OVERWRITE by        ON lesson TYPE string READONLY;
DEFINE FIELD OVERWRITE owner_statement ON lesson TYPE option<string> READONLY;
-- his own signed-in session wrote it (the API sets it from the session)
DEFINE FIELD OVERWRITE signed    ON lesson TYPE bool DEFAULT false READONLY;
-- the evidence: a record of his material and the exact slice of its text the verbatim gate found
DEFINE FIELD OVERWRITE evidence  ON lesson TYPE option<record<unit | joke | performance>> READONLY;
DEFINE FIELD OVERWRITE quote     ON lesson TYPE option<string> READONLY;
-- the one field that may change: a lesson is retired, never edited or deleted
DEFINE FIELD OVERWRITE status    ON lesson TYPE string DEFAULT 'live' ASSERT $value IN ['live', 'retired'];
DEFINE FIELD OVERWRITE at        ON lesson TYPE datetime DEFAULT time::now() READONLY;
-- CONFIRMED needs evidence in the row: words of his material (not hidden), or a statement his session signed
DEFINE EVENT OVERWRITE lesson_evidence ON lesson WHEN $event IN ['CREATE', 'UPDATE'] THEN {
    IF $after.grade = 'CONFIRMED'
        AND NOT ($after.owner_statement != NONE AND $after.signed = true)
        AND ($after.evidence = NONE OR $after.quote = NONE OR $after.evidence.hidden = true) {
        THROW 'a confirmed lesson needs evidence: words found in his material, or a statement he signed';
    };
    IF $event = 'UPDATE' AND $before.status = 'retired' AND $after.status = 'live' {
        THROW 'a retired lesson stays retired: record a new one';
    };
};
DEFINE EVENT OVERWRITE lesson_kept ON lesson WHEN $event = 'DELETE' THEN {
    THROW 'a lesson is retired, never deleted';
};
DEFINE INDEX OVERWRITE lesson_ft ON lesson FIELDS text FULLTEXT ANALYZER punch_en BM25;

-- ---- ranking configs, snapshots, the live pointer ---------------------------------------------------------------
-- A config: id = sha256 of its canonical JSON (pk_engine::rank_config::RankConfig::canonical_json), checked here.
DEFINE TABLE IF NOT EXISTS rank_config SCHEMAFULL;
DEFINE FIELD OVERWRITE canonical ON rank_config TYPE string READONLY ASSERT crypto::sha256($value) = record::id($this.id);
DEFINE FIELD OVERWRITE origin    ON rank_config TYPE string READONLY ASSERT $value IN ['default', 'owner', 'proposal', 'rollback'];
DEFINE FIELD OVERWRITE note      ON rank_config TYPE option<string> READONLY;
DEFINE FIELD OVERWRITE at        ON rank_config TYPE datetime DEFAULT time::now() READONLY;

-- A point he likes: the config and its baseline. id = the name (snapshot:r1).
DEFINE TABLE IF NOT EXISTS rank_snapshot SCHEMAFULL;
DEFINE FIELD OVERWRITE cfg       ON rank_snapshot TYPE record<rank_config> READONLY;
DEFINE FIELD OVERWRITE baseline  ON rank_snapshot TYPE object FLEXIBLE READONLY;
DEFINE FIELD OVERWRITE owner_statement ON rank_snapshot TYPE string READONLY ASSERT string::len($value) > 0;
DEFINE FIELD OVERWRITE at        ON rank_snapshot TYPE datetime DEFAULT time::now() READONLY;

-- The live pointer's history; the newest row is live. A rollback is one more row, never an edit.
DEFINE TABLE IF NOT EXISTS rank_switch SCHEMAFULL;
DEFINE FIELD OVERWRITE from_cfg  ON rank_switch TYPE option<record<rank_config>> READONLY;
DEFINE FIELD OVERWRITE to_cfg    ON rank_switch TYPE record<rank_config> READONLY;
DEFINE FIELD OVERWRITE snapshot  ON rank_switch TYPE option<record<rank_snapshot>> READONLY;
DEFINE FIELD OVERWRITE owner_statement ON rank_switch TYPE option<string> READONLY;
-- The rules sit on reason (always present): an ASSERT on an option field is not run when the field is absent.
DEFINE FIELD OVERWRITE reason    ON rank_switch TYPE string READONLY
  ASSERT $value IN ['promote', 'pin', 'rollback', 'auto-rollback', 'bootstrap']
    AND ($value NOT IN ['rollback', 'auto-rollback'] OR ($this.snapshot != NONE AND $this.snapshot.cfg = $this.to_cfg))
    AND ($value IN ['auto-rollback', 'bootstrap'] OR string::len($this.owner_statement ?? '') > 0);
DEFINE FIELD OVERWRITE guard     ON rank_switch TYPE option<object> FLEXIBLE READONLY;
DEFINE FIELD OVERWRITE fence     ON rank_switch TYPE option<object> FLEXIBLE READONLY;
DEFINE FIELD OVERWRITE by        ON rank_switch TYPE string READONLY;
DEFINE FIELD OVERWRITE at        ON rank_switch TYPE datetime DEFAULT time::now() READONLY;
DEFINE INDEX OVERWRITE rank_switch_at ON rank_switch FIELDS at;

-- ---- the learning switch -----------------------------------------------------------------------------------------
-- off < shadow < on; the newest row is the mode. Lowering (or the same mode) is open; raising needs a row his own
-- signed-in session wrote, with his statement.
DEFINE TABLE IF NOT EXISTS learning_mode SCHEMAFULL;
DEFINE FIELD OVERWRITE owner_statement ON learning_mode TYPE option<string> READONLY;
DEFINE FIELD OVERWRITE signed    ON learning_mode TYPE bool DEFAULT false READONLY;
DEFINE FIELD OVERWRITE mode      ON learning_mode TYPE string READONLY
  ASSERT $value IN ['off', 'shadow', 'on'] AND (
    {off: 0, shadow: 1, on: 2}[$value] <= {off: 0, shadow: 1, on: 2}[(SELECT VALUE mode FROM learning_mode ORDER BY at DESC LIMIT 1)[0] ?? 'shadow']
    OR ($this.signed = true AND string::len($this.owner_statement ?? '') > 0)
  );
DEFINE FIELD OVERWRITE by        ON learning_mode TYPE string READONLY;
DEFINE FIELD OVERWRITE at        ON learning_mode TYPE datetime DEFAULT time::now() READONLY;

-- ---- logged packets (the offline replay's input) ----------------------------------------------------------------
DEFINE TABLE IF NOT EXISTS packet SCHEMAFULL;
DEFINE FIELD OVERWRITE question  ON packet TYPE string READONLY;
DEFINE FIELD OVERWRITE cfg       ON packet TYPE record<rank_config> READONLY;
DEFINE FIELD OVERWRITE candidates ON packet TYPE array<object> FLEXIBLE READONLY;
DEFINE FIELD OVERWRITE trace     ON packet TYPE object FLEXIBLE READONLY;
-- the one field that moves: OPEN, then the check's verdict, once
DEFINE FIELD OVERWRITE status    ON packet TYPE string DEFAULT 'OPEN' ASSERT $value IN ['OPEN', 'NO_HITS', 'PASS', 'FAIL', 'NOT_EXECUTED'];
DEFINE FIELD OVERWRITE at        ON packet TYPE datetime DEFAULT time::now() READONLY;

-- Append-only: the learning loop's rows are never edited (READONLY above) and never deleted.
DEFINE EVENT OVERWRITE rank_config_kept   ON rank_config   WHEN $event = 'DELETE' THEN { THROW 'rank configs are append-only'; };
DEFINE EVENT OVERWRITE rank_snapshot_kept ON rank_snapshot WHEN $event = 'DELETE' THEN { THROW 'snapshots are append-only'; };
DEFINE EVENT OVERWRITE rank_switch_kept   ON rank_switch   WHEN $event = 'DELETE' THEN { THROW 'the pointer history is append-only'; };
DEFINE EVENT OVERWRITE learning_mode_kept ON learning_mode WHEN $event = 'DELETE' THEN { THROW 'the learning switch is append-only'; };
DEFINE EVENT OVERWRITE packet_kept        ON packet        WHEN $event = 'DELETE' THEN { THROW 'logged packets are append-only'; };
DEFINE EVENT OVERWRITE packet_verdict     ON packet        WHEN $event = 'UPDATE' AND $before.status != 'OPEN' THEN {
    THROW 'a packet has one verdict';
};
