// Drives Desk explorer in headless Chrome over CDP (Node 22, no deps).
// Starts its own server on a free port against a TEMP copy of the fixture.
//   node tools/explorer/test_browser.mjs
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, existsSync, copyFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import net from "node:net";

const HERE = dirname(fileURLToPath(import.meta.url));
const CHROME = [
  "C:/Program Files/Google/Chrome/Application/chrome.exe",
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
].find(existsSync);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const freePort = () => new Promise((res) => { const s = net.createServer(); s.listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => res(p)); }); });
const sha = (p) => createHash("sha256").update(readFileSync(p)).digest("hex");

const kbdir = mkdtempSync(join(tmpdir(), "explorer_br_"));
copyFileSync(join(HERE, "fixtures", "graph.sample.json"), join(kbdir, "graph.json"));
copyFileSync(join(HERE, "fixtures", "KB_REPORT.sample.md"), join(kbdir, "KB_REPORT.md"));
const g0 = sha(join(kbdir, "graph.json"));
const DEC = join(kbdir, "decisions.jsonl");
const decs = () => (existsSync(DEC) ? readFileSync(DEC, "utf8").split("\n").filter(Boolean).map(JSON.parse) : []);

const port = await freePort(), dport = await freePort();
const base = `http://127.0.0.1:${port}`;
const srv = spawn("python", [join(HERE, "server.py"), "--port", String(port), "--graph", join(kbdir, "graph.json"), "--corpus", join(HERE, "fixtures", "corpus")], { stdio: ["ignore", "pipe", "pipe"] });
let srvLog = ""; srv.stdout.on("data", (d) => (srvLog += d)); srv.stderr.on("data", (d) => (srvLog += d));
const chrome = spawn(CHROME, ["--headless=new", `--remote-debugging-port=${dport}`, `--user-data-dir=${mkdtempSync(join(tmpdir(), "dx_chrome_"))}`,
  "--no-first-run", "--no-default-browser-check", "about:blank"], { stdio: "ignore" });

