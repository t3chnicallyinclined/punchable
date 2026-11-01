// Web: no swipe actions (the HIG audit: on the web the buttons and the long-press menu stay primary), so the row
// renders as it is. iOS/Android use SwipeRow.native.tsx (labelled leading/trailing actions, gesture-handler).
import React from 'react';
import { View } from 'react-native';

export type SwipeAction = { label: string; onPress: () => void; color?: string; destructive?: boolean; testID?: string };
export type SwipeRowProps = {
  children: React.ReactNode; leading?: SwipeAction | null; trailing?: SwipeAction | null; second?: SwipeAction | null; testID?: string; onLongPress?: () => void;
  /** The row brings its own VoiceOver elements (several, with the swipe verbs among their actions): the wrapper is not
   *  one accessibility element, so it does not hide them (an accessible parent hides its children on iOS). */
  a11yPassThrough?: boolean;
};

export function SwipeRow({ children, testID }: SwipeRowProps) {
  return <View testID={testID}>{children}</View>;
}
