// Punchable brand assets: the three logo concepts as SVG, the options page, and the PNG set for the chosen mark.
//   node scripts/brand/brand.mjs            SVGs + docs/brand/logo-options.html (no browser needed)
//   node scripts/brand/brand.mjs --png      also renders every PNG with headless Chrome (CDP, no deps) + favicon.ico
//                                           with ImageMagick (`magick`), which is already on the PC.
// The geometry lives here, on a 512 grid. The wordmark is Space Grotesk Bold outlined by outline.py (glyphs.json),
// so no SVG depends on a font. Docs: docs/design/DESIGN.md §7 (logo).
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import net from "node:net";

const HERE = dirname(fileURLToPath(import.meta.url));
const APP = resolve(HERE, "..", "..");
const REPO = resolve(APP, "..");
const BRAND = join(REPO, "docs", "brand");
const G = JSON.parse(readFileSync(join(HERE, "glyphs.json"), "utf8"));

// ---- tokens (app/src/lib/theme.ts; light values from docs/design/DESIGN.md §3.2)
const T = { bg: "#111214", surface: "#1a1c20", text: "#f3f4f6", mut: "#9ca3af", amber: "#f5b544", lightBg: "#f6f6f4", ink: "#111214", amberMark: "#b47a0e" };

// ---- small path helpers
const n = (v) => +v.toFixed(2);
const pill = (x, y, w, h) => { const r = h / 2; return `M${n(x + r)} ${n(y)}h${n(w - h)}a${r} ${r} 0 0 1 0 ${h}h${n(h - w)}a${r} ${r} 0 0 1 0 ${-h}z`; };
const bar = (x, y, w, h, r) => `M${x} ${y + r}a${r} ${r} 0 0 1 ${r} ${-r}h${w - 2 * r}a${r} ${r} 0 0 1 ${r} ${r}V${y + h}H${x}z`; // round top, square foot

// =====================================================================================================================
// Concept A, "Mic P": a stage mic drawn as a P. The grille ball is the bowl, the handle is the stem, the grille slots are
// the counter. One compound path (even-odd), so it is a single colour everywhere and mono for free.
//   ball: circle (256,194) r108; handle: x 148..224, round foot at y 426. Bounding box x 148..364, y 86..426.
const MIC_BODY = "M148 194A108 108 0 1 1 224 297.15V388A38 38 0 0 1 148 388Z";
const micFull = MIC_BODY + pill(232, 139, 92, 20) + pill(232, 184, 104, 20) + pill(232, 229, 92, 20);
// 16-48 px: one fat slot (the P's counter), mark scaled up 1.16 about the centre so it fills the tile.
const micSmall = MIC_BODY + pill(232, 177, 100, 34);
const A = {
  id: "a-mic-p", name: "Mic P", idea: "A stage mic drawn as a P. The grille is the bowl, the handle is the stem.",
  bbox: [148, 86, 364, 426],
  mark: (fill) => `<path fill="${fill}" fill-rule="evenodd" d="${micFull}"/>`,
  small: (fill) => `<path fill="${fill}" fill-rule="evenodd" transform="translate(256 256) scale(1.16) translate(-256 -256)" d="${micSmall}"/>`,
};

// Concept B, "Punch line": the wordmark, with the punch line drawn under "punch". Small: a lowercase p over the line.
const pB = G.p.bounds; // [x0,y0,x1,y1] at size 100
const PS = 3.3, pW = (pB[2] - pB[0]) * PS, pH = (pB[3] - pB[1]) * PS, LINE = 40, GAP = 30, TOT = pH + GAP + LINE;
const pX = 256 - pW / 2 - pB[0] * PS, pY = 256 - TOT / 2 - pB[1] * PS;
const B = {
  id: "b-punch-line", name: "Punch line", idea: "The wordmark with the punch line underlined. Small sizes keep the p and the line.",
  bbox: [256 - pW / 2 - 6, 256 - TOT / 2, 256 + pW / 2 + 6, 256 + TOT / 2],
  mark: (fill, acc = T.amber) => `<path fill="${fill}" transform="translate(${n(pX)} ${n(pY)}) scale(${PS})" d="${G.p.d}"/><path fill="${acc}" d="${pill(n(256 - pW / 2 - 6), n(256 + TOT / 2 - LINE), n(pW + 12), LINE)}"/>`,
};
B.small = B.mark;

