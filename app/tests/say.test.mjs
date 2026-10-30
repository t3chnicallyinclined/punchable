// Unit test for src/rules/sayRules.ts (used by src/lib/say.ts): the client-side first cut of "say it or ask it" (the server's say_api.classify is
// the one recorded; this one only routes a search box) and the default short name (the same cut as
// creative/notebook.short_name_of).   node --experimental-strip-types --no-warnings tests/say.test.mjs
import { guessKind, positionIn, shortNameOf } from "../src/rules/sayRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

const examples = [
  ["what do i have about millennials that works", "question"],
  ["oh and what if the vegan brings the grill", "bit"],
  ["build me a tight 5", "command"],
  ["new 7 for friday", "command"],
  ["How do I get from one bit to the next?", "question"],
  ["anything I wrote about billionaires I never tried", "question"],
  ["my strongest opener", "question"],
  ["so um what are my untested bits", "question"],
  ["my dad thinks the microwave is listening to him", "bit"],
  ["imagine a hostage situation but the hostage keeps asking for oat milk", "bit"],
  ["the gym is just a library where everyone is wrong", "bit"],
  ["what if hotels charged by how guilty you feel", "bit"],
  ["", "bit"],
];
for (const [t, k] of examples) check(guessKind(t) === k, `${JSON.stringify(t)} -> ${k}`);

check(shortNameOf("Robot vacuum") === "Robot vacuum", "short: a short title stays");
check(shortNameOf("Haunted dishwasher saga (closer, 2019)") === "Haunted dishwasher", "short: cut at the parenthesis, then at a word break");
check(shortNameOf("The dog is the size of a couch") === "The dog is the", "short: word break inside 18");
check(shortNameOf("Antidisestablishmentarianism forever") === "Antidisestablishme", "short: one long word is cut at 18");
check(shortNameOf("Robot vacuum", "  Mine  ") === "Mine", "short: his own name wins");
check(shortNameOf("Robot vacuum", "") === "Robot vacuum", "short: an empty name is no name");

const inSet = [{ joke_id: "joke:a", position: 0 }, { joke_id: "joke:b", position: 1 }];
check(positionIn(inSet, "joke:b") === 1, "already in the set: its position (greyed, 'already at #2')");
check(positionIn(inSet, "joke:c") === null, "not in the set: null (the + is live)");

console.log(fails.length ? `${fails.length} FAILED` : "ALL PASS");
process.exit(fails.length ? 1 : 0);
