//! The learning loop's decisions, as pure functions: which ranking config is live, the offline shadow replay of
//! logged reads under a candidate config, the fence that rolls a promoted config back when it does worse, and the
//! learning switch.
//!
//! From upstream `upstream-ops/src/learning.rs` (Rust branch 9dfedad). Upstream reads and writes its rows
//! (`config_version`, `config_switch`, `snapshot`, `learning`, `ask_packet`) inside each function over a blocking
//! client; here the decisions are separated from the IO (FORK.md), so the async store (`pk-store`'s engine module)
//! fetches the rows and hands them in, and the engine never touches a database, a file or the environment. Gone
//! with the IO: the per-process cache of the live pointer, the `UPSTREAM_RANK_CONFIG` override, the owner
//! password logins, the journal write on a trip, the export folder read.
//!
//! Rules carried over: live answers always come from the live config whatever the learning mode; a replay first
//! proves it reproduces the logged packet under the packet's own config before it scores a candidate; a rollback is
//! one pointer move and never touches lessons or his material; a member may lower the learning mode, only he may
//! raise it.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::pack::{Hit, micros, pack};
use crate::rank_config::RankConfig;

/// The learning switch, lowest first. Ordered, so "raise" is `new > old`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Nothing is replayed or proposed.
    Off,
    /// Candidates are replayed offline; the live config never moves by itself.
    Shadow,
    /// A promoted config may go live (his go), held by the fence.
    On,
}

/// Why the live pointer moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    /// He promoted a config.
    Promote,
    /// He pinned the live config as a snapshot.
    Pin,
    /// He rolled back to a snapshot.
    Rollback,
    /// The fence rolled back.
    AutoRollback,
    /// The first pointer.
    Bootstrap,
}

/// The newest pointer row, as the store hands it in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Switch {
    /// The live config's id (its sha256).
    pub id: String,
    /// The live config's canonical JSON.
    pub canonical: Option<String>,
    /// Why it moved.
    pub reason: Reason,
    /// When (RFC 3339).
    pub at: String,
    /// Who.
    pub by: String,
    /// The snapshot a rollback went to.
    #[serde(default)]
    pub snapshot: Option<String>,
    /// What a promoted config is held to.
    #[serde(default)]
    pub guard: Option<Guard>,
}

/// Where the live config came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// The newest pointer.
    Pointer,
    /// No pointer yet: the built-in default.
    Default,
    /// The pointer's config could not be read by this build (another config version): the default, said so.
    Fallback,
}

/// The live config and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    /// The config.
    pub cfg: RankConfig,
    /// Its id.
    pub id: String,
    /// Where it came from.
    pub source: Source,
}

impl Live {
    /// The live config from the newest pointer: its config when it parses, validates and hashes to the pointer's id;
    /// the default when there is no pointer; the default, marked as a fallback, when the pointer cannot be read.
    pub fn resolve(newest: Option<&Switch>) -> Self {
        let of = |cfg: RankConfig, source| {
            let id = cfg.sha256();
            Live { cfg, id, source }
        };
        match newest {
            None => of(RankConfig::default(), Source::Default),
            Some(sw) => match sw.canonical.as_deref().map(RankConfig::from_json) {
                Some(Ok(cfg)) if cfg.sha256() == sw.id => Live {
                    cfg,
                    id: sw.id.clone(),
                    source: Source::Pointer,
                },
                _ => of(RankConfig::default(), Source::Fallback),
            },
        }
    }
}

/// The p-th percentile (0..=1) of `v`, nearest rank; the default of an empty list.
pub fn pct<T: Copy + Ord + Default>(v: &mut [T], p: f64) -> T {
    if v.is_empty() {
        return T::default();
    }
    v.sort_unstable();
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[i.min(v.len() - 1)]
}

/// One logged read's outcome, as the store hands it in for the KPIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    /// `PASS`, `FAIL`, `NOT_EXECUTED`, `NO_HITS`, or open.
    pub status: String,
    /// Microseconds the read took, when traced.
    pub us: Option<u64>,
    /// Bytes of text in the packet, when traced.
    pub bytes: Option<u64>,
}

