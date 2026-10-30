import React from 'react';
import { GestureHandlerRootView } from 'react-native-gesture-handler';

export function GestureRoot({ children, style }: { children: React.ReactNode; style?: any }) {
  return <GestureHandlerRootView style={[{ flex: 1 }, style]}>{children}</GestureHandlerRootView>;
}
