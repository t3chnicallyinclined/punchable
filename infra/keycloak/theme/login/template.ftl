<#-- Punchable login theme (realm punchable, auth.punchable.io). Flat dir, one ConfigMap, copied into
     /opt/keycloak/themes/punchable/login by the shared keycloak Deployment's setup-theme init container.
     Pattern (same as the other products' themes on this Keycloak): parent=base, our own registrationLayout macro, CSS
     INLINE (Keycloak versions theme resources and browsers cache them hard; inline CSS changes with the template).
     Pages we don't override (parent=base) still render inside this layout: the generic rules at the end style them.
     Look: app/src/lib/theme.ts (DARK + LIGHT_TOKENS) and the Mix* artboards: ground #0B0B0D, cards #1C1C1F, amber
     #FFB340 (light: #b47a0e marks, #8a5a00 links), red #FF453A. Both schemes via prefers-color-scheme.
     Type: the system stack (SF on the iPhone sheet, Segoe/Roboto elsewhere), Inter when it is installed. No web font:
     nothing to fetch, no layout shift, no third party on the sign-in page. The wordmark is an outlined path, so the
     brand doesn't depend on a font either.
     Shown in an iOS in-app browser sheet at 390 pt and in desktop browsers: 44 pt targets, 16 px inputs (no iOS zoom).
     No external JS: one inline script (password eye, once-only submit); the pages work without it.
     `rest...` catches any extra layout parameter a base page passes (showAnotherWayIfPresent, displayWide, ...). -->
<#macro registrationLayout bodyClass="" displayInfo=false displayMessage=true displayRequiredFields=false rest...>
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="robots" content="noindex, nofollow">
    <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
    <meta name="color-scheme" content="dark light">
    <meta name="theme-color" media="(prefers-color-scheme: dark)" content="#0B0B0D">
    <meta name="theme-color" media="(prefers-color-scheme: light)" content="#f6f6f4">
    <title>${pageTitle!"Sign in"} · Punchable</title>
    <link rel="icon" type="image/svg+xml" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 512 512'%3E%3Crect width='512' height='512' rx='96' fill='%23111214'/%3E%3Cpath fill='%23f5b544' opacity='0.35' d='M96 338a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H96z'/%3E%3Cpath fill='%23f5b544' opacity='0.6' d='M192 258a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H192z'/%3E%3Cpath fill='%23f5b544' opacity='0.85' d='M288 178a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H288z'/%3E%3Cpath fill='%23f5b544' d='M384 98a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H384z'/%3E%3C/svg%3E">
    <style>
/* Tokens = app/src/lib/theme.ts DARK; the light block = LIGHT_TOKENS. Contrast pairs are checked by
   infra/keycloak/test_theme_static.py (every text/background pair here is >= 4.5:1 in both schemes). */
:root {
    color-scheme: dark light;
    --bg: #0B0B0D; --surface: #161618; --card: #1C1C1F; --hair: #1F1F22; --border: #2d3139; --border-strong: #4b5059;
    --text: #f3f4f6; --text2: #d1d5db; --mut: #A1A1A8; --dim: #878d97;
    --mark: #FFB340; --link: #FFB340; --btn-bg: #FFB340; --btn-fg: #0B0B0D; --focus: #FFB340; --ring: rgba(255,179,64,.35);
    --err: #fca5a5; --err-line: #FF453A; --err-border: #7f1d1d; --err-bg: #2a1210;
    --warn: #FFB340; --warn-border: #5c4a1f; --warn-bg: #241e12;
    --ok: #86efac; --ok-border: #1f6a3a; --ok-bg: #0f2418;
    --info: #bae6fd; --info-border: #1e3a52; --info-bg: #10222e;
    --g-bg: #131314; --g-fg: #E3E3E3; --g-border: #8E918F;
    --tile: #1C1C1F; --tile-line: #1C1C1F;
    --shadow: 0 24px 64px rgba(0,0,0,.45);
    --font: Inter, -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
}
@media (prefers-color-scheme: light) {
    :root {
        --bg: #f6f6f4; --surface: #ffffff; --card: #ffffff; --hair: #e4e4df; --border: #d9d9d4; --border-strong: #8f8b82;
        --text: #111214; --text2: #3d4048; --mut: #5d6169; --dim: #6b7079;
        --mark: #b47a0e; --link: #8a5a00; --btn-bg: #f5b544; --btn-fg: #111214; --focus: #b47a0e; --ring: rgba(180,122,14,.35);
        --err: #b91c1c; --err-line: #b91c1c; --err-border: #f1b5b5; --err-bg: #fdecec;
        --warn: #8a5a00; --warn-border: #e2cfa3; --warn-bg: #fbf3df;
        --ok: #15803d; --ok-border: #a7dbb8; --ok-bg: #eaf7ee;
        --info: #0369a1; --info-border: #b9dcf0; --info-bg: #eaf4fa;
        --g-bg: #ffffff; --g-fg: #1F1F1F; --g-border: #747775;
        --tile: #ffffff; --tile-line: #e4e4df;
        --shadow: 0 24px 64px rgba(17,18,20,.10);
    }
}
*, *::before, *::after { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; text-size-adjust: 100%; }
body {
    margin: 0; min-height: 100vh; min-height: 100dvh; background: var(--bg); color: var(--text);
    font: 400 16px/1.45 var(--font); -webkit-font-smoothing: antialiased;
    display: flex; justify-content: center; align-items: flex-start;
    padding: max(28px, env(safe-area-inset-top)) max(20px, env(safe-area-inset-right)) max(24px, env(safe-area-inset-bottom)) max(20px, env(safe-area-inset-left));
}
.pk { width: 100%; max-width: 400px; display: flex; flex-direction: column; gap: 24px; }
/* Desktop: the same column, on a card, centred. The phone sheet is the column on the ground, like the app. */
@media (min-width: 600px) {
    body { align-items: center; padding: 40px 24px; }
    .pk { max-width: 448px; padding: 32px; border-radius: 20px; background: var(--card); border: 1px solid var(--hair); box-shadow: var(--shadow); }
}
.pk-brand { display: flex; align-items: center; gap: 12px; color: var(--text); }
.pk-brand svg { display: block; flex: none; }
.pk-mark { width: 44px; height: 44px; }
.pk-word { height: 22px; width: auto; }
.pk-head h1 { font-size: 28px; line-height: 1.15; font-weight: 800; letter-spacing: -0.02em; margin: 0; text-wrap: balance; }
.pk-sub { margin: 8px 0 0; color: var(--mut); font-size: 15px; line-height: 1.5; }
.pk-sub b, .pk-hint b { color: var(--text); font-weight: 600; }
.pk-card, .pk-content { display: flex; flex-direction: column; gap: 16px; }

/* Alerts: plain words, above the form. */
.pk-alert { border-radius: 12px; padding: 12px 14px; font-size: 15px; line-height: 1.45; border: 1px solid var(--border); background: var(--surface); color: var(--text2); }
.pk-alert-error { border-color: var(--err-border); background: var(--err-bg); color: var(--err); }
.pk-alert-warning { border-color: var(--warn-border); background: var(--warn-bg); color: var(--warn); }
.pk-alert-success { border-color: var(--ok-border); background: var(--ok-bg); color: var(--ok); }
.pk-alert-info { border-color: var(--info-border); background: var(--info-bg); color: var(--info); }

/* Sign in with Google: Google's own button, their dark variant on dark and light on light (their brand rules). */
.pk-google {
    display: flex; align-items: center; justify-content: center; gap: 12px; width: 100%; min-height: 52px; padding: 0 16px;
    border-radius: 12px; background: var(--g-bg); color: var(--g-fg); border: 1px solid var(--g-border); text-decoration: none;
    font-size: 17px; font-weight: 600; line-height: 1;
}
.pk-google:hover { filter: brightness(1.08); text-decoration: none; }
.pk-google:active { filter: brightness(.95); }
.pk-google svg { width: 22px; height: 22px; flex: none; }
.pk-social-other { background: var(--surface); color: var(--text); border-color: var(--border-strong); }
.pk-or { display: flex; align-items: center; gap: 12px; color: var(--mut); font-size: 13px; font-weight: 600; line-height: 1; letter-spacing: .04em; text-transform: uppercase; }
.pk-or::before, .pk-or::after { content: ""; flex: 1; height: 1px; background: var(--border); }

form { display: flex; flex-direction: column; gap: 16px; margin: 0; }
.pk-field { display: flex; flex-direction: column; gap: 6px; }
label, .pk-label { font-size: 14px; font-weight: 600; line-height: 1.3; color: var(--text2); }
input[type=text], input[type=email], input[type=password], input[type=tel], input[type=number], select, textarea {
    width: 100%; min-height: 48px; padding: 12px 14px; border-radius: 12px; border: 1px solid var(--border-strong);
    background: var(--surface); color: var(--text); font: 400 16px/1.3 var(--font); outline: none; /* 16 px: iOS won't zoom */
    -webkit-appearance: none; appearance: none;
}
input::placeholder { color: var(--dim); opacity: 1; }
input:focus, select:focus, textarea:focus { border-color: var(--focus); box-shadow: 0 0 0 3px var(--ring); }
input[aria-invalid=true], .pk-input-error { border-color: var(--err-line); }
.pk-err { color: var(--err); font-size: 14px; line-height: 1.4; }
.pk-hint { color: var(--mut); font-size: 14px; line-height: 1.5; margin: 0; }
/* Password with the eye: the button is 44x44 inside the field's right edge. */
.pk-pw { position: relative; }
.pk-pw input { padding-right: 52px; }
.pk-pw-toggle {
    position: absolute; right: 2px; top: 2px; width: 44px; height: 44px; margin: 0; padding: 0; border: 0; border-radius: 10px;
    background: transparent; color: var(--mut); cursor: pointer; display: flex; align-items: center; justify-content: center;
}
.pk-pw-toggle:hover { color: var(--text); }
.pk-pw-toggle svg { width: 22px; height: 22px; }
.pk-pw-toggle[aria-pressed=true] .pk-eye, .pk-pw-toggle:not([aria-pressed=true]) .pk-eye-off { display: none; }
.pk-check { display: flex; align-items: center; gap: 10px; min-height: 44px; color: var(--text2); font-size: 15px; font-weight: 400; line-height: 1.3; cursor: pointer; }
.pk-check input { width: 22px; height: 22px; accent-color: var(--mark); margin: 0; flex: none; }
.pk-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }

