// The screen shell on the web: the custom TopBar, the bottom tab bar (phone width) or the sidebar (>= 900 px), the
// pinned input bar / dock. The phone has its own shell (Screen.native.tsx): native tabs and, from B3, native headers.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { KeyboardAvoidingView, Platform, Pressable, RefreshControl, ScrollView, Text, View } from 'react-native';
import { router, usePathname } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { C, F } from './theme';
import { useSession } from './auth';
import { haptics } from './haptics';
import type { ScreenProps } from './screenTypes';
import { OfflineStrip, ROOTS, TopBar, goRoot } from './TopBar';
import { Face, Icon, SignInBtn, useUI, useWide } from './ui';

export type { ScreenProps } from './screenTypes';

// The three tabs (docs/UX_2026-10.md): Tonight · Bits · Sets. Ask is on every screen (the input bar), not a tab.
// screens one tap down, and the tab they belong under (the old tabs Write / Boards / Notebook stay as routes)
const PUSHED_ROOT: Record<string, string> = {
  joke: 'bits', bit: 'bits', map: 'bits', notes: 'bits', desk: 'bits', notebook: 'bits', shows: 'bits', set: 'bits', venue: 'bits', calendar: 'bits',
  merge: 'bits', write: 'bits', today: 'tonight', import: 'tonight', record: 'tonight', inbox: 'tonight', say: 'tonight', ask: 'tonight',
  board: 'sets', builder: 'sets', list: 'sets', archived: 'sets', bitnotes: 'sets', boards: 'sets', workingset: 'sets', setlist: 'sets', myset: 'sets', stage: 'sets',
  settings: 'tonight', profile: 'tonight', learned: 'tonight',
};

function rootOf(pathname: string, last: string) {
  if (pathname === '/' || pathname === '') return 'tonight';
  const k = pathname.split('/')[1];
  if (ROOTS.some((r) => r.key === k)) return k;
  return PUSHED_ROOT[k] && last === 'tonight' && k !== 'settings' && k !== 'profile' ? PUSHED_ROOT[k] : last;
}


function BottomNav({ active }: { active: string }) {
  const ins = useSafeAreaInsets();
  const tab = (r: (typeof ROOTS)[number]) => {
    const on = active === r.key;
    return (
      <Pressable key={r.key} testID={`tab-${r.key}`} accessibilityRole="tab" accessibilityState={{ selected: on }} accessibilityLabel={r.label}
        onPress={() => { if (!on) haptics.selection(); goRoot(r.key); }} style={{ flex: 1, alignItems: 'center', justifyContent: 'center', gap: 4, minHeight: 48 }}>
        <Icon name={r.key} color={on ? C.amber : C.tabOff} size={24} />
        <Text maxFontSizeMultiplier={1.2} style={{ fontFamily: F.semi, fontSize: 11, color: on ? C.amber : C.tabOff }}>{r.label}</Text>
      </Pressable>
    );
  };
  return (
    <View testID="bottom-nav" style={{ position: 'absolute', left: 0, right: 0, bottom: 0, zIndex: 5, flexDirection: 'row', height: 64 + ins.bottom, paddingBottom: ins.bottom, backgroundColor: C.bg, borderTopWidth: 1, borderTopColor: C.hairline }}>
      {ROOTS.map(tab)}
    </View>
  );
}

