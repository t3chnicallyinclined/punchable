import type React from 'react';
import type { Stack } from 'expo-router';
import type { SearchBarProps } from 'react-native-screens';

/** The native stack header's options (expo-router's Stack.Screen), for the phone shell. */
export type NativeHeaderOptions = NonNullable<React.ComponentProps<typeof Stack.Screen>['options']>;

/** What a screen asks of its shell (lib/Screen.tsx on the web, lib/Screen.native.tsx on the phone). */
export type ScreenProps = {
  title: string; children: React.ReactNode; back?: boolean; onBack?: () => void; right?: React.ReactNode; dock?: React.ReactNode;
  task?: boolean; header?: React.ReactNode; testID?: string; deep?: boolean; scrollKey?: string | number | null; scroll?: boolean; maxWidth?: number;
  scrollRef?: React.MutableRefObject<any>;
  /** The input bar (lib/SayBar): pinned above the tab bar, the tabs stay (unlike `dock`). */
  bar?: React.ReactNode;
  /** Pull to refresh (HIG: on every list). */
  onRefresh?: () => Promise<any> | void;
  /** The back button's label (default: the tab he came from; on the phone the system shows the previous screen's title). */
  backLabel?: string;
  /** A large title: the system's collapsing one on the phone (headerLargeTitle); on the web, drawn at the top of the content. */
  largeTitle?: boolean;
  /** Phone only: the native header's search field (HIG/search-fields; the system supplies Clear and Cancel). */
  search?: SearchBarProps;
  /** Phone only: more native header options, merged last (headerTitle, headerBackTitle, ...). */
  nativeHeader?: Partial<NativeHeaderOptions>;
};
