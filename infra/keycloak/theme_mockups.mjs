// Screenshots of the punchable login theme WITHOUT Keycloak: static mock-ups. The CSS and the brand header are lifted
// verbatim from theme/login/template.ftl and the copy from the .ftl pages and messages_en.properties; only the
// FreeMarker logic is replaced by sample values. A real render is test-realm.sh (throwaway Keycloak), which could not
// run on 2026-10-26 (Docker on the server triggers the apiservice-kicker; no Docker/WSL/Java on the PC).
//   node infra/keycloak/theme_mockups.mjs      -> docs/ux/shots-auth/kc-<page>-{m,d}.png (390x844 @2x, 1280x900)
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import net from "node:net";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = resolve(HERE, "..", "..", "docs", "ux", "shots-auth");
const tpl = readFileSync(join(HERE, "theme", "login", "template.ftl"), "utf8");
const msgs = Object.fromEntries(readFileSync(join(HERE, "theme", "login", "messages_en.properties"), "utf8").split(/\r?\n/)
  .filter((l) => l && !l.startsWith("#") && l.includes("=")).map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1).replace(/''/g, "'")]));
const head = tpl.slice(tpl.indexOf("<head>"), tpl.indexOf("</head>") + 7).replace(/<title>[\s\S]*?<\/title>/, "<title>mock</title>");
const brand = tpl.match(/<div class="pk-brand"[\s\S]*?<\/div>/)[0];
const foot = tpl.match(/<p class="pk-foot">[\s\S]*?<\/p>/)[0];
const G = readFileSync(join(HERE, "theme", "login", "login.ftl"), "utf8").match(/<svg viewBox="0 0 24 24"[\s\S]*?<\/svg>/)[0];

const page = ({ title, sub, alert, social = "", form }) => `<!DOCTYPE html><html lang="en">${head}<body><main class="pk">${brand}
<header class="pk-head"><h1>${title}</h1>${sub ? `<p class="pk-sub">${sub}</p>` : ""}</header>
<div class="pk-card">${alert ? `<div class="pk-alert pk-alert-${alert[0]}" role="alert">${alert[1]}</div>` : ""}${social}<div class="pk-content">${form}</div></div>
${foot}</main><div style="position:fixed;right:8px;bottom:6px;font:11px system-ui;color:#4b5059">static mock-up of the Keycloak theme</div></body></html>`;
const google = `<a id="social-google" class="pk-google" href="#">${G}<span>Continue with Google</span></a><div class="pk-or" role="separator">or use email</div>`;
const loginForm = (err) => `<form id="kc-form-login">
  <div class="pk-field"><label for="username">Email</label><input id="username" type="email" value="${err ? "owner@example.com" : ""}" placeholder="you@example.com"${err ? ' aria-invalid="true"' : ""}></div>
  <div class="pk-field"><label for="password">Password</label><input id="password" type="password" value="${err ? "wrongpassword" : ""}"${err ? ' aria-invalid="true"' : ""}>${err ? `<span class="pk-err" role="alert">${msgs.invalidUserMessage}</span>` : ""}</div>
  <div class="pk-row"><label class="pk-check"><input type="checkbox"> Keep me signed in</label><a class="pk-link" href="#">Forgot password?</a></div>
  <button class="pk-btn" id="kc-login" type="submit">Sign in</button></form>`;
