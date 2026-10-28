import { Stack, router, usePathname } from 'expo-router';
import * as SplashScreen from 'expo-splash-screen';
import { StatusBar } from 'expo-status-bar';
import { useEffect, useState } from 'react';
import { ActivityIndicator, Appearance, Platform, StyleSheet, Text, View, useColorScheme } from 'react-native';
import { GestureRoot } from '@/lib/GestureRoot';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { useFonts, Inter_400Regular, Inter_500Medium, Inter_600SemiBold, Inter_700Bold, Inter_800ExtraBold } from '@expo-google-fonts/inter';
import { SpaceGrotesk_500Medium, SpaceGrotesk_600SemiBold, SpaceGrotesk_700Bold } from '@expo-google-fonts/space-grotesk';
import { leaving, useSession } from '@/lib/auth'; // importing it registers the bearer with api.ts; off on a local API
import { isPublicPath, loginHref } from '@/lib/session';
import { SignInBtn, UIProvider, useUI } from '@/lib/ui';

import { WhyProvider } from '@/lib/why';
import { BoardProvider } from '@/lib/boards';
import { C, F, resolveScheme, setScheme } from '@/lib/theme';
import { getPrefs, loadPrefs, onPrefs } from '@/lib/prefs';
import * as Photos from '@/lib/photos';
import { SET_MIN_S } from '@/lib/importgroup';
import { installCrashReporter, setCrashContext } from '@/lib/crashReport';
import { CrashScreen } from '@/lib/CrashScreen';
import type { ErrorBoundaryProps } from 'expo-router';

SplashScreen.preventAutoHideAsync().catch(() => {});

/**
 * A throw anywhere in the tree ends on a readable screen instead of in RCTFatal. expo-router wraps the root route in
 * its Try boundary when the layout exports ErrorBoundary (expo-router/build/useScreens.js, fromImport); nested routes
 * render inside this layout's Stack, so their render errors propagate here too (React: the nearest boundary above).
 * No providers or fonts are assumed: the layout itself may be what failed. The record is written for the next launch.
 */
export function ErrorBoundary({ error, retry }: ErrorBoundaryProps) {
  return <CrashScreen error={error} retry={retry} />;
}
// A fatal JS error in a release build ends the process (RN: RCTExceptionsManager.reportFatal -> RCTFatal). This writes
// the error to the phone first, so the next launch shows it on /login under "Last crash" (lib/crashReport.ts).
installCrashReporter();

/** Every crash record carries where the app was. */
function CrashContext() {
  const pathname = usePathname();
  useEffect(() => {
    setCrashContext(() => ({ path: pathname }));
  }, [pathname]);
  return null;
}

/** The document title on the web; the native header title on the phone. */
const t = (s: string) => (Platform.OS === 'web' ? `${s} · Punchable` : s);
/** A full-screen modal on the phone (slides up, covers the tab bar); a plain route on the web. */
const fullScreen = Platform.OS === 'web' ? { animation: 'slide_from_bottom' as const } : { animation: 'slide_from_bottom' as const, presentation: 'fullScreenModal' as const, autoHideHomeIndicator: true };
/** A form sheet on the phone (HIG/sheets: the grabber, swipe anywhere to dismiss, the bar with Cancel leading); a plain route on the web, which draws the Modal sheet itself. */
const sheet = (detents: number[] | 'fitToContents') => (Platform.OS === 'web' ? {} : {
  presentation: 'formSheet' as const, sheetAllowedDetents: detents, sheetGrabberVisible: true, sheetInitialDetentIndex: 0, sheetExpandsWhenScrolledToEdge: true, headerShown: true, headerTintColor: C.amber,
});

