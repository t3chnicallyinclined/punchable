import { Stack, router, usePathname } from 'expo-router';
import * as SplashScreen from 'expo-splash-screen';
import { StatusBar } from 'expo-status-bar';
import { useEffect } from 'react';
import { ActivityIndicator, StyleSheet, Text, View } from 'react-native';
import { GestureRoot } from '@/lib/GestureRoot';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { useFonts, Inter_400Regular, Inter_500Medium, Inter_600SemiBold } from '@expo-google-fonts/inter';
import { SpaceGrotesk_500Medium, SpaceGrotesk_600SemiBold, SpaceGrotesk_700Bold } from '@expo-google-fonts/space-grotesk';
import { leaving, useSession } from '@/lib/auth'; // importing it registers the bearer with api.ts; off on a local API
import { isPublicPath, loginHref } from '@/lib/session';
import { SignInBtn, UIProvider, useUI } from '@/lib/ui';

import { WhyProvider } from '@/lib/why';
import { BoardProvider } from '@/lib/boards';
import { C, F } from '@/lib/theme';
import * as Photos from '@/lib/photos';
import { SET_MIN_S } from '@/lib/importgroup';

SplashScreen.preventAutoHideAsync().catch(() => {});

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

export default function RootLayout() {
  const [loaded, error] = useFonts({ Inter_400Regular, Inter_500Medium, Inter_600SemiBold, SpaceGrotesk_500Medium, SpaceGrotesk_600SemiBold, SpaceGrotesk_700Bold });
  const ready = loaded || !!error;
  useEffect(() => {
    if (ready) SplashScreen.hideAsync().catch(() => {});
  }, [ready]);
  if (!ready) return null;
  const tab = { animation: 'none' as const };
  return (
    <GestureRoot style={{ backgroundColor: C.bg }}>
      <SafeAreaProvider>
        <StatusBar style="light" />
        <UIProvider>
          <WhyProvider>
            <BoardProvider>
              <PhotosBadge />
              <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: C.bg } }}>
                <Stack.Screen name="index" options={{ ...tab, title: 'Write · Punchable' }} />
                <Stack.Screen name="boards" options={{ ...tab, title: 'Boards · Punchable' }} />
                <Stack.Screen name="notebook" options={{ ...tab, title: 'Notebook · Punchable' }} />
                <Stack.Screen name="desk" options={{ ...tab, title: 'Notebook · Punchable' }} />
                <Stack.Screen name="shows" options={{ title: 'Shows · Punchable' }} />
                <Stack.Screen name="today" options={{ title: 'After a show · Punchable' }} />
                <Stack.Screen name="joke/[id]" options={{ title: 'Joke · Punchable' }} />
                <Stack.Screen name="bit/[id]" options={{ title: 'The bit · Punchable' }} />
                <Stack.Screen name="set/[id]" options={{ title: 'Set · Punchable' }} />
                <Stack.Screen name="venue/[id]" options={{ title: 'Venue · Punchable' }} />
                <Stack.Screen name="board/[id]" options={{ title: 'Storyboard · Punchable' }} />
                <Stack.Screen name="workingset/[id]" options={{ title: 'Working set · Punchable' }} />
                <Stack.Screen name="myset/[id]" options={{ title: 'Set · Punchable' }} />
                <Stack.Screen name="setlist/[id]" options={{ animation: 'slide_from_bottom', title: 'Set list · Punchable' }} />
                <Stack.Screen name="import" options={{ animation: 'slide_from_bottom', title: 'Import sets · Punchable' }} />
                <Stack.Screen name="settings" options={{ title: 'Settings · Punchable' }} />
                <Stack.Screen name="profile" options={{ title: 'You · Punchable' }} />
                <Stack.Screen name="learned" options={{ title: 'What I’ve learned · Punchable' }} />
                <Stack.Screen name="notes" options={{ title: 'Notes · Punchable' }} />
                <Stack.Screen name="ask/index" options={{ title: 'Ask · Punchable' }} />
                <Stack.Screen name="ask/[id]" options={{ title: 'Ask · Punchable' }} />
                <Stack.Screen name="review" options={{ animation: 'slide_from_bottom', title: 'Step through · Punchable' }} />
                <Stack.Screen name="login" options={{ animation: 'none', title: 'Sign in · Punchable' }} />
                <Stack.Screen name="record" options={{ animation: 'slide_from_bottom', gestureEnabled: false, title: 'Record · Punchable' }} />
              </Stack>
              <AuthGate />
            </BoardProvider>
          </WhyProvider>
        </UIProvider>
      </SafeAreaProvider>
    </GestureRoot>
  );
}
