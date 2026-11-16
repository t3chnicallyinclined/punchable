// /bit/<id> was the older bit workspace (connections in tabs, Brainstorm). Its groups live on the bit page now, as
// "Around this bit" (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §2.2, Slice 1), so the old URL opens the bit page.
import React from 'react';
import { Redirect, useLocalSearchParams } from 'expo-router';

export default function OldBitWorkspace() {
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  return <Redirect href={`/joke/${encodeURIComponent(id)}` as any} />;
}