function Sidebar({ active }: { active: string }) {
  const ui = useUI();
  const item = (key: string, label: string, onPress: () => void, icon?: string) => {
    const on = active === key;
    return (
      <Pressable key={key} testID={`side-${key}`} accessibilityRole="button" accessibilityState={{ selected: on }} onPress={onPress}
        style={(st: any) => ({ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 44, paddingHorizontal: 12, borderRadius: 8, backgroundColor: on || st.hovered ? C.surface2 : 'transparent' })}>
        {icon && <Icon name={icon} size={18} color={on ? C.amber : C.mut} />}
        <Text style={{ fontFamily: F.medium, fontSize: 15, color: on ? C.amber : C.mut, flex: 1 }}>{label}</Text>
      </Pressable>
    );
  };
  return (
    <View testID="sidebar" style={{ width: 232, backgroundColor: C.surface, borderRightWidth: 1, borderRightColor: C.border, paddingVertical: 18, paddingHorizontal: 12, gap: 2 }}>
      <Text style={{ fontFamily: F.g7, fontSize: 20, color: C.text, paddingHorizontal: 12, paddingTop: 4, paddingBottom: 16 }}>Punchable</Text>
      {ROOTS.map((r) => item(r.key, r.label, () => goRoot(r.key), r.key))}
      <View style={{ flex: 1 }} />
      {ui.workingSet && (
        <Pressable testID="side-working-set" accessibilityRole="button" onPress={() => router.push(`/workingset/${encodeURIComponent(ui.workingSet!.id)}` as any)}
          style={{ borderWidth: 1, borderColor: C.border, borderRadius: 10, padding: 10, gap: 2, marginBottom: 8, minHeight: 44 }}>
          <Text style={{ fontFamily: F.g7, fontSize: 11, letterSpacing: 0.6, color: C.amber }}>★ WORKING SET</Text>
          <Text numberOfLines={1} style={{ fontFamily: F.medium, fontSize: 14, color: C.text }}>{ui.workingSet.title}</Text>
          <Text style={{ fontFamily: F.body, fontSize: 12, color: C.mut }}>{ui.workingSet.sub}</Text>
        </Pressable>
      )}
      <SidebarYou />
    </View>
  );
}

/** Bottom of the desktop sidebar: who is signed in (opens You), or Sign in. */
function SidebarYou() {
  const sess = useSession();
  if (sess.enabled && !sess.signedIn) return <SignInBtn testID="side-signin" full />;
  const p = sess.profile;
  return (
    <Pressable testID="side-you" accessibilityRole="button" accessibilityLabel="You: profile, sign out, settings" onPress={() => router.push('/profile' as any)}
      style={(st: any) => ({ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 48, paddingHorizontal: 8, borderRadius: 8, backgroundColor: st.hovered ? C.surface2 : 'transparent' })}>
      <Face size={32} />
      <View style={{ flex: 1, minWidth: 0 }}>
        <Text numberOfLines={1} style={{ fontFamily: F.medium, fontSize: 14, color: C.text }}>{p?.name || 'You'}</Text>
        <Text numberOfLines={1} style={{ fontFamily: F.body, fontSize: 12, color: C.mut }}>Profile · settings</Text>
      </View>
    </Pressable>
  );
}


/**
 * A screen. Root tabs get the bottom nav (phone) or the sidebar (>= 900 px).
 * Pushed screens get a back button. Tasks (Record, Import, Set list) get their own header and a dock pinned to the bottom.
 * `scroll={false}` hands the body to the child (a virtualized list).
 */
