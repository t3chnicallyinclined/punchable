// The marks tray (SPEC 2026-11-16 Slice 3; the owner's Redline: "Comments collect in a tray at the bottom. Hit Send
// review and they are batched into one prompt, in document order"): /marks?bit=<id>&on=<job>&edit=<mark>. A native form
// sheet on the phone (app/_layout.tsx), the RN Sheet on the web. Each mark: its type in words, the words it quotes, its
// note (a Change) or question (an Ask) in a field, and Remove. `edit` opens with that mark's field focused (he just
// tapped Change or Ask). Send asks the screen that owns the text to send (lib/marks sendAsked: it saves his typing
// first and knows the version), and closes the tray. Nothing here is sent anywhere until Send.
import React, { useEffect, useRef, useState } from 'react';
import { Platform, Pressable, ScrollView, Text, TextInput, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { useOnline } from './api';
import { haptics } from './haptics';
import { sendAsked, useMarks } from './marks';
import { MarkSwatch } from './MarkedText';
import { C, F, linkColor } from './theme';
import { Btn, Sheet, SheetHead, T } from './ui';
import { PROSE_CAP, ROW_CAP } from '@/rules/bitLines';
import { MAX_NOTE, TYPE_WORD, markCount, sendLabel, type Mark } from '@/rules/markRules';

const native = Platform.OS !== 'web';
const enc = encodeURIComponent;

/** Open the tray for a bit (`on` = the round whose draft is marked); `edit` = a mark whose field is focused. */
export function openTray(bit: string, on: string | null, edit?: string) {
  router.push({ pathname: '/marks', params: { bit, ...(on ? { on } : {}), ...(edit ? { edit } : {}) } } as any);
}

function Row({ m, focus, tried, onNote, onRemove }: { m: Mark; focus: boolean; tried: boolean; onNote: (t: string) => void; onRemove: () => void }) {
  // a fresh Ask is not an error before he has had the chance to type: only once he left its field or tried to send
  const [left, setLeft] = useState(false);
  const needs = m.type === 'ask' && !m.note.trim() && (left || tried);
  return (
    <View testID={`mark-${m.id}`} style={{ gap: 6, paddingVertical: 10, borderBottomWidth: 1, borderBottomColor: C.hairline }}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <MarkSwatch k={m.type} />
        <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 15, color: C.text }}>{TYPE_WORD[m.type]}{m.carried ? ' · kept last round' : ''}</Text>
        <View style={{ flex: 1 }} />
        <Pressable testID={`mark-${m.id}-remove`} accessibilityRole="button" accessibilityLabel={`Remove the ${TYPE_WORD[m.type]} mark`} onPress={onRemove} hitSlop={8}
          style={{ minHeight: 44, minWidth: 44, alignItems: 'flex-end', justifyContent: 'center' }}>
          <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 15, color: linkColor() }}>Remove</Text>
        </Pressable>
      </View>
      <Text numberOfLines={3} maxFontSizeMultiplier={PROSE_CAP} style={{ fontFamily: F.body, fontSize: 16, lineHeight: 22, color: C.text2 }}>“{m.text}”</Text>
      {m.type !== 'keep' && (
        <TextInput testID={`mark-${m.id}-note`} defaultValue={m.note} autoFocus={focus} multiline maxLength={MAX_NOTE}
          placeholder={m.type === 'ask' ? 'Your question' : 'What should change? (optional)'} placeholderTextColor={C.dim}
          accessibilityLabel={m.type === 'ask' ? 'Your question' : 'Your note on this change'} onChangeText={onNote} onBlur={() => setLeft(true)} maxFontSizeMultiplier={PROSE_CAP}
          style={{ minHeight: 44, borderWidth: 1, borderColor: needs ? C.redSoft : C.border, borderRadius: 8, paddingHorizontal: 12, paddingVertical: 10, color: C.text, fontFamily: F.body, fontSize: 16, lineHeight: 22 }} />
      )}
      {needs && <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.body, fontSize: 13, color: C.redSoft }}>An Ask needs its question</Text>}
    </View>
  );
}

