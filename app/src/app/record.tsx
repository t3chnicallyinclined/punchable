// Record a set with no set list (full-screen task; the record control per docs/design/HIG.md §3.9, 2026-10-29). Web:
// MediaRecorder; phone: expo-audio in the background (lib/recorder). Nothing records until he taps the red disc; the
// clock, the light warning and the level bars; Mark while taping; the square stops AND saves (Voice Memos). Discard is
// behind Close. A phone call shows "Paused by a call" and the disc reads Resume. Then the tape goes to the upload queue
// (lib/uploads: retried on the backoff schedule when there is no signal) as a set for the tapes inbox.
import { useEffect, useRef, useState } from 'react';
import { Platform, View } from 'react-native';
import { router } from 'expo-router';
import { haptics } from '@/lib/haptics';
import { actionSheet } from '@/lib/actionSheet';
import { useRecorder } from '@/lib/recorder';
import { RecordButton, RecordClock, RecordDock, SideButton } from '@/lib/RecordControl';
import { type RecordState, nextRecordState, statusWord } from '@/lib/recorderLogic';
import { enqueueUpload, usePendingUploads } from '@/lib/uploads';
import { C, fmtMs } from '@/lib/theme';
import { Btn, Card, Chip, Sheet, SheetHead, T, TextBtn, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TopBar } from '@/lib/TopBar';
import { isWeb } from '@/lib/media';

const TARGETS = [5, 7, 10, 15, 20];

