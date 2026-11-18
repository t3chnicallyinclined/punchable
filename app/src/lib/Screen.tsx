// The screen shell on the web: the custom TopBar, a slim sidebar at >= 900 px (the brand, Notebook, You), the pinned
// input bar / dock. No tabs (docs/design/SPEC-2026-11-17-one-notebook.md): the app is one list, Notebook, and pages
// pushed from it. The phone has its own shell (Screen.native.tsx): the system's headers.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { KeyboardAvoidingView, Platform, Pressable, RefreshControl, ScrollView, Text, View } from 'react-native';
import { router, usePathname } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { C, F } from './theme';
import { useSession } from './auth';
import type { ScreenProps } from './screenTypes';
import { OfflineStrip, TopBar, goHome, useBottomNote } from './TopBar';
import { Face, Icon, SignInBtn, useUI, useWide } from './ui';

export type { ScreenProps } from './screenTypes';

function Sidebar({ home }: { home: boolean }) {
  const ui = useUI();
  return (
    <View testID="sidebar" style={{ width: 232, backgroundColor: C.surface, borderRightWidth: 1, borderRightColor: C.border, paddingVertical: 18, paddingHorizontal: 12, gap: 2 }}>
      <Text style={{ fontFamily: F.g7, fontSize: 20, color: C.text, paddingHorizontal: 12, paddingTop: 4, paddingBottom: 16 }}>Punchable</Text>
      <Pressable testID="side-notebook" accessibilityRole="button" accessibilityState={{ selected: home }} onPress={goHome}
        style={(st: any) => ({ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 44, paddingHorizontal: 12, borderRadius: 8, backgroundColor: home || st.hovered ? C.surface2 : 'transparent' })}>
        <Icon name="notebook" size={18} color={home ? C.amber : C.mut} />
        <Text style={{ fontFamily: F.medium, fontSize: 15, color: home ? C.amber : C.mut, flex: 1 }}>Notebook</Text>
      </Pressable>
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
 * A screen. Notebook is the root; pushed screens get a back button. Tasks (Record, Import, the walk-through) get their
 * own header and a dock pinned to the bottom. `scroll={false}` hands the body to the child (a virtualized list).
 */
export function Screen({ title, children, back, onBack, right, dock, task, header, testID, deep, scrollKey, scroll = true, maxWidth, scrollRef, bar, onRefresh, backLabel, largeTitle, offlineAt = 'top', footnote, offlineNote, scrollWatch }: ScreenProps) {
  const wide = useWide();
  const sv = useRef<any>(null);
  useEffect(() => {
    if (scrollKey != null) sv.current?.scrollTo?.({ y: 0, animated: true });
  }, [scrollKey]);
  const ui = useUI();
  const pathname = usePathname();
  const ins = useSafeAreaInsets();
  const noteOn = useBottomNote(footnote) && offlineAt === 'bottom';
  const [noteMeasured, setNoteMeasured] = useState(0);
  const noteH = noteOn ? noteMeasured : 0;
  const bottomPad = (dock ? (wide ? 160 : 200) : 48 + (bar ? 120 : 0)) + noteH;
  const mw = maxWidth || (wide ? 1120 : 720);
  // the snackbar sits above whatever is pinned at the bottom (the bar, the dock)
  const pinned = (dock ? 96 : 0) + (bar ? 120 : 0) + noteH + (dock || bar ? ins.bottom : 0);
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
  // a list that owns its scrolling (`scroll={false}`) cannot report where it is: its large title is the bar's title
  const barTitle = largeTitle && scroll && !collapsed ? '' : title;
  const kav = (node: React.ReactNode) => (Platform.OS === 'ios' ? <KeyboardAvoidingView behavior="padding" style={{ flex: 1 }}>{node}</KeyboardAvoidingView> : <>{node}</>);
  return (
    <View testID={testID} style={{ flex: 1, flexDirection: 'row', backgroundColor: deep ? C.bgDeep : C.bg }}>
      {wide && !task && <Sidebar home={pathname === '/' || pathname === ''} />}
      {kav(
      <View style={{ flex: 1, minWidth: 0 }}>
        {/* HIG/layout: every header, custom or not, starts below the status bar and the Dynamic Island (the inset is the
            system's; no fixed number). The wrapper carries the bar's ground so the inset never shows a seam. */}
        <View testID="header-inset" style={{ zIndex: 4, paddingTop: ins.top, backgroundColor: task || deep ? C.bgDeep : C.bg }}>
          {header || <TopBar title={barTitle} back={back} onBack={onBack} right={right} task={task} backLabel={backLabel} />}
        </View>
        {offlineAt === 'top' && <OfflineStrip />}
        {scroll ? (
          <ScrollView ref={(r) => { sv.current = r; if (scrollRef) scrollRef.current = r; }} style={{ flex: 1 }} keyboardShouldPersistTaps="handled" keyboardDismissMode={scrollWatch?.keyboardDismissMode || 'on-drag'}
            contentContainerStyle={{ paddingHorizontal: wide ? 32 : 16, paddingTop: wide ? 24 : 12, paddingBottom: bottomPad }}
            scrollEventThrottle={32} onScroll={largeTitle ? (e) => setCollapsed(e.nativeEvent.contentOffset.y > 36) : undefined}
            refreshControl={onRefresh ? <RefreshControl refreshing={refreshing} onRefresh={refresh} tintColor={C.amber} colors={[C.amber]} progressBackgroundColor={C.surface} /> : undefined}>
            <View style={{ width: '100%', maxWidth: mw, alignSelf: 'center', gap: 16 }}>
              {largeTitle && <Text maxFontSizeMultiplier={1.3} accessibilityRole="header" style={{ fontFamily: F.black, fontSize: 34, letterSpacing: -0.7, color: C.text }}>{title}</Text>}
              {children}
            </View>
          </ScrollView>
        ) : (
          <View style={{ flex: 1, paddingBottom: bar ? 80 : 0 }}>{children}</View>
        )}
        {bar && (
          <View testID="say-dock" style={{ position: 'absolute', left: 0, right: 0, bottom: wide ? 0 : ins.bottom, zIndex: 6, paddingBottom: 12, backgroundColor: deep ? C.bgDeep : C.bg }}>
            {bar}
          </View>
        )}
        {dock && (
          <View testID="dock" style={{ position: 'absolute', left: 0, right: 0, bottom: 0, zIndex: 6, paddingHorizontal: 16, paddingTop: 12, paddingBottom: ins.bottom + 12, backgroundColor: deep ? C.bgDeep : C.surface, borderTopWidth: 1, borderTopColor: C.border, gap: 8 }}>
            <View style={{ width: '100%', maxWidth: 720, alignSelf: 'center', gap: 8 }}>{dock}</View>
          </View>
        )}
        {offlineAt === 'bottom' && noteOn && (
          <View testID="bottom-note" pointerEvents="none" onLayout={(e) => setNoteMeasured(Math.ceil(e.nativeEvent.layout.height))} style={{ position: 'absolute', left: 0, right: 0, bottom: (wide ? 0 : ins.bottom) + (dock ? 96 : 0) + (bar ? 120 : 0), zIndex: 5, backgroundColor: deep ? C.bgDeep : C.bg }}>
            <OfflineStrip placement="bottom" note={footnote} offlineNote={offlineNote} />
          </View>
        )}
      </View>)}
    </View>
  );
}
