// Desk explorer UI. Hash routes; every action is one POST that appends one decision line.
"use strict";

const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const enc = encodeURIComponent;
const main = () => document.getElementById("main");
const store = {
  get(k, d) { try { const v = localStorage.getItem("dx." + k); return v == null ? d : JSON.parse(v); } catch { return d; } },
  set(k, v) { try { localStorage.setItem("dx." + k, JSON.stringify(v)); } catch { /* private mode */ } },
};

async function api(path, opts = {}) {
  const r = await fetch(path, {
    method: opts.method || "GET",
    headers: opts.body !== undefined ? { "Content-Type": "application/json" } : {},
    body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
    cache: "no-store",
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || `HTTP ${r.status}`);
  return j;
}

let META = null;
let QTOTAL = 0;
const SKIPPED = new Set(); // review skips are per session, never written
const LEVELS = ["none", "chuckle", "laugh", "big laugh", "applause break"];
const STWORD = { confirmed: "✓ confirmed", proposed: "? proposed", rejected: "✕ rejected" };

// ---------------------------------------------------------------- shared bits
function bars(level) {
  const l = Math.max(0, Math.min(4, level | 0));
  let h = `<span class="bars l${l}" aria-hidden="true">`;
  for (let i = 1; i <= 4; i++) h += `<i class="${i <= l ? "on" : ""}"></i>`;
  return h + "</span>";
}
const barsLabel = (l) => `${bars(l)} <span>${l} · ${LEVELS[l] || "?"}</span>`;
const fmtMs = (ms) => (ms == null ? "" : `${Math.floor(ms / 60000)}:${String(Math.floor((ms % 60000) / 1000)).padStart(2, "0")}`);
const pct = (c) => (typeof c === "number" ? Math.round(c * 100) + "%" : "–");
const docUrl = (u) => (u.source === "gdoc" && u.doc_id ? `https://docs.google.com/document/d/${enc(u.doc_id)}/edit` : null);

function nodeHref(b) {
  if (!b) return "#/";
  if (b.type === "joke") return "#/joke/" + enc(b.id);
  if (b.type === "label") return "#/labels?focus=" + enc(b.id);
  if (b.type === "unit" && b.joke) return "#/joke/" + enc(b.joke);
  return null;
}
function nodeLink(b) {
  const h = nodeHref(b);
  const full = b.title || b.id;
  const t = esc(b.type === "unit" && full.length > 70 ? full.slice(0, 68) + "…" : full);
  const pre = b.type === "label" ? `<span class="tiny muted">${esc(b.kind || "label")}:</span> ` : b.type === "unit" ? `<span class="tiny muted">unit${b.joke_title ? " of " + esc(b.joke_title) : ""}:</span> ` : "";
  return h ? `${pre}<a href="${h}">${t}</a>` : `${pre}${t}`;
}

function snack(msg, undoIndex, bad = false) {
  const s = $("#snack");
  $("#snackMsg").textContent = msg;
  const b = $("#snackUndo");
  b.hidden = undoIndex == null;
  b.onclick = async () => { s.classList.remove("show"); await undo(undoIndex); };
  s.classList.toggle("bad", bad);
  s.classList.add("show");
  clearTimeout(snack._t);
  snack._t = setTimeout(() => s.classList.remove("show"), bad ? 6000 : 7000);
}

async function decide(body, msg) {
  try {
    const r = await api("/api/decision", { method: "POST", body });
    snack(msg, r.index);
    await refresh();
    return r;
  } catch (e) {
    snack("Not saved: " + e.message, null, true);
    return null;
  }
}
async function undo(index) {
  try {
    await api("/api/undo", { method: "POST", body: index == null ? {} : { index } });
    snack("Undone");
    await refresh();
  } catch (e) {
    snack("Undo failed: " + e.message, null, true);
  }
}

async function loadMeta() {
  META = await api("/api/meta");
  const q = await api("/api/queue?limit=1");
  QTOTAL = q.total;
  const b = $("#qbadge");
  b.hidden = !QTOTAL;
  b.textContent = QTOTAL;
  const gs = $("#gstate");
  gs.textContent = META.graph_state === "ok" ? `${META.counts.jokes} jokes · ${META.counts.edges} links` : "graph: " + META.graph_state;
  gs.title = META.graph_path;
}

function graphBanner() {
  if (!META || META.graph_state === "ok") return "";
  const msg = META.graph_state === "missing"
    ? `The graph isn't there yet: <span class="kbd">${esc(META.graph_path)}</span>. It is probably still being generated. This page checks again every 10 seconds.`
    : `The graph file can't be read right now (it may be mid-write). Showing the last good copy. <span class="kbd">${esc(META.graph_path)}</span>`;
  return `<div class="notice">${msg}</div>`;
}

// Picker: type to search jokes / labels; can offer "new idea" / "new label".
function pickerHtml(name, { types = ["joke", "label"], newIdea = false, newLabel = false, placeholder = "Search…", value = "", valueText = "" } = {}) {
  return `<div class="picker" data-name="${name}" data-types="${types.join(",")}" data-newidea="${newIdea ? 1 : ""}" data-newlabel="${newLabel ? 1 : ""}">
    <input type="search" autocomplete="off" placeholder="${esc(placeholder)}" aria-label="${esc(placeholder)}" value="${esc(valueText)}" data-value="${esc(value)}">
    <div class="sug" hidden></div></div>`;
}
function pickerItems(p, q) {
  const types = p.dataset.types.split(",");
  const ql = q.toLowerCase().trim();
  const out = [];
  if (types.includes("joke")) for (const j of META.jokes) out.push({ v: j.id, t: j.title, sub: `joke · ${j.stage || "?"}` });
  if (types.includes("label")) for (const l of META.labels) out.push({ v: l.id, t: l.name, sub: `label · ${l.kind}` });
  let hits = ql ? out.filter((o) => (o.t + " " + o.v).toLowerCase().includes(ql)) : out;
  hits = hits.slice(0, 12);
  if (ql && p.dataset.newlabel) hits.push({ v: "new-label:" + q.trim(), t: `+ New label “${q.trim()}”`, sub: "create it" });
  if (ql && p.dataset.newidea) hits.push({ v: "new-idea:" + q.trim(), t: `+ New idea “${q.trim()}”`, sub: "a new joke at stage idea" });
  return hits;
}
function pickerValue(p) { return $("input", p).dataset.value || ""; }
function pickerChanged(p) { p.dispatchEvent(new CustomEvent("picked", { bubbles: true, detail: pickerValue(p) })); }
document.addEventListener("input", (ev) => {
  const p = ev.target.closest(".picker");
  if (!p) return;
  const inp = ev.target;
  inp.dataset.value = "";
  pickerChanged(p);
  const sug = $(".sug", p);
  const items = pickerItems(p, inp.value);
  sug.innerHTML = items.map((o, i) => `<button type="button" data-v="${esc(o.v)}" data-t="${esc(o.t.replace(/^\+ New (label|idea) “|”$/g, ""))}" class="${i === 0 ? "hi" : ""}">${esc(o.t)} <span class="tiny muted">${esc(o.sub)}</span></button>`).join("");
  sug.hidden = !items.length;
});
document.addEventListener("click", (ev) => {
  const b = ev.target.closest(".picker .sug button");
  if (b) {
    const p = b.closest(".picker");
    const inp = $("input", p);
    inp.value = b.dataset.t;
    inp.dataset.value = b.dataset.v;
    $(".sug", p).hidden = true;
    pickerChanged(p);
    return;
  }
  for (const s of $$(".picker .sug")) if (!s.closest(".picker").contains(ev.target)) s.hidden = true;
});
document.addEventListener("keydown", (ev) => {
  const p = ev.target.closest && ev.target.closest(".picker");
  if (!p) return;
  const btns = $$(".sug button", p);
  if (!btns.length || $(".sug", p).hidden) return;
  let i = btns.findIndex((b) => b.classList.contains("hi"));
  if (ev.key === "ArrowDown" || ev.key === "ArrowUp") {
    ev.preventDefault();
    btns[i]?.classList.remove("hi");
    i = (i + (ev.key === "ArrowDown" ? 1 : -1) + btns.length) % btns.length;
    btns[i].classList.add("hi");
  } else if (ev.key === "Enter") {
    ev.preventDefault();
    (btns[i] || btns[0]).click();
  } else if (ev.key === "Escape") {
    $(".sug", p).hidden = true;
  }
});

function kindSelect(name, current, { customValue = "", includeHasLabel = true } = {}) {
  const kinds = META.edge_kinds.filter((k) => includeHasLabel || k !== "has_label");
  return `<select name="${name}" aria-label="Kind of connection">${kinds.map((k) => `<option value="${k}" ${k === current ? "selected" : ""}>${k === "custom" ? "custom…" : k}</option>`).join("")}</select>
    <input type="text" name="${name}_custom" placeholder="Your own kind, e.g. same_target" value="${esc(customValue)}" ${current === "custom" ? "" : "hidden"} list="customKinds">
    <datalist id="customKinds">${(META.custom_kinds || []).map((k) => `<option value="${esc(k)}">`).join("")}</datalist>`;
}
document.addEventListener("change", (ev) => {
  const s = ev.target;
  if (s.tagName === "SELECT" && s.form && s.form.elements[s.name + "_custom"]) {
    s.form.elements[s.name + "_custom"].hidden = s.value !== "custom";
  }
});

// Edge row with one-tap Confirm / Reject / Edit.
function edgeRow(e, { showVia = true } = {}) {
  const other = e.other || e.dst_brief;
  const dir = e.direction === "in" ? "←" : "→";
  const ev = (e.evidence || []).map((x) => `<blockquote class="ev">“${esc(x.quote)}”${x.node ? ` <span class="src">${esc(x.node)}${x.start_ms != null ? " @ " + fmtMs(x.start_ms) : ""}</span>` : ""}</blockquote>`).join("");
  const via = showVia && e.via_unit ? `<div class="tiny muted">on unit: “${esc((e.via_unit.title || "").slice(0, 80))}”</div>` : "";
  const b = e.bucket;
  return `<div class="er st-${b}" data-eid="${esc(e.id)}">
    <div class="head"><span class="stword">${STWORD[b]}</span><span class="kind">${esc(e.display_kind)} ${dir}</span>
      <span class="other grow">${nodeLink(other)}</span>
      <span class="conf">${e.yours ? '<span class="pill yours">yours</span> ' : ""}${e.edited ? '<span class="pill">edited</span> ' : ""}conf ${pct(e.confidence)}</span></div>
    ${via}
    ${e.reason ? `<div class="reason">${esc(e.reason)}</div>` : ""}
    ${ev}
    <div class="acts">
      <button class="mini yes" data-act="confirm" ${b === "confirmed" ? "disabled" : ""} aria-label="Confirm">✓ Confirm</button>
      <button class="mini no" data-act="reject" ${b === "rejected" ? "disabled" : ""} aria-label="Reject">✕ Reject</button>
      <button class="mini" data-act="edit" aria-label="Edit">✎ Edit</button>
      ${e.yours ? '<button class="mini ghost" data-act="delete">Delete</button>' : ""}
    </div><div class="editslot"></div></div>`;
}
const EDGES = new Map(); // id -> edge view currently on screen

function editBoxHtml(e) {
  const other = e.other || e.dst_brief;
  return `<form class="editbox" data-eid="${esc(e.id)}">
    <div class="small muted">Change the kind, the other end, or both. Saving marks it confirmed with your change.</div>
    <div class="row">${kindSelect("kind", e.kind, { customValue: e.custom_kind || "" })}</div>
    ${pickerHtml("target", { types: ["joke", "label"], placeholder: "Other end (joke or label)", value: other.id, valueText: other.title || other.id })}
    <textarea name="reason" placeholder="Why? (your words, optional)"></textarea>
    <div class="row"><button class="primary" type="submit">Save change</button><button type="button" data-act="cancel">Cancel</button></div></form>`;
}

async function submitEdit(form) {
  const e = EDGES.get(form.dataset.eid);
  const other = e.other || e.dst_brief;
  const kind = form.elements.kind.value;
  const after = { kind };
  if (kind === "custom") after.custom_kind = form.elements.kind_custom.value.trim();
  const tgt = pickerValue($(".picker", form));
  if (tgt && tgt !== other.id && !tgt.startsWith("new-")) after[e.src === other.id ? "src" : "dst"] = tgt;
  return decide({ target: "edge", id: e.id, action: "edit", after, reason: form.elements.reason.value }, "Changed");
}

// Delegated actions for edge rows (all pages).
document.addEventListener("click", async (ev) => {
  const btn = ev.target.closest("[data-act]");
  if (!btn) return;
  const row = btn.closest(".er");
  const act = btn.dataset.act;
  if (row && ["confirm", "reject", "edit", "delete", "cancel"].includes(act)) {
    const id = row.dataset.eid;
    if (act === "confirm") return decide({ target: "edge", id, action: "confirm" }, "Confirmed");
    if (act === "reject") return decide({ target: "edge", id, action: "reject" }, "Rejected");
    if (act === "delete") return decide({ target: "edge", id, action: "delete" }, "Deleted");
    if (act === "cancel") { $(".editslot", row).innerHTML = ""; return; }
    if (act === "edit") {
      const slot = $(".editslot", row);
      if (slot.innerHTML) { slot.innerHTML = ""; return; }
      slot.innerHTML = editBoxHtml(EDGES.get(id));
      $("select", slot).focus();
    }
    return;
  }
  if (act === "undo") return undo(Number(btn.dataset.index));
});
document.addEventListener("submit", async (ev) => {
  const f = ev.target;
  if (f.classList.contains("editbox") && f.dataset.eid && EDGES.has(f.dataset.eid)) { ev.preventDefault(); await submitEdit(f); }
});

// ---------------------------------------------------------------- router
function route() {
  const h = location.hash.replace(/^#/, "") || "/";
  const [path, qs] = h.split("?");
  const params = new URLSearchParams(qs || "");
  const parts = path.split("/").filter(Boolean);
  return { name: parts[0] || "jokes", arg: parts.length > 1 ? decodeURIComponent(parts.slice(1).join("/")) : null, params };
}
let rendering = 0;
async function render({ keepScroll = false } = {}) {
  const r = route();
  const y = window.scrollY;
  const my = ++rendering;
  $$("#nav a").forEach((a) => a.classList.toggle("on", a.dataset.r === (r.name === "joke" || r.name === "graph" ? "jokes" : r.name)));
  try {
    const page = { jokes: pageJokes, joke: pageJoke, graph: pageGraph, labels: pageLabels, review: pageReview, learned: pageLearned }[r.name] || pageJokes;
    await page(r, () => my === rendering);
    if (keepScroll) window.scrollTo(0, y);
  } catch (e) {
    main().innerHTML = `<div class="panel"><p class="err">Could not load: ${esc(e.message)}</p></div>`;
  }
}
async function refresh() { await loadMeta(); await render({ keepScroll: true }); }

// ---------------------------------------------------------------- 1. joke list
const JF = store.get("jokeFilters", { q: "", stage: "", label: "", sort: "review" });
let JLIMIT = 200;
async function pageJokes(r, alive) {
  if (!$("#jokeList")) {
    main().innerHTML = `${graphBanner()}<div class="panel">
      <div class="filters">
        <input type="search" id="jq" placeholder="Search words, titles, labels" aria-label="Search jokes" value="${esc(JF.q)}">
        <select id="jstage" aria-label="Stage"><option value="">All stages</option>${META.stages.map((s) => `<option ${s === JF.stage ? "selected" : ""}>${s}</option>`).join("")}</select>
        <select id="jlabel" aria-label="Label"><option value="">All labels</option>${META.labels.slice().sort((a, b) => a.name.localeCompare(b.name)).map((l) => `<option value="${esc(l.id)}" ${l.id === JF.label ? "selected" : ""}>${esc(l.name)} (${esc(l.kind)})</option>`).join("")}</select>
        <select id="jsort" aria-label="Sort"><option value="review">Most to review</option><option value="confirmed">Most confirmed</option><option value="recent">Recently seen</option><option value="title">Title A–Z</option></select>
      </div>
      <div class="row small muted" style="margin-top:8px"><span id="jcount" class="grow"></span><span><span class="c-prop">●</span> to review <span class="c-conf">●</span> confirmed <span class="c-rej">●</span> rejected</span></div>
      <ul class="jl" id="jokeList"></ul><div id="jmore"></div></div>`;
    $("#jsort").value = JF.sort;
    let t;
    const upd = () => { JF.q = $("#jq").value; JF.stage = $("#jstage").value; JF.label = $("#jlabel").value; JF.sort = $("#jsort").value; store.set("jokeFilters", JF); JLIMIT = 200; fillJokes(); };
    $("#jq").addEventListener("input", () => { clearTimeout(t); t = setTimeout(upd, 120); });
    ["jstage", "jlabel", "jsort"].forEach((id) => $("#" + id).addEventListener("change", upd));
  }
  await fillJokes(alive);
}
async function fillJokes(alive = () => true) {
  const qs = new URLSearchParams();
  if (JF.q) qs.set("q", JF.q);
  if (JF.stage) qs.set("stage", JF.stage);
  if (JF.label) qs.set("label", JF.label);
  const d = await api("/api/jokes?" + qs);
  if (!alive() || !$("#jokeList")) return;
  const sorters = {
    review: (a, b) => b.counts.proposed - a.counts.proposed || a.title.localeCompare(b.title),
    confirmed: (a, b) => b.counts.confirmed - a.counts.confirmed || a.title.localeCompare(b.title),
    recent: (a, b) => (b.last_seen || "").localeCompare(a.last_seen || ""),
    title: (a, b) => a.title.localeCompare(b.title),
  };
  const items = d.jokes.sort(sorters[JF.sort] || sorters.review);
  $("#jcount").textContent = `${items.length} of ${d.total} jokes`;
  $("#jokeList").innerHTML = items.slice(0, JLIMIT).map((j) => `<li><a class="j" href="#/joke/${enc(j.id)}" data-id="${esc(j.id)}">
      <div class="row"><span class="t grow">${esc(j.title)}</span><span class="pill stage">${esc(j.stage || "?")}</span>${j.yours ? '<span class="pill yours">yours</span>' : ""}</div>
      <div class="row small"><span class="counts"><span class="c-prop">● <b>${j.counts.proposed}</b> to review</span><span class="c-conf">● <b>${j.counts.confirmed}</b></span><span class="c-rej">● <b>${j.counts.rejected}</b></span></span>
        <span class="muted tiny">${j.units} units${j.last_seen ? " · last " + esc(j.last_seen) : ""}</span></div>
      <div class="chips">${j.labels.slice(0, 8).map((l) => `<span class="chip st-${l.bucket}">${esc(l.name)}</span>`).join("")}</div></a></li>`).join("") || `<li class="muted" style="padding:10px 4px">No jokes match.</li>`;
  $("#jmore").innerHTML = items.length > JLIMIT ? `<button id="jmoreBtn">Show ${Math.min(200, items.length - JLIMIT)} more</button>` : "";
  if ($("#jmoreBtn")) $("#jmoreBtn").onclick = () => { JLIMIT += 200; fillJokes(); };
}

// ---------------------------------------------------------------- 2. joke page
let SHOW_REJ = store.get("showRejected", false);
async function pageJoke(r, alive) {
  const d = await api("/api/joke/" + enc(r.arg));
  if (!alive()) return;
  EDGES.clear();
  const all = [...d.edges, ...d.unit_groups.flatMap((g) => g.units.map((u) => u.edge)), ...d.performances.map((p) => p.edge)];
  all.forEach((e) => EDGES.set(e.id, e));
  const nrej = all.filter((e) => e.bucket === "rejected").length;
  const vis = (e) => SHOW_REJ || e.bucket !== "rejected";
  const j = d.joke;
  const labels = d.edges.filter((e) => e.kind === "has_label");
  const conns = d.edges.filter((e) => e.kind !== "has_label");
  const c = { proposed: 0, confirmed: 0, rejected: 0 };
  all.forEach((e) => c[e.bucket]++);
  const unitsHtml = d.unit_groups.map((g) => {
    const us = g.units.filter((u) => vis(u.edge));
    if (!us.length) return "";
    return `<div class="date-h">${esc(g.date)}</div>` + us.map((u) => {
      const url = docUrl(u);
      EDGES.set(u.edge.id, u.edge);
      return `<div class="unit st-${u.edge.bucket} er" data-eid="${esc(u.edge.id)}" style="border-left-width:4px">
        <div class="row tiny"><span class="stword">${STWORD[u.edge.bucket]}</span><span class="pill">${esc(u.kind || "?")}</span>
          ${url ? `<a href="${url}" target="_blank" rel="noopener noreferrer">${esc(u.doc_title || u.doc_id)}</a>` : `<span class="muted">${esc(u.doc_title || u.source || "")}</span>`}
          <span class="muted grow">conf ${pct(u.edge.confidence)}</span></div>
        <div class="txt">${esc(u.text)}</div>
        ${u.edge.bucket !== "confirmed" && u.edge.reason ? `<div class="tiny muted">${esc(u.edge.reason)}</div>` : ""}
        <div class="acts"><button class="mini yes" data-act="confirm" ${u.edge.bucket === "confirmed" ? "disabled" : ""}>✓ Part of this joke</button>
          <button class="mini no" data-act="reject" ${u.edge.bucket === "rejected" ? "disabled" : ""}>✕ Not this joke</button></div><div class="editslot"></div></div>`;
    }).join("");
  }).join("") || `<p class="muted small">No units.</p>`;
  const perfHtml = d.performances.filter((p) => vis(p.edge)).map((p) => {
    const L = p.laughs;
    const lh = L ? `<div class="laughs"><span>Laughs (${esc(L.detector || "auto")}): <b>${L.hits}</b> · best ${barsLabel(L.max_bars)}</span></div>
        <div class="laughs">${L.laughs.map((l) => `<span title="${esc(l.kind)} · strength ${l.strength}">${fmtMs(l.start_ms)} ${barsLabel(l.bars ?? 0)}</span>`).join("")}</div>`
      : `<div class="tiny muted">No laugh file for this set yet.</div>`;
    return `<div class="perf st-${p.edge.bucket} er" data-eid="${esc(p.edge.id)}" style="border-left-width:4px">
      <div class="row small"><span class="stword">${STWORD[p.edge.bucket]}</span><b class="grow">${esc(p.set.title || p.set.id)}</b><span class="muted">${esc(p.set.date || "")} · ${esc(p.set.venue || "")}</span></div>
      <div class="tiny muted">${fmtMs(p.perf.start_ms)}–${fmtMs(p.perf.end_ms)} · match conf ${pct(p.edge.confidence)}</div>
      <blockquote class="ev">“${esc(p.perf.text)}”</blockquote>${lh}
      <div class="acts"><button class="mini yes" data-act="confirm" ${p.edge.bucket === "confirmed" ? "disabled" : ""}>✓ That's this joke</button>
        <button class="mini no" data-act="reject" ${p.edge.bucket === "rejected" ? "disabled" : ""}>✕ Not this joke</button></div><div class="editslot"></div></div>`;
  }).join("") || `<p class="muted small">No performances found yet.</p>`;
  const hist = d.history.slice().reverse().map((h) => `<li class="small">${esc((h.ts || "").replace("T", " ").slice(0, 16))} · <b>${esc(h.action)}</b> ${esc(h.target)} <span class="muted">${esc(h.id)}</span>${h.reason ? ` · “${esc(h.reason)}”` : ""}
      ${h.undone ? '<span class="pill">undone</span>' : h.undo_of != null ? '<span class="pill">undo</span>' : `<button class="mini ghost" data-act="undo" data-index="${h.index}">Undo</button>`}</li>`).join("");

  main().innerHTML = `${graphBanner()}
  <div class="panel">
    <div class="row"><h1 class="grow">${esc(j.title || j.id)}</h1>${d.yours ? '<span class="pill yours">yours</span>' : ""}</div>
    ${j.summary ? `<p style="margin:0 0 6px">${esc(j.summary)}</p>` : ""}
    <div class="row small muted"><span>${esc(j.first_seen || "?")} → ${esc(j.last_seen || "?")}</span>
      <span class="counts"><span class="c-prop">● <b>${c.proposed}</b> to review</span><span class="c-conf">● <b>${c.confirmed}</b> confirmed</span><span class="c-rej">● <b>${c.rejected}</b> rejected</span></span></div>
    <div class="row" style="margin-top:8px">
      <label class="small muted" for="stageSel">Stage</label>
      <select id="stageSel">${META.stages.map((s) => `<option ${s === j.stage ? "selected" : ""}>${s}</option>`).join("")}</select>
      <a href="#/graph/${enc(j.id)}"><button type="button">Graph view</button></a>
      <label class="chk"><input type="checkbox" id="showRej" ${SHOW_REJ ? "checked" : ""}> Show rejected (${nrej})</label>
    </div>
  </div>
  <div class="panel"><h2>Labels</h2>
    ${labels.filter(vis).map((e) => edgeRow(e)).join("") || '<p class="muted small">No labels yet.</p>'}
    <form id="addLabel" class="editbox"><b>Add label</b>
      ${pickerHtml("label", { types: ["label"], newLabel: true, placeholder: "Pick a label or type a new one" })}
      <div id="newLabelBits" hidden class="row"><select name="lkind" aria-label="Label kind">${META.label_kinds.map((k) => `<option ${k === "custom" ? "selected" : ""}>${k}</option>`).join("")}</select>
        <input type="text" name="ldef" placeholder="Definition (optional)" class="grow"></div>
      <textarea name="reason" placeholder="Why? (optional)"></textarea>
      <div><button class="primary" type="submit">Add label</button></div></form>
  </div>
  <div class="panel"><h2>Connections</h2>
    ${conns.filter(vis).map((e) => edgeRow(e)).join("") || '<p class="muted small">No connections yet.</p>'}
    <form id="addConn" class="editbox"><b>Add connection</b>
      ${pickerHtml("target", { types: ["joke", "label"], newIdea: true, placeholder: "Another joke, a label, or a new idea" })}
      <div class="row">${kindSelect("kind", "same_shape")}</div>
      <textarea name="reason" placeholder="Why are these connected? (required)" required></textarea>
      <div><button class="primary" type="submit">Add connection</button></div></form>
  </div>
  <div class="panel"><h2>Units</h2><p class="tiny muted" style="margin:0">Verbatim, oldest first. Doc titles open the Google Doc.</p>${unitsHtml}</div>
  <div class="panel"><h2>Performances</h2>${perfHtml}</div>
  ${hist ? `<div class="panel"><h2>Your changes here</h2><ul style="padding-left:18px;margin:0">${hist}</ul></div>` : ""}`;

  $("#stageSel").onchange = (ev) => decide({ target: "node", id: j.id, action: "edit", after: { stage: ev.target.value } }, `Stage → ${ev.target.value}`);
  $("#showRej").onchange = (ev) => { SHOW_REJ = ev.target.checked; store.set("showRejected", SHOW_REJ); render({ keepScroll: true }); };
  const al = $("#addLabel");
  al.addEventListener("picked", () => { $("#newLabelBits").hidden = !pickerValue($(".picker", al)).startsWith("new-label:"); });
  al.onsubmit = async (ev) => {
    ev.preventDefault();
    let v = pickerValue($(".picker", al));
    if (!v) { const typed = $(".picker input", al).value.trim(); if (typed) v = "new-label:" + typed; }
    if (!v) return snack("Pick a label first", null, true);
    if (v.startsWith("new-label:")) {
      const made = await api("/api/decision", { method: "POST", body: { target: "node", action: "add", after: { type: "label", kind: al.elements.lkind.value, name: v.slice(10), definition: al.elements.ldef.value }, reason: al.elements.reason.value } }).catch((e) => { snack("Not saved: " + e.message, null, true); return null; });
      if (!made) return;
      v = made.decision.id;
      await loadMeta();
    }
    await decide({ target: "edge", action: "add", after: { src: j.id, dst: v, kind: "has_label" }, reason: al.elements.reason.value.trim() || "Added by hand" }, "Label added");
  };
  const ac = $("#addConn");
  ac.onsubmit = async (ev) => {
    ev.preventDefault();
    let v = pickerValue($(".picker", ac));
    if (!v) return snack("Pick the other end first", null, true);
    const reason = ac.elements.reason.value.trim();
    if (!reason) return snack("Say why they're connected", null, true);
    if (v.startsWith("new-idea:")) {
      const made = await api("/api/decision", { method: "POST", body: { target: "node", action: "add", after: { type: "joke", title: v.slice(9), stage: "idea" }, reason } }).catch((e) => { snack("Not saved: " + e.message, null, true); return null; });
      if (!made) return;
      v = made.decision.id;
    }
    const kind = ac.elements.kind.value;
    const after = { src: j.id, dst: v, kind };
    if (kind === "custom") after.custom_kind = ac.elements.kind_custom.value.trim();
    await decide({ target: "edge", action: "add", after, reason }, "Connection added");
  };
}

// ---------------------------------------------------------------- 3. graph
const GF = store.get("graphFilters", { hops: 1, kinds: [], all: false, rejected: false });
async function pageGraph(r, alive) {
  const id = r.arg;
  const kinds = [...META.edge_kinds, ...(META.custom_kinds || [])];
  const qs = new URLSearchParams({ id, hops: GF.hops, kinds: GF.kinds.join(","), all: GF.all ? 1 : 0, rejected: GF.rejected ? 1 : 0 });
  const g = await api("/api/graph?" + qs);
  if (!alive()) return;
  const c = g.nodes.find((n) => n.id === id) || { title: id };
  main().innerHTML = `${graphBanner()}<div class="panel">
    <div class="row"><h1 class="grow">${esc(c.title)}</h1>${c.type === "joke" ? `<a href="#/joke/${enc(id)}"><button>Open joke</button></a>` : ""}</div>
    <div class="row small">
      <span class="muted">Hops</span><span class="row" role="group"><button class="mini ${GF.hops == 1 ? "primary" : ""}" data-h="1">1</button><button class="mini ${GF.hops == 2 ? "primary" : ""}" data-h="2">2</button></span>
      <label class="chk"><input type="checkbox" id="gAll" ${GF.all ? "checked" : ""}> units &amp; performances</label>
      <label class="chk"><input type="checkbox" id="gRej" ${GF.rejected ? "checked" : ""}> rejected</label>
    </div>
    <details ${GF.kinds.length ? "open" : ""}><summary class="small muted">Edge kinds ${GF.kinds.length ? "(" + GF.kinds.length + " on)" : "(all)"}</summary>
      <div class="row">${kinds.map((k) => `<label class="chk"><input type="checkbox" class="gk" value="${esc(k)}" ${GF.kinds.includes(k) ? "checked" : ""}> ${esc(k)}</label>`).join("")}</div></details>
  </div>
  <div class="gwrap"><svg id="gsvg" role="img" aria-label="Graph of ${esc(c.title)} and its neighbours"></svg></div>
  <div class="legend"><span><i style="background:#5fd08a"></i>confirmed</span><span><i style="background:#6aa8ff"></i>proposed (dashed)</span><span><i style="background:#ff6b6b"></i>rejected (dotted)</span>
    <span>● joke</span><span style="color:#c59cff">◆ label</span><span>${g.nodes.length} nodes · ${g.edges.length} links${g.truncated ? " · capped" : ""}</span></div>
  <p class="tiny muted">Tap a joke to open its graph; tap again on the centre to open the joke page. Tap a label to open it in Labels.</p>`;
  DeskGraph.render($("#gsvg"), g, (n) => {
    if (n.type === "joke") location.hash = n.id === id ? "#/joke/" + enc(n.id) : "#/graph/" + enc(n.id);
    else if (n.type === "label") location.hash = "#/labels?focus=" + enc(n.id);
    else if (n.type === "unit" && n.joke) location.hash = "#/joke/" + enc(n.joke);
  });
  const save = () => { store.set("graphFilters", GF); render(); };
  $$("[data-h]").forEach((b) => (b.onclick = () => { GF.hops = Number(b.dataset.h); save(); }));
  $("#gAll").onchange = (e) => { GF.all = e.target.checked; save(); };
  $("#gRej").onchange = (e) => { GF.rejected = e.target.checked; save(); };
  $$(".gk").forEach((b) => (b.onchange = () => { GF.kinds = $$(".gk").filter((x) => x.checked).map((x) => x.value); save(); }));
}

// ---------------------------------------------------------------- 4. labels
let LQ = "";
async function pageLabels(r, alive) {
  const d = await api("/api/labels");
  if (!alive()) return;
  const focus = r.params.get("focus");
  const mergedName = Object.fromEntries(d.merged.map((m) => [m.id, m.name]));
  const byKind = {};
  for (const l of d.labels) (byKind[l.kind] = byKind[l.kind] || []).push(l);
  const card = (l) => `<div class="labelcard ${l.id === focus ? "focus" : ""}" id="L-${esc(l.id)}" data-lid="${esc(l.id)}" data-search="${esc((l.name + " " + l.kind + " " + (l.definition || "")).toLowerCase())}">
      <div class="row"><span class="lname grow">${esc(l.name)}</span>${l.yours ? '<span class="pill yours">yours</span>' : ""}
        <span class="counts small"><span class="c-conf">✓ <b>${l.counts.confirmed}</b></span><span class="c-prop">? <b>${l.counts.proposed}</b></span><span class="c-rej">✕ <b>${l.counts.rejected}</b></span></span></div>
      <div class="small ${l.definition ? "" : "muted"}">${esc(l.definition || "No definition yet.")}</div>
      ${l.merged_from.length ? `<div class="tiny muted">Includes merged: ${l.merged_from.map((m) => `${esc(mergedName[m] || m)} <button class="mini ghost" data-act="unmerge" data-id="${esc(m)}">Unmerge</button>`).join(" ")}</div>` : ""}
      <div class="chips">${l.carriers.filter((c) => SHOW_REJ || c.bucket !== "rejected").map((c) => `<a class="chip st-${c.bucket}" href="${nodeHref(c.joke) || "#/"}" title="${c.bucket} · conf ${pct(c.confidence)}${c.via_unit ? " · via a unit" : ""}">${esc(c.joke.title)}${c.via_unit ? " ·u" : ""}</a>`).join("") || '<span class="tiny muted">No jokes carry it yet.</span>'}</div>
      <div class="acts row"><button class="mini" data-act="rename">Rename</button><button class="mini" data-act="merge">Merge into…</button></div>
      <div class="lslot"></div></div>`;
  main().innerHTML = `${graphBanner()}<div class="panel"><div class="row"><h1 class="grow">Labels</h1>
      <label class="chk"><input type="checkbox" id="lRej" ${SHOW_REJ ? "checked" : ""}> show rejected</label></div>
      <input type="search" id="lq" placeholder="Filter labels" aria-label="Filter labels" value="${esc(LQ)}" style="width:100%">
      <p class="tiny muted">Chip colour: <span class="c-conf">green confirmed</span>, <span class="c-prop">blue proposed</span>, <span class="c-rej">red rejected</span>. “·u” means it was put on one of the joke's units.</p></div>
    ${Object.entries(byKind).map(([k, ls]) => `<div class="panel lkind"><h3>${esc(k)} (${ls.length})</h3>${ls.map(card).join("")}</div>`).join("")}`;
  const filt = () => {
    const q = $("#lq").value.toLowerCase().trim();
    LQ = $("#lq").value;
    $$(".labelcard").forEach((c) => (c.hidden = q && !c.dataset.search.includes(q)));
    $$(".lkind").forEach((p) => (p.hidden = !$$(".labelcard", p).some((c) => !c.hidden)));
  };
  $("#lq").oninput = filt;
  filt();
  $("#lRej").onchange = (e) => { SHOW_REJ = e.target.checked; store.set("showRejected", SHOW_REJ); render({ keepScroll: true }); };
  if (focus && document.getElementById("L-" + focus)) document.getElementById("L-" + focus).scrollIntoView({ block: "center" });
  main().onclick = async (ev) => {
    const b = ev.target.closest("[data-act]");
    if (!b || !b.closest(".labelcard")) return;
    const cardEl = b.closest(".labelcard");
    const l = d.labels.find((x) => x.id === cardEl.dataset.lid);
    const slot = $(".lslot", cardEl);
    if (b.dataset.act === "unmerge") return decide({ target: "node", id: b.dataset.id, action: "edit", after: { merged_into: null } }, "Unmerged");
    if (b.dataset.act === "rename") {
      slot.innerHTML = `<form class="editbox" data-f="rename"><input type="text" name="name" value="${esc(l.name)}" aria-label="Label name">
        <textarea name="definition" placeholder="Definition">${esc(l.definition || "")}</textarea><textarea name="reason" placeholder="Why? (optional)"></textarea>
        <div class="row"><button class="primary" type="submit">Save</button><button type="button" data-act="lcancel">Cancel</button></div></form>`;
    } else if (b.dataset.act === "merge") {
      slot.innerHTML = `<form class="editbox" data-f="merge"><div class="small muted">Every joke carrying “${esc(l.name)}” will carry the label you pick instead. Recorded as one decision; undo or unmerge any time.</div>
        ${pickerHtml("into", { types: ["label"], placeholder: "Merge into which label?" })}<textarea name="reason" placeholder="Why? (optional)"></textarea>
        <div class="row"><button class="primary" type="submit">Merge</button><button type="button" data-act="lcancel">Cancel</button></div></form>`;
    } else if (b.dataset.act === "lcancel") slot.innerHTML = "";
    const f = $("form", slot);
    if (f) f.onsubmit = async (ev2) => {
      ev2.preventDefault();
      if (f.dataset.f === "rename") {
        const after = {};
        if (f.elements.name.value.trim() !== l.name) after.name = f.elements.name.value.trim();
        if (f.elements.definition.value.trim() !== (l.definition || "")) after.definition = f.elements.definition.value.trim();
        if (!Object.keys(after).length) return snack("Nothing changed", null, true);
        await decide({ target: "node", id: l.id, action: "edit", after, reason: f.elements.reason.value }, "Label renamed");
      } else {
        const into = pickerValue($(".picker", f));
        if (!into) return snack("Pick a label to merge into", null, true);
        await decide({ target: "node", id: l.id, action: "edit", after: { merged_into: into }, reason: f.elements.reason.value }, "Labels merged");
      }
    };
  };
}

// ---------------------------------------------------------------- 5. review
let RVMODE = "proposed";
let RVCUR = null;
async function pageReview(r, alive) {
  const d = await api(`/api/queue?status=${RVMODE}&limit=400`);
  if (!alive()) return;
  EDGES.clear();
  d.items.forEach((e) => EDGES.set(e.id, e));
  const seg = `<div class="row"><h1 class="grow">${RVMODE === "proposed" ? "Review" : "Rejected"}</h1>
    <span class="row" role="group"><button class="mini ${RVMODE === "proposed" ? "primary" : ""}" data-mode="proposed">To review (${RVMODE === "proposed" ? d.total : QTOTAL})</button>
    <button class="mini ${RVMODE === "rejected" ? "primary" : ""}" data-mode="rejected">Rejected</button></span></div>`;
  if (RVMODE === "rejected") {
    RVCUR = null;
    main().innerHTML = `${graphBanner()}<div class="panel">${seg}<p class="small muted">Rejected items are out of the queue and out of the jokes' views, but kept here. Confirm one to bring it back.</p>
      ${d.items.map((e) => edgeRow({ ...e, other: e.dst_brief, via_unit: null, display_kind: e.display_kind }) .replace('<span class="other grow">', `<span class="other grow">${nodeLink(e.src_brief)} → `)).join("") || '<p class="muted">Nothing rejected.</p>'}</div>`;
  } else {
    const items = d.items.filter((e) => !SKIPPED.has(e.id));
    const e = items[0];
    RVCUR = e || null;
    if (!e) {
      main().innerHTML = `${graphBanner()}<div class="panel">${seg}<p>${d.total ? `All ${d.total} remaining items are skipped this session. <button id="unskip">Bring skipped back</button>` : "Nothing left to review. 🎤"}</p></div>`;
      if ($("#unskip")) $("#unskip").onclick = () => { SKIPPED.clear(); render(); };
    } else {
      const end = (b, ctx) => `<div class="rv-end"><div class="tiny muted">${esc(b.type)}${b.stage ? " · " + esc(b.stage) : ""}${b.kind ? " · " + esc(b.kind) : ""}</div>
        <div><b>${nodeLink(b)}</b></div>${(ctx || []).filter(Boolean).map((t) => `<div class="txt">${esc(t)}</div>`).join("")}</div>`;
      main().innerHTML = `${graphBanner()}<div class="panel rv-card">${seg}
        <div class="small muted"><b id="rvLeft">${items.length}${d.total > d.items.length ? "+" : ""}</b> left${SKIPPED.size ? ` · ${SKIPPED.size} skipped` : ""} · ${esc(e.why)}</div>
        <div class="er st-proposed" data-eid="${esc(e.id)}" style="border-left-width:4px">
          <div class="rv-ends">${end(e.src_brief, e.context.src)}${end(e.dst_brief, e.context.dst)}</div>
          <div class="rv-kind">${esc(e.src_brief.title)} — <b>${esc(e.display_kind)}</b> → ${esc(e.dst_brief.title)}</div>
          <div class="reason" style="margin-top:8px">${esc(e.reason || "")} <span class="conf">conf ${pct(e.confidence)}</span></div>
          ${(e.evidence || []).map((x) => `<blockquote class="ev">“${esc(x.quote)}” <span class="src">${esc(x.node || "")}</span></blockquote>`).join("")}
          <textarea id="rvReason" placeholder="Why? (optional, your words)" style="margin-top:8px"></textarea>
          <div class="rv-btns" style="margin-top:8px">
            <button class="big yes" id="rvY">✓ Yes<span class="kbd">Y</span></button>
            <button class="big no" id="rvN">✕ No<span class="kbd">N</span></button>
            <button class="big" id="rvE">✎ Edit<span class="kbd">E</span></button>
            <button class="big ghost" id="rvS">Skip<span class="kbd">S</span></button>
          </div><div class="editslot"></div>
          <div class="tiny muted" style="margin-top:6px"><span class="kbd">U</span> undo last</div></div></div>`;
      const reason = () => $("#rvReason").value;
      $("#rvY").onclick = () => decide({ target: "edge", id: e.id, action: "confirm", reason: reason() }, "Confirmed");
      $("#rvN").onclick = () => decide({ target: "edge", id: e.id, action: "reject", reason: reason() }, "Rejected");
      $("#rvS").onclick = () => { SKIPPED.add(e.id); render(); };
      $("#rvE").onclick = () => {
        const slot = $(".editslot", $(".er"));
        slot.innerHTML = slot.innerHTML ? "" : editBoxHtml({ ...e, other: e.dst_brief });
        if (slot.innerHTML) { slot.querySelector("[name=reason]").value = reason(); $("select", slot).focus(); }
      };
    }
  }
  $$("[data-mode]").forEach((b) => (b.onclick = () => { RVMODE = b.dataset.mode; render(); }));
}
document.addEventListener("keydown", (ev) => {
  if (route().name !== "review" || ev.ctrlKey || ev.metaKey || ev.altKey) return;
  const t = ev.target;
  if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT")) return;
  const k = ev.key.toLowerCase();
  if (k === "u") { ev.preventDefault(); return undo(null); }
  if (!RVCUR) return;
  const id = { y: "rvY", n: "rvN", e: "rvE", s: "rvS" }[k];
  if (id && $("#" + id)) { ev.preventDefault(); $("#" + id).click(); }
});

// ---------------------------------------------------------------- 6. learned
async function pageLearned(r, alive) {
  const [s, dl] = await Promise.all([api("/api/stats"), api("/api/decisions?limit=60")]);
  if (!alive()) return;
  const rateRow = (name, x) => `<tr><td>${esc(name)}</td><td class="c-conf">${x.confirm}</td><td class="c-rej">${x.reject}</td><td>${x.edit}</td>
    <td>${x.confirm_rate == null ? '<span class="muted">–</span>' : `<div class="rate" title="${Math.round(x.confirm_rate * 100)}% confirmed"><i style="width:${x.confirm_rate * 100}%"></i></div><span class="tiny">${Math.round(x.confirm_rate * 100)}% kept</span>`}</td></tr>`;
  const table = (rows) => rows.length ? `<div class="tblwrap"><table><tr><th>What</th><th>✓</th><th>✕</th><th>✎</th><th>Kept</th></tr>${rows.join("")}</table></div>` : '<p class="muted small">Nothing yet.</p>';
  const kinds = Object.entries(s.by_kind).sort((a, b) => (b[1].confirm + b[1].reject) - (a[1].confirm + a[1].reject)).map(([k, x]) => rateRow(k, x));
  const labs = Object.entries(s.by_label).sort((a, b) => (b[1].confirm + b[1].reject) - (a[1].confirm + a[1].reject)).map(([, x]) => rateRow(x.name, x));
  const undoBtn = (i) => `<button class="mini ghost" data-act="undo" data-index="${i}">Undo</button>`;
  main().innerHTML = `${graphBanner()}<div class="panel"><h1>What I've learned</h1>
    <p class="small muted" style="margin:0">From your decisions (undone ones don't count). Pipelines read the same file, so this is what the next run learns from.</p></div>
    <div class="stat-cards">
      <div class="panel"><div class="big-num">${s.effective}</div><div class="small muted">decisions that stand</div></div>
      <div class="panel"><div class="big-num c-conf">${s.edge_status.confirmed}</div><div class="small muted">links confirmed</div></div>
      <div class="panel"><div class="big-num c-prop">${s.edge_status.proposed}</div><div class="small muted">still to review</div></div>
      <div class="panel"><div class="big-num c-rej">${s.edge_status.rejected}</div><div class="small muted">rejected</div></div></div>
    <div class="panel"><h2>By kind of connection</h2>${table(kinds)}</div>
    <div class="panel"><h2>By label</h2>${table(labs)}</div>
    <div class="panel"><h2>Your labels</h2>${s.added_labels.map((l) => `<div class="labelcard"><span class="lname">${esc(l.name)}</span> <span class="pill">${esc(l.kind)}</span> ${undoBtn(l.index)}<div class="small muted">${esc(l.definition || "")}</div></div>`).join("") || '<p class="muted small">None yet.</p>'}</div>
    <div class="panel"><h2>Your connections</h2>${s.added_edges.map((e) => `<div class="labelcard small">${nodeLink(e.src)} — <b>${esc(e.kind)}</b> → ${nodeLink(e.dst)} ${undoBtn(e.index)}<div class="muted">“${esc(e.reason || "")}”</div></div>`).join("") || '<p class="muted small">None yet.</p>'}</div>
    ${s.added_jokes.length ? `<div class="panel"><h2>Your new ideas</h2>${s.added_jokes.map((j) => `<div class="labelcard small"><a href="#/joke/${enc(j.id)}">${esc(j.name)}</a> ${undoBtn(j.index)}</div>`).join("")}</div>` : ""}
    <div class="panel"><h2>Recent decisions</h2><div class="tblwrap"><table><tr><th>When</th><th>Action</th><th>What</th><th></th></tr>
      ${dl.decisions.map((x) => `<tr><td class="tiny">${esc((x.ts || "").replace("T", " ").slice(5, 16))}</td><td><b>${esc(x.action)}</b> <span class="tiny muted">${esc(x.target)}</span></td>
        <td class="small">${esc(x.title || x.id)}${x.after && x.target === "node" && x.action === "edit" ? ` <span class="tiny muted">${esc(JSON.stringify(x.after))}</span>` : ""}${x.reason ? `<div class="tiny muted">“${esc(x.reason)}”</div>` : ""}</td>
        <td>${x.undone ? '<span class="pill">undone</span>' : x.undo_of != null ? '<span class="pill">undo</span>' : undoBtn(x.index)}</td></tr>`).join("")}</table></div>
      <p class="tiny muted">${s.total_decisions} lines in decisions.jsonl.</p></div>`;
}

// ---------------------------------------------------------------- boot
window.addEventListener("hashchange", () => { window.scrollTo(0, 0); render(); });
(async () => {
  try {
    await loadMeta();
  } catch (e) {
    main().innerHTML = `<div class="panel err">Server not reachable: ${esc(e.message)}</div>`;
    return;
  }
  await render();
  setInterval(async () => {
    if (META && META.graph_state !== "ok") {
      const h = await api("/api/health").catch(() => null);
      if (h && h.graph_state === "ok") refresh();
    }
  }, 10000);
})();
