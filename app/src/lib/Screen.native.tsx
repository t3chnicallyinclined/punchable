// The screen shell on the phone. The tab bar is the system's (NativeTabs in app/(tabs)/_layout.tsx) and lives outside
// this component; the input bar and a task's dock float above it. The insets are read from a SafeAreaProvider nested
// inside the tab's content, so `bottom` includes the tab bar (and shrinks as it minimises on iOS 26) and `top` the
// status bar: no fixed device number anywhere (docs/design/HIG.md §12). The web shell is Screen.tsx.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { KeyboardAvoidingView, Platform, RefreshControl, ScrollView, Text, View } from 'react-native';
import { SafeAreaProvider, useSafeAreaInsets } from 'react-native-safe-area-context';
import { C, F } from './theme';
import type { ScreenProps } from './screenTypes';
import { OfflineStrip, TopBar } from './TopBar';
import { useUI, useWide } from './ui';

export type { ScreenProps } from './screenTypes';

export function Screen(p: ScreenProps) {
  return (
    <SafeAreaProvider style={{ flex: 1 }}>
      <Body {...p} />
    </SafeAreaProvider>
  );
}

function Body({ title, children, back, onBack, right, dock, task, header, testID, deep, scrollKey, scroll = true, maxWidth, scrollRef, bar, onRefresh, backLabel, largeTitle }: ScreenProps) {
  const wide = useWide();
  const ui = useUI();
  const ins = useSafeAreaInsets();
  const sv = useRef<any>(null);
  useEffect(() => {
    if (scrollKey != null) sv.current?.scrollTo?.({ y: 0, animated: true });
  }, [scrollKey]);
  const barH = bar ? 120 : 0;
  const dockH = dock ? 96 : 0;
  // the snackbar sits above whatever floats at the bottom (the input bar, a task's dock, the system tab bar)
  const pinned = barH + dockH + ins.bottom;
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
  const ground = deep ? C.bgDeep : C.bg;
  const mw = maxWidth || (wide ? 1120 : 720);
  const kav = (node: React.ReactNode) => (Platform.OS === 'ios' ? <KeyboardAvoidingView behavior="padding" style={{ flex: 1 }}>{node}</KeyboardAvoidingView> : <>{node}</>);
  return (
    <View testID={testID} style={{ flex: 1, backgroundColor: ground }}>
      {kav(
      <View style={{ flex: 1, minWidth: 0 }}>
        {/* HIG/layout: every header starts below the status bar and the Dynamic Island; the inset is the system's */}
        <View testID="header-inset" style={{ zIndex: 4, paddingTop: ins.top, backgroundColor: task || deep ? C.bgDeep : C.bg }}>
          {header || <TopBar title={barTitle} back={back} onBack={onBack} right={right} task={task} backLabel={backLabel} />}
        </View>
        <OfflineStrip />
        {scroll ? (
          <ScrollView ref={(r) => { sv.current = r; if (scrollRef) scrollRef.current = r; }} style={{ flex: 1 }} contentInsetAdjustmentBehavior="automatic"
            keyboardShouldPersistTaps="handled" keyboardDismissMode="on-drag"
            contentContainerStyle={{ paddingHorizontal: wide ? 32 : 16, paddingTop: 12, paddingBottom: 24 + barH + dockH }}
            scrollEventThrottle={32} onScroll={largeTitle ? (e) => setCollapsed(e.nativeEvent.contentOffset.y > 36) : undefined}
            refreshControl={onRefresh ? <RefreshControl refreshing={refreshing} onRefresh={refresh} tintColor={C.amber} colors={[C.amber]} progressBackgroundColor={C.surface} /> : undefined}>
            <View style={{ width: '100%', maxWidth: mw, alignSelf: 'center', gap: 16 }}>
              {largeTitle && <Text maxFontSizeMultiplier={1.3} accessibilityRole="header" style={{ fontFamily: F.black, fontSize: 34, letterSpacing: -0.7, color: C.text }}>{title}</Text>}
              {children}
            </View>
          </ScrollView>
        ) : (
          <View style={{ flex: 1, paddingBottom: barH + dockH }}>{children}</View>
        )}
        {bar && (
          <View testID="say-dock" style={{ position: 'absolute', left: 0, right: 0, bottom: ins.bottom, zIndex: 6, paddingBottom: 4, backgroundColor: ground }}>
            {bar}
          </View>
        )}
        {dock && (
          <View testID="dock" style={{ position: 'absolute', left: 0, right: 0, bottom: 0, zIndex: 6, paddingHorizontal: 16, paddingTop: 12, paddingBottom: ins.bottom + 12, backgroundColor: deep ? C.bgDeep : C.surface, borderTopWidth: 1, borderTopColor: C.border, gap: 8 }}>
            <View style={{ width: '100%', maxWidth: 720, alignSelf: 'center', gap: 8 }}>{dock}</View>
          </View>
        )}
      </View>)}
    </View>
  );
}
