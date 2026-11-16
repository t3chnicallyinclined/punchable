// The line editor mock for device checks 2, 3 and 6 (SPEC 2026-11-03 §4.1, §3.3; SPEC 2026-11-16 §4.3): a full-screen
// modal like the real Write will be, every line a mounted multiline input at the stage prose size, the glass bar at
// the bottom of the screen's KeyboardAvoidingView, and the rules under test:
//   - dictation (check 3): a "new line" may arrive as "\n" inside onChangeText or as onSubmitEditing, so both become
//     a new line after the text has settled (~700 ms, or Return / blur);
//   - Return (check 6): `submitBehavior="submit"` reports a lone newline as onSubmitEditing; the line is split at the
//     caret `onSelectionChange` last reported, and the caret starts the new line;
//   - Backspace at the start of a line (check 6): whether a multiline input reports that key when nothing would be
//     deleted is not in React Native's source (the always-fires hook is the single-line adapter's), so the join is
//     wired to `onKeyPress` AND to a "Join up" key in the bar, live whenever the caret is at the start of a line. The
//     log says which one did it;
//   - the caret after a split or a join: placed with the input's `setSelection` AFTER the commit that carries the new
//     text (a zero-length controlled `selection` sent with a text change is skipped natively, and the command is
//     dropped when its event count is stale), a frame later. It does not fire onSelectionChange, so the caret kept
//     here is updated by hand.
// An event log under the inputs shows the owner which path each thing took. The header is the shipped full-screen
// task header (lib/TopBar inside the KeyboardAvoidingView, as Review does): with a native header the KAV's frame is
// short by the header and the bar would land under the keyboard for a reason unrelated to the check (iOS dev review,
// 2026-11-03). Removed when the real line editor ships (Slice 2).
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Keyboard, Platform, Pressable, Text, TextInput, View } from 'react-native';
import { router } from 'expo-router';
import { C, F, linkColor } from '@/lib/theme';
import { KEYBOARD_BAR_GLASS, KeyboardBar } from '@/lib/KeyboardBar';
import { Icon } from '@/lib/Icon';
import { TopBar } from '@/lib/TopBar';
import { Btn, T, TextBtn, useBottomInset, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const SETTLE_MS = 700;
type Line = { k: number; text: string };
let nextKey = 100;

export default function SpikeWrite() {
  const ui = useUI();
  const [lines, setLines] = useState<Line[]>([{ k: 1, text: 'They pat me down like they are checking for a pulse.' }, { k: 2, text: 'I took my belt off. My shoes. My dignity.' }, { k: 3, text: '' }]);
  const linesRef = useRef(lines);
  useEffect(() => { linesRef.current = lines; });
  const [focus, setFocus] = useState<number | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const [punch, setPunch] = useState(false);
  const refs = useRef<Record<number, React.ComponentRef<typeof TextInput> | null>>({});
  const settle = useRef<any>(null);
  // where the caret is in each line (a collapsed selection's start), as the input last reported it or as we placed it
  const caret = useRef<Record<number, number>>({});
  // where each line's selection ends: Backspace joins only on a collapsed caret at 0 (a selection from the start of
  // a line is a delete of words, and the text change that follows must find its line still there)
  const caretEnd = useRef<Record<number, number>>({});
  const [atStart, setAtStart] = useState(false);
  // a line to focus with its caret at `at`, once the commit that carries the new lines has happened
  const pending = useRef<{ k: number; at: number } | null>(null);
  const note = useCallback((s: string) => setLog((l) => [`${new Date().toISOString().slice(11, 23)} ${s}`, ...l].slice(0, 14)), []);
  // the bar clears the home indicator while the keyboard is down and sits flush on the keyboard while it is up
  // (the screen's KeyboardAvoidingView pads for the keyboard; an inset pad on top of that would be the "gap" failure)
  const inset = useBottomInset();
  const [kbUp, setKbUp] = useState(false);
  useEffect(() => {
    const a = Keyboard.addListener(Platform.OS === 'ios' ? 'keyboardWillShow' : 'keyboardDidShow', () => setKbUp(true));
    const b = Keyboard.addListener(Platform.OS === 'ios' ? 'keyboardWillHide' : 'keyboardDidHide', () => setKbUp(false));
    return () => { a.remove(); b.remove(); };
  }, []);
  // after every commit: a split or a join left a caret to place (each one sets new lines, so a commit follows it)
  useEffect(() => {
    const p = pending.current;
    if (!p) return;
    pending.current = null;
    const el = refs.current[p.k];
    el?.focus();
    requestAnimationFrame(() => {
      el?.setSelection(p.at, p.at);
      caret.current[p.k] = p.at;
      caretEnd.current[p.k] = p.at;
      setAtStart(p.at === 0);
      note(`caret placed in line ${p.k} at ${p.at}`);
    });
  });

  /** Split the line on "\n" (the settled rule); the caret goes to the new line. Reads the latest lines (a settle timer may fire after later edits). */
  const splitAt = useCallback((k: number, why: string) => {
    const ls = linesRef.current;
    const i = ls.findIndex((l) => l.k === k);
    if (i < 0) return;
    const parts = ls[i].text.split('\n');
    if (parts.length < 2) return;
    const made = parts.slice(1).map((t) => ({ k: nextKey++, text: t }));
    setLines([...ls.slice(0, i), { k, text: parts[0] }, ...made, ...ls.slice(i + 1)]);
    const last = made[made.length - 1];
    pending.current = { k: last.k, at: last.text.length };
    note(`split (${why}) into ${made.length + 1}; focus moves to the new line`);
  }, [note]);

  /** Join line `k` to the one above: no word is lost, the caret lands where the two met. */
  const joinUp = useCallback((k: number, why: string) => {
    const ls = linesRef.current;
    const i = ls.findIndex((l) => l.k === k);
    if (i <= 0) return note(`${why}: line ${k} is the first line, nothing to join`);
    const up = ls[i - 1];
    const at = up.text.length;
    // the upper line takes the focus before the lower one unmounts, so the keyboard never drops between them
    refs.current[up.k]?.focus();
    setLines([...ls.slice(0, i - 1), { k: up.k, text: up.text + ls[i].text }, ...ls.slice(i + 1)]);
    delete caret.current[k];
    delete caretEnd.current[k];
    pending.current = { k: up.k, at };
    note(`JOINED by ${why}: line ${k} into line ${up.k}; the caret goes to ${at}`);
  }, [note]);

  const onChange = (k: number, t: string) => {
    const hadNl = t.includes('\n');
    note(`onChangeText line ${k}: ${JSON.stringify(t.slice(-24))}${hadNl ? '  ← holds \\n' : ''}`);
    setLines(linesRef.current.map((l) => (l.k === k ? { ...l, text: t } : l)));
    clearTimeout(settle.current);
    if (hadNl) settle.current = setTimeout(() => splitAt(k, 'settled \\n'), SETTLE_MS);
  };
  const onSubmit = (k: number) => {
    clearTimeout(settle.current);
    const ls = linesRef.current;
    const i = ls.findIndex((l) => l.k === k);
    if (i < 0) return;
    const text = ls[i].text;
    const at = Math.max(0, Math.min(text.length, caret.current[k] ?? text.length));
    const made = { k: nextKey++, text: text.slice(at) };
    setLines([...ls.slice(0, i), { k, text: text.slice(0, at) }, made, ...ls.slice(i + 1)]);
    pending.current = { k: made.k, at: 0 };
    note(`onSubmitEditing line ${k} (Return or a spoken new line): split at ${at} of ${text.length}`);
  };
  const onKey = (k: number, key: string) => {
    if (key !== 'Backspace') return;
    const ls = linesRef.current;
    const empty = ls.find((l) => l.k === k)?.text === '';
    const at = caret.current[k];
    const collapsed = caretEnd.current[k] === at;
    note(`onKeyPress Backspace in line ${k}: caret ${at ?? 'unknown'}${collapsed ? '' : ' (a selection)'}, the line is ${empty ? 'EMPTY' : 'not empty'}`);
    if (empty || (at === 0 && collapsed)) joinUp(k, 'Backspace');
  };
  useEffect(() => () => clearTimeout(settle.current), []);
  const canJoin = focus != null && atStart && lines.findIndex((l) => l.k === focus) > 0;

  const bar = (
    <KeyboardBar testID="spike-bar">
      <Btn small testID="spike-punch" label="Punch" v={punch ? 'primary' : 'plain'} onPress={() => setPunch((x) => !x)} />
      <Btn small testID="spike-pile" label="Pile · 2" onPress={() => ui.snack('The pile sheet arrives with the real editor')} />
      {/* the fallback for Backspace at the start of a line: live only there, so it never sits under a typing thumb */}
      <Btn small testID="spike-join" label="Join up" disabled={!canJoin} accessibilityLabel="Join this line to the one above" onPress={() => { if (focus != null) joinUp(focus, 'the Join up key'); }} />
      <View style={{ flex: 1 }} />
      <Pressable testID="spike-hide" accessibilityRole="button" accessibilityLabel="Hide the keyboard" onPress={() => Keyboard.dismiss()}
        style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}>
        <Icon name="chevronDown" size={22} color={linkColor()} />
      </Pressable>
    </KeyboardBar>
  );
  const header = <TopBar title="Line editor mock" left={<TextBtn testID="spike-cancel" label="Cancel" color={linkColor()} onPress={() => router.back()} />} right={<TextBtn testID="spike-done" label="Done" color={linkColor()} onPress={() => router.back()} />} />;

  return (
    <Screen title="Line editor mock" testID="spike-write" scroll={false} task header={header}>
      <View style={{ flex: 1 }}>
        <View style={{ flex: 1, paddingHorizontal: 16, paddingTop: 8, gap: 4 }}>
          {lines.map((l, i) => (
            <View key={l.k} style={{ flexDirection: 'row', alignItems: 'flex-start', gap: 8, minHeight: 44 }}>
              <View style={{ width: 14, height: 44, alignItems: 'center', justifyContent: 'center' }}>{punch && focus === l.k && <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: C.amber }} />}</View>
              <TextInput ref={(r) => { refs.current[l.k] = r; }} testID={`spike-line-${i}`} value={l.text} multiline submitBehavior="submit"
                onChangeText={(t) => onChange(l.k, t)} onSubmitEditing={() => onSubmit(l.k)} onKeyPress={(e) => onKey(l.k, e.nativeEvent.key)}
                onSelectionChange={(e) => {
                  const { start, end } = e.nativeEvent.selection;
                  caret.current[l.k] = start;
                  caretEnd.current[l.k] = end;
                  if (focus === l.k) setAtStart(start === 0 && end === 0);
                }}
                onFocus={() => { setFocus(l.k); setAtStart(caret.current[l.k] === 0 && caretEnd.current[l.k] === 0); note(`focus line ${l.k}`); }}
                onBlur={() => { note(`blur line ${l.k}`); clearTimeout(settle.current); if (linesRef.current.find((x) => x.k === l.k)?.text.includes('\n')) splitAt(l.k, 'blur'); }}
                accessibilityLabel={`Line ${i + 1}`} accessibilityHint="Return starts a new line" placeholder={i === lines.length - 1 ? 'Next line' : undefined} placeholderTextColor={C.dim}
                style={{ flex: 1, minHeight: 44, paddingVertical: 9, fontFamily: punch && focus === l.k ? F.semi : F.body, fontSize: 20, lineHeight: 26, color: C.text, backgroundColor: focus === l.k ? C.card2 : 'transparent', borderRadius: 10, paddingHorizontal: 6 }} />
            </View>
          ))}
          <T v="small" style={{ marginTop: 8 }}>Bar: {KEYBOARD_BAR_GLASS ? 'Liquid Glass' : 'flat (no glass on this OS)'} · Join up is {canJoin ? 'live (the caret is at the start of a line)' : 'off'} · Log (newest first):</T>
          <View testID="spike-log" style={{ gap: 2 }}>
            {log.map((s, i) => <Text key={i} numberOfLines={2} style={{ fontFamily: F.body, fontSize: 12, lineHeight: 16, color: C.mut2 }}>{s}</Text>)}
          </View>
        </View>
        <View style={{ paddingBottom: kbUp ? 0 : inset, backgroundColor: C.bgDeep }}>{bar}</View>
      </View>
    </Screen>
  );
}
