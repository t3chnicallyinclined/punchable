// Screenshots of the punchable login theme WITHOUT Keycloak: static mock-ups. The <head> (all the CSS), the brand
// header, the footer, the inline script and the password-eye button are lifted verbatim from theme/login/template.ftl
// and the copy from the .ftl pages and messages_en.properties; only the FreeMarker logic is replaced by sample values.
// A real render is test-realm.sh (throwaway Keycloak; not on the server: Docker there restarts the cluster).
//   node infra/keycloak/theme_mockups.mjs
//     -> $PK_SHOTS_OUT (default docs/ux/shots-auth)/kc-<page>-<m|d>-<dark|light>.png  (390x844 @2x, 1280x900)
//        plus the mock HTML next to them, to open in a browser.
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import net from "node:net";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = process.env.PK_SHOTS_OUT || resolve(HERE, "..", "..", "docs", "ux", "shots-auth");
const ONLY = process.argv.slice(2); // page names to render; none = all
const tpl = readFileSync(join(HERE, "theme", "login", "template.ftl"), "utf8");
const msgs = Object.fromEntries(readFileSync(join(HERE, "theme", "login", "messages_en.properties"), "utf8").split(/\r?\n/)
  .filter((l) => l && !l.startsWith("#") && l.includes("=")).map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1).replace(/''/g, "'")]));
const head = tpl.slice(tpl.indexOf("<head>"), tpl.indexOf("</head>") + 7).replace(/<title>[\s\S]*?<\/title>/, "<title>mock</title>");
const brand = tpl.match(/<div class="pk-brand"[\s\S]*?<\/div>/)[0];
const foot = tpl.match(/<p class="pk-foot">[\s\S]*?<\/p>/)[0];
const script = tpl.match(/<script>[\s\S]*?<\/script>/)[0];
const eyeOf = (id) => tpl.match(/<button type="button" class="pk-pw-toggle"[\s\S]*?<\/button>/)[0].replace("${field}", id);
const G = readFileSync(join(HERE, "theme", "login", "login.ftl"), "utf8").match(/<svg viewBox="0 0 24 24"[\s\S]*?<\/svg>/)[0];

const page = ({ title, sub, alert, social = "", form }) => `<!DOCTYPE html><html lang="en">${head}<body><main class="pk">${brand}
<header class="pk-head"><h1>${title}</h1>${sub ? `<p class="pk-sub">${sub}</p>` : ""}</header>
<div class="pk-card">${alert ? `<div class="pk-alert pk-alert-${alert[0]}" role="${alert[0] === "error" ? "alert" : "status"}">${alert[1]}</div>` : ""}${social}<div class="pk-content">${form}</div></div>
${foot}</main>${script}<div style="position:fixed;right:8px;bottom:6px;font:11px system-ui;color:var(--dim)">static mock-up of the Keycloak theme</div></body></html>`;
const google = `<a id="social-google" class="pk-google" href="#">${G}<span>Sign in with Google</span></a><div class="pk-or" role="separator" aria-label="or">or</div>`;
const pw = (id, label, value = "", err = null) => `<div class="pk-field"><label for="${id}">${label}</label><div class="pk-pw"><input id="${id}" name="${id}" type="password" autocomplete="current-password" autocapitalize="none" value="${value}"${err ? ` aria-invalid="true" aria-describedby="${id}-err"` : ""}>${eyeOf(id)}</div>${err ? `<span id="${id}-err" class="pk-err" role="alert">${err}</span>` : ""}</div>`;
const email = (value = "", err = null) => `<div class="pk-field"><label for="username">Email</label><input id="username" name="username" type="email" inputmode="email" autocomplete="username" autocapitalize="none" spellcheck="false" value="${value}" placeholder="you@example.com"${err ? ' aria-invalid="true" aria-describedby="input-error-username"' : ""}>${err ? `<span id="input-error-username" class="pk-err" role="alert">${err}</span>` : ""}</div>`;
const loginForm = (err) => `<form id="kc-form-login" data-once>
  ${email(err ? "owner@example.com" : "")}
  ${pw("password", "Password", err ? "wrongpassword" : "", err ? msgs.invalidUserMessage : null)}
  <div class="pk-row"><label class="pk-check"><input id="rememberMe" type="checkbox"> Remember me</label><a class="pk-link" href="#">Forgot password?</a></div>
  <button class="pk-btn" id="kc-login" type="submit" data-busy="Signing in…">Sign in</button></form>`;