export default function RecordScreen() {
  const ui = useUI();
  const rec = useRecorder();
  const [state, setState] = useState<RecordState>('idle');
  const [ms, setMs] = useState(0);
  const [marks, setMarks] = useState<number[]>([]);
  const [levels, setLevels] = useState<number[]>(Array(28).fill(0));
  const [intr, setIntr] = useState(false);
  const [target, setTarget] = useState(7);
  const [err, setErr] = useState<string | null>(null);
  const [ask, setAsk] = useState(false);
  const [starting, setStarting] = useState(false);                   // the disc was tapped; the microphone is opening
  const pending = usePendingUploads();
  const native = Platform.OS !== 'web';
  const taping = state === 'recording' || state === 'paused' || state === 'saving';
  const tapingRef = useRef(taping);
  tapingRef.current = taping;
  const stateRef = useRef(state);                                    // the action sheet's Save reads the live state
  stateRef.current = state;
  const saving = useRef(false);

  useEffect(() => {
    const t = setInterval(() => {
      if (!tapingRef.current) return;
      setMs(rec.elapsedMs());
      setLevels((xs) => [...xs.slice(1), rec.level()]);
      setIntr(rec.interrupted());
    }, 200);
    return () => { clearInterval(t); if (rec.state() === 'recording' || rec.state() === 'paused') rec.cancel(); };
  }, [rec]);

  const leave = () => (router.canGoBack() ? router.back() : router.navigate('/' as any));
  const start = async () => {
    if (!rec.supported) { setErr(isWeb ? 'This browser can’t record audio.' : 'No microphone available'); return; }
    if (starting || stateRef.current !== 'idle') return;
    setStarting(true);
    try {
      await rec.start();
      haptics.medium();
      setErr(null);
      setState(nextRecordState('idle', 'start'));
    } catch (e: any) {
      setErr(e?.message || 'Microphone not available');
    } finally {
      setStarting(false);
    }
  };
  const pause = () => { rec.pause(); setState(nextRecordState(state, 'pause')); };
  const resume = () => { rec.resume(); setIntr(rec.interrupted()); setState(nextRecordState(state, 'resume')); };
  const discard = () => { setAsk(false); rec.cancel(); setState('idle'); setMs(0); leave(); };
  const save = async () => {
    setAsk(false);
    // one save per take (the square and the sheet's Save can both land; a second stop() has no file to hand back)
    if (saving.current || nextRecordState(stateRef.current, 'stop') !== 'saving') return;
    saving.current = true;
    setState(nextRecordState(stateRef.current, 'stop'));
    try {
      const r = await rec.stop();
      const res = await enqueueUpload(r, { kind: 'set', filename: r.name, title: `Set ${new Date().toLocaleString()}`, durationS: r.durationMs / 1000, marksMs: marks, recordedAt: new Date(Date.now() - r.durationMs).toISOString() });
      haptics.success();
      ui.snack(res.queued ? `Saved · ${fmtMs(r.durationMs)} · it goes up when Punchable is back` : `Saved · ${fmtMs(r.durationMs)}`);
      setState(nextRecordState('saving', 'saved'));
      leave();
      (globalThis as any).__punchableLastUpload = res;
    } catch (e: any) {
      // the recorder has stopped either way; a fresh take, not a Resume that cannot resume
      setState(nextRecordState('saving', 'failed'));
      setMs(0);
      setErr(`Couldn’t save: ${e?.message || e}. The tape is still on this phone.`);
    } finally {
      saving.current = false;
    }
  };
  const close = () => {
    if (!taping) return leave();
    if (!native) return setAsk(true);
    actionSheet({ title: 'Leave the recorder?', message: `${fmtMs(ms)} on tape.`, options: [
      { label: 'Discard the recording', destructive: true, onPress: discard },
      { label: 'Save the set', onPress: save },
      { label: 'Keep going', cancel: true },
    ] });
  };
  const status = statusWord(state, intr, false, starting);

  return (
    <Screen title="Record" task deep testID="screen-record"
      header={<TopBar task title="" left={<TextBtn testID="record-cancel" label="Close" onPress={close} />} right={<View style={{ width: 66 }} />} />}
      dock={
        <RecordDock
          left={<SideButton testID="record-pause" label={state === 'paused' || (state === 'recording' && intr) ? 'Paused' : 'Pause'} disabled={state !== 'recording' || intr} onPress={pause} />}
          right={<SideButton testID="record-mark" label="Mark" tone="amber" disabled={state !== 'recording'} onPress={() => { haptics.light(); setMarks((m) => [...m, ms]); ui.snack(`Marked at ${fmtMs(ms)}`); }} />}>
          <RecordButton testID="record-rec" state={state} interrupted={intr} busy={starting} onStart={start} onStop={save} onResume={resume} />
        </RecordDock>
      }>
      <RecordClock testID="record" ms={ms} status={status} targetS={target * 60} level={taping ? levels[levels.length - 1] : null} interrupted={intr} />
      <View style={{ flexDirection: 'row', alignItems: 'flex-end', gap: 3, height: 56 }} aria-hidden>
        {levels.map((l, k) => <View key={k} style={{ flex: 1, height: `${Math.max(6, l * 100)}%`, borderRadius: 2, backgroundColor: l > 0.55 ? C.amber : C.border }} />)}
      </View>
      {!taping && (
        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8, justifyContent: 'center' }}>
          {TARGETS.map((t) => <Chip key={t} label={`${t} min`} on={t === target} onPress={() => setTarget(t)} />)}
        </View>
      )}
      {marks.length > 0 && (
        <Card>
          <T v="eb">Marks</T>
          <T style={{ marginTop: 4 }}>{marks.map(fmtMs).join(' · ')}</T>
        </Card>
      )}
      {pending.length > 0 && <T v="small" testID="record-pending">{pending.length} recording{pending.length === 1 ? '' : 's'} waiting to go up (no connection). They send on their own.</T>}
      {err && <Card style={{ borderColor: C.dangerBorder }}><T>{err}</T></Card>}
      <T v="mut" style={{ textAlign: 'center' }}>{state === 'idle' ? 'Tap the red button when you’re on. ' : ''}{isWeb ? 'Phone face-down at the stage edge, mic toward the crowd. Keep this tab open while you’re on.' : 'Phone face-down at the stage edge, mic toward the crowd. Lock the screen if you like: it keeps recording.'}</T>
      <Sheet visible={ask} onClose={() => setAsk(false)} label="Leave the recorder" testID="record-leave-sheet">
        <SheetHead title="Leave the recorder?" onCancel={() => setAsk(false)} cancelLabel="Keep going" testID="record-leave" />
        <T v="mut" style={{ marginBottom: 12 }}>{fmtMs(ms)} on tape.</T>
        <View style={{ gap: 8 }}>
          <Btn testID="record-save" v="primary" label="Save the set" onPress={save} />
          <Btn testID="record-discard" v="danger" label="Discard the recording" onPress={discard} />
        </View>
      </Sheet>
    </Screen>
  );
}
