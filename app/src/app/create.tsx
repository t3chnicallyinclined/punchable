// The "+" sheet as a route (docs/design/HIG.md §3.12). Phone: a native form sheet (presentation formSheet, detents
// [0.5, 1], the grabber, swipe anywhere to dismiss: app/_layout.tsx) whose bar carries Cancel leading and, on "New set
// list", Back leading and Start trailing (HIG/sheets). Web: the Modal sheet, unchanged, over the ground when /create is
// opened directly (the tabs open it in place).
import React, { useRef, useState } from 'react';
import { Platform, ScrollView, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { CreateBody, CreateMode, CreateSheet } from '@/lib/CreateSheet';
import { C } from '@/lib/theme';

export default function CreateRoute() {
  const p = useLocalSearchParams<{ start?: string }>();
  const start: CreateMode = p.start === 'set' ? 'set' : 'menu';
  const [mode, setMode] = useState<CreateMode>(start);
  const startRef = useRef<() => void>(() => {});
  const close = () => (router.canGoBack() ? router.back() : router.navigate('/' as any));
  if (Platform.OS === 'web') {
    return (
      <View style={{ flex: 1, backgroundColor: C.bg }}>
        <CreateSheet visible onClose={close} start={start} />
      </View>
    );
  }
  const menu = mode === 'menu';
  return (
    <View testID="create-sheet" style={{ flex: 1, backgroundColor: C.bg }}>
      <Stack.Screen options={{ title: menu ? 'New' : 'New set list' }} />
      {/* HIG/sheets: Cancel on the leading edge; Done (here "Start") on the trailing edge only where there is a write */}
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button accessibilityLabel={menu ? 'Cancel' : 'Back to New'} onPress={menu ? close : () => setMode('menu')}>{menu ? 'Cancel' : 'Back'}</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="done" hidden={menu} accessibilityLabel="Start the set list" onPress={() => startRef.current()}>Start</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" keyboardShouldPersistTaps="handled" keyboardDismissMode="on-drag" contentContainerStyle={{ padding: 16, paddingBottom: 32 }}>
        <CreateBody onClose={close} mode={mode} setMode={setMode} chrome={false} startRef={startRef} />
      </ScrollView>
    </View>
  );
}