.pk-btn, input[type=submit], button[type=submit] {
    display: flex; align-items: center; justify-content: center; width: 100%; min-height: 52px; padding: 0 16px; border-radius: 12px;
    border: 1px solid var(--btn-bg); background: var(--btn-bg); color: var(--btn-fg); font: 700 17px/1 var(--font);
    cursor: pointer; text-decoration: none; -webkit-appearance: none; appearance: none;
}
.pk-btn:hover, input[type=submit]:hover, button[type=submit]:hover { filter: brightness(1.05); text-decoration: none; }
.pk-btn:active, button[type=submit]:active { filter: brightness(.95); }
.pk-btn:disabled, button[type=submit]:disabled { opacity: .55; cursor: progress; filter: none; }
.pk-btn-quiet, button.pk-btn-quiet[type=submit] { background: transparent; color: var(--text); border-color: var(--border-strong); }
a { color: var(--link); text-decoration: none; font-weight: 500; }
a:hover { text-decoration: underline; }
.pk-link { display: inline-flex; align-items: center; min-height: 44px; font-size: 15px; }
.pk-center { justify-content: center; text-align: center; }
:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
input:focus-visible { outline: none; }
.pk-foot { color: var(--dim); font-size: 13px; line-height: 1.5; text-align: center; margin: 0; }
/* base pages (parent=base) */
.pk-content [id^=kc-form-buttons], .pk-content .form-group { display: flex; flex-direction: column; gap: 6px; }
.pk-content ul { padding-left: 20px; color: var(--text2); }
.pk-content .instruction { margin: 0; color: var(--text2); }
@media (prefers-reduced-motion: reduce) { * { transition: none !important; animation: none !important; } }
    </style>
