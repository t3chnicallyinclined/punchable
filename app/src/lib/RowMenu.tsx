// A row's or card's context menu. On the web (and Android) there is no native context menu: the caller keeps its
// long press that opens the MenuSheet, and this renders the row as is. The phone's version is RowMenu.native.tsx.
import React from 'react';
import type { MenuItem } from './ui';

export type RowMenuProps = {
  /** Where a tap on the row goes (the row keeps its own onPress; the link only hosts the menu). */
  href: string;
  /** The menu's items in order; destructive ones last (HIG/context-menus). `icon` is an SF Symbol name. */
  items: MenuItem[];
  children: React.ReactElement;
};

export function RowMenu({ children }: RowMenuProps) {
  return children;
}

/** VoiceOver's custom actions for a row: the same verbs as the menu (HIG/accessibility: alternatives to gestures). */
export function rowActions(items: MenuItem[]) {
  const live = items.filter((it) => !it.disabled);
  return {
    accessibilityActions: live.map((it) => ({ name: it.label, label: it.label })),
    onAccessibilityAction: (e: { nativeEvent: { actionName: string } }) => live.find((it) => it.label === e.nativeEvent.actionName)?.onPress(),
  };
}
