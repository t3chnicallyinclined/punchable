// The switches of the Device checks screen (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §4.1; Settings › About ›
// Device checks). Held in memory only and off at every launch: a guess about the tab bar that turns out wrong on his
// phone is cleared by closing the app, and nothing here can reach a launch. The tab shell (lib/TabsShell.native)
// reads them; the check page (app/(tabs)/(bits)/spike) sets them. Plain TS, one module-level value, a subscribe for
// React (useSyncExternalStore), like lib/stageNotes. Removed with the check screens once the questions are answered.
import { useSyncExternalStore } from 'react';

export type DeviceChecks = {
  /** 4.1: a Record strip as the tab bar's bottom accessory (iOS 26+). */
  accessory: boolean;
  /** 4.1: the strip hidden through the tab host (the way one screen would hide it). */
  accessoryHidden: boolean;
  /** 4.1 fallback (a): a Record button in the tab bar's trailing slot. Turning it on or off reloads the tabs. */
  recordTab: boolean;
  /** How many times a test Record control was tapped, and which one last. */
  taps: number;
  last: string;
};

let value: DeviceChecks = { accessory: false, accessoryHidden: false, recordTab: false, taps: 0, last: '' };
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export const deviceChecks = {
  get: (): DeviceChecks => value,
  set(p: Partial<DeviceChecks>) { value = { ...value, ...p }; emit(); },
  /** A test Record control was tapped (nothing is ever recorded by one). */
  tapped(which: string) { value = { ...value, taps: value.taps + 1, last: which }; emit(); },
  subscribe(l: () => void) { listeners.add(l); return () => { listeners.delete(l); }; },
};

export function useDeviceChecks(): DeviceChecks {
  return useSyncExternalStore(deviceChecks.subscribe, deviceChecks.get, deviceChecks.get);
}
