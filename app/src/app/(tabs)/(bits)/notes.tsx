// /notes (an older route: Desk's "Talk it out", old links). Voice memos are recorded in Punchable now (SPEC
// 2026-10-31): `?talk=` opens the memo recorder on the phone; otherwise, and on the web, it goes to Bits, whose Memos
// segment lists them. The old screen uploaded a note for a server that no longer transcribes (ADR 0012).
import { Platform } from 'react-native';
import { Redirect, useLocalSearchParams } from 'expo-router';

export default function NotesScreen() {
  const params = useLocalSearchParams<{ talk?: string }>();
  if (params.talk && Platform.OS !== 'web') return <Redirect href={{ pathname: '/record', params: { kind: 'memo' } } as any} />;
  return <Redirect href={'/bits' as any} />;
}
