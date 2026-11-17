// The ground of a bar the user acts from (the marks bar, Slice 3): a flat surface with a hairline on the web and
// Android. The phone draws it on Liquid Glass where the system has it (BarGround.native.tsx), as lib/KeyboardBar does.
import React from 'react';
import { View } from 'react-native';
import { C } from './theme';

export function BarGround({ children, testID }: { children: React.ReactNode; testID?: string }) {
  return <View testID={testID} style={{ backgroundColor: C.surface, borderTopWidth: 1, borderTopColor: C.border }}>{children}</View>;
}