// Concept C, "Laugh meter": the app's 4-bar reception meter, on its amber heat ramp (theme.ts C.heat).
const BARS = [[96, 120, 0.35], [192, 200, 0.6], [288, 280, 0.85], [384, 360, 1]];
const C = {
  id: "c-laugh-meter", name: "Laugh meter", idea: "The four-bar meter the app already uses for how a room laughed, peaking.",
  bbox: [96, 76, 448, 436],
  mark: (fill, mono) => BARS.map(([x, h, o]) => `<path fill="${fill}"${mono ? "" : ` opacity="${o}"`} d="${bar(x, 436 - h, 64, h, 22)}"/>`).join(""),
};
C.small = C.mark;
const CONCEPTS = [A, B, C];

// ---- SVG documents
const svg = (vb, body, extra = "") => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${vb}"${extra}>${body}</svg>`;
const tileSvg = (k, { bg = T.bg, fg = k === B ? T.text : T.amber, small = false, rx = 112, acc } = {}) =>
  svg("0 0 512 512", `<rect width="512" height="512" rx="${rx}" fill="${bg}"/>${small ? k.small(fg, acc) : k.mark(fg, acc)}`);
const markSvg = (k, fg, opt) => {
  const [x0, y0, x1, y1] = k.bbox, p = 8;
  return svg(`${x0 - p} ${y0 - p} ${x1 - x0 + 2 * p} ${y1 - y0 + 2 * p}`, k.mark(fg, opt));
};
// Lockup: mark at the wordmark's full height (ascender -70 to descender +20 at size 100), 22 units of space, wordmark.
const lockupSvg = (k, fg, acc) => {
  const wm = G.punchable;
  if (k === B) {
    const ul = `<path fill="${acc}" d="${pill(7, 33, n(G.punch.bounds[2] - 7), 12)}"/>`;
    return svg("0 -76 492 126", `<path fill="${fg}" d="${wm.d}"/>${ul}`);
  }
  const [x0, y0, x1, y1] = k.bbox, s = 90 / (y1 - y0), mw = (x1 - x0) * s, tx = mw + 22 - wm.bounds[0];
  const mark = k === C ? C.mark(acc, false) : k.mark(acc);
  return svg(`-4 -76 ${n(tx + wm.bounds[2] + 8)} 102`,
    `<g transform="translate(0 -70) scale(${n(s)}) translate(${-x0} ${-y0})">${mark}</g><path fill="${fg}" transform="translate(${n(tx)} 0)" d="${wm.d}"/>`);
};

function writeSvgs() {
  mkdirSync(BRAND, { recursive: true });
  const files = [];
  for (const k of CONCEPTS) {
    const put = (name, s) => { writeFileSync(join(BRAND, `${k.id}-${name}.svg`), s + "\n"); files.push(`${k.id}-${name}.svg`); };
    put("icon", tileSvg(k));                                       // app icon: graphite tile, amber mark
    put("favicon", tileSvg(k, { small: true, rx: 96 }));            // 16-48 px cut
    put("mark", markSvg(k, k === B ? T.text : T.amber));            // bare, on dark
    put("mark-light", markSvg(k, k === B ? T.ink : T.amberMark, k === B ? T.amberMark : undefined)); // bare, on light
    put("mono", markSvg(k, "currentColor", k === B ? "currentColor" : true)); // one colour: inherits CSS color
    put("lockup-dark", lockupSvg(k, T.text, T.amber));
    put("lockup-light", lockupSvg(k, T.ink, T.amberMark));
  }
  return files;
}

