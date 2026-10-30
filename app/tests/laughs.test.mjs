// Unit test for the laugh-metric rules (UX_SPEC 5, P0-1). With REAL_CORPUS (or ~/punchable-data/corpus) present it also
// checks every corpus/sets/*/analysis.json there; without it, only the rule checks below run.
//   node --experimental-strip-types tests/laughs.test.mjs      (npm run test:unit)
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";
import * as L from "../src/rules/laughs.ts";

const CORPUS = process.env.REAL_CORPUS || (process.env.HOME || process.env.USERPROFILE || ".") + "/punchable-data/corpus";
const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

const HAS_CORPUS = existsSync(join(CORPUS, "sets"));
const dirs = HAS_CORPUS ? readdirSync(join(CORPUS, "sets")).filter((d) => existsSync(join(CORPUS, "sets", d, "analysis.json"))) : [];
if (HAS_CORPUS) check(dirs.length >= 1, `every analysis.json in ${CORPUS} (${dirs.length} sets)`);
else console.log(`SKIP corpus checks: ${CORPUS} not found (set REAL_CORPUS)`);

let rows = 0, nonBits = 0, derived = 0, quiet = 0;
for (const d of dirs) {
  const a = JSON.parse(readFileSync(join(CORPUS, "sets", d, "analysis.json"), "utf8"));
  const meta = existsSync(join(CORPUS, "sets", d, "meta.json")) ? JSON.parse(readFileSync(join(CORPUS, "sets", d, "meta.json"), "utf8")) : {};
  const S = a.summary;
  const roomLaughs = S.laughs;
  // the headline is laugh-s/min at 1 dp, never the detector count
  check(/^\d+\.\d$/.test(L.fmt1(S.laugh_s_per_min)) && L.fmt1(S.laugh_s_per_min) !== L.fmt1(S.laughs_per_min) || S.laugh_s_per_min === S.laughs_per_min,
    `${d}: headline ${L.fmt1(S.laugh_s_per_min)} laugh-s/min (1 dp), not ${S.laughs_per_min} laughs/min`);
  const unreliable = L.audioUnreliable({ room_heat: S.room_heat, room_laughs: roomLaughs });
  for (const j of a.jokes) {
    rows++;
    const row = { status: j.status, title: j.title, rating: null, rating_by: "ai", ai_rating: j.ai_rating, vs_room: j.vs_room, laughs: j.laughs };
    const shown = L.displayRating(row, roomLaughs);
    if (L.isNonBit(row)) {
      nonBits++;
      if (shown.word !== null) check(false, `${d} #${j.idx} ${j.title}: banter/crowd work shows no rating`);
      continue;
    }
    if (!L.roomComparable(roomLaughs, j.laughs?.window_s)) {
      quiet++;
      // only a silent long window may still say "died"
      if (shown.word && shown.word !== "died") check(false, `${d} #${j.idx}: room too quiet, but rating ${shown.word}`);
      continue;
    }
    derived++;
    const vs = j.vs_room;
    const want = j.laughs?.laugh_s === 0 && j.laughs?.window_s >= 20 ? "died" : vs >= 1.5 ? "killed" : vs >= 0.8 ? "worked" : vs >= 0.2 ? "meh" : "died";
    if (shown.word !== want) check(false, `${d} #${j.idx} ${j.title}: shows ${shown.word}, vs_room ${vs} says ${want}`);
  }
  // show date: from the title or description, never the upload date when one is written
  const sd = L.showDate({ date: `${d.slice(0, 4)}-${d.slice(4, 6)}-${d.slice(6, 8)}`, title: meta.title, description: meta.description });
  console.log(`      ${d}: show date ${sd.date} (${sd.source}) · venue ${L.venueShort(meta.venue_guess).name} · unreliable ${unreliable}`);
}
if (HAS_CORPUS) check(rows > 0, `checked ${rows} rows: ${nonBits} non-bits without ratings, ${derived} derived from vs_room, ${quiet} too quiet to compare`);
check(fails.length === 0 || !fails.some((f) => / #\d+/.test(f)), "every displayed rating agrees with its vs_room band (5.5)");

// show dates and venues (invented examples)
check(L.showDate({ date: "2017-10-10", title: "the comic @ Laugh Barn 8-16-2017" }).date === "2017-08-16", "show date Aug 16 2017 from the title");
check(L.showDate({ date: "2024-07-19", title: "the comic @ Mabel’s", description: "Mabel’s 7/13/24" }).date === "2024-07-13", "Mabel's show date Jul 13 2024 from the description");
check(L.showDate({ date: "2015-06-26", title: "the comic @ The Corner Room 6/20" }).date === "2015-06-20", "year-less 6/20 takes the upload year");
check(L.showDate({ date: "2015-02-18", title: "the comic Riverside Comedy Show" }).source === "upload", "no date in the title: upload date, not marked inferred");
// trends
check(L.trendWord([{ v: 9.1 }, { v: 0.15 }, { v: 3 }]) === "early read", "3 nights → 'early read', no trend");
check(L.trendWord([{ v: 3 }, { v: 4 }, { v: 0.6, excluded: true }, { v: 5 }, { v: 6 }]) === "rising", "≥ 4 non-excluded nights → a word (rising)");
check(L.trendWord([{ v: 3 }, { v: 4 }, { v: 0.6, excluded: true }, { v: 5 }]) === "early read", "excluded (unreliable) nights don't count toward the 4");
// merged fragments
const m = L.mergeNights([{ set: "r24", laugh_s: 3.72, window_s: 60 }, { set: "r24", laugh_s: 0.15, window_s: 60 }, { set: "ac", laugh_s: 9, window_s: 60 }]);
check(m.length === 2 && m[0].fragments === 2 && Math.abs(m[0].laugh_s_per_min - 1.935) < 0.01, "split fragments of one bit in one set merge into one night");
// formatting and words
check(L.fmt1(9.94) === "9.9" && L.fmtPct(8.5) === "9%", "rounding: 1 dp, integer % (m:ss is rules/format, tests/format.test.mjs)");
check(L.confWord(0.95) === "sure" && L.confWord(0.8) === "likely" && L.confWord(0.6) === "maybe" && L.confWord(0.3) === "long shot", "confidence words");
check(L.stabilityWord(0.2, 5) === "steady" && L.stabilityWord(0.4, 5) === "settling" && L.stabilityWord(0.9, 5) === "swinging" && L.stabilityWord(0.1, 2) === "too early", "stability words");
check(L.venueShort("Unknown (title 'Beach jokes'; maybe Seaside)").name === "Seaside?" && L.venueShort("Mabel's Comedy Club, Springfield").name === "Mabel's" && L.venueShort("Comix at Lakeside Mall, CT").name === "Comix Lakeside Mall", "venue short names");

console.log(`\n${fails.length ? fails.length + " FAILED" : "ALL PASS"}`);
process.exit(fails.length ? 1 : 0);
