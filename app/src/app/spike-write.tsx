// The focus-mode mock for device checks 2 and 3 (SPEC 2026-11-03 §4.1, §3.3): a full-screen modal like the real focus
// mode (Slice 2), three per-line inputs at the stage prose size, the glass bar (Punch · Pile · hide keyboard) at the
// bottom of the screen's KeyboardAvoidingView, and the dictation rule under test: a "new line" may arrive as "\n"
// inside onChangeText or as onSubmitEditing (§4 row 6a cannot settle which), so both become a new line after the text
// has settled (~700 ms, or Return / blur). An event log under the inputs shows the owner which path it took and
// whether the dictation session survived the hand-off. The header is the shipped full-screen task header (lib/TopBar
// inside the KeyboardAvoidingView, as Review does): with a native header the KAV's frame is short by the header and
// the bar would land under the keyboard for a reason unrelated to the check (iOS dev review, 2026-11-03). Removed in
// Slice 2 with the spike page.
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
  const focusLater = (k: number) => setTimeout(() => refs.current[k]?.focus(), 0);

  /** Split the line at the index on "\n" (the settled rule); the caret goes to the new line. Reads the latest lines (a settle timer may fire after later edits). */
  const splitAt = useCallback((k: number, why: string) => {
    const ls = linesRef.current;
    const i = ls.findIndex((l) => l.k === k);
    if (i < 0) return;
    const parts = ls[i].text.split('\n');
    if (parts.length < 2) return;
    const made = parts.slice(1).map((t) => ({ k: nextKey++, text: t }));
    setLines([...ls.slice(0, i), { k, text: parts[0] }, ...made, ...ls.slice(i + 1)]);
    focusLater(made[made.length - 1].k);
    note(`split (${why}) into ${made.length + 1}; focus moves to the new line`);
  }, [note]);

  const onChange = (k: number, t: string) => {
    const hadNl = t.includes('\n');
    note(`onChangeText line ${k}: ${JSON.stringify(t.slice(-24))}${hadNl ? '  ← holds \\n' : ''}`);
    setLines(linesRef.current.map((l) => (l.k === k ? { ...l, text: t } : l)));
    clearTimeout(settle.current);
    if (hadNl) settle.current = setTimeout(() => splitAt(k, 'settled \\n'), SETTLE_MS);
  };
  const onSubmit = (k: number) => {
    note(`onSubmitEditing line ${k} (Return or a spoken new line)`);
    clearTimeout(settle.current);
    const ls = linesRef.current;
    const i = ls.findIndex((l) => l.k === k);
    const made = { k: nextKey++, text: '' };
    setLines([...ls.slice(0, i + 1), made, ...ls.slice(i + 1)]);
    focusLater(made.k);
  };
  const onKey = (k: number, key: string) => {
    if (key !== 'Backspace') return;
    const ls = linesRef.current;
    const i = ls.findIndex((l) => l.k === k);
    if (i > 0 && ls[i].text === '') {
      note(`Backspace on an empty line ${k}: joins up`);
      setLines(ls.filter((l) => l.k !== k));
      focusLater(ls[i - 1].k);
    }
  };
  useEffect(() => () => clearTimeout(settle.current), []);

  const bar = (
    <KeyboardBar testID="spike-bar">
      <Btn small testID="spike-punch" label="Punch" v={punch ? 'primary' : 'plain'} onPress={() => setPunch((x) => !x)} />
      <Btn small testID="spike-pile" label="Pile · 2" onPress={() => ui.snack('The pile sheet arrives in Slice 2')} />
      <View style={{ flex: 1 }} />
      <Pressable testID="spike-hide" accessibilityRole="button" accessibilityLabel="Hide the keyboard" onPress={() => Keyboard.dismiss()}
        style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}>
        <Icon name="chevronDown" size={22} color={linkColor()} />
      </Pressable>
    </KeyboardBar>
  );
  const header = <TopBar title="Focus mock" left={<TextBtn testID="spike-cancel" label="Cancel" color={linkColor()} onPress={() => router.back()} />} right={<TextBtn testID="spike-done" label="Done" color={linkColor()} onPress={() => router.back()} />} />;

  return (
    <Screen title="Focus mock" testID="spike-write" scroll={false} task header={header}>
      <View style={{ flex: 1 }}>
        <View style={{ flex: 1, paddingHorizontal: 16, paddingTop: 8, gap: 4 }}>
          {lines.map((l, i) => (
            <View key={l.k} style={{ flexDirection: 'row', alignItems: 'flex-start', gap: 8, minHeight: 44 }}>
              <View style={{ width: 14, height: 44, alignItems: 'center', justifyContent: 'center' }}>{punch && focus === l.k && <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: C.amber }} />}</View>
              <TextInput ref={(r) => { refs.current[l.k] = r; }} testID={`spike-line-${i}`} value={l.text} multiline submitBehavior="submit"
                onChangeText={(t) => onChange(l.k, t)} onSubmitEditing={() => onSubmit(l.k)} onKeyPress={(e) => onKey(l.k, e.nativeEvent.key)}
                onFocus={() => { setFocus(l.k); note(`focus line ${l.k}`); }} onBlur={() => { note(`blur line ${l.k}`); clearTimeout(settle.current); if (linesRef.current.find((x) => x.k === l.k)?.text.includes('\n')) splitAt(l.k, 'blur'); }}
                accessibilityLabel={`Line ${i + 1}`} accessibilityHint="Return starts a new line" placeholder={i === lines.length - 1 ? 'Next line' : undefined} placeholderTextColor={C.dim}
                style={{ flex: 1, minHeight: 44, paddingVertical: 9, fontFamily: punch && focus === l.k ? F.semi : F.body, fontSize: 20, lineHeight: 26, color: C.text, backgroundColor: focus === l.k ? C.card2 : 'transparent', borderRadius: 10, paddingHorizontal: 6 }} />
            </View>
          ))}
          <T v="small" style={{ marginTop: 8 }}>Bar: {KEYBOARD_BAR_GLASS ? 'Liquid Glass' : 'flat (no glass on this OS)'} · Log (newest first):</T>
          <View testID="spike-log" style={{ gap: 2 }}>
            {log.map((s, i) => <Text key={i} numberOfLines={2} style={{ fontFamily: F.body, fontSize: 12, lineHeight: 16, color: C.mut2 }}>{s}</Text>)}
          </View>
        </View>
        <View style={{ paddingBottom: kbUp ? 0 : inset, backgroundColor: C.bgDeep }}>{bar}</View>
      </View>
    </Screen>
  );
}
