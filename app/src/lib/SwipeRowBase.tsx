// Web: no swipe actions (the HIG audit: on the web the buttons and the long-press menu stay primary), so the row
// renders as it is. iOS/Android use SwipeRow.native.tsx (labelled leading/trailing actions, gesture-handler).
import React from 'react';
import { View } from 'react-native';

export type SwipeAction = { label: string; onPress: () => void; color?: string; destructive?: boolean; testID?: string };
export type SwipeRowProps = { children: React.ReactNode; leading?: SwipeAction | null; trailing?: SwipeAction | null; second?: SwipeAction | null; testID?: string; onLongPress?: () => void };

export function SwipeRow({ children, testID }: SwipeRowProps) {
  return <View testID={testID}>{children}</View>;
}
