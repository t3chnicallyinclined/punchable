// Device check 5 (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §4.2): can he highlight words inside one line of a
// bit and get OUR row of marks for exactly those words? React Native has no custom items for the system's Copy
// bubble and <Text selectable> reports no range, so each read line here is a multiline TextInput with
// editable={false}: the system's own selection (press and hold, the handles), and `onSelectionChange` for the range
// (RCTBackedTextInputDelegateAdapter has no editable guard on that path; that UIKit calls it for read-only text is
// what this page finds out). The row under the line (Change · Ask · Keep · Add to set · New bit) names the words it
// would act on and does nothing else. A tap on the line must still open it (Slice 3's "tap a line"): the counter
// shows whether the tap reached the row or the text view swallowed it. One switch hides the system's bubble
// (`contextMenuHidden`), so he can say which he prefers. The lines are invented. Removed once the check is answered.
import React, { useCallback, useState } from 'react';
import { Pressable, Switch, Text, TextInput, View } from 'react-native';
import { C, F, linkColor } from '@/lib/theme';
import { T } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const LINES = [
  'My fridge has wifi now.',
  'It sends me a notification when the door is open.',
  'I know the door is open. I am standing in it.',
  'That is not a smart fridge, that is a roommate.',
];
const MARKS = ['Change', 'Ask', 'Keep', 'Add to set', 'New bit'];
type Sel = { line: number; start: number; end: number };

export default function SpikeSelect() {
  const [sel, setSel] = useState<Sel | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  const [taps, setTaps] = useState(0);
  const [hideMenu, setHideMenu] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const note = useCallback((s: string) => setLog((l) => [`${new Date().toISOString().slice(11, 23)} ${s}`, ...l].slice(0, 16)), []);
  const words = sel && sel.end > sel.start ? LINES[sel.line].slice(sel.start, sel.end) : '';

  return (
    <Screen title="Selection mock" back testID="spike-select">
      <T v="mut">Press and hold a word in a line, then drag the handles. Tap a line once, briefly. Nothing here is saved.</T>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 12, minHeight: 44 }}>
        <Text maxFontSizeMultiplier={1.6} style={{ flex: 1, fontFamily: F.body, fontSize: 16, lineHeight: 22, color: C.text }}>Hide the Copy bubble</Text>
        <Switch testID="select-hide-menu" accessibilityLabel="Hide the Copy bubble" value={hideMenu} onValueChange={(v) => { setHideMenu(v); note(`the Copy bubble is ${v ? 'hidden' : 'left alone'}`); }} />
      </View>
      <View testID="select-lines" style={{ borderTopWidth: 1, borderTopColor: C.hairline }}>
        {LINES.map((line, i) => {
          const mine = sel?.line === i && !!words;
          return (
            <View key={i} style={{ borderBottomWidth: 1, borderBottomColor: C.hairline }}>
              <Pressable testID={`select-row-${i}`} accessibilityRole="button" accessibilityLabel={`Line ${i + 1}`}
                onPress={() => { setTaps((n) => n + 1); setOpen((o) => (o === i ? null : i)); note(`tap reached the ROW of line ${i + 1}`); }}
                style={{ minHeight: 44, justifyContent: 'center', backgroundColor: open === i ? C.card2 : 'transparent' }}>
                <TextInput testID={`select-line-${i}`} multiline editable={false} scrollEnabled={false} value={line} contextMenuHidden={hideMenu}
                  onPressIn={() => note(`press began inside the TEXT of line ${i + 1}`)}
                  onSelectionChange={(e) => {
                    const { start, end } = e.nativeEvent.selection;
                    note(`onSelectionChange line ${i + 1}: ${start}-${end}${end > start ? ` “${line.slice(start, end)}”` : ''}`);
                    setSel(end > start ? { line: i, start, end } : (s) => (s?.line === i ? null : s));
                  }}
                  style={{ paddingVertical: 9, fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text }} />
              </Pressable>
              {mine && (
                <View testID="select-marks" style={{ paddingBottom: 8, gap: 6 }}>
                  <Text maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>On the words “{words}”</Text>
                  <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
                    {MARKS.map((m) => (
                      <Pressable key={m} testID={`select-mark-${m}`} accessibilityRole="button" accessibilityLabel={`${m}: ${words}`} onPress={() => note(`${m} on “${words}” (line ${i + 1}, ${sel!.start}-${sel!.end})`)}
                        style={({ pressed }) => ({ minHeight: 44, paddingHorizontal: 12, borderRadius: 12, borderWidth: 1, borderColor: C.border, justifyContent: 'center', backgroundColor: pressed ? C.card2 : C.card })}>
                        <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 15, color: linkColor() }}>{m}</Text>
                      </Pressable>
                    ))}
                  </View>
                </View>
              )}
            </View>
          );
        })}
      </View>
      <T v="semi" testID="select-taps">Taps that reached a line’s row: {taps}{open != null ? ` · line ${open + 1} is open` : ''}</T>
      <T v="small">Log (newest first):</T>
      <View testID="select-log" style={{ gap: 2 }}>
        {log.map((s, i) => <Text key={i} numberOfLines={2} style={{ fontFamily: F.body, fontSize: 12, lineHeight: 16, color: C.mut2 }}>{s}</Text>)}
      </View>
    </Screen>
  );
}