export function Screen({ title, children, back, onBack, right, dock, task, header, testID, deep, scrollKey, scroll = true, maxWidth, scrollRef, bar, onRefresh, backLabel, largeTitle }: ScreenProps) {
  const wide = useWide();
  const sv = useRef<any>(null);
  useEffect(() => {
    if (scrollKey != null) sv.current?.scrollTo?.({ y: 0, animated: true });
  }, [scrollKey]);
  const ui = useUI();
  const pathname = usePathname();
  const active = rootOf(pathname, ui.lastRoot);
  useEffect(() => {
    if (ROOTS.some((r) => r.key === active) && active !== ui.lastRoot && rootOf(pathname, '') === active) ui.setLastRoot(active);
  }, [active, pathname]); // eslint-disable-line react-hooks/exhaustive-deps
  const ins = useSafeAreaInsets();
  const showNav = !wide && !task && !dock;
  const bottomPad = dock ? (wide ? 160 : 200) : (showNav ? 104 + ins.bottom : 48) + (bar ? 120 : 0);
  const mw = maxWidth || (wide ? 1120 : 720);
  // the snackbar sits above whatever is pinned at the bottom (the input bar, the dock, the tabs)
  const pinned = (dock ? 96 : 0) + (bar ? 120 : 0) + (showNav ? 64 : 0) + (dock || showNav ? ins.bottom : 0);
  useEffect(() => {
    ui.setPinnedHeight(pinned);
  }, [pinned]); // eslint-disable-line react-hooks/exhaustive-deps
  const [refreshing, setRefreshing] = useState(false);
  const refresh = useCallback(async () => {
    if (!onRefresh) return;
    setRefreshing(true);
    try {
      await onRefresh();
    } finally {
      setRefreshing(false);
    }
  }, [onRefresh]);
  const [collapsed, setCollapsed] = useState(false);
  const barTitle = largeTitle && !collapsed ? '' : title;
  const kav = (node: React.ReactNode) => (Platform.OS === 'ios' ? <KeyboardAvoidingView behavior="padding" style={{ flex: 1 }}>{node}</KeyboardAvoidingView> : <>{node}</>);
  return (
    <View testID={testID} style={{ flex: 1, flexDirection: 'row', backgroundColor: deep ? C.bgDeep : C.bg }}>
      {wide && !task && <Sidebar active={active} />}
      {kav(
      <View style={{ flex: 1, minWidth: 0 }}>
        {/* HIG/layout: every header, custom or not, starts below the status bar and the Dynamic Island (the inset is the
            system's; no fixed number). The wrapper carries the bar's ground so the inset never shows a seam. */}
        <View testID="header-inset" style={{ zIndex: 4, paddingTop: ins.top, backgroundColor: task || deep ? C.bgDeep : C.bg }}>
          {header || <TopBar title={barTitle} back={back} onBack={onBack} right={right} task={task} backLabel={backLabel} />}
        </View>
        <OfflineStrip />
        {scroll ? (
          <ScrollView ref={(r) => { sv.current = r; if (scrollRef) scrollRef.current = r; }} style={{ flex: 1 }} keyboardShouldPersistTaps="handled" keyboardDismissMode="on-drag"
            contentContainerStyle={{ paddingHorizontal: wide ? 32 : 16, paddingTop: wide ? 24 : 12, paddingBottom: bottomPad }}
            scrollEventThrottle={32} onScroll={largeTitle ? (e) => setCollapsed(e.nativeEvent.contentOffset.y > 36) : undefined}
            refreshControl={onRefresh ? <RefreshControl refreshing={refreshing} onRefresh={refresh} tintColor={C.amber} colors={[C.amber]} progressBackgroundColor={C.surface} /> : undefined}>
            <View style={{ width: '100%', maxWidth: mw, alignSelf: 'center', gap: 16 }}>
              {largeTitle && <Text maxFontSizeMultiplier={1.3} accessibilityRole="header" style={{ fontFamily: F.black, fontSize: 34, letterSpacing: -0.7, color: C.text }}>{title}</Text>}
              {children}
            </View>
          </ScrollView>
        ) : (
          <View style={{ flex: 1, paddingBottom: (showNav ? 64 + ins.bottom : 0) + (bar ? 112 : 0) }}>{children}</View>
        )}
        {bar && (
          <View testID="say-dock" style={{ position: 'absolute', left: 0, right: 0, bottom: showNav ? 64 + ins.bottom : wide ? 0 : ins.bottom, zIndex: 6, paddingBottom: showNav ? 4 : 12, backgroundColor: deep ? C.bgDeep : C.bg }}>
            {bar}
          </View>
        )}
        {dock && (
          <View testID="dock" style={{ position: 'absolute', left: 0, right: 0, bottom: 0, zIndex: 6, paddingHorizontal: 16, paddingTop: 12, paddingBottom: ins.bottom + 12, backgroundColor: deep ? C.bgDeep : C.surface, borderTopWidth: 1, borderTopColor: C.border, gap: 8 }}>
            <View style={{ width: '100%', maxWidth: 720, alignSelf: 'center', gap: 8 }}>{dock}</View>
          </View>
        )}
        {showNav && <BottomNav active={active} />}
      </View>)}
    </View>
  );
}

