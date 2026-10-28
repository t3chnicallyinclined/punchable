import type React from 'react';

/** What a screen asks of its shell (lib/Screen.tsx on the web, lib/Screen.native.tsx on the phone). */
export type ScreenProps = {
  title: string; children: React.ReactNode; back?: boolean; onBack?: () => void; right?: React.ReactNode; dock?: React.ReactNode;
  task?: boolean; header?: React.ReactNode; testID?: string; deep?: boolean; scrollKey?: string | number | null; scroll?: boolean; maxWidth?: number;
  scrollRef?: React.MutableRefObject<any>;
  /** The input bar (lib/SayBar): pinned above the tab bar, the tabs stay (unlike `dock`). */
  bar?: React.ReactNode;
  /** Pull to refresh (HIG: on every list). */
  onRefresh?: () => Promise<any> | void;
  /** The back button's label (default: the tab he came from). */
  backLabel?: string;
  /** A large title at the top of the content that collapses into the bar as he scrolls. */
  largeTitle?: boolean;
};
