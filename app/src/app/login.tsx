// Sign in (hosted only). Look: docs/DESIGN.md §9, matching the Keycloak theme on auth.punchable.io.
// Two ways in, both the same authorization-code + PKCE request to Keycloak (src/lib/auth.ts, unchanged):
//   Continue with Google  adds kc_idp_hint=google, so Keycloak hands straight to Google. The owner's normal way in:
//                         his first Google login auto-links to his account (realm flow pk-google-link-only).
//   Use email             no hint: Keycloak's own page (password, or "Forgot password?" to set one).
import * as AuthSession from 'expo-auth-session';
import { router, useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import Svg, { Path } from 'react-native-svg';
import { Lockup } from '@/lib/Brand';
import { AUTH_ENABLED, ISSUER, authRequestConfig, completeSignIn, getAccessToken, rememberWay } from '@/lib/auth';
import { safeNext } from '@/lib/session';
import { C, F } from '@/lib/theme';

const BASE = authRequestConfig();
const GOOGLE = authRequestConfig({ kc_idp_hint: 'google' });

type Way = 'google' | 'email';

/** Errors in plain words (DESIGN.md §2): what happened, what to do. */
function plain(msg: string | undefined | null): string {
  const m = (msg || '').toLowerCase();
  if (m.includes('access_denied') || m.includes('cancel')) return 'Sign-in was cancelled. Try again when you’re ready.';
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
  const [way, setWay] = useState<Way>('google');
  const request = way === 'google' ? gRequest : eRequest;
  const result = way === 'google' ? gResult : eResult;
  const [failed, setFailed] = useState<{ for: unknown; msg: string } | null>(null);
  const [cleared, setCleared] = useState<unknown>(null);
  const [popupErr, setPopupErr] = useState<string | null>(null);
  const busy = result?.type === 'success' && failed?.for !== result;
  const ready = !!gRequest && !!eRequest;
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

  return (
    <ScrollView
      testID="login"
      style={{ flex: 1, backgroundColor: C.bg }}
      contentContainerStyle={[s.page, { paddingTop: insets.top + 40, paddingBottom: insets.bottom + 24, paddingLeft: insets.left + 20, paddingRight: insets.right + 20 }]}
      keyboardShouldPersistTaps="handled">
      <View style={s.col}>
        <Lockup size={52} />
        <View style={{ gap: 10 }}>
          <Text style={s.h1} accessibilityRole="header">Sign in</Text>
          <Text style={s.pitch} testID="login-pitch">Every bit you&apos;ve written, and how long they laughed.</Text>
          {next !== '/' ? <Text style={s.note2} testID="login-next">You&apos;ll go straight back to the page you opened.</Text> : null}
        </View>

        <View style={{ gap: 12 }}>
          <Pressable
            testID="login-google"
            accessibilityRole="button"
            accessibilityLabel="Continue with Google"
            accessibilityState={{ disabled: !ready || busy, busy: busy && way === 'google' }}
            disabled={!ready || busy}
            onPress={() => go('google')}
            style={({ pressed }) => [s.google, (!ready || busy) && s.off, pressed && { backgroundColor: '#e6e6e6' }]}>
            {busy && way === 'google' ? (
              <ActivityIndicator color="#1f1f1f" />
            ) : (
              <>
                <GoogleG />
                <Text style={s.googleText}>Continue with Google</Text>
              </>
            )}
          </Pressable>
          <Pressable
            testID="login-button"
            accessibilityRole="button"
            accessibilityLabel="Use email"
            accessibilityHint="Opens the email and password sign-in page"
            accessibilityState={{ disabled: !ready || busy, busy: busy && way === 'email' }}
            disabled={!ready || busy}
            onPress={() => go('email')}
            style={({ pressed }) => [s.email, (!ready || busy) && s.off, pressed && { backgroundColor: C.surface2 }]}>
            {busy && way === 'email' ? <ActivityIndicator color={C.text} /> : <Text style={s.emailText}>Use email</Text>}
          </Pressable>
          {!ready && !error ? <Text style={s.note}>Reaching the sign-in server…</Text> : null}
          {error ? (
            <View style={s.err} accessibilityRole="alert" testID="login-error">
              <Text style={s.errText}>{error}</Text>
            </View>
          ) : null}
        </View>

        <Text style={s.foot}>Use email only if you've set a Punchable password.</Text>
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

const s = StyleSheet.create({
  page: { flexGrow: 1, alignItems: 'center' },
  col: { width: '100%', maxWidth: 400, gap: 28 },
  h1: { color: C.text, fontFamily: F.g7, fontSize: 30, lineHeight: 35, letterSpacing: -0.6 },
  pitch: { color: C.mut, fontFamily: F.body, fontSize: 17, lineHeight: 24 },
  // Google's own light button: white, the four-colour G (their brand rules). 52 pt.
  google: { minHeight: 52, borderRadius: 12, backgroundColor: '#ffffff', flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 12, paddingHorizontal: 16 },
  googleText: { color: '#1f1f1f', fontFamily: F.semi, fontSize: 17 },
  email: { minHeight: 52, borderRadius: 12, borderWidth: 1, borderColor: C.borderStrong, alignItems: 'center', justifyContent: 'center', paddingHorizontal: 16 },
  emailText: { color: C.text, fontFamily: F.g7, fontSize: 16, letterSpacing: 0.3 },
  off: { opacity: 0.5 },
  note: { color: C.mut, fontFamily: F.body, fontSize: 14, textAlign: 'center' },
  err: { borderWidth: 1, borderColor: C.dangerBorder, backgroundColor: 'rgba(239,68,68,.08)', borderRadius: 12, padding: 12 },
  errText: { color: C.redSoft, fontFamily: F.body, fontSize: 15, lineHeight: 21 },
  note2: { color: C.dim, fontFamily: F.body, fontSize: 14, lineHeight: 20 },
  foot: { color: C.dim, fontFamily: F.body, fontSize: 13, lineHeight: 19, textAlign: 'center' },
});
