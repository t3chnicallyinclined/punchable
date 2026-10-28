// The Sets tab's stack: Sets, then a set list (/list/x), a performed set (/set/x), and the kept routes (the old
// storyboard redirect /board/x, the set builder, Boards, My set, Working set, the old set list). `initialRouteName`
// gives a cold deep link (/list/x) a Back to Sets.
import React from 'react';
import { Stack } from 'expo-router';
import { Platform } from 'react-native';
import { C } from '@/lib/theme';

export const unstable_settings = { initialRouteName: 'sets' };
const t = (s: string) => (Platform.OS === 'web' ? `${s} · Punchable` : s);

export default function SetsStack() {
  return (
    <Stack screenOptions={{ headerShown: false, headerTintColor: C.amber, contentStyle: { backgroundColor: C.bg } }}>
      <Stack.Screen name="sets" options={{ title: t('Sets') }} />
      <Stack.Screen name="list/[id]" options={{ title: t('Set list') }} />
      <Stack.Screen name="set/[id]" options={{ title: t('Set') }} />
      <Stack.Screen name="board/[id]" options={{ title: t('Storyboard') }} />
      <Stack.Screen name="builder/[id]" options={{ title: t('Set builder') }} />
      <Stack.Screen name="boards" options={{ title: t('Boards') }} />
      <Stack.Screen name="myset/[id]" options={{ title: t('Set') }} />
      <Stack.Screen name="workingset/[id]" options={{ title: t('Working set') }} />
      <Stack.Screen name="setlist/[id]" options={{ animation: 'slide_from_bottom', title: t('Set list') }} />
    </Stack>
  );
}