</head>
<body class="${bodyClass}">
<main class="pk">
    <div class="pk-brand" role="img" aria-label="Punchable">
        <svg class="pk-mark" viewBox="0 0 512 512" aria-hidden="true"><rect x="1" y="1" width="510" height="510" rx="112" fill="var(--tile)" stroke="var(--tile-line)" stroke-width="2"/><path fill="var(--mark)" opacity="0.35" d="M96 338a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H96z"/><path fill="var(--mark)" opacity="0.6" d="M192 258a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H192z"/><path fill="var(--mark)" opacity="0.85" d="M288 178a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H288z"/><path fill="var(--mark)" d="M384 98a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H384z"/></svg>
        <svg class="pk-word" viewBox="0 -72 492 94" aria-hidden="true"><path fill="currentColor" d="M7 20V-49.6H19.4V-43.6H21.2Q22.9 -46.5 26.5 -48.75Q30.1 -51 36.8 -51Q42.8 -51 47.9 -48.05Q53 -45.1 56.1 -39.4Q59.2 -33.7 59.2 -25.6V-24Q59.2 -15.9 56.1 -10.2Q53 -4.5 47.9 -1.55Q42.8 1.4 36.8 1.4Q32.3 1.4 29.25 0.35Q26.2 -0.7 24.35 -2.35Q22.5 -4 21.4 -5.7H19.6V20ZM33 -9.6Q38.9 -9.6 42.75 -13.35Q46.6 -17.1 46.6 -24.3V-25.3Q46.6 -32.5 42.7 -36.25Q38.8 -40 33 -40Q27.2 -40 23.3 -36.25Q19.4 -32.5 19.4 -25.3V-24.3Q19.4 -17.1 23.3 -13.35Q27.2 -9.6 33 -9.6ZM86.7 0.8Q80.9 0.8 76.55 -1.85Q72.2 -4.5 69.8 -9.2Q67.4 -13.9 67.4 -20V-49.6H80V-21Q80 -15.4 82.75 -12.6Q85.5 -9.8 90.6 -9.8Q96.4 -9.8 99.6 -13.65Q102.8 -17.5 102.8 -24.4V-49.6H115.4V0H103V-6.5H101.2Q100 -4 96.7 -1.6Q93.4 0.8 86.7 0.8ZM126.4 0V-49.6H138.8V-43.1H140.6Q141.8 -45.7 145.1 -48.05Q148.4 -50.4 155.1 -50.4Q160.9 -50.4 165.25 -47.75Q169.6 -45.1 172 -40.45Q174.4 -35.8 174.4 -29.6V0H161.8V-28.6Q161.8 -34.2 159.05 -37Q156.3 -39.8 151.2 -39.8Q145.4 -39.8 142.2 -35.95Q139 -32.1 139 -25.2V0ZM208.3 1.4Q201.1 1.4 195.25 -1.6Q189.4 -4.6 186 -10.3Q182.6 -16 182.6 -24.1V-25.5Q182.6 -33.6 186 -39.3Q189.4 -45 195.25 -48Q201.1 -51 208.3 -51Q215.4 -51 220.5 -48.5Q225.6 -46 228.75 -41.65Q231.9 -37.3 232.9 -31.8L220.7 -29.2Q220.3 -32.2 218.9 -34.6Q217.5 -37 214.95 -38.4Q212.4 -39.8 208.6 -39.8Q204.8 -39.8 201.75 -38.15Q198.7 -36.5 196.95 -33.25Q195.2 -30 195.2 -25.3V-24.3Q195.2 -19.6 196.95 -16.35Q198.7 -13.1 201.75 -11.45Q204.8 -9.8 208.6 -9.8Q214.3 -9.8 217.25 -12.75Q220.2 -15.7 221 -20.5L233.2 -17.6Q231.9 -12.3 228.75 -7.95Q225.6 -3.6 220.5 -1.1Q215.4 1.4 208.3 1.4ZM240.6 0V-70H253.2V-43.5H255Q255.8 -45.1 257.5 -46.7Q259.2 -48.3 262.05 -49.35Q264.9 -50.4 269.3 -50.4Q275.1 -50.4 279.45 -47.75Q283.8 -45.1 286.2 -40.45Q288.6 -35.8 288.6 -29.6V0H276V-28.6Q276 -34.2 273.25 -37Q270.5 -39.8 265.4 -39.8Q259.6 -39.8 256.4 -35.95Q253.2 -32.1 253.2 -25.2V0ZM314.6 1.4Q309.3 1.4 305.1 -0.45Q300.9 -2.3 298.45 -5.85Q296 -9.4 296 -14.5Q296 -19.6 298.45 -23.05Q300.9 -26.5 305.25 -28.25Q309.6 -30 315.2 -30H328.8V-32.8Q328.8 -36.3 326.6 -38.55Q324.4 -40.8 319.6 -40.8Q314.9 -40.8 312.6 -38.65Q310.3 -36.5 309.6 -33.1L298 -37Q299.2 -40.8 301.85 -43.95Q304.5 -47.1 308.95 -49.05Q313.4 -51 319.8 -51Q329.6 -51 335.3 -46.1Q341 -41.2 341 -31.9V-13.4Q341 -10.4 343.8 -10.4H347.8V0H339.4Q335.7 0 333.3 -1.8Q330.9 -3.6 330.9 -6.6V-6.7H329Q328.6 -5.5 327.2 -3.55Q325.8 -1.6 322.8 -0.1Q319.8 1.4 314.6 1.4ZM316.8 -8.8Q322.1 -8.8 325.45 -11.75Q328.8 -14.7 328.8 -19.6V-20.6H316.1Q312.6 -20.6 310.6 -19.1Q308.6 -17.6 308.6 -14.9Q308.6 -12.2 310.7 -10.5Q312.8 -8.8 316.8 -8.8ZM383.8 1.4Q377.1 1.4 373.5 -0.9Q369.9 -3.2 368.2 -6H366.4V0H354V-70H366.6V-43.9H368.4Q369.5 -45.7 371.35 -47.3Q373.2 -48.9 376.25 -49.95Q379.3 -51 383.8 -51Q389.8 -51 394.9 -48.05Q400 -45.1 403.1 -39.4Q406.2 -33.7 406.2 -25.6V-24Q406.2 -15.9 403.1 -10.2Q400 -4.5 394.9 -1.55Q389.8 1.4 383.8 1.4ZM380 -9.6Q385.8 -9.6 389.7 -13.35Q393.6 -17.1 393.6 -24.3V-25.3Q393.6 -32.5 389.75 -36.25Q385.9 -40 380 -40Q374.2 -40 370.3 -36.25Q366.4 -32.5 366.4 -25.3V-24.3Q366.4 -17.1 370.3 -13.35Q374.2 -9.6 380 -9.6ZM414.8 0V-70H427.4V0ZM461 1.4Q453.6 1.4 447.95 -1.75Q442.3 -4.9 439.15 -10.65Q436 -16.4 436 -24.2V-25.4Q436 -33.2 439.1 -38.95Q442.2 -44.7 447.8 -47.85Q453.4 -51 460.8 -51Q468.1 -51 473.5 -47.75Q478.9 -44.5 481.9 -38.75Q484.9 -33 484.9 -25.4V-21.1H448.8Q449 -16 452.6 -12.8Q456.2 -9.6 461.4 -9.6Q466.7 -9.6 469.2 -11.9Q471.7 -14.2 473 -17L483.3 -11.6Q481.9 -9 479.25 -5.95Q476.6 -2.9 472.2 -0.75Q467.8 1.4 461 1.4ZM448.9 -30.5H472.1Q471.7 -34.8 468.65 -37.4Q465.6 -40 460.7 -40Q455.6 -40 452.6 -37.4Q449.6 -34.8 448.9 -30.5Z"/></svg>
    </div>

    <header class="pk-head">
        <#if pageTitle??>
            <h1>${pageTitle}</h1>
        <#else>
            <h1><#nested "header"></h1>
        </#if>
        <#if pageSubtitle??><p class="pk-sub">${pageSubtitle?no_esc}</p></#if>
    </header>

    <div class="pk-card">
        <#if displayMessage && message?has_content && (message.type != 'warning' || !isAppInitiatedAction??)>
            <div class="pk-alert pk-alert-${message.type}" role="<#if message.type == 'error'>alert<#else>status</#if>">${kcSanitize(message.summary)?no_esc}</div>
        </#if>

        <#nested "socialProviders">

        <div class="pk-content">
            <#nested "form">
        </div>

        <#if displayInfo>
            <#nested "info">
        </#if>
    </div>

    <p class="pk-foot">Punchable is one comic's notebook. There's no sign-up.</p>