const PITCH = "Every bit you've written, and how long they laughed.";
const PAGES = {
  login: { title: "Sign in", sub: PITCH, social: google, form: loginForm(false) },
  "login-error": { title: "Sign in", sub: PITCH, social: google, form: loginForm(true) },
  "login-no-idp": { title: "Sign in", sub: PITCH, form: loginForm(false) },
  "login-locked": { title: "Sign in", sub: PITCH, social: google, alert: ["error", msgs.accountTemporarilyDisabledMessage], form: loginForm(false) },
  "link-google": { title: "Link your Google account", sub: "Enter your Punchable password once. After that, Google signs you straight in.",
    alert: ["info", msgs.federatedIdentityConfirmReauthenticateMessage.replace("{1}", "Google")],
    form: `<form data-once><p class="pk-hint">Signing in as <b>owner@example.com</b></p>${pw("password", "Password")}<div class="pk-row"><a class="pk-link" href="#">Forgot password?</a></div><button class="pk-btn" type="submit">Link Google</button></form>` },
  "reset-password": { title: "Reset your password", sub: "Enter your email and we'll send you a link to set a new one.",
    form: `<form data-once>${email()}<button class="pk-btn" type="submit">Send link</button><a class="pk-link pk-center" href="#">Back to sign in</a></form>` },
  "reset-password-error": { title: "Reset your password", sub: "Enter your email and we'll send you a link to set a new one.",
    form: `<form data-once>${email("owner", msgs.invalidEmailMessage)}<button class="pk-btn" type="submit">Send link</button><a class="pk-link pk-center" href="#">Back to sign in</a></form>` },
  "reset-sent": { title: "Done", form: `<p class="pk-hint">${msgs.emailSentMessage}</p><a class="pk-btn" href="#">Back to Punchable</a>` },
  "update-password": { title: "Set a new password", sub: "At least 10 characters. Length beats symbols.",
    form: `<form data-once>${pw("password-new", "New password", "0123456789ab")}${pw("password-confirm", "Confirm password", "0123456789")}<label class="pk-check"><input type="checkbox" checked> Sign out of other devices</label><button class="pk-btn" type="submit">Save password</button></form>` },
  "update-password-error": { title: "Set a new password", sub: "At least 10 characters. Length beats symbols.",
    form: `<form data-once>${pw("password-new", "New password", "0123456789ab")}${pw("password-confirm", "Confirm password", "0123456789", msgs.notMatchPasswordMessage)}<label class="pk-check"><input type="checkbox" checked> Sign out of other devices</label><button class="pk-btn" type="submit">Save password</button></form>` },
  "verify-email": { title: "Check your email",
    form: `<p class="pk-hint">We sent a link to <b>owner@example.com</b>. Open it to confirm this address, then come back here.</p><p class="pk-hint">Nothing there after a few minutes? Check spam, or send it again.</p><form data-once><button class="pk-btn pk-btn-quiet" type="submit">Send again</button><a class="pk-link pk-center" href="#">Back to sign in</a></form>` },
  "google-not-linked": { title: "That didn't work", form: `<div class="pk-alert pk-alert-error" role="alert">${msgs.federatedIdentityUnavailableMessage}</div><a class="pk-btn pk-btn-quiet" href="#">Back to Punchable</a><a class="pk-link pk-center" href="#">Start sign-in again</a>` },
  "page-expired": { title: "This page timed out", sub: "Sign-in pages only stay open for a while. Nothing was lost.",
    form: `<a class="pk-btn" href="#">Start again</a><a class="pk-link pk-center" href="#">Continue where I was</a>` },
  "idp-link-confirm": { title: "Link Google to Punchable?",
    form: `<p class="pk-hint">There's already a Punchable account for <b>owner@example.com</b>. Link it and either way in works.</p><form data-once><button class="pk-btn" type="submit">Link Google</button><a class="pk-link pk-center" href="#">Cancel</a></form>` },
  "idp-link-email": { title: "Check your email",
    form: `<p class="pk-hint">To link Google, open the link we sent to <b>owner@example.com</b>.</p><p class="pk-hint">No email? Check spam, or send it again.</p><a class="pk-btn pk-btn-quiet" href="#">Send again</a><a class="pk-link pk-center" href="#">Cancel</a>` },
};

const CHROME = ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"].find(existsSync);
if (!CHROME) { console.error("no Chrome/Edge found"); process.exit(1); }
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
for (const [name, p] of Object.entries(PAGES).filter(([k]) => !ONLY.length || ONLY.includes(k))) {
  const f = join(OUT, `kc-${name}.html`); writeFileSync(f, page(p));
  for (const scheme of ["dark", "light"]) {
    await cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: scheme }] });
    for (const [w, h, sfx] of [[390, 844, "m"], [1280, 900, "d"]]) {
      await cdp("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: w < 600 ? 2 : 1, mobile: w < 600 });
      await cdp("Page.navigate", { url: pathToFileURL(f).href }); await sleep(400);
      await cdp("Runtime.evaluate", { expression: "document.fonts.ready", awaitPromise: true });
      const r = await cdp("Page.captureScreenshot", { format: "png" });
      writeFileSync(join(OUT, `kc-${name}-${sfx}-${scheme}.png`), Buffer.from(r.data, "base64"));
    }
  }
  console.log("  shot", name);
}
ws.close(); ch.kill(); await sleep(300); try { rmSync(tmp, { recursive: true, force: true }); } catch {}
console.log("->", OUT);
