// Add to a set: the multi-tick picker (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §4.5; Slice 1). It replaces the
// one-choice picker that lived in lib/boards. Phone: a native form sheet at the medium detent, large on drag, the
// grabber (app/_layout.tsx `setpick`); Cancel leading, Done trailing (HIG/sheets). Web: the RN Sheet.
//
// Rows are his live set lists (rules/setListRules pickerLists), each a checkbox with a trailing checkmark, ticked
// where the bit already is (GET /api/jokes/<id>/around `sets`). Ticks are STAGED: nothing is written until Done, so a
// mis-tap in the dark costs nothing. "New set" is a field row in the same sheet, never a second sheet (HIG: one sheet
// at a time). Done sends the calls of rules/aroundRules pickerCalls, then one snack ("Added to 2 sets") whose Undo
// sends pickerUndo (also when one of several changes failed: what was written can still be taken back). A bit already
// on a list (409) counts as nothing to do.
//
// Staged work is never lost silently (HIG/sheets: "If people have unsaved changes in the sheet when they begin
// swiping to dismiss it, use an action sheet to let them confirm"): while a tick or a name is staged the sheet does
// not swipe away (`gestureEnabled`, as the sessions sheet does while a change is kept), and Cancel asks once,
// Discard changes or Keep editing. With nothing staged, a swipe or Cancel just closes.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Platform, Pressable, ScrollView, Text, TextInput, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { ApiError, patch, post, useApi, useOnline } from './api';
import { actionSheet } from './actionSheet';
import { aroundChanged, aroundPath } from './Around';
import type { BoardListItem } from './creative';
import { haptics } from './haptics';
import { Icon } from './Icon';
import { C, F, linkColor } from './theme';
import { Sheet, SheetHead, T, useUI } from './ui';
import { ROW_CAP } from '@/rules/bitLines';
import { pickerLists } from '@/rules/setListRules';
import * as A from '@/rules/aroundRules';

const enc = encodeURIComponent;
const native = Platform.OS !== 'web';
const send = (c: A.Call) => (c.method === 'POST' ? post<any>(c.path, c.body) : patch<any>(c.path, c.body));

/** Open the picker for a bit (the route /setpick). `note` = words he took from the bit (Slice 3, Take it elsewhere):
 *  they go under it as a note in each set it is added to. */
export function openSetPicker(bit: string, title?: string, note?: string) {
  router.push({ pathname: '/setpick', params: { bit, ...(title ? { title } : {}), ...(note ? { note } : {}) } } as any);
}

