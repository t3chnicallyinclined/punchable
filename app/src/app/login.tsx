// Sign in (hosted only). The screen before Keycloak's page (infra/keycloak/theme, auth.punchable.io), matched to it:
// the lockup, one line of what Punchable is, and the way in.
//   Phone: ONE button, "Sign in". It opens Keycloak's page in the in-app browser sheet (expo-web-browser auth session,
//          redirect punchable://auth/callback); that page offers Google and email. No Google button in the native
//          app (owner decision; App Store guideline 4.8 would then require Sign in with Apple as an equal option).
//   Web:   the same, plus "Sign in with Google" (kc_idp_hint=google: Keycloak hands straight to Google) when the realm
//          offers the IdP (EXPO_PUBLIC_OIDC_GOOGLE=0 hides it). The email button is then "Sign in with email".
// Both are the same authorization-code + PKCE request to Keycloak (src/lib/auth.ts, unchanged).
import * as AuthSession from 'expo-auth-session';
import { router, useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import { ActivityIndicator, Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import Svg, { Path } from 'react-native-svg';
import { Lockup } from '@/lib/Brand';
import { AUTH_ENABLED, ISSUER, authRequestConfig, completeSignIn, getAccessToken, rememberWay } from '@/lib/auth';
import { safeNext } from '@/lib/session';
import { C, F } from '@/lib/theme';

const BASE = authRequestConfig();
const GOOGLE = authRequestConfig({ kc_idp_hint: 'google' });
/** Google is a web-only shortcut; the realm offers it (realm-punchable.yaml identityProviders) unless the build says no. */
const GOOGLE_OFFERED = Platform.OS === 'web' && (process.env as Record<string, string | undefined>).EXPO_PUBLIC_OIDC_GOOGLE !== '0';
const PITCH = 'Every bit you’ve written, and how long they laughed.';

type Way = 'google' | 'email';

/** Errors in plain words: what happened, what to do. */
function plain(msg: string | undefined | null): string {
  const m = (msg || '').toLowerCase();
  if (m.includes('access_denied') || m.includes('cancel') || m.includes('dismiss')) return 'Sign-in was cancelled. Try again when you’re ready.';
  if (m.includes('network') || m.includes('fetch')) return 'Can’t reach the sign-in server. Check your connection and try again.';
  return msg ? `Sign-in didn’t finish: ${msg}. Try again.` : 'Sign-in didn’t finish. Try again.';
}

export default function Login() {
  const insets = useSafeAreaInsets();
  // /login?next=/joke/… : the page he asked for before signing in (only a path inside this app; else Today)
  const next = safeNext(useLocalSearchParams<{ next?: string }>().next);
  const discovery = AuthSession.useAutoDiscovery(ISSUER);
  const [gRequest, gResult, gPrompt] = AuthSession.useAuthRequest(GOOGLE, discovery);
  const [eRequest, eResult, ePrompt] = AuthSession.useAuthRequest(BASE, discovery);
  const [way, setWay] = useState<Way>(GOOGLE_OFFERED ? 'google' : 'email');
  const request = way === 'google' ? gRequest : eRequest;
  const result = way === 'google' ? gResult : eResult;
  const [failed, setFailed] = useState<{ for: unknown; msg: string } | null>(null);
  const [cleared, setCleared] = useState<unknown>(null);
  const [popupErr, setPopupErr] = useState<string | null>(null);
  const busy = result?.type === 'success' && failed?.for !== result;
  const ready = !!eRequest && (!GOOGLE_OFFERED || !!gRequest);
  const error =
    popupErr ??
    (cleared === result ? null : failed?.for === result ? failed.msg : result?.type === 'error' ? plain(result.error?.message || result.error?.code) : null);

  // Already signed in (a stored refresh token still works), or no auth at all (local dev): go straight in.
  useEffect(() => {
    if (!AUTH_ENABLED) {
      router.replace(next as any);
      return;
    }
    getAccessToken().then((t) => {
      if (t) router.replace(next as any);
    });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (result?.type !== 'success' || !result.params.code) return;
    completeSignIn(result.params.code, request?.codeVerifier)
      .then(() => {
        rememberWay(way);
        router.replace(next as any);
      })
      .catch((e) => setFailed({ for: result, msg: plain(e?.message) }));
  }, [result, request]); // eslint-disable-line react-hooks/exhaustive-deps

  const go = (w: Way) => {
    setWay(w);
    setCleared(w === 'google' ? gResult : eResult);
    setPopupErr(null);
    (w === 'google' ? gPrompt : ePrompt)().catch((e) =>
      setPopupErr(/popup/i.test(String(e?.message)) ? 'Your browser blocked the sign-in window. Allow pop-ups for this site and try again.' : plain(e?.message)),
    );
  };
  const off = !ready || busy;
  // Colours are read here, at render: the tree remounts on a scheme change (theme.ts) and the palette follows.
  const light = C.scheme === 'light';
  const ink = light ? '#111214' : '#0B0B0D'; // text on the amber button, both schemes (theme.ts: amber with ink text)
  const amber = light ? '#f5b544' : C.amber;

  return (
    <ScrollView
      testID="login"
      style={{ flex: 1, backgroundColor: C.bg }}
      accessibilityLabel="Sign in"
      contentContainerStyle={[s.page, { paddingTop: insets.top + 40, paddingBottom: insets.bottom + 24, paddingLeft: insets.left + 20, paddingRight: insets.right + 20 }]}
      keyboardShouldPersistTaps="handled">
      <View style={s.col}>
        <Lockup size={44} />
        <View style={{ gap: 8 }}>
          <Text style={[s.h1, { color: C.text }]} accessibilityRole="header" maxFontSizeMultiplier={1.4}>
            Sign in
          </Text>
          <Text style={[s.pitch, { color: C.mut2 }]} testID="login-pitch">
            {PITCH}
          </Text>
          {next !== '/' ? (
            <Text style={[s.note2, { color: C.dim }]} testID="login-next">
              You’ll go straight back to the page you opened.
            </Text>
          ) : null}
        </View>

        <View style={{ gap: 12 }}>
          {GOOGLE_OFFERED ? (
            <Pressable
              testID="login-google"
              accessibilityRole="button"
              accessibilityLabel="Sign in with Google"
              accessibilityState={{ disabled: off, busy: busy && way === 'google' }}
              disabled={off}
              onPress={() => go('google')}
              style={({ pressed }) => [s.google, light ? s.googleLight : s.googleDark, off && s.off, pressed && { opacity: 0.85 }]}>
              {busy && way === 'google' ? (
                <ActivityIndicator color={light ? '#1F1F1F' : '#E3E3E3'} />
              ) : (
                <>
                  <GoogleG />
                  <Text style={[s.googleText, { color: light ? '#1F1F1F' : '#E3E3E3' }]}>Sign in with Google</Text>
                </>
              )}
            </Pressable>
          ) : null}
          <Pressable
            testID="login-button"
            accessibilityRole="button"
            accessibilityLabel={GOOGLE_OFFERED ? 'Sign in with email' : 'Sign in'}
            accessibilityHint="Opens the sign-in page"
            accessibilityState={{ disabled: off, busy: busy && way === 'email' }}
            disabled={off}
            onPress={() => go('email')}
            style={({ pressed }) => [s.primary, { backgroundColor: amber }, off && s.off, pressed && { opacity: 0.9 }]}>
            {busy && way === 'email' ? <ActivityIndicator color={ink} /> : <Text style={[s.primaryText, { color: ink }]}>{GOOGLE_OFFERED ? 'Sign in with email' : 'Sign in'}</Text>}
          </Pressable>
          {!ready && !error ? <Text style={[s.note, { color: C.mut2 }]}>Reaching the sign-in server…</Text> : null}
          {error ? (
            <View style={[s.err, { borderColor: C.dangerBorder, backgroundColor: light ? '#fdecec' : '#2a1210' }]} accessibilityRole="alert" testID="login-error">
              <Text style={[s.errText, { color: C.redSoft }]}>{error}</Text>
            </View>
          ) : null}
        </View>

        <Text style={[s.foot, { color: C.dim }]}>Punchable is one comic’s notebook. There’s no sign-up.</Text>
      </View>
    </ScrollView>
  );
}

function GoogleG() {
  return (
    <Svg width={22} height={22} viewBox="0 0 24 24" aria-hidden>
      <Path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z" />
      <Path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" />
      <Path fill="#FBBC05" d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l3.66-2.84z" />
      <Path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" />
    </Svg>
  );
}

// Same measures as the Keycloak page: 52 pt buttons, 12 pt radius, 28/800 title, muted 15 pt pitch, 13 pt footer.
// Colours are applied in render (above) so a scheme change shows; only shape lives here.
const s = StyleSheet.create({
  page: { flexGrow: 1, alignItems: 'center' },
  col: { width: '100%', maxWidth: 400, gap: 24 },
  h1: { fontFamily: F.black, fontSize: 28, lineHeight: 33, letterSpacing: -0.5 },
  pitch: { fontFamily: F.body, fontSize: 15, lineHeight: 22 },
  // Google's own button in their dark / light variant (brand rules), same as the Keycloak page.
  google: { minHeight: 52, borderRadius: 12, borderWidth: 1, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 12, paddingHorizontal: 16 },
  googleDark: { backgroundColor: '#131314', borderColor: '#8E918F' },
  googleLight: { backgroundColor: '#ffffff', borderColor: '#747775' },
  googleText: { fontFamily: F.semi, fontSize: 17 },
  // The amber button: ink text on amber in both schemes (theme.ts: buttons stay amber with ink text).
  primary: { minHeight: 52, borderRadius: 12, alignItems: 'center', justifyContent: 'center', paddingHorizontal: 16 },
  primaryText: { fontFamily: F.bold, fontSize: 17 },
  off: { opacity: 0.55 },
  note: { fontFamily: F.body, fontSize: 14, textAlign: 'center' },
  err: { borderWidth: 1, borderRadius: 12, padding: 12 },
  errText: { fontFamily: F.body, fontSize: 15, lineHeight: 21 },
  note2: { fontFamily: F.body, fontSize: 14, lineHeight: 20 },
  foot: { fontFamily: F.body, fontSize: 13, lineHeight: 19, textAlign: 'center' },
});
