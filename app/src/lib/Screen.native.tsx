// The screen shell on the phone. The header is the system's (the native stack header: large title that collapses,
// the standard Back carrying the previous title, the search field, Dynamic Type and Bold Text for free), set from
// here with Stack.Screen; a full-screen task (`task`) or a screen with its own `header` keeps a custom bar under
// the safe-area inset. The tab bar is the system's too (app/(tabs)/_layout.tsx). The insets come from a
// SafeAreaProvider nested inside the tab's content, so `bottom` includes the tab bar (and shrinks as it minimises on
// iOS 26): no fixed device number anywhere (docs/design/HIG.md §12). The web shell is Screen.tsx.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { KeyboardAvoidingView, Platform, RefreshControl, ScrollView, View } from 'react-native';
import { Stack } from 'expo-router';
import { SafeAreaProvider, useSafeAreaInsets } from 'react-native-safe-area-context';
import { C } from './theme';
import type { NativeHeaderOptions, ScreenProps } from './screenTypes';
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

function Body({ title, children, back, onBack, right, dock, task, header, testID, deep, scrollKey, scroll = true, maxWidth, scrollRef, bar, onRefresh, backLabel, largeTitle, search, nativeHeader }: ScreenProps) {
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
  const ground = deep ? C.bgDeep : C.bg;
  const mw = maxWidth || (wide ? 1120 : 720);
  const custom = !!header || !!task;
  // HIG/toolbars: the system header. The tint colours the buttons only; title styles stay default so SF Pro, Dynamic
  // Type and Bold Text apply. No bar background (the scroll-edge effect does the work).
  // On iOS 26 (Liquid Glass) expo-router's header is see-through unless something sets its background
  // (react-navigation/native-stack/views/useHeaderConfigProps.js: usesSystemBackground → headerTransparent, a
  // 'transparent' bar, translucent), and react-native-screens then lays the content out UNDER the bar with no inset
  // (ScreenStackItem getSafeAreaEdges: translucent → no top edge). That is right for the shell's own ScrollView
  // (contentInsetAdjustmentBehavior="automatic" pushes it down and the scroll-edge effect blurs it under the bar).
  // A screen that brings its own body (`scroll={false}`: Bits' FlatList with the pinned search field, the maps, the
  // notebook) gets no inset, so its list ran under the search field (2026-10-29, build 12). For those the bar is not
  // transparent: the content starts below it (RNS insets it with a SafeAreaView), the bar keeps the screen's
  // background and the large title still collapses. expo-router's own rule: "If you don't have a ScrollView, specify
  // headerTransparent: false" (docs.expo.dev/router/advanced/stack, headerSearchBarOptions).
  const options: NativeHeaderOptions = custom
    ? { headerShown: false }
    : {
        headerShown: true, title, headerLargeTitle: !!largeTitle, headerLargeTitleShadowVisible: false, headerBackButtonDisplayMode: 'default',
        headerBackTitle: backLabel, headerTintColor: C.amber, headerSearchBarOptions: search, contentStyle: { backgroundColor: ground },
        ...(scroll ? {} : { headerTransparent: false }),
        headerRight: right ? () => <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>{right}</View> : undefined,
        ...nativeHeader,
      };
  const kav = (node: React.ReactNode) => (Platform.OS === 'ios' ? <KeyboardAvoidingView behavior="padding" style={{ flex: 1 }}>{node}</KeyboardAvoidingView> : <>{node}</>);
  return (
    <View testID={testID} style={{ flex: 1, backgroundColor: ground }}>
      <Stack.Screen options={options} />
      {kav(
      <View style={{ flex: 1, minWidth: 0 }}>
        {custom && (
          // HIG/layout: a custom header starts below the status bar and the Dynamic Island; the inset is the system's
          <View testID="header-inset" style={{ zIndex: 4, paddingTop: ins.top, backgroundColor: task || deep ? C.bgDeep : C.bg }}>
            {header || <TopBar title={title} back={back} onBack={onBack} right={right} task={task} backLabel={backLabel} />}
          </View>
        )}
        <OfflineStrip />
        {scroll ? (
          // the automatic inset lets the large title collapse and the content scroll under the Liquid Glass bars
          <ScrollView ref={(r) => { sv.current = r; if (scrollRef) scrollRef.current = r; }} style={{ flex: 1 }} contentInsetAdjustmentBehavior="automatic"
            keyboardShouldPersistTaps="handled" keyboardDismissMode="on-drag"
            contentContainerStyle={{ paddingHorizontal: wide ? 32 : 16, paddingTop: 12, paddingBottom: 24 + barH + dockH }}
            refreshControl={onRefresh ? <RefreshControl refreshing={refreshing} onRefresh={refresh} tintColor={C.amber} colors={[C.amber]} progressBackgroundColor={C.surface} /> : undefined}>
            <View style={{ width: '100%', maxWidth: mw, alignSelf: 'center', gap: 16 }}>
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
