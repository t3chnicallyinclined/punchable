// The Rant tab's stack (SPEC 2026-11-06 R1: the first tab, Tonight's place): Rant, then Settings, You and the kept
// routes Today / Learned. The inbox, the conversation (Ask your material), Ask and Import moved to the Bits stack with
// "N to sort" and the search they are reached from (R2), so no tap there switches tabs under him. URLs are unchanged:
// a group's name never appears in one. `anchor` gives a cold deep link (/settings) a Back to Rant.
import React from 'react';
import { Stack } from 'expo-router';
import { Platform } from 'react-native';
import { C } from '@/lib/theme';

export const unstable_settings = { anchor: 'index' }; // expo-router 58: `initialRouteName` is deprecated for `anchor` (layoutAnchor.js)
const t = (s: string) => (Platform.OS === 'web' ? `${s} · Punchable` : s);

export default function RantStack() {
  return (
    <Stack screenOptions={{ headerShown: false, headerTintColor: C.amber, contentStyle: { backgroundColor: C.bg } }}>
      <Stack.Screen name="index" options={{ title: t('Rant') }} />
      {/* the creative space (SPEC 2026-11-06 §12.7): a topic is a page, its script a push, a bit opened from it stays here */}
      <Stack.Screen name="new-topic" options={{ title: t('New topic') }} />
      <Stack.Screen name="topic/[topic]/index" options={{ title: t('Topic') }} />
      <Stack.Screen name="topic/[topic]/script" options={{ title: t('The rant') }} />
      <Stack.Screen name="topic/[topic]/bit/[id]" options={{ title: t('Joke') }} />
      <Stack.Screen name="filmed" options={{ title: t('Filmed') }} />
      <Stack.Screen name="settings" options={{ title: t('Settings') }} />
      {/* his voice profile: a form sheet on the phone (HIG/sheets: the grabber, swipe down to dismiss, full height for the editor); a page on the web */}
      <Stack.Screen name="voice" options={{ title: t('Voice profile'), ...(Platform.OS === 'web' ? {} : { presentation: 'formSheet', sheetAllowedDetents: [1.0], sheetGrabberVisible: true, sheetExpandsWhenScrolledToEdge: true }) }} />
      <Stack.Screen name="profile" options={{ title: t('You') }} />
      <Stack.Screen name="today" options={{ title: t('After a show') }} />
      <Stack.Screen name="learned" options={{ title: t('What I’ve learned') }} />
    </Stack>
  );
}
