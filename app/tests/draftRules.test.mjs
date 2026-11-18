// Unit test for rules/draftRules.ts (the new draft compared with his words: SPEC 2026-11-17 one notebook, screen 4
// and §5). Every line is invented.
//   node --experimental-strip-types --no-warnings tests/draftRules.test.mjs               the suite
//   node --experimental-strip-types --no-warnings tests/draftRules.test.mjs --red         every red arm
//   node --experimental-strip-types --no-warnings tests/draftRules.test.mjs --red=<arm>   one of: exact, back, outside, option, page
// A red arm plants a plausible wrong rule and must FAIL the same check the real rule passes:
//   exact    a compare that calls whole lines changed (his unchanged words on a changed line would be underlined)
//   back     a compose that ignores "Put mine back" (Use it would save Claude's words he turned down)
//   outside  an opening that shows every change (a change on a line he never marked would be used unasked)
//   option   a compose that always uses the first of 3 options (the take he picked would not be the one saved)
//   page     a page that draws his put-back words as new (he could not tell what he is about to save)
import * as D from "../src/rules/draftRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const MINE = [
  "The machine says unexpected item like I built it wrong.",
  "",
  "I work here now. No training, no vest, and somehow I'm already in trouble with my manager, who is a screen.",
  "",
  "Then a real employee walks over and looks at me like I tried to steal a lime.",
  "",
  "Other ideas: the lime costs 40 cents.",
].join("\n");
const DRAFT = [
  "The machine says unexpected item like I built it wrong.",
  "",
  "I work here now. No training, no vest, and I've already been written up by a screen.",
  "",
  "Then a real employee walks over and looks at me like I'm the reason limes are locked up now.",
  "",
  "Other ideas: the lime costs 50 cents.",
].join("\n");
const MARKS = [
  { type: "change", text: "somehow I'm already in trouble with my manager, who is a screen.", at: MINE.indexOf("somehow") },
  { type: "change", text: "looks at me like I tried to steal a lime.", at: MINE.indexOf("looks at me") },
];

// ---- the checks, each taking the rule under test so a red arm can hand in a wrong one

/** Only the words that differ are a change: the start of a changed line is still his, three changes are found, and
 *  putting the draft's words in place of his gives the draft exactly. */
function exact(hunks) {
  const hs = hunks(MINE, DRAFT);
  const firstNew = hs[0] ? DRAFT.slice(hs[0].b0, hs[0].b1) : "";
  return hs.length === 3
    && !firstNew.includes("I work here now")
    && firstNew.includes("written up by")
    && DRAFT.slice(hs[2].b0, hs[2].b1).trim() === "50"
    && D.compose(MINE, DRAFT, hs, hs.map(() => 0)) === DRAFT
    && D.compose(MINE, DRAFT, hs, hs.map(() => "mine")) === MINE;
}
/** Put mine back on one change: his words there, the draft's everywhere else. */
function putsBack(compose) {
  const hs = D.hunks(MINE, DRAFT);
  const out = compose(MINE, DRAFT, hs, ["mine", 0, 0]);
  return out.includes("somehow I'm already in trouble with my manager, who is a screen.")
    && out.includes("I'm the reason limes are locked up now.")
    && !out.includes("written up by");
}
/** A change on a line he put no mark on opens put back, and is counted; the ones on his marks open as the draft. */
function outsideIsBack(initialChoices) {
  const hs = D.hunks(MINE, DRAFT);
  const marked = D.onMarkedLines(MINE, hs, MARKS);
  const o = initialChoices(MINE, hs, marked, new Set());
  const text = D.compose(MINE, DRAFT, hs, o.choices);
  return eq(marked, [true, true, false])
    && eq(o.choices, [0, 0, "mine"])
    && eq(o.auto, [false, false, true])
    && text.includes("40 cents") && !text.includes("50 cents")
    && D.changesLines(o.choices, o.auto).main === "2 changes, both where you marked. Tap one to put yours back."
    && D.changesLines(o.choices, o.auto).also === "1 change outside your marks was put back.";
}
/** 3 options: the one he picks is the one in the text; the change covers the whole take. */
function picksOption(compose) {
  const texts = ["looks at me like I'm the reason limes are locked up now.", "looks at me like limes have a security detail.", "checks my pockets for citrus."];
  const hs = D.hunks(MINE, DRAFT, [{ item: 2, texts, at: DRAFT.indexOf(texts[0]) }]);
  const k = hs.findIndex((h) => h.options);
  if (k < 0 || hs[k].options.length !== 3) return false;
  const choices = hs.map((_, i) => (i === k ? 2 : 0));
  const out = compose(MINE, DRAFT, hs, choices);
  return DRAFT.slice(hs[k].b0, hs[k].b1) === texts[0]
    && MINE.slice(hs[k].a0, hs[k].a1) === "looks at me like I tried to steal a lime."
    && out.includes("Then a real employee walks over and checks my pockets for citrus.")
    && !out.includes("locked up now");
}
/** The page is the text he will save, line for line, and a put-back change is drawn as his. */
function pageIsTheText(paragraphs) {
  const hs = D.hunks(MINE, DRAFT);
  const choices = ["mine", 0, "mine"];
  const paras = paragraphs(MINE, DRAFT, hs, choices);
  const text = paras.map((p) => p.runs.map((r) => r.text).join("")).join("\n");
  const kinds = paras.flatMap((p) => p.runs.filter((r) => r.hunk != null).map((r) => [r.hunk, r.kind]));
  return text === D.compose(MINE, DRAFT, hs, choices)
    && eq(kinds, [[0, "mine"], [1, "new"], [2, "mine"]])
    && eq(paras.map((p) => p.ends), [[], [], [0], [], [1], [], [2]]);
}