/// Real reads of one config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kpis {
    /// Reads.
    pub asks: usize,
    /// Reads with a trace.
    pub traced: usize,
    /// Answers that passed the check.
    pub pass: usize,
    /// Answers that failed it.
    pub fail: usize,
    /// Answers with nothing read.
    pub not_executed: usize,
    /// Reads that found nothing.
    pub no_hits: usize,
    /// Reads not answered yet.
    pub open: usize,
    /// Median microseconds per read.
    pub us_p50: u64,
    /// p95 microseconds per read.
    pub us_p95: u64,
    /// Median packet bytes.
    pub bytes_median: usize,
}

impl Kpis {
    /// The KPIs of a set of outcomes.
    pub fn of(rows: &[Outcome]) -> Self {
        let mut k = Kpis::default();
        let (mut us, mut bytes) = (Vec::new(), Vec::new());
        for r in rows {
            k.asks += 1;
            match r.status.as_str() {
                "PASS" => k.pass += 1,
                "FAIL" => k.fail += 1,
                "NOT_EXECUTED" => k.not_executed += 1,
                "NO_HITS" => k.no_hits += 1,
                _ => k.open += 1,
            }
            if let Some(u) = r.us {
                k.traced += 1;
                us.push(u);
            }
            if let Some(b) = r.bytes {
                bytes.push(usize::try_from(b).unwrap_or(usize::MAX));
            }
        }
        k.us_p50 = pct(&mut us, 0.5);
        k.us_p95 = pct(&mut us, 0.95);
        k.bytes_median = pct(&mut bytes, 0.5);
        k
    }

    /// Failed and unexecuted answers over all answered.
    pub fn fail_rate(&self) -> f64 {
        let answered = self.pass + self.fail + self.not_executed;
        if answered == 0 {
            0.0
        } else {
            (self.fail + self.not_executed) as f64 / answered as f64
        }
    }
}

/// One logged read, rebuilt by the store from its trace and the records as they are now (a read whose records
/// changed since is not handed in: it is stale).
#[derive(Debug, Clone, PartialEq)]
pub struct Logged {
    /// The packet's id.
    pub packet: String,
    /// The question.
    pub question: String,
    /// The packet's place limit.
    pub limit: usize,
    /// The config the packet was made under (`None`: the default).
    pub own: Option<RankConfig>,
    /// The lessons of the read.
    pub facts: Vec<Hit>,
    /// The hits of the read, in read order.
    pub hits: Vec<Hit>,
    /// The packet's candidates' keys, in packet order.
    pub logged: Vec<String>,
    /// The keys a passed answer's support quotes cited.
    pub cited: Vec<String>,
}

/// One logged read replayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplayItem {
    /// The packet.
    pub packet: String,
    /// Its own config gave the logged packet exactly.
    pub reproduced: bool,
    /// For a passed answer with support quotes: every cited record is in the (own config, candidate) packet.
    pub cited: Option<(bool, bool)>,
    /// Packet bytes under its own config.
    pub live_bytes: usize,
    /// Packet bytes under the candidate.
    pub cand_bytes: usize,
    /// Candidates under its own config.
    pub live_kept: usize,
    /// Candidates under the candidate.
    pub cand_kept: usize,
    /// Microseconds the candidate's pack took.
    pub cand_us: u64,
}

/// A whole replay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Replay {
    /// Logged reads replayed (held-out ones are not).
    pub packets: usize,
    /// Logged reads left out because their question is in a held-out set (never replayed or scored).
    pub held_out: usize,
    /// Reads whose own config gave exactly the logged packet.
    pub reproduced: usize,
    /// The first few that did not, by id.
    pub not_reproduced: Vec<String>,
    /// Every read.
    pub items: Vec<ReplayItem>,
}

fn keys_of(facts: &[Hit], hits: &[Hit]) -> Vec<String> {
    facts.iter().chain(hits).map(|h| h.key.clone()).collect()
}

