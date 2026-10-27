// Notes: voice notes in the inbox (queued for processing) and "Talk it out".
import { useEffect, useRef, useState } from 'react';
import { Text, View } from 'react-native';
import { useLocalSearchParams } from 'expo-router';
import { upload, useApi } from '@/lib/api';
import { C, F, fmtDate, fmtMs } from '@/lib/theme';
import { Badge, Btn, Card, Empty, ErrorCard, List, ListRow, Screen, T, s, useUI, useWide } from '@/lib/ui';
import { Recorder, createRecorder } from '@/lib/media';

type Item = { file: string; bytes: number; kind: string; title: string; received_at?: string; duration_s?: number; status: string };

export default function NotesScreen() {
  const ui = useUI();
  const wide = useWide();
  const params = useLocalSearchParams<{ talk?: string }>();
  const { data, error, reload } = useApi<{ items: Item[] }>('/api/inbox');
  const [sel, setSel] = useState<string | null>(null);
  const [rec, setRec] = useState<'off' | 'on' | 'saving'>('off');
  const [ms, setMs] = useState(0);
  const [level, setLevel] = useState(0);
  const r = useRef<Recorder | null>(null);
  const tick = useRef<any>(null);

  const start = async () => {
    try {
      const x = createRecorder();
      await x.start();
      r.current = x;
      setRec('on');
      tick.current = setInterval(() => { setMs(x.elapsedMs()); setLevel(x.level()); }, 200);
    } catch (e: any) {
      ui.snack(e.message || 'Microphone not available');
    }
  };
  const stop = async () => {
    const x = r.current;
    if (!x) return;
    clearInterval(tick.current);
    setRec('saving');
    const dur = x.elapsedMs();
    const blob = await x.stop();
    try {
      const res = await upload(blob, { kind: 'note', filename: `note.${blob.type.includes('mp4') ? 'm4a' : 'webm'}`, title: `Voice note ${new Date().toLocaleString()}`, durationS: dur / 1000 });
      ui.snack(`Note saved (${fmtMs(dur)}) · queued for processing`);
      await reload();
      setSel(res.file);
    } catch (e: any) {
      ui.snack(`Upload failed: ${e.message}`);
    } finally {
      setRec('off');
      setMs(0);
      r.current = null;
    }
  };
  useEffect(() => () => { clearInterval(tick.current); r.current?.cancel(); }, []);
  useEffect(() => {
    if (params.talk && rec === 'off') start();
  }, [params.talk]);

  const items = data?.items || [];
  const notes = items.filter((x) => x.kind === 'note');
  const other = items.filter((x) => x.kind !== 'note');
  const cur = items.find((x) => x.file === sel) || notes[0];

  return (
    <Screen title="Notes" testID="screen-notes">
      <T v="h1">Notes</T>
      {rec !== 'off' ? (
        <Card style={{ borderWidth: 2, borderColor: C.amber, gap: 12 }} testID="talking">
          <View style={s.row}>
            <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: C.red }} />
            <T v="eb" style={{ color: C.amber }}>Listening · <Text style={{ fontFamily: F.g6 }}>{fmtMs(ms)}</Text></T>
          </View>
          <View style={{ height: 8, borderRadius: 4, backgroundColor: C.surface2, overflow: 'hidden' }}>
            <View style={{ width: `${Math.max(3, level * 100)}%`, height: '100%', backgroundColor: C.amber }} />
          </View>
          <T v="mut">Talk it out. The transcript and any funny lines show up here after processing.</T>
          <Btn testID="talk-stop" v="primary" label={rec === 'saving' ? 'Saving…' : 'Stop & save'} disabled={rec === 'saving'} onPress={stop} />
        </Card>
      ) : (
        <Btn testID="talk-it-out" v="primary" label="Talk it out" onPress={() => ui.withMic(start)} />
      )}
      {error && <ErrorCard error={error} onRetry={reload} />}
      <View style={{ flexDirection: wide ? 'row' : 'column', gap: 16, alignItems: 'flex-start' }}>
        <View style={{ flex: wide ? 1.3 : undefined, alignSelf: 'stretch', gap: 16, minWidth: 0 }}>
          <Card style={{ paddingVertical: 4 }} testID="notes-list">
            <List>
              {notes.map((n) => (
                <ListRow key={n.file} onPress={() => setSel(n.file)} titleColor={cur?.file === n.file ? C.amber : undefined}
                  title={n.title} sub={`Voice · ${n.duration_s ? fmtMs(n.duration_s * 1000) : '—'} · ${fmtDate(n.received_at)} · ${n.status}`} />
              ))}
            </List>
            {data && !notes.length && <Empty text="No voice notes yet. Tap Talk it out." />}
          </Card>
          {other.length > 0 && (
            <>
              <T v="eb">Other recordings in the inbox</T>
              <Card style={{ paddingVertical: 4 }}>
                <List>
                  {other.map((n) => (
                    <ListRow key={n.file} onPress={() => setSel(n.file)} title={n.title}
                      sub={`${n.kind === 'set' ? 'Set' : 'Upload'} · ${n.duration_s ? fmtMs(n.duration_s * 1000) : `${Math.round(n.bytes / 1024)} KB`} · ${fmtDate(n.received_at)}`}
                      right={<Badge label={n.status} fg={C.blue} bg={C.blueSoft} />} />
                  ))}
                </List>
              </Card>
            </>
          )}
        </View>
        {cur && (
          <Card style={{ flex: wide ? 1 : undefined, alignSelf: 'stretch', gap: 10 }}>
            <T v="h2">{cur.title}</T>
            <View style={[s.inner, s.row]}>
              <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: C.blue }} />
              <T style={{ flex: 1 }}>Queued for processing</T>
            </View>
            <T v="mut">Saved to the tapes inbox as {cur.file}. Transcription and “Found in this note” come when the pipeline is wired in.</T>
          </Card>
        )}
      </View>
    </Screen>
  );
}
