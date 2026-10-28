// Settings > Voice profile: what the AI reads before it drafts in his voice (riffs, punch-ups, bridges, Ask). One
// document on the server (GET/PUT /api/profile/voice, api-dev/profile_api.py), shown, edited and saved here; Save is
// undone by restoring the previous version (the API keeps one). On the phone this is a form sheet
// ((tonight)/_layout.tsx); on the web a page. The rules are in lib/voiceProfile.ts (tested); the words are his.
import React, { useEffect, useRef, useState } from 'react';
import { TextInput, View } from 'react-native';
import { router } from 'expo-router';
import { ApiError, put, useApi } from '@/lib/api';
import { haptics } from '@/lib/haptics';
import { C, F } from '@/lib/theme';
import { Btn, Card, Skeleton, T, s, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { MAX_CHARS, VoiceDoc, canSave, learnedRows, learnedWords, normalize, savedWords, statusLine, undoVersion, wordCount } from '@/lib/voiceProfile';

export default function VoiceProfilePage() {
  const ui = useUI();
  const api = useApi<VoiceDoc>('/api/profile/voice', { cache: false });
  const [draft, setDraft] = useState('');
  const [saving, setSaving] = useState(false);
  const seeded = useRef(false);
  useEffect(() => {
    // the server's text fills the editor once; a later refresh never overwrites what he is typing
    if (api.data && !seeded.current) {
      seeded.current = true;
      setDraft(normalize(api.data.text));
    }
  }, [api.data]);

  const saved = api.data?.text ?? null;
  const gate = canSave(draft, saved);
  const words = wordCount(draft);

  const save = async (text: string, snack: boolean) => {
    setSaving(true);
    try {
      const doc = await put<VoiceDoc>('/api/profile/voice', { text });
      haptics.success();
      setDraft(normalize(doc.text));
      api.reload();
      const prev = undoVersion(doc);
      if (snack) ui.snack(savedWords(doc), prev ? () => save(prev.text, false) : undefined);
      else ui.snack('Restored the previous version.');
    } catch (e: any) {
      haptics.warning();
      // 422 is the legal rule: the server's own words (court material is never kept); anything else, plainly
      ui.snack(e instanceof ApiError ? e.message : `Couldn’t save: ${e?.message || 'try again'}. Nothing changed.`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Screen title="Voice profile" back testID="voice" backLabel="Settings">
      <Card testID="voice-about" style={{ gap: 6 }}>
        <T v="eb">What the AI reads first</T>
        <T v="mut">Before every riff, punch-up, bridge or answer, the AI reads this: how you talk, your moves by name, who the people in your bits are. Write it the way you’d brief a writer. It lives on your server, nowhere else.</T>
        <T v="small" testID="voice-status">{api.loading && !api.data ? 'Checking…' : api.error && !api.data ? 'Can’t reach Punchable to read it.' : statusLine(api.data)}</T>
      </Card>
      <Card testID="voice-editor" style={{ gap: 10 }}>
        {api.loading && !api.data ? (
          <Skeleton lines={8} testID="voice-skeleton" />
        ) : (
          <TextInput testID="voice-text" value={draft} onChangeText={setDraft} multiline scrollEnabled={false} autoCapitalize="sentences" autoCorrect
            accessibilityLabel="Your voice profile" placeholder="How you sound on stage. Your moves, by the names you use. Who the people in your bits are." placeholderTextColor={C.dim}
            style={{ minHeight: 320, borderWidth: 1, borderColor: C.border, borderRadius: 8, padding: 12, color: C.text, fontFamily: F.body, fontSize: 16, lineHeight: 23, textAlignVertical: 'top' }} />
        )}
        <View style={[s.row, { alignItems: 'center', gap: 12 }]}>
          <T v="small" testID="voice-count" style={{ flex: 1 }}>{`${words.toLocaleString('en-US')} words${normalize(draft).length > MAX_CHARS ? ' · too long' : ''}${!gate.ok && gate.why && gate.why !== 'Nothing changed.' ? ` · ${gate.why}` : ''}`}</T>
          <Btn small testID="voice-revert" label="Revert" disabled={saving || normalize(draft) === normalize(saved)} onPress={() => setDraft(normalize(saved))} accessibilityLabel="Revert to the saved text" />
          <Btn small testID="voice-save" label={saving ? 'Saving…' : 'Save'} v="primary" disabled={saving || !gate.ok} onPress={() => save(normalize(draft), true)} accessibilityLabel={gate.ok ? 'Save the voice profile' : `Save: ${gate.why}`} />
        </View>
      </Card>
      <Card testID="voice-learned" style={{ gap: 8 }}>
        <T v="eb">What it has learned</T>
        <T v="mut">Filled in by Punchable, not typed: the drafts you took as yours (“Use as mine”) and the ones you left beside them. The AI reads this after your profile. Nothing here changes your text.</T>
        <T v="small" testID="voice-learned-status">{learnedWords(api.data?.learned)}</T>
        {learnedRows(api.data?.learned?.kept).length > 0 && (
          <View style={{ gap: 4 }}>
            <T v="small" style={{ color: C.amber }}>Kept</T>
            {learnedRows(api.data?.learned?.kept).map((r, i) => (
              <T key={`k${i}`} testID={`voice-kept-${i}`}>{r.text}{r.when ? <T v="small">{`  ${r.when}`}</T> : null}</T>
            ))}
          </View>
        )}
        {learnedRows(api.data?.learned?.dropped).length > 0 && (
          <View style={{ gap: 4 }}>
            <T v="small" style={{ color: C.dim }}>Passed over</T>
            {learnedRows(api.data?.learned?.dropped).map((r, i) => (
              <T key={`d${i}`} v="mut" testID={`voice-dropped-${i}`}>{r.text}{r.when ? <T v="small">{`  ${r.when}`}</T> : null}</T>
            ))}
          </View>
        )}
      </Card>
      <Card style={{ gap: 6 }}>
        <T v="small">A save keeps the version before it: Undo in the bar restores it. Court or legal text is refused and never kept. The AI never quotes this; it only reads it.</T>
        <Btn small testID="voice-done" label="Back to Settings" onPress={() => (router.canGoBack() ? router.back() : router.navigate('/settings' as any))} />
      </Card>
    </Screen>
  );
}
