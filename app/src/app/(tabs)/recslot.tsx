// The route behind the trailing-slot Record trigger of the Device checks (SPEC 2026-11-16 §4.1 fallback a;
// lib/TabsShell.native, lib/deviceChecks). A NativeTabs trigger needs a route file to exist; this one is never shown:
// the trigger is disabled, so the system reports the press and selects nothing. With the check switched off there is
// no trigger and this file is not a tab (opened by URL then, the tabs show their first tab). If it is ever in front it
// sends him to Bits. Removed with the check.
import React from 'react';
import { Redirect } from 'expo-router';

export default function RecordSlot() {
  return <Redirect href={'/bits' as any} />;
}
