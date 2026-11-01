// Voice memos (Bits › Memos; SPEC 2026-10-31). PhoneMemos: the memos recorded in Punchable, kept on this iPhone with
// play, transcribed here and filed as ideas on the right bit (lib/recordings, lib/RecRows), plus "Import from Files…".
// VoiceMemos: the earlier memos (the retired Drive + PC route, GET /api/memos; gsync/memos.py), dated, with their
// words and where they were filed. Nothing here asks him to send recordings anywhere: the audio stays on the phone.
import { useState } from 'react';
import { Pressable, View } from 'react-native';
import { router } from 'expo-router';
import { post, undo, useApi } from './api';
import { HearIt, Reference } from './hear';
import { importFiles, recordingsSupported, useRecordings } from './recordings';
import { RecRows } from './RecRows';
import { C } from './theme';
import { clockMs, fmtDate } from '@/rules/format';
import { Btn, Card, Empty, List, ListRow, T, TextBtn, useUI } from './ui';

export type Memo = {
  id: string; title: string; status: 'listed' | 'downloaded' | 'transcribing' | 'done' | 'failed';
  date: string | null; recorded_at: string | null; duration_s: number | null; text: string; text_cut: boolean;
  joke_id: string | null; attached_to: { joke_id: string; title: string } | null; long: boolean; no_speech: boolean;
  audio: string | null; error: string | null;
  /** online-first: no audio is kept; this is where the recording lives (his Drive, or the file's name and date) */
  reference?: Reference | null;
};
export type MemosResp = {
  available: boolean; items: Memo[];
  progress: { line: string; total: number; transcribed: number; waiting: number; failed: number; working: boolean; paused: { reason: string; free_mb?: number } | null };
};

// "New Recording 12", "Brooklyn, NY 3", "123 Main St": the phone's names, not his. Show his opening words instead.
const GENERIC = /^(new recording|recording|voice memo|audio|memo|untitled)\b|^\d|,/i;
export function memoTitle(m: Pick<Memo, 'title' | 'text'>) {
  if (!GENERIC.test(m.title || '') || !m.text) return m.title || 'Voice memo';
  const w = m.text.split(/\s+/);
  return w.slice(0, 8).join(' ') + (w.length > 8 ? '…' : '');
}

function statusWord(m: Memo) {
  if (m.status === 'done') {
    if (m.no_speech) return 'no speech in it';
    if (m.long) return 'long recording · kept as a note';
    return m.attached_to ? `added to ${m.attached_to.title}` : 'saved as a new idea';
  }
  return { listed: 'not transcribed', downloaded: 'not transcribed', transcribing: 'not transcribed', failed: 'couldn’t transcribe this one' }[m.status] || m.status;
}

// "Not a joke?": memos Claude read as a conversation or a note. Only a suggestion: nothing is hidden until he taps Hide
// (his own decision, with Undo), and Keep means it is never suggested again. GET /api/suggestions/not-a-joke.
type NotJoke = { joke_id: string; title: string; text: string; kind: string; why: string; confidence: number };

export function NotAJoke() {
  const ui = useUI();
  const { data, reload } = useApi<{ items: NotJoke[]; count: number }>('/api/suggestions/not-a-joke', { cache: false });
  const [busy, setBusy] = useState<string | null>(null);
  if (!data || !data.items.length) return null;
  const act = async (x: NotJoke, what: 'hide' | 'keep') => {
    setBusy(x.joke_id);
    try {
      const r = await post<{ undo?: { indexes: number[] } }>(`/api/suggestions/not-a-joke/${what}`, { joke_id: x.joke_id });
      await reload();
      if (what === 'hide') {
        ui.snack(`Hid "${x.title}"`, async () => { await undo(r.undo?.indexes || []); await reload(); });
      } else {
        ui.snack(`Kept "${x.title}" as yours`);
      }
    } catch (e: any) {
      ui.snack(`Couldn't ${what} it: ${e.message}`);
    } finally {
      setBusy(null);
    }
  };
  return (
    <Card style={{ gap: 8 }} testID="not-a-joke">
      <T v="eb">Not a joke? ({data.count})</T>
      <T v="small">These voice memos sound like a conversation or a note, not material. Nothing is hidden unless you tap Hide.</T>
      {data.items.map((x) => (
        <View key={x.joke_id} testID={`notjoke-${x.joke_id}`} style={{ gap: 6, paddingVertical: 6, borderTopWidth: 1, borderTopColor: C.border }}>
          <T v="semi">{x.title}</T>
          <T v="small" style={{ color: C.text2 }}>{x.why}</T>
          <View style={{ flexDirection: 'row', gap: 8 }}>
            <Btn small testID={`notjoke-hide-${x.joke_id}`} label="Hide" disabled={busy === x.joke_id} onPress={() => act(x, 'hide')} />
            <Btn small testID={`notjoke-keep-${x.joke_id}`} label="It's a joke" disabled={busy === x.joke_id} onPress={() => act(x, 'keep')} />
          </View>
        </View>
      ))}
    </Card>
  );
}

