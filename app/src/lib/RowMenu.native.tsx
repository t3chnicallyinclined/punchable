// A row's or card's context menu on the phone. iOS: the system context menu on a long press (expo-router's
// Link.Menu over react-native-screens' native link preview: the row lifts with the system blur, destructive items
// are red and last, the system plays its own haptic). The row keeps its own tap: the link's onPress prevents the
// default so nothing navigates twice. Android: no native context menu here, the caller's long press (MenuSheet)
// stays. The web version is RowMenu.tsx.
import React from 'react';
import { Platform } from 'react-native';
import { Link } from 'expo-router';
import type { RowMenuProps } from './RowMenu';

export type { RowMenuProps } from './RowMenu';
export { rowActions } from './RowMenu';

export function RowMenu({ href, items, children }: RowMenuProps) {
  if (Platform.OS !== 'ios') return children;
  return (
    <Link href={href as any} asChild onPress={(e) => e.preventDefault()}>
      <Link.Trigger>{children}</Link.Trigger>
      <Link.Menu>
        {items.map((it) => (
          <Link.MenuAction key={it.label} icon={it.icon as any} destructive={it.destructive} disabled={it.disabled} onPress={it.onPress}>{it.label}</Link.MenuAction>
        ))}
      </Link.Menu>
    </Link>
  );
}
