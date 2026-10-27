<#-- Punchable login theme (realm punchable, auth.punchable.io). (flat dir,
     one ConfigMap, copied into /opt/keycloak/themes/punchable/login by the shared keycloak Deployment's setup-theme init
     container, mounted by hand). Look: docs/DESIGN.md "Backstage Amber" + §9 Sign-in.
     CSS is inline (Keycloak's resource versioning caches theme CSS hard). Pages that are not ours (parent=base) still render
     inside this layout, so the generic form rules at the bottom of the CSS style them too.
     `rest...` catches any extra layout parameter a base page passes (showAnotherWayIfPresent, displayWide, ...). -->
<#macro registrationLayout bodyClass="" displayInfo=false displayMessage=true displayRequiredFields=false rest...>
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="robots" content="noindex, nofollow">
    <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
    <meta name="theme-color" content="#111214">
    <meta name="color-scheme" content="dark">
    <title>${pageTitle!"Sign in"} · Punchable</title>
    <link rel="icon" type="image/svg+xml" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 512 512'%3E%3Crect width='512' height='512' rx='96' fill='%23111214'/%3E%3Cpath fill='%23f5b544' opacity='0.35' d='M96 338a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H96z'/%3E%3Cpath fill='%23f5b544' opacity='0.6' d='M192 258a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H192z'/%3E%3Cpath fill='%23f5b544' opacity='0.85' d='M288 178a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H288z'/%3E%3Cpath fill='%23f5b544' d='M384 98a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H384z'/%3E%3C/svg%3E">
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=Space+Grotesk:wght@600;700&display=swap" rel="stylesheet">
    <style>
/* Tokens: app/src/lib/theme.ts (C). Contrast table: docs/DESIGN.md §3. */
:root {
    --bg: #111214; --surface: #1a1c20; --surface2: #22252a; --border: #2d3139; --border-strong: #4b5059;
    --text: #f3f4f6; --text2: #d1d5db; --mut: #9ca3af; --dim: #878d97;
    --amber: #f5b544; --amber-soft: rgba(245,181,68,.15); --red-soft: #fca5a5; --danger-border: #7f1d1d; --green: #22c55e;
    --blue: #38bdf8;
    --display: 'Space Grotesk', ui-sans-serif, system-ui, -apple-system, 'Segoe UI', sans-serif;
    --body: 'Inter', ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif;
    color-scheme: dark;
}
*, *::before, *::after { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body {
    margin: 0; min-height: 100vh; min-height: 100dvh; background: var(--bg); color: var(--text);
    font: 16px/1.45 var(--body); -webkit-font-smoothing: antialiased;
    display: flex; justify-content: center; align-items: flex-start;
    padding: max(32px, env(safe-area-inset-top)) max(16px, env(safe-area-inset-right)) max(24px, env(safe-area-inset-bottom)) max(16px, env(safe-area-inset-left));
}
@media (min-width: 600px) { body { align-items: center; } }
.pk { width: 100%; max-width: 400px; display: flex; flex-direction: column; gap: 24px; }
.pk-brand { display: flex; align-items: center; gap: 12px; }
.pk-brand svg { display: block; }
.pk-mark { width: 44px; height: 44px; flex: none; }
.pk-head h1 { font: 700 28px/1.15 var(--display); letter-spacing: -0.02em; margin: 0; text-wrap: balance; }
.pk-sub { margin: 8px 0 0; color: var(--mut); font-size: 15px; line-height: 1.5; }
.pk-sub b { color: var(--text); font-weight: 600; }
.pk-card { display: flex; flex-direction: column; gap: 16px; }

/* Alerts: plain words, above the form. Error in red-soft on a red-tinted card; never a joke (DESIGN.md §2). */
.pk-alert { border-radius: 12px; padding: 12px 14px; font-size: 15px; line-height: 1.45; border: 1px solid var(--border); background: var(--surface); color: var(--text2); }
.pk-alert-error { border-color: var(--danger-border); background: rgba(239,68,68,.08); color: var(--red-soft); }
.pk-alert-warning { border-color: #5c4a1f; background: #241e12; color: var(--amber); }
.pk-alert-success { border-color: rgba(34,197,94,.45); background: rgba(34,197,94,.08); color: #86efac; }
.pk-alert-info { border-color: #1e3a52; background: rgba(56,189,248,.08); color: #bae6fd; }

/* Continue with Google: the big one. Google's own light button (white, the four-colour G), 52 px. */
.pk-google {
    display: flex; align-items: center; justify-content: center; gap: 12px; width: 100%; min-height: 52px; padding: 0 16px;
    border-radius: 12px; background: #ffffff; color: #1f1f1f; border: 1px solid #ffffff; text-decoration: none;
    font: 600 17px/1 var(--body); letter-spacing: 0.01em;
}
.pk-google:hover { background: #f2f2f2; }
.pk-google:active { background: #e6e6e6; }
.pk-google svg { width: 22px; height: 22px; flex: none; }
.pk-social-other { background: var(--surface2); color: var(--text); border-color: var(--border-strong); }

.pk-or { display: flex; align-items: center; gap: 12px; color: var(--mut); font: 700 12px/1 var(--display); letter-spacing: .08em; text-transform: uppercase; }
.pk-or::before, .pk-or::after { content: ""; flex: 1; height: 1px; background: var(--border); }

form { display: flex; flex-direction: column; gap: 16px; margin: 0; }
.pk-field { display: flex; flex-direction: column; gap: 6px; }
label, .pk-label { font: 600 14px/1.3 var(--body); color: var(--text2); }
input[type=text], input[type=email], input[type=password], input[type=tel], input[type=number], select, textarea {
    width: 100%; min-height: 48px; padding: 12px 14px; border-radius: 12px; border: 1px solid var(--border-strong);
    background: var(--surface); color: var(--text); font: 400 16px/1.3 var(--body); outline: none; /* 16 px: iOS won't zoom */
}
input::placeholder { color: var(--dim); }
input:focus, select:focus, textarea:focus { border-color: var(--amber); box-shadow: 0 0 0 3px rgba(245,181,68,.25); }
input[aria-invalid=true], .pk-input-error { border-color: #ef4444; }
.pk-err { color: var(--red-soft); font-size: 14px; }
.pk-hint { color: var(--mut); font-size: 13px; }
.pk-check { display: flex; align-items: center; gap: 10px; min-height: 44px; color: var(--text2); font: 400 15px/1.3 var(--body); cursor: pointer; }
.pk-check input { width: 20px; height: 20px; accent-color: var(--amber); margin: 0; }
.pk-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }

.pk-btn, input[type=submit], button[type=submit] {
    display: flex; align-items: center; justify-content: center; width: 100%; min-height: 52px; padding: 0 16px; border-radius: 12px;
    border: 1px solid var(--amber); background: var(--amber); color: var(--bg); font: 700 16px/1 var(--display); letter-spacing: .02em;
    cursor: pointer; text-decoration: none; -webkit-appearance: none; appearance: none;
}
.pk-btn:hover, input[type=submit]:hover, button[type=submit]:hover { filter: brightness(1.06); }
.pk-btn:disabled { opacity: .5; cursor: progress; }
.pk-btn-quiet { background: transparent; color: var(--text); border-color: var(--border-strong); }
a, .pk-link { color: var(--amber); text-decoration: none; font-weight: 500; }
a:hover, .pk-link:hover { text-decoration: underline; }
.pk-link { display: inline-flex; align-items: center; min-height: 44px; }
:focus-visible { outline: 2px solid var(--amber); outline-offset: 2px; }
.pk-foot { color: var(--dim); font-size: 13px; line-height: 1.5; text-align: center; }
.pk-center { text-align: center; }
/* base pages (parent=base) */
.pk-content [id^=kc-form-buttons], .pk-content .form-group { display: flex; flex-direction: column; gap: 6px; }
.pk-content ul { padding-left: 20px; color: var(--text2); }
@media (prefers-reduced-motion: reduce) { * { transition: none !important; animation: none !important; } }
    </style>
</head>
<body class="${bodyClass}">
<main class="pk">
    <div class="pk-brand" aria-label="Punchable">
        <svg class="pk-mark" viewBox="0 0 512 512" aria-hidden="true"><rect width="512" height="512" rx="112" fill="#1a1c20"/><path fill="#f5b544" opacity="0.35" d="M96 338a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H96z"/><path fill="#f5b544" opacity="0.6" d="M192 258a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H192z"/><path fill="#f5b544" opacity="0.85" d="M288 178a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H288z"/><path fill="#f5b544" d="M384 98a22 22 0 0 1 22 -22h20a22 22 0 0 1 22 22V436H384z"/></svg>
        <span style="font: 700 24px/1 var(--display); letter-spacing: -0.03em; color: var(--text);">punchable</span>
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

    <p class="pk-foot">Punchable is a private notebook. Only its owner has an account.</p>
</main>
</body>
</html>
</#macro>
