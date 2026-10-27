// Record a set (full-screen task). Web: MediaRecorder, timer, light warning, level bars, Mark / Pause / Done,
// then upload to the tapes inbox ("queued"; processing is wired later).
// TODO(native): locked-screen recording (expo-audio recorder + UIBackgroundModes audio) and a Live Activity timer.
import { useEffect, useRef, useState } from 'react';
import { Text, View } from 'react-native';
import { router } from 'expo-router';
import { upload } from '@/lib/api';
import { C, F, fmtMs } from '@/lib/theme';
import { Badge, Btn, Card, Chip, Screen, T, TextBtn, TopBar, s, useUI } from '@/lib/ui';
import { Recorder, createRecorder, isWeb, micSupported } from '@/lib/media';

const TARGETS = [5, 7, 10, 15, 20];

export default function RecordScreen() {
  const ui = useUI();
  const rec = useRef<Recorder | null>(null);
  const [state, setState] = useState<'idle' | 'recording' | 'paused' | 'saving' | 'error'>('idle');
  const [ms, setMs] = useState(0);
  const [marks, setMarks] = useState<number[]>([]);
  const [levels, setLevels] = useState<number[]>(Array(28).fill(0));
  const [target, setTarget] = useState(7);
  const [err, setErr] = useState<string | null>(null);
  const light = (target - 1) * 60000;

  useEffect(() => {
    if (!isWeb || !micSupported()) {
      setState('error');
      setErr(isWeb ? 'This browser can’t record audio.' : 'Recording on the phone build is coming next (expo-audio, locked screen, Live Activity).');
      return;
    }
    const r = createRecorder();
    rec.current = r;
    r.start().then(() => setState('recording')).catch((e) => { setState('error'); setErr(e?.message || 'Microphone not available'); });
    const t = setInterval(() => {
      setMs(r.elapsedMs());
      setLevels((xs) => [...xs.slice(1), r.level()]);
    }, 200);
    return () => {
      clearInterval(t);
      if (r.state() === 'recording' || r.state() === 'paused') r.cancel();
    };
  }, []);

  const cancel = () => {
    rec.current?.cancel();
    router.canGoBack() ? router.back() : router.navigate('/' as any);
  };
  const done = async () => {
    const r = rec.current;
    if (!r) return;
    setState('saving');
    const dur = r.elapsedMs();
    const blob = await r.stop();
    try {
      const ext = blob.type.includes('mp4') ? 'm4a' : blob.type.includes('ogg') ? 'ogg' : 'webm';
      const res = await upload(blob, { kind: 'set', filename: `set.${ext}`, title: `Set ${new Date().toLocaleString()}`, durationS: dur / 1000, marksMs: marks });
      ui.snack(`Set saved (${fmtMs(dur)}) · queued for processing`);
      router.canGoBack() ? router.back() : router.navigate('/' as any);
      (globalThis as any).__punchableLastUpload = res;
    } catch (e: any) {
      setState('error');
      setErr(`Upload failed: ${e.message}. The recording is still in this tab; try Done again.`);
      (globalThis as any).__punchableUnsaved = blob;
    }
  };
  const pct = Math.min(100, (ms / (target * 60000)) * 100);
  const toLight = light - ms;

  return (
    <Screen title="Record" task deep testID="screen-record"
      header={<TopBar task title="" left={<TextBtn testID="record-cancel" label="Cancel" onPress={cancel} />}
        right={<View style={{ width: 66 }} />} />}
      dock={
        <View style={{ flexDirection: 'row', gap: 8 }}>
          <Btn testID="record-mark" label="Mark" style={{ flex: 1, minHeight: 60 }} disabled={state !== 'recording'} onPress={() => { setMarks((m) => [...m, ms]); ui.snack(`Marked at ${fmtMs(ms)}`); }} />
          <Btn testID="record-pause" label={state === 'paused' ? 'Resume' : 'Pause'} style={{ flex: 1, minHeight: 60 }} disabled={state !== 'recording' && state !== 'paused'}
            onPress={() => { if (state === 'paused') { rec.current?.resume(); setState('recording'); } else { rec.current?.pause(); setState('paused'); } }} />
          <Btn testID="record-done" v="primary" label={state === 'saving' ? 'Saving…' : 'Done'} style={{ flex: 1, minHeight: 60 }} disabled={state !== 'recording' && state !== 'paused'} onPress={done} />
        </View>
      }>
      <View style={{ alignItems: 'center' }}>
        <Badge label={state === 'paused' ? 'Paused' : state === 'recording' ? '● REC' : state === 'saving' ? 'Saving' : 'Ready'} fg={state === 'recording' ? C.redSoft : C.text} style={{ borderWidth: 2, borderColor: C.amber, height: 28 }} />
      </View>
      <Card style={{ alignItems: 'center' }}>
        <T v="eb">On stage</T>
        <Text testID="record-timer" style={{ fontFamily: F.g7, fontSize: 72, lineHeight: 76, color: C.text, marginTop: 6, fontVariant: ['tabular-nums'] }}>{fmtMs(ms)}</Text>
        <Text style={{ fontFamily: F.g7, fontSize: 18, color: toLight > 0 ? C.amber : C.redSoft }}>{toLight > 0 ? `Light in ${fmtMs(toLight)}` : ms > target * 60000 ? `Over by ${fmtMs(ms - target * 60000)}` : 'Light’s on: wrap it up'}</Text>
        <View style={{ alignSelf: 'stretch', height: 8, borderRadius: 4, backgroundColor: C.surface2, marginTop: 16, overflow: 'hidden' }}>
          <View style={{ width: `${pct}%`, height: '100%', backgroundColor: toLight > 0 ? C.amber : C.red }} />
          <View style={{ position: 'absolute', left: `${((target - 1) / target) * 100}%`, top: 0, bottom: 0, width: 2, backgroundColor: C.text }} />
        </View>
        <View style={[s.row, { alignSelf: 'stretch', justifyContent: 'space-between', marginTop: 6 }]}>
          <T v="mut">0:00</T><T v="mut">light {target - 1}:00</T><T v="mut">{target}:00</T>
        </View>
      </Card>
      <View style={{ flexDirection: 'row', alignItems: 'flex-end', gap: 3, height: 56 }} aria-hidden>
        {levels.map((l, k) => <View key={k} style={{ flex: 1, height: `${Math.max(6, l * 100)}%`, borderRadius: 2, backgroundColor: l > 0.55 ? C.amber : C.border }} />)}
      </View>
      <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8, justifyContent: 'center' }}>
        {TARGETS.map((t) => <Chip key={t} label={`${t} min`} on={t === target} onPress={() => setTarget(t)} />)}
      </View>
      {marks.length > 0 && (
        <Card>
          <T v="eb">Marks</T>
          <T style={{ marginTop: 4 }}>{marks.map(fmtMs).join(' · ')}</T>
        </Card>
      )}
      {err && <Card style={{ borderColor: C.dangerBorder }}><T>{err}</T></Card>}
      <T v="mut" style={{ textAlign: 'center' }}>Phone face-down at the stage edge, mic toward the crowd. Keep this tab open while you’re on.</T>
    </Screen>
  );
}
