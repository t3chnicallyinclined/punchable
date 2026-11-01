// Record a set with no set list, or a voice memo (`?kind=memo[&bit=<id>&title=<short name>]`): a full-screen task
// with the record control (docs/design/HIG.md §3.9; SPEC 2026-10-31 §5). Web: MediaRecorder; phone: expo-audio in
// the background (lib/recorder). The set recorder waits for the red disc (he opens it before he walks on); the memo
// recorder starts as soon as it opens (the tap on "Record a memo" is the decision, as in Voice Memos). The square
// stops AND saves. Discard is behind Close. A phone call shows "Paused by a call" and the disc reads Resume.
// Saving on the phone keeps the audio in Documents/recordings and queues its transcription ON the phone
// (lib/recordings): only the words go to the server. The web cannot transcribe a file: a set taped there goes to the
// upload queue and waits on the server (ADR 0012), and there is no memo recorder on the web.
import { useEffect, useRef, useState } from 'react';
import { AccessibilityInfo, Linking, Platform, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { haptics } from '@/lib/haptics';
import { actionSheet } from '@/lib/actionSheet';
import { useRecorder } from '@/lib/recorder';
import { RecordButton, RecordClock, RecordDock, SideButton } from '@/lib/RecordControl';
import { type RecordState, nextRecordState, statusWord } from '@/rules/recorderLogic';
import { enqueueUpload, usePendingUploads } from '@/lib/uploads';
import { addRecording, recordingsSupported, useRecordings } from '@/lib/recordings';
import { isPending } from '@/rules/memoRules';
import { C } from '@/lib/theme';
import { clockMs } from '@/rules/format';
import { Btn, Card, Chip, Sheet, SheetHead, T, TextBtn, useReduceMotion, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TopBar } from '@/lib/TopBar';
import { isWeb } from '@/lib/media';

const TARGETS = [5, 7, 10, 15, 20];
const one = (v: string | string[] | undefined) => (Array.isArray(v) ? v[0] : v) || '';

export default function RecordScreen() {
  const ui = useUI();
  const rec = useRecorder();
  const params = useLocalSearchParams<{ kind?: string; bit?: string; title?: string }>();
  const memo = one(params.kind) === 'memo';
  const bitId = one(params.bit);
  const bit = memo && bitId ? { id: bitId, title: one(params.title).slice(0, 60) || 'this bit' } : null;
  const noun = memo ? 'memo' : 'set';
  const reduceMotion = useReduceMotion();
  const [state, setState] = useState<RecordState>('idle');
  const [ms, setMs] = useState(0);
  const [marks, setMarks] = useState<number[]>([]);
  const [levels, setLevels] = useState<number[]>(Array(28).fill(0));
  const [intr, setIntr] = useState(false);
  const [target, setTarget] = useState(7);
  const [err, setErr] = useState<string | null>(null);
  const [micOff, setMicOff] = useState(false);
  const [ask, setAsk] = useState(false);
  const [starting, setStarting] = useState(false);                   // the disc was tapped; the microphone is opening
  const pending = usePendingUploads();
  const local = useRecordings();
  const waiting = local.filter((r) => r.kind === 'set' && isPending(r)).length;
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
      setMicOff(false);
      setState(nextRecordState('idle', 'start'));
      // queued, so VoiceOver finishes reading the screen first instead of cutting the word off
      if (memo) {
        if (AccessibilityInfo.announceForAccessibilityWithOptions) AccessibilityInfo.announceForAccessibilityWithOptions('Recording', { queue: true });
        else AccessibilityInfo.announceForAccessibility?.('Recording');
      }
    } catch (e: any) {
      const msg = String(e?.message || 'Microphone not available');
      if (/microphone access is off/i.test(msg)) setMicOff(true);
      else setErr(msg);
    } finally {
      setStarting(false);
    }
  };
  // the memo recorder starts as it opens (SPEC 2026-10-31 §5); the set recorder waits for the disc (HIG.md §3.9)
  useEffect(() => {
    if (!memo || !recordingsSupported) return;
    const t = setTimeout(() => void start(), 0);   // after the first frame, never a state change inside the effect
    return () => clearTimeout(t);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

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
      const recordedAt = new Date(Date.now() - r.durationMs).toISOString();
      if (recordingsSupported) {
        // the phone: the file is already in Documents/recordings; it is transcribed here and only the words go up
        await addRecording(r, { kind: memo ? 'memo' : 'set', recordedAt, bit, marksMs: memo ? undefined : marks });
        haptics.success();
        ui.snack(memo ? `Memo saved · ${clockMs(r.durationMs)}` : `Set saved · ${clockMs(r.durationMs)} · transcribing on this phone`);
      } else {
        const res = await enqueueUpload(r, { kind: 'set', filename: r.name, title: `Set ${new Date().toLocaleString()}`, durationS: r.durationMs / 1000, marksMs: marks, recordedAt });
        haptics.success();
        ui.snack(res.queued ? `Saved · ${clockMs(r.durationMs)} · it goes up when Punchable is back` : `Saved · ${clockMs(r.durationMs)} · it waits on the server. Sets taped in the iPhone app get their words right away.`);
        (globalThis as any).__punchableLastUpload = res;
      }
      setState(nextRecordState('saving', 'saved'));
      leave();
    } catch (e: any) {
      // the recorder has stopped either way; a fresh take, not a Resume that cannot resume
      setState(nextRecordState('saving', 'failed'));
      setMs(0);
      setErr(`Couldn’t save: ${e?.message || e}. The recording is still on this ${native ? 'phone' : 'device'}.`);
    } finally {
      saving.current = false;
    }
  };
  const close = () => {
    if (state === 'saving') return;     // the save is already stopping the take and will leave on its own [review]
    if (!taping) return leave();
    if (!native) return setAsk(true);
    actionSheet({ title: memo ? 'Leave the memo?' : 'Leave the recorder?', message: memo ? `${clockMs(ms)} recorded.` : `${clockMs(ms)} on tape.`, options: [
      { label: memo ? 'Discard the memo' : 'Discard the recording', destructive: true, onPress: discard },
      { label: memo ? 'Save the memo' : 'Save the set', onPress: save },
      { label: 'Keep going', cancel: true },
    ] });
  };
  const status = statusWord(state, intr, false, starting);

  if (memo && !recordingsSupported) {
    return (
      <Screen title="Memo" task deep testID="screen-record"
        header={<TopBar task title="" left={<TextBtn testID="record-cancel" label="Close" onPress={leave} />} right={<View style={{ width: 66 }} />} />}>
        <T v="mut" style={{ textAlign: 'center' }}>Voice memos are recorded in the iPhone app. Their words show up here.</T>
      </Screen>
    );
  }

  return (
    <Screen title={memo ? 'Memo' : 'Record'} task deep testID="screen-record"
      header={<TopBar task title="" left={<TextBtn testID="record-cancel" label="Close" onPress={close} />} right={<View style={{ width: 66 }} />} />}
      dock={
        <RecordDock
          left={<SideButton testID="record-pause" label={state === 'paused' || (state === 'recording' && intr) ? 'Paused' : 'Pause'} disabled={state !== 'recording' || intr} onPress={pause} />}
          right={memo ? <View /> : <SideButton testID="record-mark" label="Mark" tone="amber" disabled={state !== 'recording'} onPress={() => { haptics.light(); setMarks((m) => [...m, ms]); ui.snack(`Marked at ${clockMs(ms)}`); }} />}>
          <RecordButton testID="record-rec" state={state} interrupted={intr} busy={starting} onStart={start} onStop={save} onResume={resume}
            stopLabel={memo ? 'Stop and save the memo' : 'Stop and save the recording'} />
        </RecordDock>
      }>
      <RecordClock testID="record" ms={ms} status={status} targetS={memo ? undefined : target * 60} level={taping ? levels[levels.length - 1] : null} interrupted={intr} />
      {bit && <T v="semi" testID="record-for" numberOfLines={2} style={{ textAlign: 'center', color: C.text2 }}>For {bit.title}</T>}
      {!reduceMotion && (
        <View style={{ flexDirection: 'row', alignItems: 'flex-end', gap: 3, height: 56 }} aria-hidden>
          {levels.map((l, k) => <View key={k} style={{ flex: 1, height: `${Math.max(6, l * 100)}%`, borderRadius: 2, backgroundColor: l > 0.55 ? C.amber : C.border }} />)}
        </View>
      )}
      {!taping && !memo && (
        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8, justifyContent: 'center' }}>
          {TARGETS.map((t) => <Chip key={t} label={`${t} min`} on={t === target} onPress={() => setTarget(t)} />)}
        </View>
      )}
      {marks.length > 0 && (
        <Card>
          <T v="eb">Marks</T>
          <T style={{ marginTop: 4 }}>{marks.map(clockMs).join(' · ')}</T>
        </Card>
      )}
      {!memo && pending.length > 0 && <T v="small" testID="record-pending">{pending.length} recording{pending.length === 1 ? '' : 's'} waiting to go up (no connection). They send on their own.</T>}
      {!memo && waiting > 0 && <T v="small" testID="record-waiting">{waiting} set{waiting === 1 ? '' : 's'} on this phone still being transcribed or sent. Sets › Recorded shows where each one is.</T>}
      {micOff && (
        <Card style={{ gap: 10 }} testID="record-mic-off">
          <T>Punchable can’t use the microphone.</T>
          <Btn testID="record-open-settings" label="Open Settings" onPress={() => void Linking.openSettings()} />
        </Card>
      )}
      {err && <Card style={{ borderColor: C.dangerBorder }}><T>{err}</T></Card>}
      {!micOff && <T v="mut" style={{ textAlign: 'center' }}>
        {memo
          ? 'Lock the screen if you like: it keeps recording.'
          : `${state === 'idle' ? 'Tap the red button when you’re on. ' : ''}${isWeb ? 'Phone face-down at the stage edge, mic toward the crowd. Keep this tab open while you’re on.' : 'Phone face-down at the stage edge, mic toward the crowd. Lock the screen if you like: it keeps recording.'}`}
      </T>}
      <Sheet visible={ask} onClose={() => setAsk(false)} label="Leave the recorder" testID="record-leave-sheet">
        <SheetHead title="Leave the recorder?" onCancel={() => setAsk(false)} cancelLabel="Keep going" testID="record-leave" />
        <T v="mut" style={{ marginBottom: 12 }}>{clockMs(ms)} {memo ? 'recorded' : 'on tape'}.</T>
        <View style={{ gap: 8 }}>
          <Btn testID="record-save" v="primary" label={`Save the ${noun}`} onPress={save} />
          <Btn testID="record-discard" v="danger" label={`Discard the ${noun === 'set' ? 'recording' : 'memo'}`} onPress={discard} />
        </View>
      </Sheet>
    </Screen>
  );
}
