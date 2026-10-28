"""Static checks on the Punchable login theme (infra/keycloak/theme/login). No Keycloak, no Docker: runs anywhere.
   python infra/keycloak/test_theme_static.py          exit 0 = all pass
   python infra/keycloak/test_theme_static.py --red    mutates a copy (a missing autocomplete, an error without
                                                       aria-describedby, an external script, no light scheme, a
                                                       contrast that fails, 14 px inputs, ...) and REQUIRES each
                                                       mutation to be caught.
What it pins (the rules the theme was built to): FreeMarker shape (balanced directives, no ?html, nulls defaulted,
every page in our layout), the sign-in flow (Google only through the IdP's loginUrl and only when the realm offers it,
forgot-password link, no registration link, the base theme's field names and error ids), the phone sheet (16 px inputs so
iOS doesn't zoom, 44 pt targets, the right autocomplete/inputmode/autocapitalize, no autofocus over the Google button),
accessibility (labels bound to inputs, aria-describedby on every invalid field, a password eye on every password,
focus rings) and the look (dark AND light via prefers-color-scheme, every text/background pair >= 4.5:1, no external
JS or CSS). A real render needs FreeMarker inside Keycloak (test-realm.sh, a throwaway Keycloak; never on the server);
the mock-ups are theme_mockups.mjs."""
import os, re, sys, shutil, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
THEME = os.path.join(HERE, 'theme', 'login')
PAGES = ['login.ftl', 'login-reset-password.ftl', 'login-verify-email.ftl', 'login-update-password.ftl',
         'login-idp-link-confirm.ftl', 'login-idp-link-email.ftl', 'error.ftl', 'info.ftl', 'login-page-expired.ftl']
PAIRS = [('<#if', '</#if>'), ('<#list', '</#list>'), ('<#macro', '</#macro>'), ('<@layout.registrationLayout', '</@layout.registrationLayout>')]
# text / background pairs that must read in both schemes (WCAG AA, 4.5:1)
CONTRAST = [('text', 'bg'), ('text2', 'bg'), ('mut', 'bg'), ('dim', 'bg'), ('link', 'bg'),
            ('text', 'surface'), ('text2', 'surface'), ('mut', 'surface'), ('dim', 'surface'),
            ('text', 'card'), ('text2', 'card'), ('mut', 'card'), ('dim', 'card'), ('link', 'card'),
            ('btn-fg', 'btn-bg'), ('g-fg', 'g-bg'), ('err', 'err-bg'), ('err', 'bg'), ('err', 'card'),
            ('warn', 'warn-bg'), ('ok', 'ok-bg'), ('info', 'info-bg')]
MIN_TARGET = 44


def lum(hex6):
    r, g, b = (int(hex6[i:i + 2], 16) / 255 for i in (1, 3, 5))
    f = lambda c: c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)


def contrast(a, b):
    la, lb = lum(a), lum(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)


def tokens(block):
    return {m.group(1): m.group(2).lower() for m in re.finditer(r'--([a-z0-9-]+):\s*(#[0-9a-fA-F]{6})\b', block)}


def css_rule(css, selector_start):
    """The declaration block of the first rule whose selector list starts with selector_start."""
    m = re.search(re.escape(selector_start) + r'[^{]*\{([^}]*)\}', css)
    return m.group(1) if m else ''


def px(decl, prop):
    m = re.search(prop + r':\s*(\d+)px', decl)
    return int(m.group(1)) if m else None


