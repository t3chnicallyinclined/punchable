// The Tonight tab's stack: Tonight, then everything one tap down from it (the inbox, the conversation, Settings, You,
// the kept routes Today / Import / Learned / Ask). `initialRouteName` gives a cold deep link (/inbox, /say) a Back to
// Tonight. Headers: the custom bar for now (lib/Screen); the native header arrives with the next step (HIG.md B3).
import React from 'react';
import { Stack } from 'expo-router';
import { Platform } from 'react-native';
import { C } from '@/lib/theme';

export const unstable_settings = { initialRouteName: 'index' };
const t = (s: string) => (Platform.OS === 'web' ? `${s} · Punchable` : s);

export default function TonightStack() {
  return (
    <Stack screenOptions={{ headerShown: false, headerTintColor: C.amber, contentStyle: { backgroundColor: C.bg } }}>
      <Stack.Screen name="index" options={{ title: t('Tonight') }} />
      <Stack.Screen name="inbox" options={{ title: t('Inbox') }} />
      <Stack.Screen name="say" options={{ title: t('Asking') }} />
      <Stack.Screen name="settings" options={{ title: t('Settings') }} />
      <Stack.Screen name="profile" options={{ title: t('You') }} />
      <Stack.Screen name="today" options={{ title: t('After a show') }} />
      <Stack.Screen name="import" options={{ animation: 'slide_from_bottom', title: t('Import sets') }} />
      <Stack.Screen name="learned" options={{ title: t('What I’ve learned') }} />
      <Stack.Screen name="ask/index" options={{ title: t('Ask') }} />
      <Stack.Screen name="ask/[id]" options={{ title: t('Ask') }} />
    </Stack>
  );
}
