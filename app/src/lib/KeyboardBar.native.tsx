// The bar above the keyboard in focus mode (SPEC 2026-11-03 §3.3; DESIGN.md §5.4: the one other place glass is
// allowed). Liquid Glass where the system draws it (iOS 26+, `isLiquidGlassAvailable`), following the app's own
// scheme so an in-app Light override never leaves dark glass; below that a flat surface with a hairline. Not an
// `InputAccessoryView` (deprecated in RN 0.88): a plain view at the bottom of the screen's KeyboardAvoidingView.
// Shipped first in the spike screen (device check 2); the focus route uses it from Slice 2. The web twin is KeyboardBar.tsx.
import React from 'react';
import { View } from 'react-native';
import { GlassView, isLiquidGlassAvailable } from 'expo-glass-effect';
import { KEYBOARD_BAR_H, type KeyboardBarProps } from './KeyboardBarBase';
import { C } from './theme';

let glass = false;
try {
  glass = isLiquidGlassAvailable();
} catch {
  glass = false;
}
/** Whether the bar draws on Liquid Glass here (the spike shows the word so the owner knows which path he is looking at). */
export const KEYBOARD_BAR_GLASS = glass;

export function KeyboardBar({ children, testID }: KeyboardBarProps) {
  const inner = { height: KEYBOARD_BAR_H, flexDirection: 'row' as const, alignItems: 'center' as const, gap: 8, paddingHorizontal: 10 };
  if (glass) {
    return (
      <GlassView testID={testID} glassEffectStyle="regular" colorScheme={C.scheme} style={inner}>
        {children}
      </GlassView>
    );
  }
  return <View testID={testID} style={[inner, { backgroundColor: C.surface, borderTopWidth: 1, borderTopColor: C.border }]}>{children}</View>;
}
