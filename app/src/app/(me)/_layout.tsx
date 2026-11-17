// The stack behind the avatar (SPEC 2026-11-16 Slice 6): You, Settings, Voice profile, What it has learned, Usage. It
// lived inside the Rant tab until the tabs became Bits · Sets · Tapes; now it is one stack at the root, presented over
// whichever tab he is on (a page sheet on the phone, swipe down to close), so the avatar opens the same place from every
// tab. URLs are unchanged (/profile, /settings, /voice, /learned, /usage): a group's name never appears in one. `anchor`
// gives a cold deep link (/settings) a Back to You.
import React from 'react';
import { Stack } from 'expo-router';
import { Platform } from 'react-native';
import { C } from '@/lib/theme';

export const unstable_settings = { anchor: 'profile' }; // expo-router 58: `initialRouteName` is deprecated for `anchor`
const t = (s: string) => (Platform.OS === 'web' ? `${s} · Punchable` : s);

export default function MeStack() {
  return (
    <Stack screenOptions={{ headerShown: false, headerTintColor: C.amber, contentStyle: { backgroundColor: C.bg } }}>
      <Stack.Screen name="profile" options={{ title: t('You') }} />
      <Stack.Screen name="settings" options={{ title: t('Settings') }} />
      {/* his voice profile: a form sheet on the phone (HIG/sheets: the grabber, swipe down to dismiss, full height for the editor); a page on the web */}
      <Stack.Screen name="voice" options={{ title: t('Voice profile'), ...(Platform.OS === 'web' ? {} : { presentation: 'formSheet', sheetAllowedDetents: [1.0], sheetGrabberVisible: true, sheetExpandsWhenScrolledToEdge: true }) }} />
      <Stack.Screen name="learned" options={{ title: t('What I’ve learned') }} />
      {/* SPEC 2026-11-17 §13: what Punchable costs and how much it is used (owner only on the server) */}
      <Stack.Screen name="usage" options={{ title: t('Usage') }} />
    </Stack>
  );
}
