// Bits › Map. Two maps of all his bits: the CLOUD (lib/Cloud, 2026-10-29: every bit placed by what it says, from its
// vector) and the LINKS map (lib/Galaxy: near = a stored link). The cloud is the default; the choice is remembered
// (prefs.mapMode). docs/UX_2026-10.md "All-bits map", docs/design/HIG.md §3.10.
import React, { useState } from 'react';
import { Cloud } from '@/lib/Cloud';
import { Galaxy } from '@/lib/Galaxy';
import { getPrefs, setPrefs } from '@/lib/prefs';

export default function AllBitsMap() {
  const [mode, setMode] = useState<'cloud' | 'links'>(() => getPrefs().mapMode);
  const go = (m: 'cloud' | 'links') => { setPrefs({ mapMode: m }); setMode(m); };
  return mode === 'cloud' ? <Cloud onLinks={() => go('links')} /> : <Galaxy onCloud={() => go('cloud')} />;
}
