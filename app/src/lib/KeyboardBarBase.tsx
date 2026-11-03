// The keyboard bar's shared part (docs/ENGINEERING.md §1: Base + shim + .native). The web and Android draw a flat
// bar; iOS 26 draws it on Liquid Glass (KeyboardBar.native.tsx).
import React from 'react';
import { View } from 'react-native';
import { C } from './theme';

/** The bar's height: three 44-pt items with 6 pt above and below (SPEC 2026-11-03 §4.1, check 2). */
export const KEYBOARD_BAR_H = 56;
export type KeyboardBarProps = { children: React.ReactNode; testID?: string };
export const KEYBOARD_BAR_GLASS = false;

export function KeyboardBar({ children, testID }: KeyboardBarProps) {
  return (
    <View testID={testID} style={{ height: KEYBOARD_BAR_H, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 10, backgroundColor: C.surface, borderTopWidth: 1, borderTopColor: C.border }}>
      {children}
    </View>
  );
}
