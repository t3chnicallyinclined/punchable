// The phone's tab bar: the system's (expo-router NativeTabs over react-native-screens; Liquid Glass on iOS 26, and
// the bar minimises as he scrolls down). One stack per tab ((rant), (bits), (sets)), so switching tabs keeps each
// tab's navigation state and the tabs stay visible on every pushed screen (HIG/tab-bars). Filled symbols in the tab
// bar (HIG/sf-symbols). Rant is first (SPEC 2026-11-06 R1, owner: "one of the main features"; Tonight went, its parts
// are on Sets and Bits, R2). The badge on Bits is the "N to sort" count, only when there is one (HIG/tab-bars:
// "Reserve badges for critical information"): "N to sort" lives on Bits now. The web shell is TabsShell.tsx.
import React from 'react';
import { NativeTabs } from 'expo-router/native-tabs';
import { C } from './theme';
import { useUI } from './ui';

export function TabsShell() {
  const ui = useUI();
  const toSort = ui.toSort > 0 ? String(ui.toSort) : '';
  return (
    <NativeTabs tintColor={C.amber} minimizeBehavior="onScrollDown" backBehavior="history">
      {/* Owner decision 2026-10-28: no input bar in the tab bar. R4: the tab bar navigates; recording is a tab's content. */}
      <NativeTabs.Trigger name="(rant)" testID="tab-rant">
        <NativeTabs.Trigger.Label>Rant</NativeTabs.Trigger.Label>
        <NativeTabs.Trigger.Icon sf="megaphone.fill" />
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="(bits)" testID="tab-bits">
        <NativeTabs.Trigger.Label>Bits</NativeTabs.Trigger.Label>
        <NativeTabs.Trigger.Icon sf="square.grid.2x2.fill" />
        <NativeTabs.Trigger.Badge hidden={!toSort}>{toSort}</NativeTabs.Trigger.Badge>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="(sets)" testID="tab-sets">
        <NativeTabs.Trigger.Label>Sets</NativeTabs.Trigger.Label>
        <NativeTabs.Trigger.Icon sf="list.bullet.rectangle.portrait.fill" />
      </NativeTabs.Trigger>
    </NativeTabs>
  );
}
