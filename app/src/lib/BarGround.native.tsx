// The ground of a bar the user acts from, on the phone (the marks bar, Slice 3): Liquid Glass where the system draws it
// (iOS 26+, `isLiquidGlassAvailable`), in the app's own scheme; below that the flat surface of BarGround.tsx. The HIG on
// a view above the keyboard: "If your view looks out of place above the keyboard, apply Liquid Glass to the view".
import React from 'react';
import { View } from 'react-native';
import { GlassView, isLiquidGlassAvailable } from 'expo-glass-effect';
import { C } from './theme';

let glass = false;
try {
  glass = isLiquidGlassAvailable();
} catch {
  glass = false;
}

export function BarGround({ children, testID }: { children: React.ReactNode; testID?: string }) {
  if (glass) return <GlassView testID={testID} glassEffectStyle="regular" colorScheme={C.scheme}>{children}</GlassView>;
  return <View testID={testID} style={{ backgroundColor: C.surface, borderTopWidth: 1, borderTopColor: C.border }}>{children}</View>;
}
