// The one stack (docs/design/SPEC-2026-11-17-one-notebook.md §3): Notebook at `/`, then every page pushed from it. The
// three tabs' stacks ((tabs)/(bits), (sets), (tapes)) became this one group; a group's name never appears in a URL, so
// every deep link keeps working. `anchor` gives a cold deep link (/joke/x from a universal link) a Back to Notebook.
// The old tab roots (/bits, /sets, /tapes, /filmed) pop back to Notebook with their filter on (lib/PopTo: never a
// Redirect, which would leave two copies of the root; journal 2026-11-17).
import React from 'react';
import { Stack } from 'expo-router';
import { Platform } from 'react-native';
import { C } from '@/lib/theme';

export const unstable_settings = { anchor: 'index' }; // expo-router 58: `initialRouteName` is deprecated for `anchor`
const t = (s: string) => (Platform.OS === 'web' ? `${s} · Punchable` : s);
/** An old tab root: it pops back at once, with no slide. */
const pop = { animation: 'none' as const };

export default function NotebookStack() {
  return (
    <Stack screenOptions={{ headerShown: false, headerTintColor: C.amber, contentStyle: { backgroundColor: C.bg } }}>
      <Stack.Screen name="index" options={{ title: t('Notebook') }} />
      <Stack.Screen name="bits" options={{ ...pop, title: t('Notebook') }} />
      <Stack.Screen name="sets" options={{ ...pop, title: t('Notebook') }} />
      <Stack.Screen name="tapes" options={{ ...pop, title: t('Notebook') }} />
      <Stack.Screen name="filmed" options={{ ...pop, title: t('Notebook') }} />
      {/* a bit is a page; its new draft is a sheet over it (HIG/sheets: Keep mine and Use it are Cancel and Done) */}
      <Stack.Screen name="joke/[id]" options={{ title: t('Page') }} />
      <Stack.Screen name="joke/[id]/draft" options={{ title: t('New draft'), ...(Platform.OS === 'web' ? {} : { presentation: 'modal' as const }) }} />
      <Stack.Screen name="joke/[id]/tape" options={{ title: t('On tape') }} />
      {/* a set is a page too; a performed set is a tape */}
      <Stack.Screen name="list/[id]" options={{ title: t('Set') }} />
      <Stack.Screen name="set/[id]" options={{ title: t('Tape') }} />
      {/* the lists Notebook's ⋯ opens: every bit with sort and merge, every set with its drafts, the archive */}
      <Stack.Screen name="allbits" options={{ title: t('All bits') }} />
      <Stack.Screen name="allsets" options={{ title: t('All sets') }} />
      <Stack.Screen name="archived" options={{ title: t('Archived') }} />
      {/* the device checks (SPEC 2026-11-03 §4.1, SPEC 2026-11-16 §4): removed once the owner has answered them */}
      <Stack.Screen name="spike" options={{ title: t('Device checks') }} />
      <Stack.Screen name="spike-select" options={{ title: t('Selection mock') }} />
      <Stack.Screen name="merge" options={{ title: t('Merge') }} />
      <Stack.Screen name="inbox" options={{ title: t('Inbox') }} />
      <Stack.Screen name="say" options={{ title: t('Asking') }} />
      <Stack.Screen name="ask/index" options={{ title: t('Ask') }} />
      <Stack.Screen name="ask/[id]" options={{ title: t('Ask') }} />
      <Stack.Screen name="import" options={{ animation: 'slide_from_bottom', title: t('Import sets') }} />
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
      {/* the old Rant tab's topics (SPEC 2026-11-16 Slice 6): one made into a set opens that set */}
      <Stack.Screen name="topic/[topic]/index" options={{ title: t('Topic') }} />
      <Stack.Screen name="topic/[topic]/script" options={{ title: t('The rant') }} />
      <Stack.Screen name="topic/[topic]/bit/[id]" options={{ title: t('Page') }} />
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
