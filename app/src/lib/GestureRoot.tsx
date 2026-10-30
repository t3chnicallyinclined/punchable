// Web: no gesture-handler root needed (the web drag uses pointer events), so the library stays out of the web bundle.
import React from 'react';
import { View } from 'react-native';

export function GestureRoot({ children, style }: { children: React.ReactNode; style?: any }) {
  return <View style={[{ flex: 1 }, style]}>{children}</View>;
}