// =====================================================================================================================
// logo-options.html: every concept at every size, dark and light, for the comic to choose. Self-contained.
function optionsHtml() {
  const enc = (s) => "data:image/svg+xml;base64," + Buffer.from(s).toString("base64");
  const SIZES = [16, 32, 48, 64, 180];
  const tileRow = (k, theme) => SIZES.map((s) => {
    const src = s <= 48 ? tileSvg(k, { small: true, rx: 96 }) : tileSvg(k);
    const px = s <= 32 ? `<canvas class="px" data-src="${enc(src)}" data-n="${s}" width="${s}" height="${s}" aria-hidden="true"></canvas>` : "";
    return `<figure class="sz"><img src="${enc(src)}" width="${s}" height="${s}" alt="${k.name} at ${s} px on ${theme}">${px}<figcaption>${s}${s === 180 ? " · iOS" : ""}</figcaption></figure>`;
  }).join("");
  const big = (k) => `<figure class="sz"><img class="big" src="${enc(tileSvg(k))}" alt="${k.name} app icon"><figcaption>1024 app icon, shown at 240</figcaption></figure>`;
  const section = (k, i) => `
  <section class="concept${i === 0 ? " pick" : ""}" id="${k.id}">
    <header class="ch">
      <p class="eyebrow">Concept ${"ABC"[i]}${i === 0 ? ' <span class="rec">Recommended</span>' : ""}</p>
      <h2>${k.name}</h2>
      <p class="idea">${k.idea}</p>
    </header>
    <div class="panes">
      <div class="pane dark">
        <p class="pl">On dark (a dark browser tab)</p>
        <div class="row">${tileRow(k, "dark")}</div>
        <div class="row">${big(k)}
          <figure class="sz"><img class="bare" src="${enc(markSvg(k, k === B ? T.text : T.amber))}" alt=""><figcaption>bare mark</figcaption></figure>
          <figure class="sz mono"><span class="monobox">${markSvg(k, "currentColor", k === B ? "currentColor" : true)}</span><figcaption>mono</figcaption></figure>
        </div>
        <img class="lock" src="${enc(lockupSvg(k, T.text, T.amber))}" alt="${k.name} lockup on dark">
      </div>
      <div class="pane light">
        <p class="pl">On light</p>
        <div class="row">${tileRow(k, "light")}</div>
        <div class="row">${big(k)}
          <figure class="sz"><img class="bare" src="${enc(markSvg(k, k === B ? T.ink : T.amberMark, k === B ? T.amberMark : undefined))}" alt=""><figcaption>bare mark</figcaption></figure>
          <figure class="sz mono"><span class="monobox">${markSvg(k, "currentColor", k === B ? "currentColor" : true)}</span><figcaption>mono</figcaption></figure>
        </div>
        <img class="lock" src="${enc(lockupSvg(k, T.ink, T.amberMark))}" alt="${k.name} lockup on light">
      </div>
    </div>
    <dl class="verdict">${VERDICT[k.id]}</dl>
  </section>`;
  return `<title>Punchable Logo Options</title>
<meta name="claude:viewport-fit-sides" content="cover">
<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@500;700&family=Inter:wght@400;600&display=swap">
<style>
/* Layout: one long sheet, a concept per band; each band shows the same specimens on a dark and a light pane side by side.
   Dark-first, in the app's own Backstage Amber tokens (app/src/lib/theme.ts). */
:root { --bg: #111214; --surface: #1a1c20; --line: #2d3139; --text: #f3f4f6; --mut: #9ca3af; --amber: #f5b544; --pane-d: #202124; --pane-l: #f6f6f4; --ink: #111214; --mut-l: #5d6169;
  --display: "Space Grotesk", "Segoe UI", system-ui, sans-serif; --body: Inter, "Segoe UI", system-ui, sans-serif; color-scheme: dark; }
@media (prefers-color-scheme: light) { :root:not([data-theme="dark"]) { --bg: #f6f6f4; --surface: #ffffff; --line: #d9d9d4; --text: #111214; --mut: #5d6169; --amber: #8a5a00; color-scheme: light; } }
:root[data-theme="light"] { --bg: #f6f6f4; --surface: #ffffff; --line: #d9d9d4; --text: #111214; --mut: #5d6169; --amber: #8a5a00; color-scheme: light; }
body { background: var(--bg); color: var(--text); font: 15px/1.5 var(--body); }
main { max-width: 1180px; margin: 0 auto; padding-block: 32px 64px; padding-left: max(16px, env(safe-area-inset-left, 0px)); padding-right: max(16px, env(safe-area-inset-right, 0px)); display: grid; gap: 40px; }
h1, h2 { font-family: var(--display); text-wrap: balance; margin: 0; }
h1 { font-size: clamp(28px, 5vw, 44px); letter-spacing: -0.02em; line-height: 1.05; }
h2 { font-size: 26px; letter-spacing: -0.01em; }
.lede { color: var(--mut); max-width: 62ch; margin: 10px 0 0; }
.eyebrow { font: 700 12px/1 var(--display); letter-spacing: .08em; text-transform: uppercase; color: var(--mut); margin: 0 0 8px; display: flex; gap: 10px; align-items: center; }
.rec { color: #111214; background: #f5b544; border-radius: 4px; padding: 3px 7px; letter-spacing: .06em; }
.idea { margin: 6px 0 0; color: var(--mut); max-width: 60ch; }
.concept { display: grid; gap: 16px; padding-top: 28px; border-top: 1px solid var(--line); }
.panes { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
@media (max-width: 820px) { .panes { grid-template-columns: 1fr; } }
.pane { border-radius: 12px; padding: 18px; display: grid; gap: 18px; min-width: 0; }
.pane.dark { background: var(--pane-d); color: #f3f4f6; border: 1px solid #3c4043; --cap: #bdc1c6; }
.pane.light { background: var(--pane-l); color: var(--ink); border: 1px solid #d9d9d4; --cap: var(--mut-l); }
.pl { margin: 0; font: 700 11px/1 var(--display); letter-spacing: .08em; text-transform: uppercase; color: var(--cap); }
.row { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 18px; }
.sz { margin: 0; display: grid; justify-items: center; gap: 6px; }
.sz figcaption { font: 12px/1 var(--body); color: var(--cap); font-variant-numeric: tabular-nums; }
.sz img { display: block; }
.px { width: 64px; height: 64px; image-rendering: pixelated; border-radius: 2px; }
.big { width: 240px; max-width: 100%; height: auto; }
.bare { height: 120px; width: auto; }
.monobox { display: block; height: 120px; color: inherit; }
.monobox svg { height: 100%; width: auto; display: block; }
.lock { height: 44px; width: auto; max-width: 100%; }
.verdict { display: grid; grid-template-columns: max-content 1fr; gap: 6px 16px; margin: 0; max-width: 90ch; }
.verdict dt { font: 600 13px/1.5 var(--body); color: var(--mut); }
.verdict dd { margin: 0; }
.note { color: var(--mut); font-size: 13px; margin: 0; }
</style>
<main>
  <header>
    <p class="eyebrow">Punchable · brand</p>
    <h1>Three marks for Punchable. Pick one.</h1>
    <p class="lede">Each concept at 16, 32, 48, 64 and 180 px and as the 1024 app icon, on dark and on light, plus the bare mark, a one-colour version and the lockup with the wordmark. The 16 and 32 px tiles also appear blown up pixel for pixel, so you see what a browser tab actually draws.</p>
  </header>
  ${CONCEPTS.map(section).join("")}
  <p class="note">Generated by app/scripts/brand/brand.mjs. The wordmark is Space Grotesk Bold, outlined. Recommendation and reasons: docs/DESIGN.md §7.</p>
</main>
<script>
// Draw each small tile at its true pixel size, then show it 4x with no smoothing: what a 16 px favicon really looks like.
for (const c of document.querySelectorAll("canvas.px")) {
  const img = new Image();
  img.onload = () => { const n = +c.dataset.n, x = c.getContext("2d"); x.clearRect(0, 0, n, n); x.drawImage(img, 0, 0, n, n); };
  img.src = c.dataset.src;
}
</script>
`;
}