/// Replay one logged read under its own config (it must give the logged packet) and under `candidate`. No model,
/// no tokens: [`pack`] over the logged read.
pub fn replay_one(read: &Logged, candidate: &RankConfig) -> ReplayItem {
    let own = read.own.clone().unwrap_or_default();
    let live = pack(&read.question, read.limit, read.facts.clone(), read.hits.clone(), &own);
    let started = std::time::Instant::now();
    let cand = pack(
        &read.question,
        read.limit,
        read.facts.clone(),
        read.hits.clone(),
        candidate,
    );
    let cand_us = micros(started);
    let live_keys = keys_of(&live.facts, &live.hits);
    let reproduced = live_keys.iter().take(read.limit).cloned().collect::<Vec<_>>() == read.logged;
    let cand_keys = keys_of(&cand.facts, &cand.hits);
    let cited = (!read.cited.is_empty()).then(|| {
        (
            read.cited.iter().all(|c| live_keys.contains(c)),
            read.cited.iter().all(|c| cand_keys.contains(c)),
        )
    });
    ReplayItem {
        packet: read.packet.clone(),
        reproduced,
        cited,
        live_bytes: live.trace.bytes,
        cand_bytes: cand.trace.bytes,
        live_kept: live_keys.len(),
        cand_kept: cand_keys.len(),
        cand_us,
    }
}

/// A question's name in a held-out set: sha256 of its words, whitespace folded, lowercased (upstream's
/// `eval::question_hash`, unchanged).
pub fn question_hash(question: &str) -> String {
    let norm = question.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    hex::encode(Sha256::digest(norm.as_bytes()))
}

/// Replay every logged read under `candidate`, leaving out every read whose question is in `held_out` (question
/// hashes of the gold sets the engine is measured on: a held-out question never feeds a replay, upstream's
/// security rule F3).
pub fn replay(reads: &[Logged], candidate: &RankConfig, held_out: &BTreeSet<String>) -> Replay {
    let kept: Vec<&Logged> = reads
        .iter()
        .filter(|r| !held_out.contains(&question_hash(&r.question)))
        .collect();
    let mut r = Replay {
        packets: kept.len(),
        held_out: reads.len() - kept.len(),
        ..Replay::default()
    };
    for read in kept {
        let item = replay_one(read, candidate);
        if item.reproduced {
            r.reproduced += 1;
        } else if r.not_reproduced.len() < 10 {
            r.not_reproduced.push(item.packet.clone());
        }
        r.items.push(item);
    }
    r
}

impl Replay {
    /// The side-by-side numbers, over the reads that reproduced.
    pub fn kpis(&self) -> Value {
        let ok: Vec<&ReplayItem> = self.items.iter().filter(|i| i.reproduced).collect();
        let cited: Vec<(bool, bool)> = ok.iter().filter_map(|i| i.cited).collect();
        let median = |mut v: Vec<usize>| pct(&mut v, 0.5);
        let mut us: Vec<u64> = ok.iter().map(|i| i.cand_us).collect();
        json!({
            "reproduced": self.reproduced,
            "cited_n": cited.len(),
            "live_kept": cited.iter().filter(|c| c.0).count(),
            "cand_kept": cited.iter().filter(|c| c.1).count(),
            "live_bytes_median": median(ok.iter().map(|i| i.live_bytes).collect()),
            "cand_bytes_median": median(ok.iter().map(|i| i.cand_bytes).collect()),
            "cand_us_p50": pct(&mut us.clone(), 0.5),
            "cand_us_p95": pct(&mut us, 0.95),
        })
    }
}

/// What a promoted config is held to, against the snapshot it falls back to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Guard {
    /// The snapshot to fall back to.
    pub snapshot: String,
    /// Reads needed before the fence judges.
    pub min_asks: usize,
    /// Trip when the live p95 latency is above the snapshot's times this.
    pub max_p95_ratio: f64,
    /// Trip when the live median packet bytes are above the snapshot's times this.
    pub max_bytes_ratio: f64,
    /// Trip when the live fail rate is above the snapshot's plus this.
    pub max_fail_delta: f64,
}

