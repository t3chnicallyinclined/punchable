// The phone's tab bar: the system's (expo-router NativeTabs over react-native-screens; Liquid Glass on iOS 26, and
// the bar minimises as he scrolls down). One stack per tab ((bits), (sets), (tapes)), so switching tabs keeps each
// tab's navigation state and the tabs stay visible on every pushed screen (HIG/tab-bars). Filled symbols in the tab
// bar (HIG/sf-symbols). Bits · Sets · Tapes (SPEC 2026-11-16 Slice 6): a rant is a set he films, so the Rant tab went;
// its topics are sets, its takes are under Tapes, and Settings and You moved behind the avatar on every tab root. The
// badge on Bits is the "N to sort" count, only when there is one (HIG/tab-bars: "Reserve badges for critical
// information"). The web shell is TabsShell.tsx.
//
// Device checks (SPEC 2026-11-16 §4.1; lib/deviceChecks, off at every launch, set only from Settings › About ›
// Device checks): two ways a Record control could sit by the tab bar, so he can see both on his phone and choose.
// Neither records anything. With every switch off this file renders exactly the three tabs, as before.
//   - the strip: NativeTabs.BottomAccessory (UITabBarController's bottom accessory, iOS 26+; a no-op below it and on
//     Android: react-native-screens TabsHost.ios.tsx gates on iOS 26). Mounted and unmounted at run time; hidden
//     through the host's `bottomAccessoryHidden` (the way one screen would hide it). The system draws the child
//     twice (regular and inline), so the strip holds no state of its own.
//   - the trailing slot: a fourth trigger with the search role, disabled, so the system never selects it and still
//     reports the press (`tabPress` with `isPrevented`). It needs a route file ((tabs)/recslot). The trigger names
//     are the tab bar's key, so switching it on or off rebuilds the tabs once (the check page says so).
import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { NativeTabs } from 'expo-router/native-tabs';
import { C, F } from './theme';
import { useUI } from './ui';
import { deviceChecks, useDeviceChecks } from './deviceChecks';

/** The test strip: a red disc and the word (the disc alone when the bar is minimised and the strip sits inline). */
function RecordStrip() {
  const placement = NativeTabs.BottomAccessory.usePlacement();
  return (
    <Pressable testID="check-record-strip" accessibilityRole="button" accessibilityLabel="Record (a test control: nothing is recorded)"
      onPress={() => deviceChecks.tapped(`the strip, ${placement}`)}
      style={{ flex: 1, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 8 }}>
      <View style={{ width: 22, height: 22, borderRadius: 11, backgroundColor: C.red }} />
      {placement === 'regular' && <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.semi, fontSize: 15, color: C.text }}>Record</Text>}
    </Pressable>
  );
}

export function TabsShell() {
  const ui = useUI();
  const checks = useDeviceChecks();
  const toSort = ui.toSort > 0 ? String(ui.toSort) : '';
  return (
    <NativeTabs tintColor={C.amber} minimizeBehavior="onScrollDown" backBehavior="history"
      unstable_nativeProps={checks.accessory ? { ios: { bottomAccessoryHidden: checks.accessoryHidden } } : undefined}>
      {checks.accessory && (
        <NativeTabs.BottomAccessory><RecordStrip /></NativeTabs.BottomAccessory>
      )}
      {/* Owner decision 2026-10-28: no input bar in the tab bar. R4: the tab bar navigates; recording is a tab's content. */}
      <NativeTabs.Trigger name="(bits)" testID="tab-bits">
        <NativeTabs.Trigger.Label>Bits</NativeTabs.Trigger.Label>
        <NativeTabs.Trigger.Icon sf="square.grid.2x2.fill" />
        <NativeTabs.Trigger.Badge hidden={!toSort}>{toSort}</NativeTabs.Trigger.Badge>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="(sets)" testID="tab-sets">
        <NativeTabs.Trigger.Label>Sets</NativeTabs.Trigger.Label>
        <NativeTabs.Trigger.Icon sf="list.bullet.rectangle.portrait.fill" />
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="(tapes)" testID="tab-tapes">
        <NativeTabs.Trigger.Label>Tapes</NativeTabs.Trigger.Label>
        <NativeTabs.Trigger.Icon sf="waveform" />
      </NativeTabs.Trigger>
      {checks.recordTab && (
        <NativeTabs.Trigger name="recslot" role="search" disabled disableAutomaticSearchActivation testID="tab-record-check"
          listeners={{ tabPress: (e: any) => { if (e?.data?.isPrevented !== false) deviceChecks.tapped('the trailing slot'); } }}>
          <NativeTabs.Trigger.Label>Record</NativeTabs.Trigger.Label>
          <NativeTabs.Trigger.Icon sf="record.circle.fill" />
        </NativeTabs.Trigger>
      )}
    </NativeTabs>
  );
}
