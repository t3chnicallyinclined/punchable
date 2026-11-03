// A collapsed section on the bit page (SPEC 2026-11-03 §3.1): label · meta · chevron. `kind: 'inline'` turns the
// chevron down and mounts its content under the row (no animation under Reduce Motion); `kind: 'push'` keeps the
// disclosure indicator on the right and navigates (HIG/lists-and-tables: "A disclosure indicator reveals the next
// level in a hierarchy; it doesn't show details about the item"). Height from the OS text size (the control cap,
// 1.6×); from the re-stack point the meta wraps under the label (HIG/typography: a stacked layout at large sizes).
import React from 'react';
import { LayoutAnimation, PixelRatio, Pressable, Text, View } from 'react-native';
import { ROW_CAP, rowHeight, stackMeta } from '@/rules/bitLines';
import { C, F } from './theme';
import { Icon, prefersReducedMotion } from './ui';

export type SectionRowProps = {
  label: string;
  /** The row's facts after the label ("7", "v8 · Nov 2 · 1:40"); none hides the dot. */
  meta?: string | null;
  kind: 'inline' | 'push';
  /** Inline rows: whether the content under the row is mounted. */
  open?: boolean;
  onPress: () => void;
  /** A trailing control that stays its own target (the non-timed Undo after a write). */
  right?: React.ReactNode;
  /** The spoken row when "label, meta" is not enough ("Versions, 8, newest November 2, run time 1:40"). */
  accessibilityLabel?: string;
  /** A quieter row (the Claude line): secondary text, a smaller size. */
  quiet?: boolean;
  /** VoiceOver actions the row offers besides its tap (the non-timed Undo that `right` draws). */
  extraActions?: { label: string; onPress: () => void }[];
  /** A leading mark (the AI badge on the Claude line). */
  leading?: React.ReactNode;
  disabled?: boolean;
  testID?: string;
  children?: React.ReactNode;
};

export function SectionRow({ label, meta, kind, open, onPress, right, accessibilityLabel, quiet, leading, disabled, extraActions, testID, children }: SectionRowProps) {
  const fs = PixelRatio.getFontScale();
  const stacked = stackMeta(fs);
  const press = () => {
    if (kind === 'inline' && !prefersReducedMotion()) LayoutAnimation.configureNext(LayoutAnimation.Presets.easeInEaseOut);
    onPress();
  };
  const size = quiet ? 15 : 17;
  const color = disabled ? C.dim : quiet ? C.text2 : C.text;
  return (
    <View>
      <Pressable testID={testID} accessibilityRole="button" disabled={disabled}
        accessibilityLabel={accessibilityLabel || (meta ? `${label}, ${meta}` : label)}
        accessibilityState={kind === 'inline' ? { expanded: !!open, disabled: !!disabled } : { disabled: !!disabled }}
        accessibilityActions={extraActions?.length ? extraActions.map((a) => ({ name: a.label, label: a.label })) : undefined}
        onAccessibilityAction={extraActions?.length ? (e) => extraActions.find((a) => a.label === e.nativeEvent.actionName)?.onPress() : undefined}
        onPress={press}
        style={({ pressed }) => ({ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: rowHeight(quiet ? 44 : 48, fs), paddingVertical: 6, borderBottomWidth: 1, borderBottomColor: C.hairline, backgroundColor: pressed ? C.card2 : 'transparent' })}>
        {leading}
        <View style={{ flex: 1, minWidth: 0, flexDirection: stacked ? 'column' : 'row', alignItems: stacked ? 'flex-start' : 'center', gap: stacked ? 2 : 6 }}>
          <Text maxFontSizeMultiplier={ROW_CAP} numberOfLines={stacked ? 2 : 1} style={{ fontFamily: F.semi, fontSize: size, lineHeight: size + 5, color }}>{label}</Text>
          {!!meta && <Text maxFontSizeMultiplier={ROW_CAP} numberOfLines={stacked ? 3 : 1} style={{ flexShrink: 1, fontFamily: F.body, fontSize: size, lineHeight: size + 5, color: disabled ? C.dim : C.mut2 }}>{stacked ? meta : `· ${meta}`}</Text>}
        </View>
        {right}
        {/* HIG/disclosure-controls: a disclosure button points down when its content is hidden and up when visible; the indicator (right) only on rows that push */}
        <Icon name={kind === 'inline' ? (open ? 'chevronUp' : 'chevronDown') : 'chevron'} size={Math.round(16 * Math.min(ROW_CAP, fs))} color={C.dim} />
      </Pressable>
      {kind === 'inline' && open && children}
    </View>
  );
}