/// The fence: why the live config must roll back to its guard's snapshot, if it must (empty: it holds, or it has
/// too few reads to judge). The caller writes the one rollback pointer and lowers learning from on to shadow.
pub fn fence(guard: &Guard, now: &Kpis, base: &Kpis) -> Vec<String> {
    let mut tripped = Vec::new();
    if now.asks < guard.min_asks {
        return tripped;
    }
    if base.us_p95 > 0 && now.us_p95 as f64 > base.us_p95 as f64 * guard.max_p95_ratio {
        tripped.push(format!(
            "read p95 {} us > {} x {} us",
            now.us_p95, guard.max_p95_ratio, base.us_p95
        ));
    }
    if base.bytes_median > 0 && now.bytes_median as f64 > base.bytes_median as f64 * guard.max_bytes_ratio {
        tripped.push(format!(
            "packet bytes median {} > {} x {}",
            now.bytes_median, guard.max_bytes_ratio, base.bytes_median
        ));
    }
    if now.fail_rate() > base.fail_rate() + guard.max_fail_delta {
        tripped.push(format!(
            "fail rate {:.3} > {:.3} + {}",
            now.fail_rate(),
            base.fail_rate(),
            guard.max_fail_delta
        ));
    }
    tripped
}

/// One learning-mode row, as the store hands it in, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeRow {
    /// The mode written.
    pub mode: Mode,
    /// Written by his own session (or the store's root), not a member.
    pub by_owner: bool,
}

/// The mode, decided at read time: his newest row, lowered by any later row a member wrote (a member can only lower,
/// never raise). No row of his: shadow.
pub fn decide_mode(rows: &[ModeRow]) -> Mode {
    let base_at = rows.iter().rposition(|r| r.by_owner);
    let mut mode = base_at.map_or(Mode::Shadow, |i| rows[i].mode);
    for r in &rows[base_at.map_or(0, |i| i + 1)..] {
        if r.mode < mode {
            mode = r.mode;
        }
    }
    mode
}

