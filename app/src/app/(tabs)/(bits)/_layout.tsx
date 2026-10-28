// The Bits tab's stack: Bits, then the bit page, Merge review, the all-bits map, and the kept routes (the old joke
// map and workspace, Notes, Notebook, Desk, Shows, Venue, Calendar, Write). `initialRouteName` gives a cold deep
// link (/joke/x from a universal link) a Back to Bits.
import React from 'react';
import { Stack } from 'expo-router';
import { Platform } from 'react-native';
import { C } from '@/lib/theme';

export const unstable_settings = { anchor: 'bits' }; // expo-router 58: `initialRouteName` is deprecated for `anchor` (layoutAnchor.js)
const t = (s: string) => (Platform.OS === 'web' ? `${s} · Punchable` : s);

export default function BitsStack() {
  return (
    <Stack screenOptions={{ headerShown: false, headerTintColor: C.amber, contentStyle: { backgroundColor: C.bg } }}>
      <Stack.Screen name="bits" options={{ title: t('Bits') }} />
      <Stack.Screen name="joke/[id]" options={{ title: t('Joke') }} />
      <Stack.Screen name="merge" options={{ title: t('Merge') }} />
      <Stack.Screen name="map/index" options={{ title: t('Map') }} />
      <Stack.Screen name="map/[id]" options={{ title: t('Joke map') }} />
      <Stack.Screen name="bit/[id]" options={{ title: t('The bit') }} />
      <Stack.Screen name="notes" options={{ title: t('Notes') }} />
      <Stack.Screen name="notebook" options={{ title: t('Notebook') }} />
      <Stack.Screen name="desk" options={{ title: t('Notebook') }} />
      <Stack.Screen name="shows" options={{ title: t('Shows') }} />
      <Stack.Screen name="venue/[id]" options={{ title: t('Venue') }} />
      <Stack.Screen name="calendar" options={{ title: t('Calendar') }} />
      <Stack.Screen name="write" options={{ title: t('Write') }} />
    </Stack>
  );
}