</main>
<script>
(function () {
    // Password eye: swaps the field's type; the label says what the button will do next.
    var eyes = document.querySelectorAll('[data-pw-toggle]');
    for (var i = 0; i < eyes.length; i++) {
        eyes[i].addEventListener('click', function () {
            var f = document.getElementById(this.getAttribute('aria-controls'));
            if (!f) return;
            var show = f.type === 'password';
            f.type = show ? 'text' : 'password';
            this.setAttribute('aria-pressed', show ? 'true' : 'false');
            this.setAttribute('aria-label', show ? 'Hide password' : 'Show password');
        });
    }
    // Once-only submit: after the form data is built (next tick), the buttons go quiet so a double tap can't resend.
    var forms = document.querySelectorAll('form[data-once]');
    for (var j = 0; j < forms.length; j++) {
        forms[j].addEventListener('submit', function (e) {
            var btns = this.querySelectorAll('button[type=submit]'), s = e.submitter;
            setTimeout(function () {
                for (var k = 0; k < btns.length; k++) {
                    btns[k].disabled = true;
                    if (btns[k] === s && s.getAttribute('data-busy')) s.textContent = s.getAttribute('data-busy');
                }
            }, 0);
        });
    }
})();
</script>
</body>
</html>
</#macro>

<#-- The eye on a password field: `field` is the input's id. The inline script above makes it work; without JS it is inert
     and the field is still a plain password box. -->
<#macro pwToggle field>
<button type="button" class="pk-pw-toggle" data-pw-toggle aria-controls="${field}" aria-pressed="false" aria-label="Show password">
    <svg class="pk-eye" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/></svg>
    <svg class="pk-eye-off" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 3l18 18"/><path d="M10.6 10.6a3 3 0 0 0 4.2 4.2"/><path d="M9.9 5.2A10.4 10.4 0 0 1 12 5c6.5 0 10 7 10 7a17.4 17.4 0 0 1-3.2 4.2"/><path d="M6.6 6.6A17.3 17.3 0 0 0 2 12s3.5 7 10 7a10 10 0 0 0 4.4-1"/></svg>
</button>
</#macro>
