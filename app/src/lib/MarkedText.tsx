// Marks drawn on a text (SPEC 2026-11-16 Slice 3): quiet highlights by type behind the words, never in them.
//   - MarkLayer: a copy of the text laid out exactly as the input above it lays it out (same type, same width), in
//     transparent ink; only the grounds and the lines of its marked runs show. The bit's document (lib/BitDoc) draws it
//     under his words while the keyboard is down; the draft screen draws it under the draft.
//   - MarkedDoc: a document that is read and selected, never typed into (the draft he is reading, round two): one
//     read-only multiline input, so a long-press selects words as anywhere on iOS and the page hears the range.
// Never colour alone (HIG/color, HIG/accessibility "offer visual indicators, like distinct shapes"): a Change is also
// underlined, an Ask underlined with dots, a Keep underlined twice, a changed line of a draft a pale ground only; the
// bar's swatches carry the same lines (MarkSwatch), and the tray names every mark's type in words.
import React from 'react';
import { Platform, Text, TextInput, View, type TextStyle } from 'react-native';
import { C } from './theme';
import { PROSE_CAP } from '@/rules/bitLines';
import { highlights, type Mark, type MarkType } from '@/rules/markRules';

type Kind = MarkType | 'tint';

/** The ground of each kind of run. Read at render, so the Light / Dark switch reaches it. */
export function markGround(k: Kind): string {
  return { change: C.amberPill, ask: C.violetSoft, keep: C.greenSoft, tint: C.blueSoft }[k];
}

/** The line under each kind of run (none for a tint). The ink of the layer is transparent, so the line's colour is set
 *  on its own. */
export function markLine(k: Kind): TextStyle {
  if (k === 'change') return { textDecorationLine: 'underline', textDecorationStyle: 'solid', textDecorationColor: C.amber };
  if (k === 'ask') return { textDecorationLine: 'underline', textDecorationStyle: 'dotted', textDecorationColor: C.cluster2 };
  if (k === 'keep') return { textDecorationLine: 'underline', textDecorationStyle: 'double', textDecorationColor: C.green };
  return {};
}

/** The bar's and the tray's legend: the mark's ground with its line. */
export function MarkSwatch({ k }: { k: MarkType }) {
  // iOS draws a dotted border only when every side has the same colour (RCTBorderDrawing): the colour is set on all
  // four, the width on the bottom alone
  const line = k === 'change' ? { borderBottomWidth: 2, borderColor: C.amber }
    : k === 'ask' ? { borderBottomWidth: 2, borderColor: C.cluster2, borderStyle: 'dotted' as const }
    : { borderBottomWidth: 3, borderColor: C.green }; // Keep: a heavier line (RN has no double border)
  return <View style={[{ width: 12, height: 12, borderRadius: 2, backgroundColor: markGround(k) }, line]} />;
}

export function MarkLayer({ text, marks, tint, style }: { text: string; marks: readonly Mark[]; tint?: readonly boolean[]; style: TextStyle }) {
  const runs = highlights(text, marks, tint);
  return (
    <Text accessibilityElementsHidden importantForAccessibility="no-hide-descendants" pointerEvents="none" maxFontSizeMultiplier={PROSE_CAP}
      style={[style, { position: 'absolute', left: 0, right: 0, top: 0, color: 'transparent' }]}>
      {runs.map((r, i) => <Text key={i} style={r.type ? [{ backgroundColor: markGround(r.type) }, markLine(r.type)] : undefined}>{r.text}</Text>)}
    </Text>
  );
}

/** A document he reads and selects: the draft (with its changed lines tinted and his marks on it), or his own words. */
export function MarkedDoc({ text, marks, tint, style, onSelection, testID, label }: {
  text: string; marks: readonly Mark[]; tint?: readonly boolean[]; style: TextStyle;
  onSelection?: (sel: { start: number; end: number }) => void; testID?: string; label: string;
}) {
  return (
    <View testID={testID}>
      <MarkLayer text={text} marks={marks} tint={tint} style={style} />
      <TextInput testID={testID ? `${testID}-text` : undefined} value={text} editable={false} multiline scrollEnabled={false}
        maxFontSizeMultiplier={PROSE_CAP} accessibilityLabel={label} textAlignVertical="top"
        onSelectionChange={(e) => onSelection?.(e.nativeEvent.selection)}
        style={[style, Platform.OS === 'web' ? ({ outlineStyle: 'none' } as any) : null, { color: C.text }]} />
    </View>
  );
}