const PAGES = {
  login: { title: "Sign in", sub: "Every bit you've written, and how long they laughed.", social: google, form: loginForm(false) },
  "login-error": { title: "Sign in", sub: "Every bit you've written, and how long they laughed.", social: google, form: loginForm(true) },
  /* "link-google" (password re-auth) retired: Google first login auto-links since 9538150 */ "_link-google": { title: "Link your Google account", sub: "Enter your Punchable password once. After that, Continue with Google signs you straight in.",
    alert: ["info", msgs.federatedIdentityConfirmReauthenticateMessage.replace("{1}", "google")],
    form: `<form><p class="pk-hint">Signing in as <b style="color:var(--text)">owner@example.com</b></p><div class="pk-field"><label for="password">Password</label><input id="password" type="password"></div><div class="pk-row"><a class="pk-link" href="#">Forgot password?</a></div><button class="pk-btn" type="submit">Link Google</button></form>` },
  "google-not-linked": { title: "That didn't work", form: `<div class="pk-alert pk-alert-error" role="alert">${msgs.federatedIdentityUnavailableMessage}</div><a class="pk-btn pk-btn-quiet" href="#">Back to Punchable</a><a class="pk-link" href="#">Start sign-in again</a>` },
  "reset-password": { title: "Reset your password", sub: "Enter your email. We'll send a link to set a new password.",
    form: `<form><div class="pk-field"><label for="username">Email</label><input id="username" type="email" placeholder="you@example.com"></div><button class="pk-btn" type="submit">Email me a link</button><a class="pk-link" href="#">Back to sign in</a></form>` },
  "update-password": { title: "Set a new password", sub: "At least 10 characters. Length beats symbols: a line you'd never say on stage works fine.",
    alert: ["error", msgs.notMatchPasswordMessage],
    form: `<form><div class="pk-field"><label for="pn">New password</label><input id="pn" type="password" value="0123456789ab"></div><div class="pk-field"><label for="pc">Type it again</label><input id="pc" type="password" value="0123456789" aria-invalid="true"></div><label class="pk-check"><input type="checkbox" checked> Sign out on my other devices</label><button class="pk-btn" type="submit">Save password</button></form>` },
  "verify-email": { title: "Check your email",
    form: `<p class="pk-sub" style="margin:0">We sent a link to <b>owner@example.com</b>. Open it to confirm this address, then come back here.</p><p class="pk-hint">Nothing there after a few minutes? Check spam, or send it again.</p><a class="pk-btn pk-btn-quiet" href="#">Send the link again</a><a class="pk-link" href="#">Back to sign in</a>` },
};

const CHROME = ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"].find(existsSync);
const tmp = mkdtempSync(join(tmpdir(), "pk_kcmock_"));
const port = await new Promise((r) => { const s = net.createServer(); s.listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => r(p)); }); });
const ch = spawn(CHROME, ["--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${join(tmp, "p")}`, "--no-first-run", "--hide-scrollbars", "about:blank"], { stdio: "ignore" });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let t; for (let i = 0; i < 200 && !t; i++) { await sleep(100); try { t = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((x) => x.type === "page"); } catch {} }
const ws = new WebSocket(t.webSocketDebuggerUrl); await new Promise((r) => (ws.onopen = r));
let id = 0; const pend = new Map();
ws.onmessage = (e) => { const m = JSON.parse(e.data); if (m.id && pend.has(m.id)) { pend.get(m.id)(m.result); pend.delete(m.id); } };
const cdp = (method, params = {}) => new Promise((r) => { const i = ++id; pend.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
await cdp("Page.enable");
mkdirSync(OUT, { recursive: true });
for (const [name, p] of Object.entries(PAGES).filter(([k]) => !k.startsWith("_"))) {
  const f = join(tmp, name + ".html"); writeFileSync(f, page(p));
  for (const [w, h, sfx] of [[390, 844, "m"], [1280, 900, "d"]]) {
    await cdp("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: w < 600 ? 2 : 1, mobile: w < 600 });
    await cdp("Page.navigate", { url: pathToFileURL(f).href }); await sleep(900);
    await cdp("Runtime.evaluate", { expression: "document.fonts.ready", awaitPromise: true });
    const r = await cdp("Page.captureScreenshot", { format: "png" });
    writeFileSync(join(OUT, `kc-${name}-${sfx}.png`), Buffer.from(r.data, "base64"));
  }
  console.log("  shot", name);
}
ws.close(); ch.kill(); await sleep(300); try { rmSync(tmp, { recursive: true, force: true }); } catch {}