export function MarkTray() {
  const p = useLocalSearchParams<{ bit: string; on?: string; edit?: string }>();
  const bit = decodeURIComponent(String(p.bit || ''));
  const on = p.on ? decodeURIComponent(String(p.on)) : null;
  const edit = p.edit ? String(p.edit) : null;
  const { marks, replace } = useMarks(bit, on);
  const online = useOnline();
  const [tried, setTried] = useState(false);
  // the mark he removed last, and the tray before it: Undo stays in the tray, in place, until the tray closes
  const [removed, setRemoved] = useState<{ mark: Mark; before: Mark[] } | null>(null);
  // a note is kept as he types (the tray is one copy shared with the page; the file is rewritten a moment later)
  const latest = useRef(marks);
  useEffect(() => { latest.current = marks; }, [marks]);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const pendingNotes = useRef<Record<string, string>>({});
  const flush = () => {
    clearTimeout(timer.current);
    const notes = pendingNotes.current;
    pendingNotes.current = {};
    if (!Object.keys(notes).length) return;
    void replace(latest.current.map((m) => (m.id in notes && m.type !== 'keep' ? { ...m, note: notes[m.id] } : m)));
  };
  useEffect(() => () => flush(), []); // eslint-disable-line react-hooks/exhaustive-deps
  const note = (id: string, t: string) => {
    pendingNotes.current[id] = t;
    clearTimeout(timer.current);
    timer.current = setTimeout(flush, 300);
  };
  // Remove takes effect at once, with Undo in the snack (HIG.md §7): a question typed is not lost to a mis-tap
  const remove = (id: string) => {
    flush();
    haptics.selection();
    const before = latest.current;
    const gone = before.find((m) => m.id === id);
    void replace(before.filter((m) => m.id !== id));
    if (gone) setRemoved({ mark: gone, before });
  };
  const close = () => { flush(); if (router.canGoBack()) router.back(); else router.replace(`/joke/${enc(bit)}` as any); };
  const canSend = online && marks.length > 0;
  const send = () => {
    setTried(true);
    flush();
    if (!canSend) return;
    if (latest.current.some((m) => m.type === 'ask' && !m.note.trim())) return void haptics.warning();
    close();
    setTimeout(() => sendAsked.emit({ bit, on }), 50);
  };
  const title = marks.length ? markCount(marks.length) : 'Marks';

  const body = (
    <View style={{ gap: 4 }}>
      {!marks.length && (
        <T v="mut" testID="marks-empty">Select words in the bit, or put the cursor on a line, and tap Change, Ask or Keep. Your marks wait here until you send them.</T>
      )}
      {marks.map((m) => <Row key={m.id} m={m} focus={m.id === edit} tried={tried} onNote={(t) => note(m.id, t)} onRemove={() => remove(m.id)} />)}
      {!!removed && (
        <View testID="marks-removed" style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44, paddingVertical: 4 }}>
          <Text maxFontSizeMultiplier={ROW_CAP} style={{ flex: 1, fontFamily: F.body, fontSize: 15, color: C.mut2 }}>Removed the {TYPE_WORD[removed.mark.type]} mark</Text>
          <Pressable testID="marks-removed-undo" accessibilityRole="button" accessibilityLabel={`Undo: put the ${TYPE_WORD[removed.mark.type]} mark back`} hitSlop={8}
            onPress={() => { void replace(removed.before); setRemoved(null); }} style={{ minHeight: 44, minWidth: 44, alignItems: 'flex-end', justifyContent: 'center' }}>
            <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 15, color: linkColor() }}>Undo</Text>
          </Pressable>
        </View>
      )}
      {marks.length > 0 && (
        <View style={{ gap: 8, paddingTop: 12 }}>
          <T v="small">Claude answers your Asks, keeps your Keeps word for word, and sends back a whole new draft. You choose to use it or not.</T>
          {!online && <T v="mut" testID="marks-offline">Offline · your marks wait on this iPhone</T>}
          {!native && <Btn testID="marks-send" v="primary" label={sendLabel(marks.length)} disabled={!canSend} onPress={send} />}
        </View>
      )}
    </View>
  );
  if (!native) {
    return (
      <View style={{ flex: 1, backgroundColor: C.bg }}>
        <Sheet visible onClose={close} label={title} testID="marks-sheet">
          <SheetHead title={title} onCancel={close} cancelLabel="Close" testID="marks" />
          {body}
        </Sheet>
      </View>
    );
  }
  return (
    <View testID="marks-sheet" style={{ flex: 1, backgroundColor: C.surface }}>
      <Stack.Screen options={{ title }} />
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button icon="xmark" accessibilityLabel="Close" onPress={close} />
      </Stack.Toolbar>
      {/* HIG/sheets: the action that finishes the sheet on the trailing edge, prominent; above the keyboard, always in reach */}
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="prominent" disabled={!canSend} accessibilityLabel={sendLabel(marks.length)} accessibilityHint={online ? 'Claude sends back a whole new draft' : 'Offline'} onPress={send}>Send</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" keyboardShouldPersistTaps="handled" keyboardDismissMode="interactive" contentContainerStyle={{ padding: 16, paddingBottom: 48 }}>
        {body}
      </ScrollView>
    </View>
  );
}
