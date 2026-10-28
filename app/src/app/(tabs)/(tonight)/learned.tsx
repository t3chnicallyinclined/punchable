// What I've learned about you (UX_SPEC 4.12): the learned section of his voice profile, as the server keeps it
// (GET /api/profile/voice .learned, api-dev/profile_api.py): the drafts he took as his ("Use as mine"), the ones he
// dismissed or passed over. Read-only; the same rows as Settings > Voice profile (lib/voiceProfile.ts, tested).
import React from 'react';
import { View } from 'react-native';
import { useApi } from '@/lib/api';
import { C } from '@/lib/theme';
import { Card, T } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { VoiceDoc, learnedRows, learnedWords } from '@/lib/voiceProfile';

export default function Learned() {
  const api = useApi<VoiceDoc>('/api/profile/voice', { cache: false });
  const kept = learnedRows(api.data?.learned?.kept);
  const dropped = learnedRows(api.data?.learned?.dropped);
  return (
    <Screen title="What I’ve learned" back testID="learned">
      <T v="h1">What I’ve learned about you</T>
      <Card style={{ gap: 6 }} testID="learned-status">
        <T testID="learned-line">{api.loading && !api.data ? 'Checking…' : api.error && !api.data ? 'Can’t reach Punchable to read it.' : learnedWords(api.data?.learned)}</T>
        <T v="mut">Filled in from what you do with the AI’s drafts: “Use as mine” keeps one, “Dismiss” drops one, and the drafts beside a kept line count as passed over. The AI reads this after your voice profile, before it drafts.</T>
      </Card>
      {kept.length > 0 && (
        <Card style={{ gap: 4 }} testID="learned-kept">
          <T v="small" style={{ color: C.amber }}>Kept</T>
          {kept.map((r, i) => <T key={`k${i}`} testID={`learned-kept-${i}`}>{r.text}{r.when ? <T v="small">{`  ${r.when}`}</T> : null}</T>)}
        </Card>
      )}
      {dropped.length > 0 && (
        <Card style={{ gap: 4 }} testID="learned-dropped">
          <T v="small" style={{ color: C.dim }}>Passed over</T>
          {dropped.map((r, i) => <T key={`d${i}`} v="mut" testID={`learned-dropped-${i}`}>{r.text}{r.when ? <T v="small">{`  ${r.when}`}</T> : null}</T>)}
        </Card>
      )}
      <View>
        <T v="small">Your profile text is yours alone: nothing here edits it. Settings › Voice profile shows the same lines under the editor.</T>
      </View>
    </Screen>
  );
}