def check(d):
    fails = []
    def need(c, m):
        if not c: fails.append(m)
    files = {f: open(os.path.join(d, f), encoding='utf8').read() for f in os.listdir(d)}
    need(files.get('theme.properties', '').find('parent=base') >= 0, 'theme.properties: parent=base')
    for p in PAGES + ['template.ftl']:
        need(p in files, f'{p} exists')
    for name, s in files.items():
        if not name.endswith('.ftl'):
            continue
        body = re.sub(r'<#--.*?-->', '', s, flags=re.S)
        for a, b in PAIRS:
            na = len(re.findall(re.escape(a) + r'[\s>]', body)); nb = body.count(b)
            need(na == nb, f'{name}: {a} x{na} vs {b} x{nb}')
        need('?html' not in body, f'{name}: ?html is not allowed (Keycloak auto-escapes; use ${{}} or ?no_esc)')
        need('registrationUrl' not in body, f'{name}: no registration link (registrationAllowed: false)')
        need('<script src' not in body and not re.search(r'<link[^>]*stylesheet', body), f'{name}: no external JS or CSS')
        if name != 'template.ftl':
            need('<#import "template.ftl" as layout>' in body, f'{name}: imports template.ftl')
        for m in re.finditer(r'\$\{([^}]*)\}', body):
            expr = m.group(1)
            # a bare dotted lookup that may be null must have a default or be inside a known-safe object
            if re.fullmatch(r'(user|brokerContext|auth|client)\.\w+', expr.strip()):
                fails.append(f'{name}: ${{{expr}}} has no default; write (x.y)!\'\'')
        # ---- inputs: the phone-sheet attributes, labels, errors, the eye
        ids = set(re.findall(r'\bid="([^"$]+)"', body))
        # FreeMarker directives inside a tag (<#if bad>aria-invalid=...</#if>) would end the tag early: blank them
        flat = re.sub(r'</?#[^>]*>', ' ', body)
        for tag in re.findall(r'<input\b[^>]*>', flat):
            t = re.search(r'type="?(\w+)', tag); typ = t.group(1) if t else 'text'
            iid = re.search(r'\bid="([^"]+)"', tag); iid = iid.group(1) if iid else '?'
            if typ in ('hidden', 'checkbox', 'submit'):
                continue
            need('autocomplete="' in tag, f'{name}: input #{iid} has no autocomplete')
            if typ == 'email':
                need('inputmode="email"' in tag and 'autocapitalize="none"' in tag and 'spellcheck="false"' in tag,
                     f'{name}: email input #{iid} needs inputmode=email, autocapitalize=none, spellcheck=false')
            if typ == 'password':
                need('autocapitalize="none"' in tag, f'{name}: password input #{iid} needs autocapitalize=none')
                need(f'<@layout.pwToggle field="{iid}"/>' in body, f'{name}: password input #{iid} has no eye (layout.pwToggle)')
            if 'aria-invalid="true"' in tag:
                m = re.search(r'aria-describedby="([^"]+)"', tag)
                need(m is not None, f'{name}: input #{iid} is marked invalid without aria-describedby')
                if m:
                    need(m.group(1) in ids, f'{name}: aria-describedby="{m.group(1)}" points at no element')
        for lab in re.findall(r'<label for="([^"]+)"', body):
            need(lab in ids, f'{name}: label for="{lab}" has no input')
        for form in re.findall(r'<form\b[^>]*>', body):
            need('data-once' in form, f'{name}: form without data-once (double submit)')
            need('${url.loginAction}' in form, f'{name}: form must post to url.loginAction')
    t = files.get('template.ftl', '')
    need('rest...' in t, 'template.ftl: registrationLayout catches extra params (rest...)')
    for sec in ('"form"', '"socialProviders"', '"info"', '"header"'):
        need(f'<#nested {sec}>' in t, f'template.ftl: renders section {sec}')
    need('kcSanitize(message.summary)' in t, 'template.ftl: message summary sanitized')
    need('<#macro pwToggle field>' in t and 'data-pw-toggle' in t and 'aria-controls="${field}"' in t, 'template.ftl: the password eye macro')
    need('width=device-width' in t and 'viewport-fit=cover' in t, 'template.ftl: viewport for the phone sheet')
    need(re.search(r':focus-visible\s*\{[^}]*outline:\s*2px', t) is not None, 'template.ftl: visible focus ring (:focus-visible with a 2px outline)')
    need('<meta name="color-scheme" content="dark light">' in t, 'template.ftl: declares both colour schemes')
    css = t[t.find('<style>'):t.find('</style>')]
    dark = re.search(r':root\s*\{(.*?)\}', css, re.S)
    light = re.search(r'@media \(prefers-color-scheme: light\)\s*\{\s*:root\s*\{(.*?)\}', css, re.S)
    need(dark is not None, 'template.ftl: :root tokens')
    need(light is not None, 'template.ftl: a light scheme under prefers-color-scheme')
    for scheme, blk in (('dark', dark), ('light', light)):
        if not blk:
            continue
        tk = tokens(blk.group(1))
        for fg, bg in CONTRAST:
            if fg not in tk or bg not in tk:
                fails.append(f'template.ftl ({scheme}): token --{fg} or --{bg} missing or not a 6-digit hex')
                continue
            c = contrast(tk[fg], tk[bg])
            need(c >= 4.5, f'template.ftl ({scheme}): --{fg} on --{bg} is {c:.2f}:1, needs 4.5:1')
    inp = css_rule(css, 'input[type=text]')
    need(px(inp, 'font: 400') == 16 or re.search(r'font:[^;]*\b16px', inp), 'template.ftl: inputs are 16 px (iOS zooms below that)')
    need((px(inp, 'min-height') or 0) >= MIN_TARGET, f'template.ftl: inputs are at least {MIN_TARGET} px tall')
    for sel, props in (('.pk-pw-toggle', ('width', 'height')), ('.pk-link', ('min-height',)), ('.pk-check', ('min-height',)),
                       ('.pk-btn, input[type=submit]', ('min-height',)), ('.pk-google', ('min-height',))):
        r = css_rule(css, sel)
        for prop in props:
            v = px(r, prop)
            need(v is not None and v >= MIN_TARGET, f'template.ftl: {sel} {prop} is {v}, needs >= {MIN_TARGET} px (tap target)')
    lg = files.get('login.ftl', '')
    need('social?? && social.providers?has_content' in lg, 'login.ftl: Google only when the realm offers an IdP')
    need('p.alias == "google"' in lg and 'Sign in with Google' in lg and re.search(r'id="social-google"[^>]*href="\$\{p\.loginUrl\}"', lg) is not None,
         'login.ftl: Sign in with Google uses the IdP loginUrl')
    need('usernameHidden??' in lg, 'login.ftl: re-auth for IdP linking (usernameHidden) handled')
    need('loginResetCredentialsUrl' in lg, 'login.ftl: forgot password link')
    need('name="rememberMe"' in lg and 'realm.rememberMe' in lg, 'login.ftl: remember me')
    need('name="credentialId"' in lg, 'login.ftl: credentialId hidden field (base theme contract)')
    um = re.search(r'<input id="username"[^>]*>', lg)
    need(um is not None and 'autofocus' not in um.group(0), 'login.ftl: no autofocus on email (the keyboard would cover the Google button in the sheet)')
    up = files.get('login-update-password.ftl', '')
    for f in ('name="password-new"', 'name="password-confirm"', 'name="logout-sessions"', 'name="cancel-aia"', 'id="input-error-password"', 'id="input-error-password-confirm"'):
        need(f in up, f'login-update-password.ftl: {f} (base theme contract)')
    ve = files.get('login-verify-email.ftl', '')
    need(re.search(r'<form[^>]*method="post"', ve) is not None and 'verifyEmail??' in ve, 'login-verify-email.ftl: resend is a POST form, verifyEmail handled')
    need('?resend=true' not in files.get('login-idp-link-email.ftl', ''), 'login-idp-link-email.ftl: resend is the plain loginAction link')
    msgs = files.get('messages_en.properties', '')
    need('federatedIdentityUnavailableMessage=' in msgs, 'messages: Google-not-linked error in plain words')
    for line in msgs.splitlines():
        if line and not line.startswith('#') and '=' in line:
            v = line.split('=', 1)[1]
            need(not re.search(r"(?<!')'(?!')", v), f"messages: single quote must be doubled for MessageFormat: {line[:50]}")
            need(not re.search(r'(?i)\b(oops|uh oh|whoops)\b|!$', v), f'messages: not cute about errors: {line[:50]}')
    return fails


