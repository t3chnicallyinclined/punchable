// Add a joke / Swap as a route (docs/design/HIG.md §3.5). Phone: a native form sheet at the large detent with the
// grabber (app/_layout.tsx), Cancel leading, no Done (a tap on "+" is the write). A route cannot take onPick, so the
// sheet writes the board op itself (the board's own per-item add / swap ops, Undo = the inverse op) and the set list
// re-reads on focus. Params: `board` (the id); `swap=1` with `at` (the position) and `old` (the joke id) for a swap.
// Web: the Modal sheet, unchanged, over the ground when /add/x is opened directly (the set list opens it in place).
import React, { useMemo } from 'react';
import { Platform, ScrollView, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { patch, useApi } from '@/lib/api';
import { AddJokeBody, AddJokeSheet, Pick, addJokeTitle } from '@/lib/AddJokeSheet';
import { creative } from '@/lib/creative';
import { haptics } from '@/lib/haptics';
import { shortNameOf } from '@/lib/say';
import { Build } from '@/lib/simple';
import { C } from '@/lib/theme';
import { useUI } from '@/lib/ui';

const enc = encodeURIComponent;

export default function AddJokeRoute() {
  const p = useLocalSearchParams<{ board: string; swap?: string; at?: string; old?: string }>();
  const id = decodeURIComponent(String(p.board || ''));
  const swap = p.swap === '1' ? { at: Number(p.at || 0), old: decodeURIComponent(String(p.old || '')) } : null;
  const mode = swap ? 'swap' : 'add';
  const ui = useUI();
  const b = useApi<Build>(`/api/boards/${enc(id)}/build`);
  const B = b.data;
  const steps = B?.steps || [];
  const inSet = useMemo(() => steps.map((x, i) => ({ joke_id: x.joke_id, position: i })), [steps]);
  const target = B?.board.target_s;
  const left = target ? target - (B?.runtime_s || 0) : null;
  const close = () => (router.canGoBack() ? router.back() : router.navigate(`/list/${enc(id)}` as any));
  const boardPatch = (body: any) => patch<any>(`/api/boards/${enc(id)}`, body);
  const nameOf = (j: string) => { const x = steps.find((y) => y.joke_id === j); return x ? x.short_name || shortNameOf(x.title) : ''; };

  // the write, then the outcome (HIG/playing-haptics: success "indicates that a task or action has completed")
  const onPick = async (pk: Pick) => {
    close();
    try {
      if (swap) {
        await boardPatch({ add: { joke_id: pk.joke_id, at: swap.at }, remove: { joke_id: swap.old } });
        haptics.success();
        ui.snack(`Swapped ${nameOf(swap.old)} for ${pk.short_name}`, () => boardPatch({ add: { joke_id: swap.old, at: swap.at }, remove: { joke_id: pk.joke_id } }));
      } else {
        await creative.addItem(id, pk.joke_id);
        haptics.success();
        ui.snack(`Added ${pk.short_name}`, () => boardPatch({ remove: { joke_id: pk.joke_id } }));
      }
    } catch (e: any) {
      ui.snack(`Couldn't save: ${e.message}`);
    }
  };

  const props = { onClose: close, boardId: id, inSet, fits: B?.next || [], onPick, left, mode } as const;
  if (Platform.OS === 'web') {
    return (
      <View style={{ flex: 1, backgroundColor: C.bg }}>
        <AddJokeSheet visible {...props} />
      </View>
    );
  }
  return (
    <View testID="add-sheet" style={{ flex: 1, backgroundColor: C.bg }}>
      <Stack.Screen options={{ title: addJokeTitle(mode, left) }} />
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button accessibilityLabel="Cancel" onPress={close}>Cancel</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" keyboardShouldPersistTaps="handled" keyboardDismissMode="on-drag" contentContainerStyle={{ padding: 16, paddingBottom: 32 }}>
        <AddJokeBody {...props} chrome={false} />
      </ScrollView>
    </View>
  );
}
