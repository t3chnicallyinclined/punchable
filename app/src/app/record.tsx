// Record a set (full-screen task). Web: MediaRecorder; phone: expo-audio in the background (lib/recorder). The
// timer, the light warning, level bars, Mark / Pause / Done, then the tape goes to the upload queue (lib/uploads:
// retried on the backoff schedule when there is no signal) as a set for the tapes inbox.
import { useEffect, useRef, useState } from 'react';
import { Text, View } from 'react-native';
import { router } from 'expo-router';
import { haptics } from '@/lib/haptics';
import { useRecorder } from '@/lib/recorder';
import { enqueueUpload, usePendingUploads } from '@/lib/uploads';
import { C, F, fmtMs } from '@/lib/theme';
import { Badge, Btn, Card, Chip, Sheet, SheetHead, T, TextBtn, s, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TopBar } from '@/lib/TopBar';
import { isWeb } from '@/lib/media';

const TARGETS = [5, 7, 10, 15, 20];

export default function RecordScreen() {
  const ui = useUI();
  const rec = useRecorder();
  const [state, setState] = useState<'idle' | 'recording' | 'paused' | 'saving' | 'error'>('idle');
  const [ms, setMs] = useState(0);
  const [marks, setMarks] = useState<number[]>([]);
  const [levels, setLevels] = useState<number[]>(Array(28).fill(0));
  const [target, setTarget] = useState(7);
  const [err, setErr] = useState<string | null>(null);
  const [ask, setAsk] = useState(false);
  const pending = usePendingUploads();
  const started = useRef(false);
  const light = (target - 1) * 60000;

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    if (!rec.supported) {
      setState('error');
      setErr('This browser can’t record audio.');
      return;
    }
    rec.start().then(() => setState('recording')).catch((e) => { setState('error'); setErr(e?.message || 'Microphone not available'); });
    const t = setInterval(() => {
      setMs(rec.elapsedMs());
      setLevels((xs) => [...xs.slice(1), rec.level()]);
    }, 200);
    return () => {
      clearInterval(t);
      if (rec.state() === 'recording' || rec.state() === 'paused') rec.cancel();
    };
  }, [rec]);

  const leave = () => (router.canGoBack() ? router.back() : router.navigate('/' as any));
  const cancel = () => {
    if (state === 'recording' || state === 'paused') { haptics.warning(); setAsk(true); return; }
    leave();
  };
  const discard = () => { setAsk(false); rec.cancel(); leave(); };
  const done = async () => {
    setAsk(false);
    setState('saving');
    try {
      const r = await rec.stop();
      const res = await enqueueUpload(r, { kind: 'set', filename: r.name, title: `Set ${new Date().toLocaleString()}`, durationS: r.durationMs / 1000, marksMs: marks, recordedAt: new Date().toISOString() });
      haptics.success();
      ui.snack(res.queued ? `Set saved on this ${isWeb ? 'tab' : 'phone'} (${fmtMs(r.durationMs)}) · it goes up when Punchable is back` : `Set saved (${fmtMs(r.durationMs)}) · queued for processing`);
      leave();
      (globalThis as any).__punchableLastUpload = res;
    } catch (e: any) {
      setState('error');
      setErr(`Couldn’t save: ${e.message}.`);
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
          <Btn testID="record-mark" label="Mark" style={{ flex: 1, minHeight: 60 }} disabled={state !== 'recording'} onPress={() => { haptics.light(); setMarks((m) => [...m, ms]); ui.snack(`Marked at ${fmtMs(ms)}`); }} />
          <Btn testID="record-pause" label={state === 'paused' ? 'Resume' : 'Pause'} style={{ flex: 1, minHeight: 60 }} disabled={state !== 'recording' && state !== 'paused'}
            onPress={() => { if (state === 'paused') { rec.resume(); setState('recording'); } else { rec.pause(); setState('paused'); } }} />
          <Btn testID="record-done" v="primary" label={state === 'saving' ? 'Saving…' : 'Done'} style={{ flex: 1, minHeight: 60 }} disabled={state !== 'recording' && state !== 'paused'} onPress={done} />
        </View>
      }>
      <View style={{ alignItems: 'center' }}>
        <Badge label={state === 'paused' ? 'Paused' : state === 'recording' ? '● REC' : state === 'saving' ? 'Saving' : 'Ready'} fg={state === 'recording' ? C.redSoft : C.text} style={{ borderWidth: 2, borderColor: C.amber, minHeight: 28 }} />
      </View>
      <Card style={{ alignItems: 'center' }}>
        <T v="eb">On stage</T>
        <Text testID="record-timer" maxFontSizeMultiplier={1.2} style={{ fontFamily: F.g7, fontSize: 72, lineHeight: 76, color: C.text, marginTop: 6, fontVariant: ['tabular-nums'] }}>{fmtMs(ms)}</Text>
        <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.g7, fontSize: 18, color: toLight > 0 ? C.amber : C.redSoft }}>{toLight > 0 ? `Light in ${fmtMs(toLight)}` : ms > target * 60000 ? `Over by ${fmtMs(ms - target * 60000)}` : 'Light’s on: wrap it up'}</Text>
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
      {pending.length > 0 && <T v="small" testID="record-pending">{pending.length} recording{pending.length === 1 ? '' : 's'} waiting to go up (no connection). They send on their own.</T>}
      {err && <Card style={{ borderColor: C.dangerBorder }}><T>{err}</T></Card>}
      <T v="mut" style={{ textAlign: 'center' }}>{isWeb ? 'Phone face-down at the stage edge, mic toward the crowd. Keep this tab open while you’re on.' : 'Phone face-down at the stage edge, mic toward the crowd. Lock the screen if you like: it keeps recording.'}</T>
      <Sheet visible={ask} onClose={() => setAsk(false)} label="Leave the recorder" testID="record-leave-sheet">
        <SheetHead title="Stop recording?" onCancel={() => setAsk(false)} cancelLabel="Keep going" testID="record-leave" />
        <T v="mut" style={{ marginBottom: 12 }}>{fmtMs(ms)} on tape.</T>
        <View style={{ gap: 8 }}>
          <Btn testID="record-save" v="primary" label="Save the set" onPress={done} />
          <Btn testID="record-discard" v="danger" label="Discard the recording" onPress={discard} />
        </View>
      </Sheet>
    </Screen>
  );
}