MUTATIONS = [
    ('login.ftl', '</#if>', '', 'unbalanced #if'),
    ('login.ftl', 'href="${p.loginUrl}"', 'href="#"', 'Google button without the IdP loginUrl'),
    ('login.ftl', 'social?? && social.providers?has_content', 'social??', 'Google shown without checking the realm offers it'),
    ('login-reset-password.ftl', 'Back to sign in', '${url.registrationUrl}', 'registration link'),
    ('login-verify-email.ftl', "${(user.email)!'your email address'}", '${user.email?html}', '?html'),
    ('messages_en.properties', "don''t match", "don't match", 'single quote'),
    ('login.ftl', 'autocomplete="username" ', '', 'email input without autocomplete'),
    ('login.ftl', 'inputmode="email" ', '', 'email input without inputmode'),
    ('login-reset-password.ftl', ' aria-describedby="input-error-username"', '', 'invalid field without aria-describedby'),
    ('login-update-password.ftl', '<@layout.pwToggle field="password-confirm"/>', '', 'password without the eye'),
    ('login.ftl', '<input id="username" name="username"', '<input id="username" autofocus name="username"', 'autofocus on the email field'),
    ('login-reset-password.ftl', ' data-once', '', 'form without once-only submit'),
    ('template.ftl', '</head>', '<script src="https://example.com/x.js"></script></head>', 'external script'),
    ('template.ftl', '@media (prefers-color-scheme: light)', '@media (min-width: 99999px)', 'no light scheme'),
    ('template.ftl', '--mut: #A1A1A8;', '--mut: #55555a;', 'dark muted text below 4.5:1'),
    ('template.ftl', '--mut: #5d6169;', '--mut: #9a9ea6;', 'light muted text below 4.5:1'),
    ('template.ftl', 'font: 400 16px/1.3 var(--font)', 'font: 400 14px/1.3 var(--font)', '14 px inputs (iOS zoom)'),
    ('template.ftl', 'width: 44px; height: 44px; margin: 0;', 'width: 32px; height: 32px; margin: 0;', 'eye under 44 pt'),
    ('template.ftl', ':focus-visible', ':focus-visiblex', 'no focus ring'),
    ('login-idp-link-email.ftl', 'href="${url.loginAction}"', 'href="${url.loginAction}?resend=true"', 'idp-link-email resend with the old query'),
]


def main():
    if '--red' in sys.argv:
        bad = 0
        for f, a, b, what in MUTATIONS:
            tmp = tempfile.mkdtemp(); d = os.path.join(tmp, 'login'); shutil.copytree(THEME, d)
            p = os.path.join(d, f); s = open(p, encoding='utf8').read(); assert a in s, (f, a)
            open(p, 'w', encoding='utf8').write(s.replace(a, b, 1))
            caught = check(d); shutil.rmtree(tmp)
            print(('RED OK   ' if caught else 'RED MISS ') + what + (f'  <- {caught[0]}' if caught else '')); bad += not caught
        print(f'{len(MUTATIONS) - bad}/{len(MUTATIONS)} mutations caught')
        sys.exit(1 if bad else 0)
    fails = check(THEME)
    for f in fails:
        print('FAIL', f)
    print(f'{"ALL PASS" if not fails else str(len(fails)) + " FAILED"}: theme static checks over {len(PAGES) + 1} templates')
    sys.exit(1 if fails else 0)


if __name__ == '__main__':
    main()
