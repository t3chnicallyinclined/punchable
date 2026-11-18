// /marks was the marks tray (SPEC 2026-11-16 Slice 3). One notebook (SPEC 2026-11-17) lists a page's marks under the
// page itself, so an old link opens the page it names (?bit=<id>), else the notebook.
import React from 'react';
import { Redirect, useLocalSearchParams } from 'expo-router';

export default function Marks() {
  const p = useLocalSearchParams<{ bit?: string }>();
  const bit = String(p.bit || '');
  return <Redirect href={(bit ? `/joke/${encodeURIComponent(bit)}` : '/') as any} />;
}
