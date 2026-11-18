// /board/<id> (the old deep link) opens the set list (docs/UX_2026-10.md: a set list IS a board). The old set
// builder stays at /builder/<id>.
import { Redirect, useLocalSearchParams } from 'expo-router';

export default function BoardRedirect() {
  const p = useLocalSearchParams<{ id: string }>();
  return <Redirect href={`/list/${encodeURIComponent(decodeURIComponent(String(p.id || '')))}` as any} />;
}