/** Native: the Capture badge = set-length videos from the last 14 days (only once he has granted Photos access). */
function PhotosBadge() {
  const ui = useUI();
  useEffect(() => {
    if (!Photos.supportsLibrary) return;
    let alive = true;
    (async () => {
      try {
        const a = await Photos.access();
        if (a !== 'granted' && a !== 'limited') return;
        const v = await Photos.recentVideos(14, SET_MIN_S);
        if (alive) ui.setImportCount(v.filter((c) => c.duration >= SET_MIN_S && !c.imported).length);
      } catch {}
    })();
    return () => {
      alive = false;
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  return null;
}

/**
 * Hosted: every page but /login, /auth/callback, /privacy and /terms needs him signed in. A signed-out visitor who opens
 * a deep link (punchable.io/joke/…) goes to /login?next=/joke/… and lands back on that page after signing in.
 * While the stored sign-in is being checked, the page is covered so nothing private flashes.
 */
function AuthGate() {
  const sess = useSession();
  const pathname = usePathname();
  const open = !sess.enabled || isPublicPath(pathname);
  useEffect(() => {
    if (open || sess.checking || sess.signedIn || leaving) return;
    router.replace(loginHref(pathname, ((globalThis as any).location?.search as string) || '') as any);
  }, [open, sess.checking, sess.signedIn, pathname]);
  if (open || sess.signedIn) return null;
  return (
    <View testID="auth-gate" style={[StyleSheet.absoluteFill, { zIndex: 50, backgroundColor: C.bg, alignItems: 'center', justifyContent: 'center', gap: 16, padding: 24 }]}>
      {sess.checking ? (
        <ActivityIndicator color={C.amber} accessibilityLabel="Checking your sign-in" />
      ) : (
        <>
          <Text style={{ color: C.text, fontFamily: F.g7, fontSize: 20 }}>You’re signed out</Text>
          <SignInBtn testID="gate-signin" />
        </>
      )}
    </View>
  );
}

/** Appearance: the system scheme, overridden per device from Settings (System / Dark / Light). The tokens are
 *  swapped in place (theme.setScheme) and the tree remounts under a new key so every static style follows. */
function useAppearance() {
  const system = useColorScheme();
  const [prefsReady, setPrefsReady] = useState(Platform.OS === 'web'); // the phone reads its prefs file first
  const [pref, setPref] = useState(getPrefs().appearance);
  useEffect(() => {
    loadPrefs().then((p) => { setPref(p.appearance); setPrefsReady(true); });
    return onPrefs((p) => setPref(p.appearance));
  }, []);
  const scheme = resolveScheme(pref, system);
  setScheme(scheme); // during render on purpose: the children read C.* in this same pass
  useEffect(() => {
    // the override reaches UIKit too (native headers, the tab bar, form sheets, menus, action sheets); System = follow the OS
    if (Platform.OS !== 'web' && typeof Appearance.setColorScheme === 'function') Appearance.setColorScheme(pref === 'system' ? 'unspecified' : pref);
  }, [pref]);
  return { scheme, prefsReady };
}

export default function RootLayout() {
  const [loaded, error] = useFonts({ Inter_400Regular, Inter_500Medium, Inter_600SemiBold, Inter_700Bold, Inter_800ExtraBold, SpaceGrotesk_500Medium, SpaceGrotesk_600SemiBold, SpaceGrotesk_700Bold });
  const { scheme, prefsReady } = useAppearance();
  const ready = (loaded || !!error) && prefsReady;
  useEffect(() => {
    if (ready) SplashScreen.hideAsync().catch(() => {});
  }, [ready]);
  if (!ready) return null;
  return (
    <GestureRoot key={scheme} style={{ backgroundColor: C.bg }}>
      <SafeAreaProvider>
        <StatusBar style={scheme === 'light' ? 'dark' : 'light'} />
        <UIProvider>
          <WhyProvider>
            <BoardProvider>
              <PhotosBadge />
              <CrashContext />
              <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: C.bg } }}>
                {/* The three tabs, each its own stack: app/(tabs). Route groups keep every URL (/joke/x, /list/x, /inbox, ...). */}
                <Stack.Screen name="(tabs)" options={{ animation: 'none' }} />
                {/* Full-screen tasks (HIG/modality: "a full-screen modal style for ... a complex task"); nothing swipes him off the stage. */}
                <Stack.Screen name="stage/[id]" options={{ ...fullScreen, gestureEnabled: false, title: t('On stage') }} />
                <Stack.Screen name="record" options={{ ...fullScreen, gestureEnabled: false, title: t('Record') }} />
                <Stack.Screen name="review" options={{ ...fullScreen, title: t('Step through') }} />
                {/* Short tasks as form sheets: "+" at the medium detent (the menu fits in half; "New set list" expands), Add a joke at the large one */}
                <Stack.Screen name="create" options={{ ...sheet([0.5, 1]), title: t('New') }} />
                <Stack.Screen name="add/[board]" options={{ ...sheet([1]), title: t('Add a joke') }} />
                <Stack.Screen name="login" options={{ animation: 'none', title: 'Sign in · Punchable' }} />
                <Stack.Screen name="auth/callback" options={{ animation: 'none', title: 'Signing in · Punchable' }} />
                <Stack.Screen name="auth/logout" options={{ animation: 'none', title: 'Signed out · Punchable' }} />
              </Stack>
              <AuthGate />
            </BoardProvider>
          </WhyProvider>
        </UIProvider>
      </SafeAreaProvider>
    </GestureRoot>
  );
}
