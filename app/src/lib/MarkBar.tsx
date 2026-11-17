// The marks bar (SPEC 2026-11-16 Slice 3; the owner's Redline: "Edit · Question · Keep · Fork"). React Native has no
// custom items for the system's edit menu (§4.2), so the bar is driven by the input's selection: while he types on his
// bit it sits on the keyboard (a plain view at the keyboard's top, the repo's keyboard-bar pattern: lib/KeyboardBar);
// on the draft he reads, at the bottom while words are selected. One short line says what a mark would be on (the
// selected words, or the line the caret is on), or what just happened ("Kept", with Undo), or the page's footnote; the
// row under it offers the four marks; the count opens the tray. Every target is at least 44 pt and says its name, and
// each mark's swatch carries the same line style as its highlight (HIG/color: never colour alone).
import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { BarGround } from './BarGround';
import { C, F, linkColor } from './theme';
import { markCount, type MarkType } from '@/rules/markRules';
import { MarkSwatch } from './MarkedText';

export type BarAction = MarkType | 'elsewhere';
/** The labels' Dynamic Type cap: four buttons must fit one row on the narrowest iPhone. */
const BAR_CAP = 1.3;
const ACTIONS: { k: BarAction; label: string; hint: string }[] = [
  { k: 'change', label: 'Change', hint: 'Marks these words to change, with a note if you want' },
  { k: 'ask', label: 'Ask', hint: 'Asks Claude about these words' },
  { k: 'keep', label: 'Keep', hint: 'Claude must leave these words exactly as they are' },
  { k: 'elsewhere', label: 'Copy to…', hint: 'Puts a copy of these words in a set or a new bit' },
];

/** The words a mark would be on, cut for one line of the bar. */
function preview(t: string): string {
  const one = t.replace(/\s+/g, ' ').trim();
  return one.length > 44 ? `${one.slice(0, 43)}…` : one;
}

export function MarkBar({ target, count, flash, note, offline, onAct, onUndo, onTray, testID }: {
  /** What a mark would be on now; null = nothing to mark (a blank line). */ target: { text: string } | null;
  /** Marks in the tray. */ count: number;
  /** A short word after a mark was made ("Kept"), shown for a moment in place of the preview. */ flash?: string | null;
  /** The page's own footnote (offline, saved), shown when there is nothing else to say. */ note?: string | null;
  /** No connection: the line starts with it, whatever else it says (the page's own offline line is under the bar). */ offline?: boolean;
  onAct: (a: BarAction) => void;
  /** With a flash: takes the mark just made back. */ onUndo?: (() => void) | null;
  onTray: () => void; testID?: string;
}) {
  const on = !!target;
  const said = flash || (target ? `On “${preview(target.text)}”` : note || 'Select words, or put the cursor on a line');
  const line = offline && !said.startsWith('Offline') ? `Offline · ${said}` : said;
  const body = (
    <View style={{ paddingHorizontal: 8, paddingTop: 2, paddingBottom: 6, gap: 2 }}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 28, paddingHorizontal: 4 }}>
        <Text numberOfLines={1} maxFontSizeMultiplier={BAR_CAP} style={{ flex: 1, fontFamily: F.body, fontSize: 13, color: C.mut2 }}>{line}</Text>
        {count > 0 && !flash && (
          <Pressable testID={testID ? `${testID}-tray` : undefined} accessibilityRole="button" accessibilityLabel={`${markCount(count)}. Open the marks`}
            onPress={onTray} hitSlop={10} style={{ minHeight: 28, paddingHorizontal: 10, borderRadius: 14, backgroundColor: C.amberPill, justifyContent: 'center' }}>
            <Text maxFontSizeMultiplier={BAR_CAP} style={{ fontFamily: F.semi, fontSize: 13, color: C.text }}>{markCount(count)}</Text>
          </Pressable>
        )}
      </View>
      <View style={{ flexDirection: 'row', gap: 6 }}>
        {ACTIONS.map((a) => {
          // right after a Keep, its own button is its Undo for a moment: the full 44 pt, where the thumb just was
          const undo = a.k === 'keep' && !!flash && !!onUndo;
          return (
          <Pressable key={a.k} testID={testID ? `${testID}-${undo ? 'undo' : a.k}` : undefined} accessibilityRole="button" accessibilityLabel={undo ? 'Undo the Keep' : a.label}
            accessibilityHint={undo ? undefined : target ? `${a.hint}: ${preview(target.text)}` : a.hint} accessibilityState={{ disabled: !on && !undo }} disabled={!on && !undo}
            onPress={() => (undo ? onUndo?.() : onAct(a.k))}
            style={({ pressed }) => ({
              flex: 1, minHeight: 44, borderRadius: 10, alignItems: 'center', justifyContent: 'center', flexDirection: 'row', gap: 6, paddingHorizontal: 4,
              backgroundColor: pressed ? C.card2 : C.card, opacity: on || undo ? 1 : 0.45,
            })}>
            {a.k !== 'elsewhere' && !undo && <MarkSwatch k={a.k} />}
            <Text numberOfLines={1} maxFontSizeMultiplier={BAR_CAP} style={{ fontFamily: F.semi, fontSize: 15, color: undo ? linkColor() : C.text }}>{undo ? 'Undo' : a.label}</Text>
          </Pressable>
          );
        })}
      </View>
    </View>
  );
  return <BarGround testID={testID}>{body}</BarGround>;
}

/** The bar's height on the keyboard (one 28-pt line and one 44-pt row with their padding), for keeping the caret above it. */
export const MARK_BAR_H = 82;
