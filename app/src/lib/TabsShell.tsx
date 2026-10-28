// The web's tab shell: nothing of its own. The matched tab group renders through the Slot, and the existing custom
// chrome (lib/Screen.tsx: the bottom tab bar at phone width, the sidebar at >= 900 px) draws the tabs exactly as
// before. The phone's shell is TabsShell.native.tsx (the system tab bar).
import React from 'react';
import { Slot } from 'expo-router';

export function TabsShell() {
  return <Slot />;
}