const VERDICT = {
  "a-mic-p": `<dt>Reads as</dt><dd>A P and a stage mic at once. Nothing else on a phone's home screen looks like it.</dd>
    <dt>At 16 px</dt><dd>Holds: a lollipop P with one slot. The three-slot grille comes back from 64 px up.</dd>
    <dt>Mono</dt><dd>One shape with cut-outs, so it is a single colour already (Android themed icon, Safari tab).</dd>
    <dt>Risk</dt><dd>Mics are the obvious comedy object. Drawing it as the P is what makes it ours.</dd>`,
  "b-punch-line": `<dt>Reads as</dt><dd>The name, with the punch line underlined: the one line in a joke that matters.</dd>
    <dt>At 16 px</dt><dd>A p over an amber dash. Legible, but it is a letter in a box, like many apps.</dd>
    <dt>Mono</dt><dd>Fine. Without the amber the line stops meaning "punch line".</dd>
    <dt>Risk</dt><dd>Weak as an app icon; strong as a wordmark. It can live on as the wordmark under any mark.</dd>`,
  "c-laugh-meter": `<dt>Reads as</dt><dd>The app's own reception meter: how hard the room laughed.</dd>
    <dt>At 16 px</dt><dd>Four bars survive, but at that size they read as phone signal strength.</dd>
    <dt>Mono</dt><dd>Loses the heat ramp, so all four bars look equal: it becomes a bar chart.</dd>
    <dt>Risk</dt><dd>Says "analytics" before it says "comedy". Better as an in-app motif than as the logo.</dd>`,
};

