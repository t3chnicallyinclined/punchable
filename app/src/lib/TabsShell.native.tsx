// The phone's tab bar: the system's (expo-router NativeTabs over react-native-screens; Liquid Glass on iOS 26, and
// the bar minimises as he scrolls down). One stack per tab ((tonight), (bits), (sets)), so switching tabs keeps each
// tab's navigation state and the tabs stay visible on every pushed screen (HIG/tab-bars). Filled symbols in the tab
// bar (HIG/sf-symbols). The badge on Tonight is the "N to sort" count, only when there is one (HIG/tab-bars:
// "Reserve badges for critical information"). The web shell is TabsShell.tsx.
import React from 'react';
import { NativeTabs } from 'expo-router/native-tabs';
import { C } from './theme';
import { useUI } from './ui';
import { ACCESSORY } from './accessory';
import { useFlags } from './useFlags';
import { SayBar } from './SayBar';

/** The input bar inside the tab bar's bottom accessory. Two copies exist (regular and inline); the system draws the glass. */
function AccessoryBar() {
  const ui = useUI();
  const placement = NativeTabs.BottomAccessory.usePlacement();
  if (!ui.bar) return null;
  return <SayBar {...ui.bar} variant={placement === 'inline' ? 'inline' : 'accessory'} />;
}

export function TabsShell() {
  const ui = useUI();
  const toSort = ui.toSort > 0 ? String(ui.toSort) : '';
  // Liquid Glass available (ACCESSORY) AND not switched off remotely (/api/health flags.glass_accessory, lib/flags):
  // the switch exists so the accessory can be turned off on every phone without a build if it misbehaves.
  const glass = ACCESSORY && useFlags().glassAccessory;
  return (
    <NativeTabs tintColor={C.amber} minimizeBehavior="onScrollDown" backBehavior="history">
      {glass && !!ui.bar && (
        <NativeTabs.BottomAccessory>
          <AccessoryBar />
        </NativeTabs.BottomAccessory>
      )}
      <NativeTabs.Trigger name="(tonight)" testID="tab-tonight">
        <NativeTabs.Trigger.Label>Tonight</NativeTabs.Trigger.Label>
        <NativeTabs.Trigger.Icon sf="moon.stars.fill" />
        <NativeTabs.Trigger.Badge hidden={!toSort}>{toSort}</NativeTabs.Trigger.Badge>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="(bits)" testID="tab-bits">
        <NativeTabs.Trigger.Label>Bits</NativeTabs.Trigger.Label>
        <NativeTabs.Trigger.Icon sf="square.grid.2x2.fill" />
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="(sets)" testID="tab-sets">
        <NativeTabs.Trigger.Label>Sets</NativeTabs.Trigger.Label>
        <NativeTabs.Trigger.Icon sf="list.bullet.rectangle.portrait.fill" />
      </NativeTabs.Trigger>
    </NativeTabs>
  );
}
