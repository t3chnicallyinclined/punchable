// The Sets tab's stack: Sets, then a set list (/list/x), a topic from the old Rant tab (/topic/x and its rant script,
// until it is a set: SPEC 2026-11-16 Slice 6), and the kept routes (the old
// storyboard redirect /board/x, the set builder, Boards, My set, Working set, the old set list). `initialRouteName`
// gives a cold deep link (/list/x) a Back to Sets.
import React from 'react';
import { Stack } from 'expo-router';
import { Platform } from 'react-native';
import { C } from '@/lib/theme';

export const unstable_settings = { anchor: 'sets' }; // expo-router 58: `initialRouteName` is deprecated for `anchor` (layoutAnchor.js)
const t = (s: string) => (Platform.OS === 'web' ? `${s} · Punchable` : s);

export default function SetsStack() {
  return (
    <Stack screenOptions={{ headerShown: false, headerTintColor: C.amber, contentStyle: { backgroundColor: C.bg } }}>
      <Stack.Screen name="sets" options={{ title: t('Sets') }} />
      <Stack.Screen name="list/[id]" options={{ title: t('Set list') }} />
      <Stack.Screen name="archived" options={{ title: t('Archived') }} />
      {/* the Rant tab's topics (Slice 6): a topic made into a set opens that set; one not yet made into a set opens as before */}
      <Stack.Screen name="topic/[topic]/index" options={{ title: t('Topic') }} />
      <Stack.Screen name="topic/[topic]/script" options={{ title: t('The rant') }} />
      <Stack.Screen name="topic/[topic]/bit/[id]" options={{ title: t('Joke') }} />
      <Stack.Screen name="new-topic" options={{ title: t('New topic') }} />
      <Stack.Screen name="today" options={{ title: t('After a show') }} />
      <Stack.Screen name="board/[id]" options={{ title: t('Storyboard') }} />
      <Stack.Screen name="builder/[id]" options={{ title: t('Set builder') }} />
      <Stack.Screen name="boards" options={{ title: t('Boards') }} />
      <Stack.Screen name="myset/[id]" options={{ title: t('Set') }} />
      <Stack.Screen name="workingset/[id]" options={{ title: t('Working set') }} />
      <Stack.Screen name="setlist/[id]" options={{ animation: 'slide_from_bottom', title: t('Set list') }} />
    </Stack>
  );
}