// =====================================================================================================================
// PNGs for the chosen concept, with headless Chrome over CDP. the comic picked C, Laugh meter (2026-10-26).
const K = C;
async function renderPngs() {
  const CHROME = ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"].find(existsSync);
  if (!CHROME) throw new Error("no Chrome/Edge found");
  const tmp = mkdtempSync(join(tmpdir(), "pk_brand_"));
  const port = await new Promise((r) => { const s = net.createServer(); s.listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => r(p)); }); });
  const ch = spawn(CHROME, ["--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${join(tmp, "prof")}`, "--no-first-run", "--hide-scrollbars", "about:blank"], { stdio: "ignore" });
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  let t; for (let i = 0; i < 200 && !t; i++) { await sleep(100); try { t = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((x) => x.type === "page"); } catch {} }
  const ws = new WebSocket(t.webSocketDebuggerUrl); await new Promise((r) => (ws.onopen = r));
  let id = 0; const pend = new Map();
  ws.onmessage = (e) => { const m = JSON.parse(e.data); if (m.id && pend.has(m.id)) { pend.get(m.id)(m); pend.delete(m.id); } };
  const cdp = (method, params = {}) => new Promise((res, rej) => { const i = ++id; pend.set(i, (m) => (m.error ? rej(new Error(method + ": " + m.error.message)) : res(m.result))); ws.send(JSON.stringify({ id: i, method, params })); });
  await cdp("Page.enable");
  await cdp("Emulation.setDefaultBackgroundColorOverride", { color: { r: 0, g: 0, b: 0, a: 0 } });
  const inter = pathToFileURL(join(APP, "node_modules", "@expo-google-fonts", "inter", "400Regular", "Inter_400Regular.ttf")).href;
  const render = async (out, w, h, body, bg = "transparent") => {
    const f = join(tmp, "r.html");
    writeFileSync(f, `<!doctype html><style>@font-face{font-family:I;src:url(${inter})}html,body{margin:0;width:${w}px;height:${h}px;overflow:hidden;background:${bg}}svg{display:block}</style>${body}`);
    await cdp("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 1, mobile: false });
    await cdp("Page.navigate", { url: pathToFileURL(f).href }); await sleep(350);
    await cdp("Runtime.evaluate", { expression: "document.fonts.ready", awaitPromise: true });
    const r = await cdp("Page.captureScreenshot", { format: "png", clip: { x: 0, y: 0, width: w, height: h, scale: 1 } });
    mkdirSync(dirname(out), { recursive: true }); writeFileSync(out, Buffer.from(r.data, "base64")); console.log("  png", out.replace(REPO + "\\", "").replace(REPO + "/", ""), `${w}x${h}`);
  };
  const sized = (s, w, h) => s.replace("<svg ", `<svg width="${w}" height="${h}" `);
  const IMG = join(APP, "assets", "images"), PUB = join(APP, "public");
  const square = (bodySvg) => svg("0 0 512 512", bodySvg);
  // Full-bleed squares: the OS rounds iOS icons and masks Android ones itself.
  const iconFull = square(`<rect width="512" height="512" fill="${T.bg}"/>${K.mark(T.amber)}`);
  const scaled = (k, s, fill) => `<g transform="translate(256 256) scale(${s}) translate(-256 -256)">${k.mark(fill)}</g>`;
  await render(join(IMG, "icon.png"), 1024, 1024, sized(iconFull, 1024, 1024));
  await render(join(PUB, "apple-touch-icon.png"), 180, 180, sized(iconFull, 180, 180));
  // Android adaptive: 108 dp canvas, the mask may cut to a 66 dp circle, so the mark sits at 0.74.
  await render(join(IMG, "android-icon-foreground.png"), 1024, 1024, sized(square(scaled(K, 0.74, T.amber)), 1024, 1024));
  await render(join(IMG, "android-icon-background.png"), 1024, 1024, sized(square(`<rect width="512" height="512" fill="${T.bg}"/>`), 1024, 1024));
  await render(join(IMG, "android-icon-monochrome.png"), 1024, 1024, sized(square(`<g transform="translate(256 256) scale(0.74) translate(-256 -256)">${K.mark("#ffffff", true)}</g>`), 1024, 1024));
  // Splash: the bare mark on the splash background (app.json expo-splash-screen backgroundColor).
  await render(join(IMG, "splash-icon.png"), 1024, 1024, sized(square(K.mark(T.amber)), 1024, 1024));
  // Favicons: the small cut on a rounded tile.
  const fav = tileSvg(K, { small: true, rx: 96 });
  writeFileSync(join(PUB, "favicon.svg"), fav + "\n");
  for (const s of [16, 32, 48]) await render(join(tmp, `fav${s}.png`), s, s, sized(fav, s, s));
  await render(join(IMG, "favicon.png"), 48, 48, sized(fav, 48, 48));
  await render(join(PUB, "icon-192.png"), 192, 192, sized(tileSvg(K, { rx: 112 }), 192, 192));
  await render(join(PUB, "icon-512.png"), 512, 512, sized(tileSvg(K, { rx: 112 }), 512, 512));
  const ico = spawnSync("magick", [join(tmp, "fav16.png"), join(tmp, "fav32.png"), join(tmp, "fav48.png"), join(PUB, "favicon.ico")], { encoding: "utf8" });
  if (ico.status !== 0) throw new Error("magick failed: " + ico.stderr);
  console.log("  ico app/public/favicon.ico 16+32+48");
  // og-image 1200x630: the lockup, the pitch, and a set's laugh strip along the foot (heat ramp, theme.ts C.heat).
  const heat = [0.12, 0.35, 0.6, 0.85, 1];
  const strip = [0, 1, 0, 2, 3, 1, 0, 0, 2, 4, 3, 1, 0, 1, 2, 2, 0, 3, 4, 4, 2, 0, 1, 0, 2, 3, 1, 0, 0, 1, 3, 4, 2, 1, 0, 2, 4, 3, 1, 0];
  const sw = 1200 / strip.length;
  const og = svg("0 0 1200 630", `<rect width="1200" height="630" fill="${T.bg}"/>
    <g transform="translate(96 212) scale(0.55) translate(-96 -76)">${K.mark(T.amber)}</g>
    <path fill="${T.text}" transform="translate(330 300) scale(1.42)" d="${G.punchable.d}"/>
    <text x="336" y="400" fill="${T.mut}" font-family="I" font-size="31">Every bit you've written, and how long they laughed.</text>
    ${strip.map((h, i) => `<rect x="${n(i * sw)}" y="598" width="${n(sw - 2)}" height="32" fill="${T.amber}" opacity="${heat[h]}"/>`).join("")}`);
  await render(join(PUB, "og-image.png"), 1200, 630, sized(og, 1200, 630), T.bg);
  ws.close(); ch.kill(); await sleep(300); try { rmSync(tmp, { recursive: true, force: true }); } catch {}
}

const files = writeSvgs();
writeFileSync(join(BRAND, "logo-options.html"), optionsHtml());
console.log(`wrote ${files.length} SVGs + logo-options.html to docs/brand/`);
if (process.argv.includes("--png")) await renderPngs();
