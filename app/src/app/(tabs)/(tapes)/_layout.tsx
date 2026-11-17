// The Tapes tab's stack (SPEC 2026-11-16 Slice 6): every set, video and memo he has recorded in one list, then a
// performed set (/set/x, moved here from Sets: a recorded night is a tape). /filmed redirects here. `anchor` gives a cold
// deep link (/set/x) a Back to Tapes.
import React from 'react';
import { Stack } from 'expo-router';
import { Platform } from 'react-native';
import { C } from '@/lib/theme';

export const unstable_settings = { anchor: 'tapes' }; // expo-router 58: `initialRouteName` is deprecated for `anchor` (layoutAnchor.js)
const t = (s: string) => (Platform.OS === 'web' ? `${s} · Punchable` : s);

export default function TapesStack() {
  return (
    <Stack screenOptions={{ headerShown: false, headerTintColor: C.amber, contentStyle: { backgroundColor: C.bg } }}>
      <Stack.Screen name="tapes" options={{ title: t('Tapes') }} />
      <Stack.Screen name="set/[id]" options={{ title: t('Set') }} />
      <Stack.Screen name="filmed" options={{ animation: 'none', title: t('Tapes') }} />
    </Stack>
  );
}