/** The memos on this phone (newest first) and Import from Files… (copied in under generated names, transcribed here).
 *  The web shows where they come from instead: memos are recorded in the iPhone app. */
export function PhoneMemos() {
  const ui = useUI();
  const recs = useRecordings().filter((r) => r.kind === 'memo');
  const [busy, setBusy] = useState(false);
  if (!recordingsSupported) return <T v="mut" testID="phone-memos-web">Voice memos are recorded in the iPhone app. Their words land on your bits here.</T>;
  const go = async () => {
    setBusy(true);
    try {
      const n = await importFiles();
      if (n) ui.snack(`Added ${n} · transcribing on this phone`);
    } catch (e: any) {
      ui.snack(`Couldn’t add them: ${e?.message || e}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <View style={{ gap: 8 }} testID="phone-memos">
      {recs.length ? (
        <Card style={{ paddingVertical: 4 }}><RecRows recs={recs} testID="memo" /></Card>
      ) : (
        <Empty text="No voice memos yet. Tap the mic to record one." />
      )}
      <TextBtn testID="memos-import" label={busy ? 'Adding…' : 'Import from Files…'} onPress={() => { if (!busy) void go(); }} />
    </View>
  );
}

export function VoiceMemos() {
  // live-first like every other screen: changes arrive over /api/live; the health poll runs only while that
  // socket is down (useApi `poll`), never a timer of its own
  const { data } = useApi<MemosResp>('/api/memos', { cache: false, poll: 30000 });
  const [open, setOpen] = useState<string | null>(null);
  const [all, setAll] = useState(false);
  if (!data) return null;
  const items = all ? data.items : data.items.slice(0, 40);

  return (
    <View style={{ gap: 12 }} testID="voice-memos">
      <NotAJoke />
      {items.length > 0 && <T v="eb" accessibilityRole="header">Earlier memos</T>}
      {items.length > 0 && (
        <Card style={{ paddingVertical: 4 }} testID="memos-list">
          <List>
            {items.map((m) => (
              <View key={m.id}>
                <ListRow testID={`memo-${m.id}`} title={memoTitle(m)} chevron={false}
                  titleColor={open === m.id ? C.amber : undefined}
                  sub={[m.date ? fmtDate(m.date) : null, m.duration_s ? clockMs(m.duration_s * 1000) : null, statusWord(m)].filter(Boolean).join(' · ')}
                  onPress={m.status === 'done' ? () => setOpen(open === m.id ? null : m.id) : undefined} />
                {open === m.id && (
                  <View style={{ gap: 10, paddingBottom: 12 }} testID={`memo-open-${m.id}`}>
                    {m.text ? <T style={{ color: C.text }}>“{m.text}”</T> : <T v="mut">No words in this one.</T>}
                    {m.text_cut && <T v="small">The full transcript is in the note.</T>}
                    <HearIt audio={m.audio} reference={m.reference} play={{ start_ms: 0, end_ms: (m.duration_s || 0) * 1000 + 1000 }} k={`memo:${m.id}`}
                      sub={m.duration_s ? clockMs(m.duration_s * 1000) : 'the recording'} testID={`memo-play-${m.id}`} />
                    {m.joke_id && (
                      <Pressable accessibilityRole="link" onPress={() => router.push(`/joke/${encodeURIComponent(m.attached_to?.joke_id || m.joke_id!)}` as any)}>
                        <T style={{ color: C.amber }}>{m.attached_to ? `In ${m.attached_to.title} ›` : 'Open the idea ›'}</T>
                      </Pressable>
                    )}
                  </View>
                )}
              </View>
            ))}
          </List>
          {!all && data.items.length > items.length && <TextBtn label={`Show all ${data.items.length}`} onPress={() => setAll(true)} />}
        </Card>
      )}
    </View>
  );
}