/// Whether a mode write shows: the mode reads what was written, or, for a lowering, anything at or below it.
pub fn readback_holds(requested: Mode, before: Mode, now: Mode) -> bool {
    if requested < before {
        now <= requested
    } else {
        now == requested
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(key: &str, text: &str, t: usize) -> Hit {
        Hit {
            key: key.into(),
            text: text.into(),
            hash: format!("{key}h"),
            text_rank: Some(t),
            ..Hit::default()
        }
    }

    #[test]
    fn the_live_config_is_the_pointers_when_it_checks_out() {
        assert_eq!(Live::resolve(None).source, Source::Default);
        let cfg = RankConfig::upstream();
        let good = Switch {
            id: cfg.sha256(),
            canonical: Some(cfg.canonical_json()),
            reason: Reason::Promote,
            at: "2026-11-02T00:00:00Z".into(),
            by: "owner".into(),
            snapshot: None,
            guard: None,
        };
        let live = Live::resolve(Some(&good));
        assert_eq!((live.source, live.cfg), (Source::Pointer, cfg));
        let wrong_id = Switch {
            id: "0".repeat(64),
            ..good.clone()
        };
        assert_eq!(Live::resolve(Some(&wrong_id)).source, Source::Fallback);
        let unreadable = Switch {
            canonical: Some("{\"future\": 1}".into()),
            ..good
        };
        assert_eq!(Live::resolve(Some(&unreadable)).source, Source::Fallback);
    }

    #[test]
    fn a_replay_reproduces_the_logged_packet_before_it_scores_a_candidate() {
        let hits: Vec<Hit> = (1..=6)
            .map(|i| hit(&format!("unit:{i}"), &format!("dog {}", "x ".repeat(i)), i))
            .collect();
        let own = RankConfig::upstream();
        let logged_packet = pack("dog", 4, vec![], hits.clone(), &own);
        let read = Logged {
            packet: "packet:1".into(),
            question: "dog".into(),
            limit: 4,
            own: Some(own),
            facts: vec![],
            hits: hits.clone(),
            logged: logged_packet.hits.iter().map(|h| h.key.clone()).collect(),
            cited: vec!["unit:6".into()],
        };
        let none = BTreeSet::new();
        let r = replay(std::slice::from_ref(&read), &RankConfig::default(), &none);
        assert_eq!(r.reproduced, 1, "{:?}", r.not_reproduced);
        let item = &r.items[0];
        assert!(item.cited.is_some());
        // A held-out question never feeds a replay, however it is spaced or cased.
        let held: BTreeSet<String> = [question_hash("  DOG ")].into();
        let r = replay(std::slice::from_ref(&read), &RankConfig::default(), &held);
        assert_eq!((r.packets, r.held_out, r.items.len()), (0, 1, 0));
        // A logged packet that its own config does not give is reported, not scored as if it were.
        let tampered = Logged {
            logged: vec!["unit:9".into()],
            ..read
        };
        let r = replay(&[tampered], &RankConfig::default(), &none);
        assert_eq!((r.reproduced, r.not_reproduced.len()), (0, 1));
        assert_eq!(r.kpis()["reproduced"], 0);
    }

    #[test]
    fn the_fence_trips_on_latency_bytes_or_failures_after_enough_reads() {
        let guard = Guard {
            snapshot: "s1".into(),
            min_asks: 10,
            max_p95_ratio: 1.5,
            max_bytes_ratio: 1.5,
            max_fail_delta: 0.1,
        };
        let base = Kpis {
            asks: 100,
            pass: 90,
            fail: 10,
            us_p95: 1000,
            bytes_median: 10_000,
            ..Kpis::default()
        };
        let mut now = Kpis {
            asks: 5,
            pass: 1,
            fail: 4,
            us_p95: 9000,
            bytes_median: 90_000,
            ..Kpis::default()
        };
        assert!(fence(&guard, &now, &base).is_empty(), "too few reads to judge");
        now.asks = 20;
        assert_eq!(fence(&guard, &now, &base).len(), 3);
        let fine = Kpis {
            asks: 20,
            pass: 19,
            fail: 1,
            us_p95: 1100,
            bytes_median: 11_000,
            ..Kpis::default()
        };
        assert_eq!(fence(&guard, &fine, &base).len(), 0);
    }

    #[test]
    fn kpis_count_and_percentile() {
        let rows: Vec<Outcome> = [("PASS", 10), ("FAIL", 30), ("NO_HITS", 20), ("OPEN", 40)]
            .iter()
            .map(|(s, u)| Outcome {
                status: (*s).into(),
                us: Some(*u),
                bytes: Some(100),
            })
            .collect();
        let k = Kpis::of(&rows);
        assert_eq!(
            (k.asks, k.pass, k.fail, k.no_hits, k.open, k.traced),
            (4, 1, 1, 1, 1, 4)
        );
        assert_eq!((k.us_p50, k.us_p95, k.bytes_median), (30, 40, 100));
        assert!((k.fail_rate() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn only_he_raises_the_learning_mode() {
        let row = |mode, by_owner| ModeRow { mode, by_owner };
        assert_eq!(decide_mode(&[]), Mode::Shadow);
        assert_eq!(decide_mode(&[row(Mode::On, true)]), Mode::On);
        assert_eq!(
            decide_mode(&[row(Mode::On, true), row(Mode::Off, false)]),
            Mode::Off,
            "a member lowers"
        );
        assert_eq!(
            decide_mode(&[row(Mode::Off, true), row(Mode::On, false)]),
            Mode::Off,
            "a member cannot raise"
        );
        assert_eq!(
            decide_mode(&[row(Mode::Off, false), row(Mode::On, true)]),
            Mode::On,
            "his newest row wins"
        );
        assert!(
            readback_holds(Mode::Shadow, Mode::On, Mode::Off),
            "a lowering already lower has done its job"
        );
        assert!(!readback_holds(Mode::On, Mode::Shadow, Mode::Shadow));
    }
}
