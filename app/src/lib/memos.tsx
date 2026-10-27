// Voice memos from the iPhone (Notebook › Notes). gsync on his PC pulls them from Google Drive › Punchable ›
// Voice memos, transcribes them and files each one (GET /api/memos; gsync/memos.py). Here: the progress line,
// how to send them, and each memo dated, with its words, its audio and where it was filed.
import { useEffect, useState } from 'react';
import { Pressable, View } from 'react-native';
import { router } from 'expo-router';
import { useApi } from './api';
import { HearIt } from './hear';
import { C, fmtDate, fmtMs } from './theme';
import { Card, List, ListRow, T, TextBtn } from './ui';

export type Memo = {
  id: string; title: string; status: 'listed' | 'downloaded' | 'transcribing' | 'done' | 'failed';
  date: string | null; recorded_at: string | null; duration_s: number | null; text: string; text_cut: boolean;
  joke_id: string | null; attached_to: { joke_id: string; title: string } | null; long: boolean; no_speech: boolean;
  audio: string | null; error: string | null;
};
export type MemosResp = {
  available: boolean; items: Memo[];
  progress: { line: string; total: number; transcribed: number; waiting: number; failed: number; working: boolean; paused: { reason: string; free_mb?: number } | null };
};

const STEPS = [
  'On your iPhone, open Voice Memos.',
  'All Recordings › Edit (top right) › tap every memo you want (or Select All, if it shows).',
  'Tap the Share button (square with the arrow), then “Save to Files”.',
  'Pick Google Drive › Punchable › Voice memos, then Save. (Drive shows up in Files once the Google Drive app is installed and signed in.)',
  'That’s it. Your PC picks them up within a minute or two and transcribes them. Leave the PC on; it works through them one at a time.',
  'Later, one memo: open it › ••• › Share › Drive (or Save to Files) › Punchable › Voice memos.',
];

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
  return { listed: 'in Google Drive · waiting', downloaded: 'waiting to be transcribed', transcribing: 'transcribing…', failed: 'couldn’t transcribe this one' }[m.status] || m.status;
}

export function VoiceMemos() {
  const { data, reload } = useApi<MemosResp>('/api/memos', { cache: false });
  const [open, setOpen] = useState<string | null>(null);
  const [help, setHelp] = useState(false);
  const [all, setAll] = useState(false);
  const p = data?.progress;
  const busy = !!p && p.waiting > 0;
  useEffect(() => {
    if (!busy) return;
    const t = setInterval(reload, 15000);
    return () => clearInterval(t);
  }, [busy, reload]);
  if (!data) return null;
  const items = all ? data.items : data.items.slice(0, 40);

  return (
    <View style={{ gap: 12 }} testID="voice-memos">
      <Card style={{ gap: 6 }} testID="memos-progress">
        <T v="eb">Voice memos from your iPhone</T>
        {data.available && p && p.total > 0 ? (
          <>
            <T v="semi" testID="memos-line">{p.line}</T>
            {busy && !p.paused && <T v="small">{p.working ? 'Working through them now. New ones show up here as they finish.' : 'Waiting for your PC to pick them up.'}</T>}
            {p.failed > 0 && <T v="small">{p.failed} couldn’t be transcribed; they stay in Google Drive.</T>}
          </>
        ) : (
          <T v="mut">{data.available ? 'Send them to Google Drive › Punchable › Voice memos. Each one becomes a dated note here, or lands in the bit it belongs to.' : 'Voice memos are transcribed on your PC; open Punchable there to see them.'}</T>
        )}
        <TextBtn testID="memos-howto" label={help ? 'Hide the steps' : 'How to send them from your iPhone'} onPress={() => setHelp(!help)} />
        {help && (
          <View style={{ gap: 6 }} testID="memos-steps">
            {STEPS.map((x, i) => <T key={i} v="small" style={{ color: C.text2 }}>{i + 1}. {x}</T>)}
          </View>
        )}
      </Card>
      {items.length > 0 && (
        <Card style={{ paddingVertical: 4 }} testID="memos-list">
          <List>
            {items.map((m) => (
              <View key={m.id}>
                <ListRow testID={`memo-${m.id}`} title={memoTitle(m)} chevron={false}
                  titleColor={open === m.id ? C.amber : undefined}
                  sub={[m.date ? fmtDate(m.date) : null, m.duration_s ? fmtMs(m.duration_s * 1000) : null, statusWord(m)].filter(Boolean).join(' · ')}
                  onPress={m.status === 'done' ? () => setOpen(open === m.id ? null : m.id) : undefined} />
                {open === m.id && (
                  <View style={{ gap: 10, paddingBottom: 12 }} testID={`memo-open-${m.id}`}>
                    {m.text ? <T style={{ color: C.text }}>“{m.text}”</T> : <T v="mut">No words in this one.</T>}
                    {m.text_cut && <T v="small">The full transcript is in the note.</T>}
                    <HearIt audio={m.audio} play={{ start_ms: 0, end_ms: (m.duration_s || 0) * 1000 + 1000 }} k={`memo:${m.id}`}
                      sub={m.duration_s ? fmtMs(m.duration_s * 1000) : 'the recording'} testID={`memo-play-${m.id}`} />
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
