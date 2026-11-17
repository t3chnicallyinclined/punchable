// /stage/<id> (the old On stage screen) opens the walk-through (SPEC 2026-11-16 Slice 5): one screen records a set one
// bit at a time with his notes, and keeps where each bit sat on the tape. Old links and the notes sheet land there.
import { Redirect, useLocalSearchParams } from 'expo-router';

export default function Stage() {
  const p = useLocalSearchParams<{ id: string }>();
  return <Redirect href={{ pathname: '/run/[id]', params: { id: String(p.id || ''), mode: 'sound' } } as any} />;
}
