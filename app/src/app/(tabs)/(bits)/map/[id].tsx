// The map opened on one bit (/map/<id>, the bit page's "Full map ›"): the same galaxy as /map, centred and focused on
// that bit. The old per-joke walk (rows per group, Start a set, Ideas) lives on the bit page (/joke/<id>) and /bit/<id>.
import React from 'react';
import { useLocalSearchParams } from 'expo-router';
import { Galaxy } from '@/lib/Galaxy';

export default function JokeMapScreen() {
  const p = useLocalSearchParams<{ id: string }>();
  return <Galaxy focusId={decodeURIComponent(String(p.id || ''))} />;
}