// ---- the red arms: each a plausible wrong rule
const RED = {
  exact: () => exact((mine, draft) => {
    // whole lines: every line that differs is one change
    const a = mine.split("\n"); const b = draft.split("\n"); const out = []; let pa = 0; let pb = 0;
    for (let i = 0; i < Math.min(a.length, b.length); i++) {
      if (a[i] !== b[i]) out.push({ a0: pa, a1: pa + a[i].length, b0: pb, b1: pb + b[i].length });
      pa += a[i].length + 1; pb += b[i].length + 1;
    }
    return out;
  }),
  back: () => putsBack((mine, draft) => draft),
  outside: () => outsideIsBack((mine, hs) => ({ choices: hs.map(() => 0), auto: hs.map(() => false) })),
  option: () => picksOption((mine, draft, hs, choices) => D.compose(mine, draft, hs, choices.map((c) => (c === "mine" ? c : 0)))),
  page: () => pageIsTheText((mine, draft, hs, choices) => D.paragraphs(mine, draft, hs, choices).map((p) => ({ ...p, runs: p.runs.map((r) => (r.hunk == null ? r : { ...r, kind: "new" })) }))),
};

const red = process.argv.find((a) => a.startsWith("--red"));
if (red) {
  const which = red.includes("=") ? [red.split("=")[1]] : Object.keys(RED);
  for (const k of which) {
    if (!RED[k]) { console.log(`no red arm "${k}"`); process.exit(2); }
    const passed = RED[k]();
    console.log(`${passed ? "NOT RED" : "RED "} ${k}`);
    if (passed) fails.push(`the red arm "${k}" passed: its check cannot fail`);
  }
} else {
  check(exact(D.hunks), "only the words that differ are a change; all new is the draft, all back is his");
  check(putsBack(D.compose), "Put mine back: his words there, the draft's elsewhere");
  check(outsideIsBack(D.initialChoices), "a change on a line he did not mark opens put back, and is said");
  check(picksOption(D.compose), "the option he picks is the one saved");
  check(pageIsTheText(D.paragraphs), "the page is the text he will save; a put-back change reads as his");

  // the edges
  check(eq(D.hunks(MINE, MINE), []), "his own words back: no changes");
  check(D.changesLines([], []).main === "Nothing changed in this draft.", "no changes says so");
  check(eq(D.hunks("a b c ", "a b c"), []) && eq(D.hunks("a\n\nb\n", "a\n\nb"), []), "a trailing space or last newline is not a change");
  const shorter = D.hunks("It knows where I cry and it tells the fridge.", "It knows where I cry.");
  check(shorter.length === 1 && "It knows where I cry.".slice(shorter[0].b0, shorter[0].b1).trim().length > 0, "words only taken away: the change holds a word to tap");
  check(D.compose("It knows where I cry and it tells the fridge.", "It knows where I cry.", shorter, ["mine"]) === "It knows where I cry and it tells the fridge.", "and putting it back gives his line");
  const added = D.hunks("One.\nTwo.", "One.\nA new line.\nTwo.");
  check(added.length === 1 && D.compose("One.\nTwo.", "One.\nA new line.\nTwo.", added, ["mine"]) === "One.\nTwo.", "a whole new line is one change, and goes when put back");
  check(eq(D.onMarkedLines("One.\nTwo.", added, [{ type: "change", text: "One.", at: 0 }]), [true]), "a line added right under a marked line counts as where he marked");
  check(eq(D.onMarkedLines(MINE, D.hunks(MINE, DRAFT), [{ type: "change", text: "words he has since rewritten", at: 0 }]), [true, true, true]), "a mark that cannot be found: nothing is put back by itself");
  check(eq(D.onMarkedLines(MINE, D.hunks(MINE, DRAFT), MARKS, true), [true, true, true]), "a draft made from older words: nothing is put back by itself");
  const hs = D.hunks(MINE, DRAFT);
  const kept = new Set([D.hunkKey(MINE, hs[0])]);
  const again = D.initialChoices(MINE, hs, [true, true, true], kept);
  check(eq(again.choices, ["mine", 0, 0]) && eq(again.auto, [false, false, false]), "a change he put back stays back in the next draft");
  check(D.changesLines(again.choices, again.auto).also === null && D.changesLines(again.choices, again.auto).main === "2 changes, both where you marked. Tap one to put yours back.", "his own put-backs are not called outside his marks");
  const took = D.initialChoices(MINE, hs, [true, true, false], new Set(), new Set([D.hunkKey(MINE, hs[2])]));
  check(eq(took.choices, [0, 0, 0]) && eq(took.auto, [false, false, true]), "a change outside his marks that he took stays taken on that draft");
  check(D.changesLines(took.choices, took.auto).main === "3 changes. Tap one to put yours back." && D.changesLines(took.choices, took.auto).also === null, "and the page no longer says every change is where he marked");
  check(D.changesLines(["mine"], [false]).main === "Every change is put back: this is your page as you wrote it.", "everything back says so");
  check(D.changesLines([0], [false]).main === "1 change, where you marked. Tap it to put yours back.", "one change");
  check(D.changesLines([0, 0, 0], [false, false, false]).main === "3 changes, all where you marked. Tap one to put yours back.", "three changes");
  const t = D.againTarget(DRAFT, hs[0]);
  check(!!t && DRAFT.startsWith(t.text, t.at) && t.text === t.text.trim(), "Try again quotes the change as it stands in the draft");
  check(D.againTarget("a  b", { a0: 1, a1: 1, b0: 1, b1: 3 }) === null, "a change with no words has nothing to try again");
  check(D.brief("  a   long   run of words that goes on and on and on  ", 20) === "a long run of words…", "a change in a few words");
  const two = D.hunks("a b c d e f g h i j k l", "a X c d e f g h i j Y l");
  check(two.length === 2, "two changes far apart on one line stay two");
  const near = D.hunks("a b c d e", "a X c Y e");
  check(near.length === 1 && "a X c Y e".slice(near[0].b0, near[0].b1) === "X c Y", "two changes a word apart read as one");
  check(D.compose("x\r\ny", "x\r\nz", D.hunks("x\r\ny", "x\r\nz"), [0]) === "x\nz", "Windows line ends are read as line ends");
  const none = D.hunks(MINE, DRAFT, [{ item: 1, texts: ["not in the draft", "another"], at: 0 }]);
  check(none.every((h) => !h.options), "options whose take is not in the draft are left out");
  const twice = D.hunks(MINE, DRAFT, [{ item: 1, texts: ["the", "a"], at: 0 }]);
  check(twice.every((h) => !h.options), "options whose take stands in the draft more than once are left out: they name no one place");
}

if (fails.length) {
  console.log(`\n${fails.length} FAILED`);
  process.exit(1);
}
console.log("\nall passed");