export function SetPicker() {
  const p = useLocalSearchParams<{ bit: string; title?: string; note?: string }>();
  const bit = decodeURIComponent(String(p.bit || ''));
  const name = String(p.title || '');
  const note = String(p.note || '');
  const ui = useUI();
  const online = useOnline();
  const bs = useApi<{ boards: BoardListItem[] }>('/api/boards', { cache: false });
  // where the bit is now; a server without the read answers nothing and every row starts unticked
  const ar = useApi<A.Around>(bit ? `${aroundPath(bit)}?cap=1` : null, { cache: false });
  const ready = !!bs.data && (!!ar.data || !!ar.rawError);
  const boards = useMemo(() => pickerLists(bs.data?.boards || []), [bs.data]);
  const sets = useMemo(() => ar.data?.sets || [], [ar.data]);
  const was = useMemo(() => new Set(sets.map((s) => s.board)), [sets]);
  const [now, setNow] = useState<Set<string> | null>(null);
  const [typed, setTyped] = useState('');
  const [saving, setSaving] = useState(false);
  // the sheet may be gone by the time the calls answer: then nothing is popped
  const alive = useRef(true);
  useEffect(() => () => { alive.current = false; }, []);
  // the ticks start from where the bit is; `now` holds them only once he has changed one
  const ticked = now || was;
  const rows = A.pickerRows(boards, ticked);
  const diff = A.stagedDiff(was, ticked);
  const dirty = A.hasChanges(diff, typed);
  const canDone = online && ready && !saving && dirty;
  const close = () => (router.canGoBack() ? router.back() : router.replace(`/joke/${enc(bit)}` as any));
  const cancel = () => {
    if (!dirty || saving || !native) return close();
    actionSheet({ options: [{ label: 'Discard changes', destructive: true, onPress: close }, { label: 'Keep editing', cancel: true }] });
  };

  const done = async () => {
    if (!canDone) return;
    setSaving(true);
    const out: A.PickDone = { added: [], removed: [], created: [] };
    const titles: Record<string, string> = Object.fromEntries(boards.map((b) => [b.id, b.title]));
    let failed = '';
    for (const call of A.pickerCalls(bit, diff, typed, note)) {
      try {
        const r = await send(call);
        if (call.path === '/api/boards') {
          const id = r?.board?.id as string | undefined;
          if (id) { out.created.push(id); titles[id] = r.board.title; }
        } else {
          const board = decodeURIComponent(call.path.split('/')[3] || '');
          (call.method === 'POST' ? out.added : out.removed).push(board);
        }
      } catch (e: any) {
        // already on that list (409): nothing to do, and nothing to undo
        if (!(e instanceof ApiError && e.status === 409)) failed = e?.message || 'try again';
      }
    }
    const places: Record<string, number> = Object.fromEntries(sets.map((s) => [s.board, s.position]));
    const words = A.pickerSnack(out, titles);
    if (alive.current) { setSaving(false); close(); }
    aroundChanged.emit({ bit });
    if (!words) { if (failed) ui.snack(`Couldn’t save: ${failed}`); return; }
    const undoIt = async () => {
      for (const c of A.pickerUndo(bit, out, places)) await send(c).catch(() => {});
      aroundChanged.emit({ bit });
    };
    if (!failed) haptics.success();
    ui.snack(failed ? `${words}. One change didn’t save: ${failed}` : words, undoIt);
  };

  const body = (
    <View style={{ gap: 4 }}>
      {!!name && <T v="mut" style={{ marginBottom: 4 }}>{name}</T>}
      {!!note && (
        // Take it elsewhere (Slice 3): the words he picked travel with the bit, as its note in each set he ticks now
        <Text testID="setpick-note" maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 15, lineHeight: 21, color: C.mut2, paddingBottom: 6 }}>
          With the note “{note}” in each set you tick (a set you make here gets the bit without it).
        </Text>
      )}
      {!ready && <T v="mut" testID="setpick-loading">{bs.rawError ? 'Can’t reach Punchable.' : 'Loading your sets…'}</T>}
      {ready && rows.length === 0 && <T v="mut" testID="setpick-none">No set of yours yet. Name one below.</T>}
      {ready && rows.map((r) => (
        <Pressable key={r.id} testID={`setpick-row-${r.id}`} accessibilityRole="checkbox" accessibilityState={{ checked: r.on }}
          accessibilityLabel={[r.title, r.sub].filter(Boolean).join(', ')} onPress={() => { haptics.selection(); setNow(A.toggled(ticked, r.id)); }}
          style={({ pressed }) => ({ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 52, paddingVertical: 6, borderBottomWidth: 1, borderBottomColor: C.hairline, backgroundColor: pressed ? C.card2 : 'transparent' })}>
          <View style={{ flex: 1, minWidth: 0 }}>
            <Text numberOfLines={1} maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 17, lineHeight: 22, color: C.text }}>{r.title}</Text>
            {!!r.sub && <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>{r.sub}</Text>}
          </View>
          {/* the trailing checkmark of a multi-select list (HIG/lists-and-tables); its place is kept so rows do not jump */}
          <View style={{ width: 28, alignItems: 'center' }}>{r.on && <Icon name="check" size={20} color={linkColor()} sw={2.6} />}</View>
        </Pressable>
      ))}
      {ready && (
        <TextInput testID="setpick-new" value={typed} onChangeText={setTyped} maxLength={200} placeholder="New set name" placeholderTextColor={C.dim} accessibilityLabel="New set name"
          accessibilityHint="Name a new set to put this bit in; Done makes it" returnKeyType="default" clearButtonMode="while-editing"
          style={{ minHeight: 48, color: C.text, fontFamily: F.body, fontSize: 17, borderBottomWidth: 1, borderBottomColor: C.hairline }} />
      )}
      {!online && <T v="mut" testID="setpick-offline" style={{ marginTop: 8 }}>Offline · reading only</T>}
    </View>
  );

  if (!native) {
    return (
      <View style={{ flex: 1, backgroundColor: C.bg }}>
        <Sheet visible onClose={close} label="Add to a set" testID="setpick">
          <SheetHead title="Add to a set" onCancel={close} onDone={() => void done()} doneLabel={saving ? 'Saving…' : 'Done'} doneDisabled={!canDone} testID="setpick-head" />
          {body}
        </Sheet>
      </View>
    );
  }
  return (
    <View testID="setpick" style={{ flex: 1, backgroundColor: C.surface }}>
      {/* with something staged the sheet does not swipe away: Cancel asks, Done saves */}
      <Stack.Screen options={{ title: 'Add to a set', gestureEnabled: !dirty && !saving }} />
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button accessibilityLabel="Cancel" onPress={cancel}>Cancel</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="done" disabled={!canDone} accessibilityLabel="Done" accessibilityHint={online ? undefined : 'Offline'} onPress={() => void done()}>Done</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" automaticallyAdjustKeyboardInsets keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: 16, paddingTop: 8, paddingBottom: 48 }}>
        {body}
      </ScrollView>
    </View>
  );
}
