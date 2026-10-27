"""Static checks on the Punchable login theme (infra/keycloak/theme/login). No Keycloak, no Docker: runs anywhere.
   python infra/keycloak/test_theme_static.py          exit 0 = all pass
   python infra/keycloak/test_theme_static.py --red    mutates a copy (registration link, unbalanced #if, ?html, no
                                                       Google button) and REQUIRES each mutation to be caught.
A real render needs FreeMarker inside Keycloak: see test-theme.sh (throwaway Keycloak; NOT on the server while the
apiservice-kicker restarts MicroK8s on Docker events, infra/DEPLOY_LOG.md 2026-10-26)."""
import os, re, sys, shutil, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
THEME = os.path.join(HERE, 'theme', 'login')
PAGES = ['login.ftl', 'login-reset-password.ftl', 'login-verify-email.ftl', 'login-update-password.ftl',
         'login-idp-link-confirm.ftl', 'login-idp-link-email.ftl', 'error.ftl', 'info.ftl', 'login-page-expired.ftl']
PAIRS = [('<#if', '</#if>'), ('<#list', '</#list>'), ('<#macro', '</#macro>'), ('<@layout.registrationLayout', '</@layout.registrationLayout>')]


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
        if name != 'template.ftl':
            need('<#import "template.ftl" as layout>' in body, f'{name}: imports template.ftl')
        for m in re.finditer(r'\$\{([^}]*)\}', body):
            expr = m.group(1)
            # a bare dotted lookup that may be null must have a default or be inside a known-safe object
            if re.fullmatch(r'(user|brokerContext|auth|client)\.\w+', expr.strip()):
                fails.append(f'{name}: ${{{expr}}} has no default; write (x.y)!\'\'')
    t = files.get('template.ftl', '')
    need('rest...' in t, 'template.ftl: registrationLayout catches extra params (rest...)')
    for sec in ('"form"', '"socialProviders"', '"info"', '"header"'):
        need(f'<#nested {sec}>' in t, f'template.ftl: renders section {sec}')
    need('kcSanitize(message.summary)' in t, 'template.ftl: message summary sanitized')
    lg = files.get('login.ftl', '')
    need('p.alias == "google"' in lg and 'Continue with Google' in lg and '${p.loginUrl}' in lg, 'login.ftl: Continue with Google uses the IdP loginUrl')
    need('usernameHidden??' in lg, 'login.ftl: re-auth for Google linking (usernameHidden) handled')
    need('loginResetCredentialsUrl' in lg, 'login.ftl: forgot password link')
    msgs = files.get('messages_en.properties', '')
    need('federatedIdentityUnavailableMessage=' in msgs, 'messages: Google-not-linked error in plain words')
    for line in msgs.splitlines():
        if line and not line.startswith('#') and '=' in line:
            v = line.split('=', 1)[1]
            need(not re.search(r"(?<!')'(?!')", v), f"messages: single quote must be doubled for MessageFormat: {line[:50]}")
            need(not re.search(r'(?i)\b(oops|uh oh|whoops)\b|!$', v), f'messages: not cute about errors: {line[:50]}')
    return fails


def main():
    if '--red' in sys.argv:
        muts = [('login.ftl', '</#if>', '', 'unbalanced #if'),
                ('login.ftl', 'Continue with Google', 'Sign in with G', 'Google button text'),
                ('login-reset-password.ftl', 'Back to sign in', '${url.registrationUrl}', 'registration link'),
                ('login-verify-email.ftl', "${(user.email)!'your email'}", '${user.email?html}', '?html'),
                ('messages_en.properties', "don''t match", "don't match", 'single quote')]
        bad = 0
        for f, a, b, what in muts:
            tmp = tempfile.mkdtemp(); d = os.path.join(tmp, 'login'); shutil.copytree(THEME, d)
            p = os.path.join(d, f); s = open(p, encoding='utf8').read(); assert a in s, (f, a)
            open(p, 'w', encoding='utf8').write(s.replace(a, b, 1))
            caught = bool(check(d)); shutil.rmtree(tmp)
            print(('RED OK   ' if caught else 'RED MISS ') + what); bad += not caught
        sys.exit(1 if bad else 0)
    fails = check(THEME)
    for f in fails:
        print('FAIL', f)
    print(f'{"ALL PASS" if not fails else str(len(fails)) + " FAILED"}: theme static checks over {len(PAGES) + 1} templates')
    sys.exit(1 if fails else 0)


if __name__ == '__main__':
    main()
