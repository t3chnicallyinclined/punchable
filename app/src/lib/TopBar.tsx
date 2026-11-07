// The custom top bar (web; on the phone only the full-screen tasks: Stage, Record, Review, Import, the old set list)
// and the offline strip. The three tab roots live here because the bar's back fallback goes to one.
import React, { useEffect } from 'react';
import { AccessibilityInfo, Pressable, Text, View } from 'react-native';
import { router } from 'expo-router';
import { C, F, linkColor } from './theme';
import { useOnline } from './api';
import { useSession } from './auth';
import { Icon, SignInBtn, useUI } from './ui';

// The three tabs: Rant · Bits · Sets (SPEC 2026-11-06 R1: Rant took Tonight's place at '/'; the key stays
// `tonight` inside the app so the remembered last tab and the back fallback keep working). Ask is the Bits search.
export const ROOTS = [
  { key: 'tonight', label: 'Rant', path: '/' },
  { key: 'bits', label: 'Bits', path: '/bits' },
  { key: 'sets', label: 'Sets', path: '/sets' },
];

export function goRoot(key: string) {
  const r = ROOTS.find((x) => x.key === key);
  if (r) router.navigate(r.path as any);
}

const ROOT_LABEL: Record<string, string> = { tonight: 'Rant', bits: 'Bits', sets: 'Sets' };
/** The bar itself. The safe-area inset is NOT added here: `Screen` wraps every header (this one or a custom one) in it. */
export function TopBar({ title, back, onBack, right, task, left, backLabel }: { title: string; back?: boolean; onBack?: () => void; right?: React.ReactNode; task?: boolean; left?: React.ReactNode; backLabel?: string }) {
  const ui = useUI();
  // HIG: the back button carries the previous screen's name (the tab he came from unless the screen says otherwise)
  const label = backLabel ?? (ROOT_LABEL[ui.lastRoot] || 'Back');
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4, minHeight: 52, paddingTop: 4, paddingBottom: 4, paddingHorizontal: 8, backgroundColor: task ? C.bgDeep : C.bg, borderBottomWidth: 1, borderBottomColor: C.border }}>
      {left ? left : back ? (
        <Pressable testID="back" accessibilityRole="button" accessibilityLabel={`Back to ${label}`} onPress={onBack || (() => (router.canGoBack() ? router.back() : goRoot('tonight')))}
          style={{ minWidth: 44, height: 44, maxWidth: 140, borderRadius: 22, flexDirection: 'row', alignItems: 'center', paddingRight: 8 }}>
          <Icon name="back" size={24} color={C.amber} sw={2.4} />
          <Text maxFontSizeMultiplier={1.3} numberOfLines={1} style={{ fontFamily: F.medium, fontSize: 16, color: C.amber, marginLeft: -4 }}>{label}</Text>
        </Pressable>
      ) : <View style={{ width: 44 }} />}
      <Text maxFontSizeMultiplier={1.3} numberOfLines={1} style={{ flex: 1, textAlign: 'center', fontFamily: F.g6, fontSize: 17, color: C.text }}>{title}</Text>
      {right || <TopBarRight />}
    </View>
  );
}
function TopBarRight() {
  const sess = useSession();
  return sess.enabled && !sess.signedIn && !sess.checking ? <SignInBtn testID="topbar-signin" /> : <View style={{ width: 44 }} />;
}

/**
 * The offline line. `top` (default): the strip under the bar. `bottom`: one footnote line pinned above the bottom
 * (the bit page: status near the controls it describes, HIG/feedback); while online that slot shows `note` instead
 * (the one-word reason under the bottom button, "Reading v5"), or nothing.
 */
export function OfflineStrip({ placement = 'top', note }: { placement?: 'top' | 'bottom'; note?: string | null }) {
  const online = useOnline();
  const text = placement === 'bottom' ? (online ? note : 'Offline · reading only') : null;
  // iOS has no live region (accessibilityLiveRegion is Android-only): say the change once
  useEffect(() => {
    if (text) AccessibilityInfo.announceForAccessibility?.(text);
  }, [text]);
  if (placement === 'bottom') {
    if (!text) return null;
    return (
      <View testID={online ? 'footnote' : 'offline'} pointerEvents="none" style={{ paddingHorizontal: 16, paddingVertical: 4 }}>
        <Text maxFontSizeMultiplier={1.6} numberOfLines={2} style={{ fontFamily: F.semi, fontSize: 13, lineHeight: 18, color: online ? C.mut2 : linkColor(), textAlign: 'center' }}>{text}</Text>
      </View>
    );
  }
  if (online) return null;
  return (
    <View testID="offline" style={{ paddingHorizontal: 16, paddingVertical: 6, backgroundColor: C.warnBg, borderBottomWidth: 1, borderBottomColor: C.addBorder }}>
      <Text style={{ fontFamily: F.semi, fontSize: 13, color: C.amber, textAlign: 'center' }}>Offline · showing what this phone saved · imports wait in your outbox</Text>
    </View>
  );
}
/** Is the bottom slot showing anything (so a shell can pad its scroll content for it)? */
export function useBottomNote(note?: string | null): boolean {
  const online = useOnline();
  return !online || !!note;
}