async function waitFor(fn, ms = 10000) { const t = Date.now(); for (;;) { try { const v = await fn(); if (v) return v; } catch {} if (Date.now() - t > ms) throw new Error("timeout waiting for " + fn); await sleep(100); } }
await waitFor(() => fetch(base + "/api/health").then((r) => r.ok));
const target = await waitFor(() => fetch(`http://127.0.0.1:${dport}/json/list`).then((r) => r.json()).then((l) => l.find((t) => t.type === "page")));
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0; const pending = new Map(); const errors = [];
ws.onmessage = (ev) => {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
  if (m.method === "Runtime.exceptionThrown") errors.push(m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text);
  if (m.method === "Runtime.consoleAPICalled" && m.params.type === "error") errors.push(m.params.args.map((a) => a.value || a.description).join(" "));
  if (m.method === "Log.entryAdded" && m.params.entry.level === "error" && !/favicon/.test(m.params.entry.url || "")) errors.push("log: " + m.params.entry.text + " " + (m.params.entry.url || ""));
};
const cdp = (method, params = {}) => new Promise((res, rej) => { const i = ++id; pending.set(i, (m) => (m.error ? rej(new Error(method + ": " + m.error.message)) : res(m.result))); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async (expr) => { const r = await cdp("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text); return r.result.value; };
const nav = async (hash, reload = false) => {
  if (reload) { await cdp("Page.navigate", { url: base + "/" + hash }); await sleep(300); await waitFor(() => ev("document.readyState === 'complete'")); }
  else await ev(`location.hash = ${JSON.stringify(hash)}`);
  await sleep(250);
  await waitFor(() => ev("!document.querySelector('#main').textContent.startsWith('Loading')"));
};
const center = (sel) => ev(`(() => { const e = document.querySelector(${JSON.stringify(sel)}); if (!e) throw new Error('no element ' + ${JSON.stringify(sel)}); e.scrollIntoView({block:'center'}); const r = e.getBoundingClientRect(); return {x: r.left + r.width/2, y: r.top + r.height/2}; })()`);
const mouse = (type, x, y) => cdp("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons: type === "mouseReleased" ? 0 : 1, clickCount: 1, pointerType: "mouse" });
const click = async (sel, wait = 350) => { const c = await center(sel); await mouse("mousePressed", c.x, c.y); await mouse("mouseReleased", c.x, c.y); await sleep(wait); };
const key = async (k) => { const p = { key: k, code: "Key" + k.toUpperCase(), windowsVirtualKeyCode: k.toUpperCase().charCodeAt(0), text: k };
  await cdp("Input.dispatchKeyEvent", { type: "keyDown", ...p }); await cdp("Input.dispatchKeyEvent", { type: "keyUp", ...p }); await sleep(400); };
const typeInto = async (sel, text) => { await click(sel, 50); await ev(`(() => { const e = document.querySelector(${JSON.stringify(sel)}); e.value = ''; })()`); await cdp("Input.insertText", { text }); await sleep(150); };
const getJSON = (p) => fetch(base + p).then((r) => r.json());
const E = (src, kind, dst) => "e:" + createHash("sha1").update(`${src}|${kind}|${dst}`).digest("hex").slice(0, 12);
const row = (eid) => `.er[data-eid="${eid}"]`;

try {
  await cdp("Page.enable"); await cdp("Runtime.enable"); await cdp("Log.enable");
  await cdp("Emulation.setDeviceMetricsOverride", { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
  await nav("#/", true);

  // ---------------- 1. joke list ----------------
  await waitFor(() => ev("document.querySelectorAll('#jokeList a.j').length === 6"));
  check(true, "list: 6 jokes");
  const first = await ev("document.querySelector('#jokeList a.j').dataset.id");
  check(first === "joke:airport-belt", `list: 'most to review' first = ${first}`);
  check(await ev("document.querySelector('#jokeList .c-prop b').textContent") === "12", "list: proposed count shown (12)");
  await typeInto("#jq", "sniffed"); await sleep(500);
  check(await ev("[...document.querySelectorAll('#jokeList a.j')].map(a => a.dataset.id).join()") === "joke:airport-belt", "list: text search finds unit words");
  await typeInto("#jq", "analogy"); await sleep(500);
  check(await ev("document.querySelectorAll('#jokeList a.j').length") === 2, "list: search matches labels");
  await typeInto("#jq", ""); await ev("document.getElementById('jq').dispatchEvent(new Event('input'))"); await sleep(400);
  await ev("(() => { const s = document.getElementById('jstage'); s.value = 'retired'; s.dispatchEvent(new Event('change')); })()"); await sleep(400);
  check(await ev("[...document.querySelectorAll('#jokeList a.j')].map(a => a.dataset.id).join()") === "joke:old-phone", "list: stage filter");
  await ev("(() => { const s = document.getElementById('jstage'); s.value = ''; s.dispatchEvent(new Event('change')); const l = document.getElementById('jlabel'); l.value = 'label:topic:dating'; l.dispatchEvent(new Event('change')); })()"); await sleep(400);
  check(await ev("[...document.querySelectorAll('#jokeList a.j')].map(a => a.dataset.id).join()") === "joke:dating-40", "list: label filter");
  await ev("(() => { const l = document.getElementById('jlabel'); l.value = ''; l.dispatchEvent(new Event('change')); })()"); await sleep(300);
  await click('#jokeList a.j[data-id="joke:airport-belt"]', 600);

  // ---------------- 2. joke page ----------------
  check(await ev("location.hash") === "#/joke/joke%3Aairport-belt", "list: tap opens the joke page");
  await waitFor(() => ev("document.querySelectorAll('.date-h').length === 2"));
  check(true, "joke: units grouped by 2 dates");
  check(await ev("[...document.querySelectorAll('.unit a')].some(a => a.href.startsWith('https://docs.google.com/document/d/1AbCdocBelt'))"), "joke: doc title links to the Google Doc");
  check(await ev("document.querySelectorAll('.perf .bars.l4').length") >= 1 && await ev("document.querySelector('.perf .laughs').textContent.includes('4 · applause break')"), "joke: laughs shown as labelled 0–4 bars");
  check(await ev("document.querySelectorAll('.er.st-proposed').length") >= 8, "joke: edges shown as rows with status colour");
  check(await ev("document.querySelector('.er blockquote.ev') !== null && document.querySelector('.er .conf').textContent.includes('%')"), "joke: rows show confidence + evidence quotes");

  const spec = E("joke:airport-belt", "has_label", "label:device:specificity");
  await click(`${row(spec)} [data-act=confirm]`, 600);
  check(decs().length === 1 && decs()[0].action === "confirm" && decs()[0].id === spec, "confirm: one line appended");
  check(await ev(`document.querySelector('${row(spec)}').classList.contains('st-confirmed')`), "confirm: row turns green (confirmed)");

  const shape = E("joke:airport-belt", "same_shape", "joke:old-phone");
  await click(`${row(shape)} [data-act=reject]`, 600);
  check(decs().length === 2 && decs()[1].action === "reject", "reject: one line appended");
  check(await ev(`document.querySelector('${row(shape)}') === null`), "reject: row hidden by default");
  await click("#showRej", 500);
  check(await ev(`document.querySelector('${row(shape)}')?.classList.contains('st-rejected')`), "reject: visible (red) under 'show rejected'");
  await click("#showRej", 400);

  const seg = E("joke:gps-voice", "segue", "joke:airport-belt");
  await click(`${row(seg)} [data-act=edit]`, 300);
  await ev(`(() => { const f = document.querySelector('${row(seg)} form.editbox'); f.elements.kind.value = 'custom'; f.elements.kind.dispatchEvent(new Event('change', {bubbles:true})); })()`);
  await typeInto(`${row(seg)} input[name=kind_custom]`, "same_target");
  await typeInto(`${row(seg)} .picker input`, "Dating");
  await click(`${row(seg)} .picker .sug button`, 150);
  await typeInto(`${row(seg)} textarea[name=reason]`, "both are about being judged");
  await click(`${row(seg)} button[type=submit]`, 700);
  let D = decs().at(-1);
  check(D.action === "edit" && D.after.kind === "custom" && D.after.custom_kind === "same_target" && D.after.src === "joke:dating-40" && D.reason === "both are about being judged",
    "edit: kind → custom 'same_target', other end → Dating at 40, reason saved (" + JSON.stringify(D.after) + ")");
  // the edge left this joke (its other end changed), so it is now on Dating at 40
  const dj = await getJSON("/api/joke/joke:dating-40");
  check(dj.edges.some((e) => e.id === seg && e.display_kind === "same_target" && e.bucket === "confirmed" && e.edited), "edit: shows on the new joke as edited + confirmed");

  await typeInto("#addConn .picker input", "Plant");
  await click("#addConn .picker .sug button", 150);
  await ev("(() => { const s = document.querySelector('#addConn select[name=kind]'); s.value = 'callback_to'; s.dispatchEvent(new Event('change', {bubbles:true})); })()");
  await typeInto("#addConn textarea[name=reason]", "Suspenders come back in the cactus bit");
  await click("#addConn button[type=submit]", 700);
  D = decs().at(-1);
  const addId = E("joke:airport-belt", "callback_to", "joke:plant-dad");
  check(D.action === "add" && D.id === addId && D.after.kind === "callback_to" && D.reason.startsWith("Suspenders"), "add connection: one 'add' line with reason");
  check(await ev(`!!document.querySelector('${row(addId)} .pill.yours')`), "add connection: row appears marked 'yours'");

  const n0 = decs().length;
  await typeInto("#addLabel .picker input", "Self own");
  await waitFor(() => ev("[...document.querySelectorAll('#addLabel .sug button')].some(b => b.dataset.v === 'new-label:Self own')"));
  await click('#addLabel .sug button[data-v="new-label:Self own"]', 200);
  check(await ev("!document.getElementById('newLabelBits').hidden"), "add label: new label shows kind + definition fields");
  await typeInto("#addLabel input[name=ldef]", "The punch is on me");
  await click("#addLabel button[type=submit]", 900);
  const L2 = decs().slice(n0);
  check(L2.length === 2 && L2[0].action === "add" && L2[0].target === "node" && L2[0].id === "label:custom:self-own" && L2[0].after.definition === "The punch is on me"
    && L2[1].action === "add" && L2[1].after.dst === "label:custom:self-own", "add custom label: node add + has_label add");
  const selfEdge = L2[1].id;
  check(await ev(`!!document.querySelector('${row(selfEdge)}')`), "add custom label: label row on the joke");

  // undo via the snackbar
  check(await ev("document.getElementById('snack').classList.contains('show')"), "snackbar with Undo shown after an action");
  await click("#snackUndo", 800);
  D = decs().at(-1);
  check(D.action === "delete" && D.id === selfEdge && D.undo_of === decs().length - 2, "undo: appended inverse decision (delete, undo_of)");
  check(await ev(`document.querySelector('${row(selfEdge)}') === null`), "undo: label row gone");

  // stage
  await ev("(() => { const s = document.getElementById('stageSel'); s.value = 'working'; s.dispatchEvent(new Event('change')); })()"); await sleep(700);
  D = decs().at(-1);
  check(D.target === "node" && D.action === "edit" && D.after.stage === "working" && D.before.stage === "testing", "stage change recorded");

  // reload persistence (full page reload; server rebuilds from decisions.jsonl)
  await nav("#/joke/" + encodeURIComponent("joke:airport-belt"), true);
  await waitFor(() => ev(`!!document.querySelector('${row(spec)}')`));
  check(await ev(`document.querySelector('${row(spec)}').classList.contains('st-confirmed')`) && await ev(`!!document.querySelector('${row(addId)}')`)
    && await ev("document.getElementById('stageSel').value") === "working" && await ev(`document.querySelector('${row(shape)}') === null`)
    && await ev(`document.querySelector('${row(selfEdge)}') === null`), "reload: confirm, reject, add, undo and stage all persist");

  // ---------------- 3. review ----------------
  await nav("#/review");
  await waitFor(() => ev("!!document.getElementById('rvY')"));
  const q = await getJSON("/api/queue");
  check(await ev("document.querySelector('.rv-card .er').dataset.eid") === q.report_ids[0], "review: KB_REPORT's first item comes first");
  let before = decs().length;
  await key("y");
  check(decs().length === before + 1 && decs().at(-1).action === "confirm" && decs().at(-1).id === q.report_ids[0], "review: Y confirms");
  await waitFor(() => ev(`document.querySelector('.rv-card .er').dataset.eid === ${JSON.stringify(q.report_ids[1])}`));
  await key("s");
  check(decs().length === before + 1 && await ev(`document.querySelector('.rv-card .er').dataset.eid !== ${JSON.stringify(q.report_ids[1])}`), "review: S skips without writing");
  const nid = await ev("document.querySelector('.rv-card .er').dataset.eid");
  await key("n");
  check(decs().at(-1).action === "reject" && decs().at(-1).id === nid, "review: N rejects");
  await key("e");
  check(await ev("!!document.querySelector('.rv-card form.editbox')"), "review: E opens the edit form");
  await ev("document.activeElement.blur()");
  await key("u");
  check(decs().at(-1).undo_of != null && decs().at(-1).id === nid, "review: U undoes the last decision");
  check(await ev("document.getElementById('rvY').getBoundingClientRect().height") >= 60, "review: big buttons");
  await click('[data-mode="rejected"]', 500);
  check(await ev(`!!document.querySelector('.er[data-eid="${shape}"]')`), "review: rejected list keeps rejected items");
  await click('[data-mode="proposed"]', 400);

  // ---------------- 4. graph ----------------
  await nav("#/graph/" + encodeURIComponent("joke:gps-voice"));
  await waitFor(() => ev("document.querySelectorAll('#gsvg g.n').length > 3"));
  const n1 = await ev("document.querySelectorAll('#gsvg g.n').length");
  await click('[data-h="2"]', 600);
  const n2 = await ev("document.querySelectorAll('#gsvg g.n').length");
  check(n2 > n1, `graph: 2 hops shows more nodes (${n1} → ${n2})`);
  const pos = await ev("[...document.querySelectorAll('#gsvg g.n')].map(g => g.getAttribute('transform')).every(t => !/NaN/.test(t))");
  check(pos, "graph: layout positions are finite");
  await ev("(() => { document.querySelector('details').open = true; })()");
  await ev("(() => { const c = [...document.querySelectorAll('.gk')].find(x => x.value === 'has_label'); c.checked = true; c.dispatchEvent(new Event('change')); })()"); await sleep(600);
  check(await ev("[...document.querySelectorAll('#gsvg line title')].every(t => t.textContent.startsWith('has_label'))"), "graph: edge-kind filter");
  await click('#gsvg g.n[data-id="label:device:analogy"]', 600);
  check(await ev("location.hash").then((h) => h.startsWith("#/labels?focus=label%3Adevice%3Aanalogy")), "graph: tap a label opens it in Labels");
  await ev("(() => { localStorage.removeItem('dx.graphFilters'); })()");

  // ---------------- 5. labels ----------------
  await waitFor(() => ev("!!document.querySelector('.labelcard.focus')"));
  check(await ev("document.querySelector('.labelcard.focus').dataset.lid") === "label:device:analogy", "labels: focused label highlighted");
  await click('.labelcard[data-lid="label:device:repetition"] [data-act=rename]', 200);
  await typeInto('.labelcard[data-lid="label:device:repetition"] input[name=name]', "repeat");
  await click('.labelcard[data-lid="label:device:repetition"] button[type=submit]', 700);
  D = decs().at(-1);
  check(D.target === "node" && D.action === "edit" && D.after.name === "repeat" && D.id === "label:device:repetition", "labels: rename recorded as one decision");
  await click('.labelcard[data-lid="label:attitude:hard"] [data-act=merge]', 200);
  await typeInto('.labelcard[data-lid="label:attitude:hard"] .picker input', "dating");
  await click('.labelcard[data-lid="label:attitude:hard"] .sug button[data-v="label:topic:dating"]', 150);
  await click('.labelcard[data-lid="label:attitude:hard"] button[type=submit]', 700);
  D = decs().at(-1);
  check(D.after.merged_into === "label:topic:dating", "labels: merge recorded as one decision");
  check(await ev("!document.querySelector('.labelcard[data-lid=\"label:attitude:hard\"]') && document.querySelector('.labelcard[data-lid=\"label:topic:dating\"]').textContent.includes('Includes merged')"), "labels: merged label folded in");

  // ---------------- 6. learned ----------------
  await nav("#/learned");
  await waitFor(() => ev("document.querySelectorAll('table').length >= 2"));
  const txt = await ev("document.getElementById('main').textContent");
  check(txt.includes("has_label") && txt.includes("same_shape") && txt.includes("Self own") && txt.includes("Suspenders come back"), "learned: per-kind table, custom label and connection listed");

  // ---------------- phone width ----------------
  await cdp("Emulation.setDeviceMetricsOverride", { width: 375, height: 780, deviceScaleFactor: 2, mobile: true });
  for (const h of ["#/", "#/joke/" + encodeURIComponent("joke:airport-belt"), "#/graph/" + encodeURIComponent("joke:airport-belt"), "#/labels", "#/review", "#/learned"]) {
    await nav(h, true); await sleep(500);
    if (h.startsWith("#/joke")) { await click('.er [data-act=edit]', 300); await click("#showRej", 400); }
    await ev("window.scrollTo(0, 0)");
    const sw = await ev("Math.max(document.documentElement.scrollWidth, document.body.scrollWidth)");
    const wide = await ev("[...document.querySelectorAll('body *')].filter(e => { const r = e.getBoundingClientRect(); return r.width && r.right > 375.5 && !e.closest('.sug,[hidden],svg'); }).slice(0, 3).map(e => e.tagName + '.' + e.className + ' ' + Math.round(e.getBoundingClientRect().right))");
    check(sw <= 375 && !wide.length, `phone 375px: no horizontal scroll on ${h} (scrollWidth ${sw}${wide.length ? "; too wide: " + wide.join(", ") : ""})`);
    if (process.env.SHOTS) {
      const shot = await cdp("Page.captureScreenshot", { format: "png", captureBeyondViewport: true, clip: { x: 0, y: 0, width: 375, height: Math.min(2400, await ev("document.documentElement.scrollHeight")), scale: 1 } });
      (await import("node:fs")).writeFileSync(join(process.env.SHOTS, h.replace(/[^a-z]+/gi, "_") + ".png"), Buffer.from(shot.data, "base64"));
    }
  }
  check(errors.length === 0, "no JS errors" + (errors.length ? ": " + errors.slice(0, 3).join(" | ") : ""));
} catch (e) {
  check(false, "exception: " + e.stack);
} finally {
  ws.close(); chrome.kill(); srv.kill();
}
check(sha(join(kbdir, "graph.json")) === g0, "graph.json byte-identical after the whole run");
const badLog = srvLog.split(/\r?\n/).filter((l) => /ERROR|Traceback/.test(l));
check(!badLog.length, "server log clean" + (badLog.length ? ": " + badLog.slice(0, 4).join(" | ") : ""));
console.log(`\n${fails.length ? fails.length + " FAILED" : "ALL PASS"}  (temp kb: ${kbdir}, ${decs().length} decisions)`);
process.exit(fails.length ? 1 : 0);
