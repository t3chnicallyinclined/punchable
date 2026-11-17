// The walk-through (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §2.4, §4.4, Slice 5): Record or Film a set from one
// screen that shows ONE bit at a time, his notes for it (or the bit's full lines, a tap on the switch), with the dock
// Back (60 pt) · the disc / stop square (92 pt) · Next (60 pt). Each Next or Back while recording closes a SPLIT for
// the bit he leaves on the recording's own position (rules/runRules), so the take comes back cut up by bit (After,
// app/after/[rec].tsx) and the server is told which set it followed and where each bit sat (POST /api/sets/transcript
// `board`, `splits`, `mode`).
//   ?mode=sound (default): expo-audio through lib/recorder (keeps taping with the phone locked; a call pauses it and
//     the clock holds, so no split covers the call). Nothing tapes until the red disc. The square stops and saves to
//     the phone (lib/recordings: transcribed here, the words go up) and opens After. Discard is behind Close.
//   ?mode=video: the front camera on top (expo-camera, as Film it), the bit below it, 3 · 2 · 1, the take capped at
//     SET_TAKE_S; the square goes to the review (Redo / Keep); Keep puts the one copy in Photos (lib/film keepSetTake)
//     and opens After. Locking the phone or a call ends a video take (the system stops the camera).
// No haptic on Next or Back while a microphone is live (HIG/playing-haptics; Film it's rule). Next never turns into
// Finish under the thumb: it greys as "Last bit". The web records sound only (MediaRecorder; the tape waits on the
// server: ADR 0012) and has no After. /stage/<id> redirects here.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { AccessibilityInfo, AppState, Linking, Platform, Pressable, ScrollView, Text, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { useKeepAwake } from 'expo-keep-awake';
import { useApi } from '@/lib/api';
import { haptics } from '@/lib/haptics';
import { actionSheet } from '@/lib/actionSheet';
import { setRecording } from '@/lib/audioBusy';
import { isWeb } from '@/lib/media';
import { useRecorder } from '@/lib/recorder';
import { RecordButton, RecordClock, RecordDock, SideButton } from '@/lib/RecordControl';
import { enqueueUpload } from '@/lib/uploads';
import { addRecording, recordingsSupported } from '@/lib/recordings';
import * as Film from '@/lib/film';
import type { CameraModule, Take, VideoModule } from '@/lib/film';
import type { Build, Step } from '@/lib/simple';
import { shortNameOf } from '@/lib/say';
import { C, DARK, F, linkColor } from '@/lib/theme';
import { Btn, Empty, Skeleton, T, TextBtn, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TopBar } from '@/lib/TopBar';
import { clockMs } from '@/rules/format';
import { type RecordState, nextRecordState, statusWord } from '@/rules/recorderLogic';
import * as FR from '@/rules/filmRules';
import type { FilmState } from '@/rules/filmRules';
import * as RR from '@/rules/runRules';

const enc = encodeURIComponent;
const native = Platform.OS !== 'web';
const TICK_MS = 250;

type View_ = 'notes' | 'lines';

/** The bit on screen: its name, then his notes (or the app's, muted, while he has written none) or its full lines. */
function BitPane({ step, i, n, view, onToggle }: { step: Step | undefined; i: number; n: number; view: View_; onToggle: () => void }) {
  if (!step) return null;
  const name = step.short_name || shortNameOf(step.title);
  const notes = step.notes || [];
  const his = step.notes_from === 'his';
  const lines = step.text ? [step.text.setup, step.text.reveal].filter((x): x is string => !!x) : step.idea ? [step.idea] : [];
  const showLines = view === 'lines' || !notes.length;
  return (
    <View testID="run-bit" style={{ gap: 10 }}>
      <Text accessibilityRole="header" maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 15, color: C.mut }}>{`Bit ${i + 1} of ${n}`}</Text>
      <Text testID="run-bit-name" maxFontSizeMultiplier={1.3} style={{ fontFamily: F.black, fontSize: 36, lineHeight: 42, letterSpacing: -0.8, color: C.amber }}>{name}</Text>
      {/* his words are plain text VoiceOver reads (design review); only the switch row below toggles them */}
      <View testID="run-words" style={{ gap: 8 }}>
        {showLines ? (
          lines.length ? lines.map((l, k) => (
            <Text key={k} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 22, lineHeight: 30, color: C.text }}>{l}</Text>
          )) : <T v="mut">No words on this bit yet.</T>
        ) : notes.map((x, k) => (
          <Text key={k} maxFontSizeMultiplier={1.6} style={{ fontFamily: his ? F.semi : F.body, fontSize: 24, lineHeight: 32, color: his ? C.text : C.mut }}>{`• ${x}`}</Text>
        ))}
      </View>
      {/* Notes / Full lines (§4.4): one 44-pt switch row under the words */}
      {!!notes.length && (
        <Pressable testID="run-view" accessibilityRole="button"
          onPress={onToggle} style={({ pressed }) => ({ minHeight: 44, justifyContent: 'center', alignSelf: 'flex-start', opacity: pressed ? 0.7 : 1 })}>
          <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 15, color: linkColor() }}>{showLines ? 'Show my notes' : 'Show full lines'}</Text>
        </Pressable>
      )}
    </View>
  );
}

export default function Run() {
  const p = useLocalSearchParams<{ id: string; mode?: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const video = p.mode === 'video';
  return video && native && Film.filmSupported ? <RunVideo id={id} /> : <RunSound id={id} video={video} />;
}

/** Leave the walk-through: back to the set, or the set's page on a cold link. */
const leaveTo = (id: string) => (router.canGoBack() ? router.back() : router.navigate(`/list/${enc(id)}` as any));

function useSet(id: string) {
  const b = useApi<Build>(`/api/boards/${enc(id)}/build`);
  const steps = useMemo(() => b.data?.steps || [], [b.data]);
  const ids = useMemo(() => steps.map((s) => s.joke_id), [steps]);
  const bits = useMemo<RR.RunBit[]>(() => steps.map((s) => ({ id: s.joke_id, name: s.short_name || shortNameOf(s.title) })), [steps]);
  return { b, B: b.data, steps, ids, bits };
}

// ------------------------------------------------------------------ sound
function RunSound({ id, video }: { id: string; video: boolean }) {
  useKeepAwake();
  const ui = useUI();
  const { b, B, steps, ids, bits } = useSet(id);
  const recorder = useRecorder();
  const [run, setRun] = useState<RR.Run>(RR.runIdle());
  const runRef = useRef(run);
  useEffect(() => { runRef.current = run; }, [run]);
  const [view, setView] = useState<View_>('notes');
  const [ms, setMs] = useState(0);
  const [rec, setRec] = useState<RecordState>('idle');
  const recRef = useRef(rec);
  useEffect(() => { recRef.current = rec; }, [rec]);
  const [lvl, setLvl] = useState(0);
  const [intr, setIntr] = useState(false);
  const [starting, setStarting] = useState(false);
  const saving = useRef(false);
  const taping = rec === 'recording' || rec === 'paused' || rec === 'saving';
  const target = B?.board.target_s || 0;

  useEffect(() => {
    if (!taping) return;
    const t = setInterval(() => { setMs(recorder.elapsedMs()); setLvl(recorder.level()); setIntr(recorder.interrupted()); }, TICK_MS);
    return () => clearInterval(t);
  }, [taping, recorder]);
  useEffect(() => () => { if (recorder.state() === 'recording' || recorder.state() === 'paused') recorder.cancel(); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const go = (to: number) => {
    const live = recRef.current === 'recording' || recRef.current === 'paused';
    if (!live) haptics.selection();   // never while the microphone is live (HIG/playing-haptics)
    setRun((r) => RR.runGo(r, to, live ? recorder.elapsedMs() : 0, ids));
  };
  const start = async () => {
    if (!recorder.supported) return ui.snack(isWeb ? 'This browser can’t record audio.' : 'No microphone available');
    if (starting || recRef.current !== 'idle') return;
    setStarting(true);
    haptics.medium();   // the one haptic, on the press: before the microphone opens, so it never lands on the tape (HIG/playing-haptics)
    try {
      await recorder.start();
      setRun((r) => RR.runStart(RR.runIdle(r.i), 0));
      setRec(nextRecordState('idle', 'start'));
    } catch (e: any) {
      ui.snack(e?.message || 'Microphone not available');
    } finally {
      setStarting(false);
    }
  };
  const pause = () => { recorder.pause(); setRec(nextRecordState(recRef.current, 'pause')); };
  const resume = () => { recorder.resume(); setIntr(recorder.interrupted()); setRec(nextRecordState(recRef.current, 'resume')); };
  const save = async () => {
    if (saving.current || nextRecordState(recRef.current, 'stop') !== 'saving') return;
    saving.current = true;
    const splits = RR.runEnd(runRef.current, recorder.elapsedMs(), ids);
    setRec(nextRecordState(recRef.current, 'stop'));
    const title = B?.board.title || 'Set';
    try {
      const r = await recorder.stop();
      const recordedAt = new Date(Date.now() - r.durationMs).toISOString();
      if (recordingsSupported) {
        const row = await addRecording(r, { kind: 'set', mode: 'sound', recordedAt, board: { id, title }, title, runBits: bits, splits });
        haptics.success();
        setRec(nextRecordState('saving', 'saved'));
        router.replace({ pathname: '/after/[rec]', params: { rec: row.id } } as any);
      } else {
        const res = await enqueueUpload(r, { kind: 'set', filename: r.name, title, durationS: r.durationMs / 1000, recordedAt, showGroup: id });
        haptics.success();
        ui.snack(res.queued ? `Saved · ${clockMs(r.durationMs)} · it goes up when there’s signal` : `Saved · ${clockMs(r.durationMs)} · it waits on the server. Sets taped in the iPhone app get their words right away.`);
        setRec(nextRecordState('saving', 'saved'));
        leaveTo(id);
      }
    } catch (e: any) {
      setRec(nextRecordState('saving', 'failed'));
      setMs(0);
      ui.snack(`Couldn’t save the set: ${e?.message || e}. The tape is still on this ${native ? 'phone' : 'device'}.`);
    } finally {
      saving.current = false;
    }
  };
  const discard = () => {
    recorder.cancel();
    setRec('idle');
    ui.snack('Recording discarded');
    leaveTo(id);
  };
  const close = () => {
    if (recRef.current === 'saving') return;
    if (!taping) return leaveTo(id);
    actionSheet({ title: 'Leave the set?', message: `${clockMs(ms)} on tape.`, options: [
      { label: 'Discard the recording', destructive: true, onPress: discard },
      { label: 'Save the set', onPress: () => void save() },
      { label: 'Keep going', cancel: true },
    ] });
  };
  const next = RR.nextControl(run.i, steps.length);
  const back = RR.backControl(run.i);
  const status = statusWord(rec, intr, false, starting);
  const right = rec === 'recording' && !intr
    ? <TextBtn testID="run-pause" label="Pause" onPress={pause} />
    : <View style={{ width: 66 }} />;

  return (
    <Screen title="Record" task deep testID="run"
      header={<TopBar task title={B?.board.title || ''} left={<TextBtn testID="run-close" label="Close" onPress={close} />} right={right} />}
      dock={
        <RecordDock
          left={<SideButton testID="run-back" label={back.label} disabled={back.disabled || !steps.length || rec === 'saving'} onPress={() => go(run.i - 1)} />}
          right={<SideButton testID="run-next" prominent label={next.label} disabled={next.disabled || rec === 'saving'} onPress={() => go(run.i + 1)} />}>
          <RecordButton testID="run-rec" state={rec} interrupted={intr} busy={starting} onStart={start} onStop={() => void save()} onResume={resume} />
        </RecordDock>
      }>
      {!B ? (b.rawError ? <Empty text={`Can’t open the set: ${b.rawError}`} /> : <Skeleton lines={5} height={28} />) : !steps.length ? (
        <Empty text="This set has no bits yet. Add a bit, then record it." />
      ) : (
        <View style={{ gap: 18 }}>
          {video && <T v="mut" testID="run-video-web">Filming a set is in the iPhone app. This records sound.</T>}
          <RecordClock testID="run" ms={ms} status={status} targetS={target} level={taping ? lvl : null} interrupted={intr} />
          <BitPane step={steps[run.i]} i={run.i} n={steps.length} view={view} onToggle={() => setView((v) => (v === 'notes' ? 'lines' : 'notes'))} />
          {rec === 'idle' && <T v="small" style={{ textAlign: 'center' }}>Tap the red button to record · Next marks where each bit ends</T>}
        </View>
      )}
    </Screen>
  );
}

// ------------------------------------------------------------------ video
function RunVideo({ id }: { id: string }) {
  useKeepAwake();
  const ui = useUI();
  const { b, B, steps, ids, bits } = useSet(id);
  const [view, setView] = useState<View_>('notes');
  const [cam, setCam] = useState<CameraModule | null | undefined>(undefined);
  const [vid, setVid] = useState<VideoModule | null | undefined>(undefined);
  const [access, setAccess] = useState<Film.Access | null>(null);
  const [free, setFree] = useState<number | null>(null);
  const [codec, setCodec] = useState<'hvc1' | undefined>(undefined);
  const [camReady, setCamReady] = useState(false);
  const [mountErr, setMountErr] = useState<string | null>(null);
  const [camKey, setCamKey] = useState(0);
  const [state, setState] = useState<FilmState>('idle');
  const stateRef = useRef<FilmState>('idle');
  useEffect(() => { stateRef.current = state; }, [state]);
  const [count, setCount] = useState(FR.COUNTDOWN_S);
  const [ms, setMs] = useState(0);
  const [run, setRun] = useState<RR.Run>(RR.runIdle());
  const runRef = useRef(run);
  useEffect(() => { runRef.current = run; }, [run]);
  const [splits, setSplits] = useState<RR.Split[]>([]);
  const [take, setTake] = useState<Take | null>(null);
  const [stopping, setStopping] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [photosOff, setPhotosOff] = useState(false);
  const [player, setPlayer] = useState<any>(null);
  const camRef = useRef<InstanceType<CameraModule['CameraView']> | null>(null);
  const startedAt = useRef(0);
  const weStopped = useRef<false | 'square' | 'bg'>(false);
  const after = useRef<'keep' | 'discard' | null>(null);
  // a filmed set needs room for a long take (RR.SET_MIN_FREE_BYTES), not the one-bit take's floor (iOS dev review)
  const reason = useMemo(() => (cam === null || mountErr ? 'no camera' : FR.discReason({ camera: cam === undefined || !access || access.camera, mic: !access || access.mic, freeBytes: RR.setRoom(free) })), [cam, mountErr, access, free]);
  const synced = useRef(false);
  const recording = state === 'recording';
  const reviewing = state === 'review' || state === 'saving';
  const pos = () => Math.max(0, Date.now() - startedAt.current);

  const keep = async (t: Take | null = take, sp: RR.Split[] = splits) => {
    if (!t || FR.nextFilmState(stateRef.current, 'keep') !== 'saving') return;
    stateRef.current = 'saving';
    setState('saving');
    setErr(null);
    try {
      const row = await Film.keepSetTake(t, { board: { id, title: B?.board.title || 'Set' }, runBits: bits, splits: sp });
      haptics.success();
      setState(FR.nextFilmState('saving', 'kept'));
      router.replace({ pathname: '/after/[rec]', params: { rec: row.id } } as any);
    } catch (e: any) {
      setState(FR.nextFilmState('saving', 'failed'));
      if (String(e?.message) === 'no Photos') setPhotosOff(true);
      else setErr(`Couldn’t save to Photos: ${e?.message || e}. The take is still here.`);
    }
  };
  const discard = async () => {
    const t = take;
    setTake(null);
    setState('idle');
    await Film.discardTake(t);
    ui.snack('Take discarded');
    leaveTo(id);
  };
  const onEnded = async (res: { uri: string } | undefined) => {
    if (stateRef.current !== 'recording') return;
    const durationMs = pos();
    const sp = RR.runEnd(runRef.current, durationMs, ids);
    setSplits(sp);
    const cutShort = FR.cutShortOf(weStopped.current !== false, weStopped.current === 'bg' || AppState.currentState !== 'active', durationMs, RR.SET_TAKE_S);
    setStopping(false);
    if (!res?.uri) { setErr('Couldn’t record. Try again.'); setState('idle'); setMs(0); return; }
    try {
      const t = await Film.stashTake(res.uri, durationMs, cutShort);
      setTake(t);
      setMs(durationMs);
      const nx = FR.nextFilmState('recording', weStopped.current === 'square' ? 'stop' : 'ended');
      stateRef.current = nx;
      setState(nx);
      const then = after.current;
      after.current = null;
      if (then === 'discard') { await Film.discardTake(t); setTake(null); ui.snack('Take discarded'); leaveTo(id); }
      else if (then === 'keep') void keep(t, sp);
    } catch (e: any) {
      setErr(`Couldn’t keep the file: ${e?.message || e}`);
      setState('idle');
      setMs(0);
    }
  };
  const goRec = () => {
    if (stateRef.current !== 'countdown' || !camRef.current) return;
    startedAt.current = Date.now();
    weStopped.current = false;
    setMs(0);
    // the start time is read before the camera writes its first frame: video splits sit early by its start-up (§3.4)
    setRun((r) => RR.runStart(RR.runIdle(r.i), 0));
    stateRef.current = 'recording';
    setState('recording');
    haptics.medium();   // the first frame; nothing hums after this until the take is over
    synced.current = false;
    camRef.current.recordAsync({ codec, maxDuration: RR.SET_TAKE_S, ...(RR.setFileCap(free) ? { maxFileSize: RR.setFileCap(free)! } : {}) }).then(onEnded).catch((e: any) => {
      setStopping(false);
      setErr(`Couldn’t record: ${e?.message || e}`);
      if (stateRef.current === 'recording') { stateRef.current = 'idle'; setState('idle'); }
      setMs(0);
    });
  };
  const arm = () => {
    if (stateRef.current !== 'idle' || reason || !camReady || !steps.length) return;
    setErr(null);
    setCount(FR.COUNTDOWN_S);
    stateRef.current = 'countdown';
    setState('countdown');
  };
  const stop = () => {
    if (stateRef.current !== 'recording' || weStopped.current) return;
    weStopped.current = 'square';
    setStopping(true);
    try { camRef.current?.stopRecording(); } catch (e: any) { setStopping(false); setErr(`Couldn’t stop: ${e?.message || e}`); }
  };
  const go = (to: number) => {
    const live = stateRef.current === 'recording';
    if (!live) haptics.selection();
    setRun((r) => RR.runGo(r, to, live ? pos() : 0, ids));
  };
  const redo = async () => {
    if (stateRef.current !== 'review') return;
    const t = take;
    setTake(null);
    setPhotosOff(false);
    setErr(null);
    setSplits([]);
    setRun((r) => RR.runIdle(r.i));
    stateRef.current = 'idle';
    setState('idle');
    setMs(0);
    await Film.discardTake(t);
    if (!reason) { setCount(FR.COUNTDOWN_S); setState((s) => FR.nextFilmState(s, 'arm')); }
  };
  const remount = () => { setMountErr(null); setCamReady(false); setCamKey((k) => k + 1); };
  const close = () => {
    const s = stateRef.current;
    if (s === 'saving') return;
    if (s === 'countdown') { stateRef.current = 'idle'; setState('idle'); return; }
    if (s === 'idle' || s === 'done') return leaveTo(id);
    actionSheet({ title: 'Leave the take?', message: FR.leaveMessage(ms), options: [
      { label: 'Discard the take', destructive: true, onPress: () => { if (stateRef.current === 'recording') { after.current = 'discard'; stop(); } else void discard(); } },
      { label: 'Keep the take', onPress: () => { if (stateRef.current === 'recording') { after.current = 'keep'; stop(); } else void keep(); } },
      { label: 'Keep going', cancel: true },
    ] });
  };

  useEffect(() => {
    setRecording(true);
    const c = camRef;
    return () => {
      setRecording(false);
      try { c.current?.stopRecording(); } catch {}
    };
  }, []);
  useEffect(() => {
    let alive = true;
    void Film.loadCamera().then(async (m) => {
      if (!alive) return;
      setCam(m);
      if (!m) return;
      const a = await Film.requestCameraAccess(m);
      if (!alive) return;
      setAccess(a);
      const [f, c] = await Promise.all([Film.freeBytes(), a.camera ? Film.takeCodec(m) : Promise.resolve(undefined)]);
      if (!alive) return;
      setFree(f);
      setCodec(c);
    });
    void Film.loadVideo().then((m) => alive && setVid(m));
    return () => { alive = false; };
  }, []);
  useEffect(() => {
    if (!recording) return;
    const t = setInterval(() => setMs(pos()), TICK_MS);
    return () => clearInterval(t);
  }, [recording]);
  useEffect(() => {
    if (state !== 'countdown') return;
    let n = FR.COUNTDOWN_S;
    AccessibilityInfo.announceForAccessibility?.(String(n));
    const t = setInterval(() => {
      n -= 1;
      if (n <= 0) { clearInterval(t); goRec(); return; }
      haptics.selection();
      setCount(n);
      AccessibilityInfo.announceForAccessibility?.(String(n));
    }, 1000);
    return () => clearInterval(t);
  }, [state]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    const sub = AppState.addEventListener('change', (s) => {
      if (s === 'background') {
        if (stateRef.current === 'recording' && !weStopped.current) { weStopped.current = 'bg'; try { camRef.current?.stopRecording(); } catch {} }
        else if (stateRef.current === 'countdown') { stateRef.current = 'idle'; setState('idle'); }
      } else if (s === 'active' && stateRef.current === 'idle') void Film.freeBytes().then(setFree);
    });
    return () => sub.remove();
  }, []);
  useEffect(() => {
    if (!reviewing || !vid || !take) return;
    let made: any = null;
    let gone = false;
    const t = setTimeout(() => {
      void Film.playbackMode().then(() => {
        if (gone) return;
        try { made = vid.createVideoPlayer(take.uri); made.loop = true; made.play(); setPlayer(made); } catch { setPlayer(null); }
      });
    }, 0);
    return () => { gone = true; clearTimeout(t); setPlayer(null); try { made?.release?.(); } catch {} };
  }, [reviewing, vid, take]);

  const camOff = cam === null || (access ? !access.camera : false);
  const opening = state === 'idle' && !camOff && !mountErr && (cam === undefined || !access || !camReady);
  const status = state === 'countdown' ? FR.readyStatus(reason, true)
    : recording ? (stopping ? 'Saving…' : 'Recording')
    : state === 'saving' ? 'Saving…'
    : reviewing ? FR.reviewStatus(ms, take?.cutShort)
    : opening ? 'Opening…'
    : FR.readyStatus(reason);
  const CameraView = cam?.CameraView;
  const VideoView = vid?.VideoView;
  const next = RR.nextControl(run.i, steps.length);
  const back = RR.backControl(run.i);

  return (
    <Screen title="Film" task deep scroll={false} testID="run" offlineAt="none"
      header={<TopBar task title={B?.board.title || 'Film'} left={<TextBtn testID="run-close" label="Close" onPress={close} />} right={<View style={{ width: 66 }} />} />}
      dock={reviewing ? (
        <RecordDock
          left={<SideButton testID="run-redo" label="Redo" disabled={state === 'saving'} onPress={() => void redo()} />}
          right={<SideButton testID="run-keep" prominent label="Keep" disabled={state === 'saving' || photosOff} accessibilityLabel={photosOff ? 'Keep, no Photos' : state === 'saving' ? 'Keep, saving' : 'Keep'} onPress={() => void keep()} />}>
          <View style={{ width: 92, height: 92 }} />
        </RecordDock>
      ) : (
        <RecordDock
          left={<SideButton testID="run-back" label={back.label} disabled={back.disabled || stopping || !steps.length} onPress={() => go(run.i - 1)} />}
          right={<SideButton testID="run-next" prominent={recording} label={next.label} disabled={next.disabled || stopping} onPress={() => go(run.i + 1)} />}>
          <RecordButton testID="run-rec" state={recording ? (stopping ? 'saving' : 'recording') : 'idle'} busy={state === 'countdown' || (opening && !reason)} reason={reason}
            onStart={arm} onStop={stop} onResume={() => {}} stopLabel="Stop and review the take" />
        </RecordDock>
      )}>
      <StatusBar hidden={state !== 'done'} />
      {/* the camera on top (nearest the lens), the bit under it; the preview 9:16 at a third of the screen */}
      <View style={{ alignItems: 'center', paddingTop: 8 }}>
        <View testID="run-preview" style={{ height: 260, aspectRatio: 9 / 16, borderRadius: 14, overflow: 'hidden', backgroundColor: C.surface2, borderWidth: 1, borderColor: recording ? C.rec : C.border }}>
          {CameraView && !!access?.camera && (
            <CameraView key={camKey} ref={camRef} style={{ flex: 1 }} facing="front" mode="video" mute={!access.mic} mirror={false} videoQuality={FR.VIDEO_QUALITY} videoStabilizationMode="auto"
              active={!reviewing} responsiveOrientationWhenOrientationLocked={false}
              onCameraReady={() => setCamReady(true)} onMountError={(e: { message: string }) => setMountErr(e?.message || 'camera')}
              // the file's own clock (§3.4): the first progress report moves the start to when the file began, so the
              // splits do not sit early by the camera's start-up (iOS dev review)
              onRecordingProgress={(e: any) => {
                const d = Number(e?.duration ?? e?.nativeEvent?.duration);
                if (!synced.current && stateRef.current === 'recording' && d > 0) { synced.current = true; startedAt.current = Date.now() - d * 1000; }
              }} />
          )}
          {reviewing && VideoView && player && (
            <VideoView player={player} style={{ position: 'absolute', left: 0, top: 0, right: 0, bottom: 0 }} nativeControls contentFit="contain" />
          )}
          {state === 'countdown' && (
            <Pressable testID="run-countdown" accessibilityRole="button" accessibilityLabel={`${count}. Skip the countdown`} onPress={goRec}
              style={{ position: 'absolute', left: 0, top: 0, right: 0, bottom: 0, alignItems: 'center', justifyContent: 'center', backgroundColor: 'rgba(0,0,0,0.5)' }}>
              <Text maxFontSizeMultiplier={1.2} style={{ fontFamily: F.black, fontSize: 64, lineHeight: 70, color: '#fff', fontVariant: ['tabular-nums'] }}>{count}</Text>
            </Pressable>
          )}
          {(camOff || mountErr) && (
            <View testID="run-camera-off" style={{ position: 'absolute', left: 0, top: 0, right: 0, bottom: 0, alignItems: 'center', justifyContent: 'center', gap: 8, padding: 8 }}>
              <T style={{ textAlign: 'center' }}>{cam === null ? 'Not in this build yet.' : mountErr ? 'Couldn’t open the camera.' : 'Punchable can’t use the camera.'}</T>
              {cam !== null && (mountErr ? <Btn small testID="run-camera-retry" label="Try again" onPress={remount} />
                : <Btn small testID="run-open-settings" label="Open Settings" onPress={() => void Linking.openSettings()} />)}
            </View>
          )}
        </View>
        <View testID="run-clock" style={{ flexDirection: 'row', alignItems: 'center', gap: 6, paddingTop: 8, minHeight: 28 }}>
          {recording && !stopping && <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: DARK.rec }} />}
          <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 15, color: recording && !stopping ? C.rec : C.mut, fontVariant: ['tabular-nums'] }}>
            {reviewing ? status : `${clockMs(ms)} · ${status}`}
          </Text>
        </View>
        {photosOff && (
          <View testID="run-photos-off" style={{ flexDirection: 'row', alignItems: 'center', gap: 12 }}>
            <T v="mut">no Photos</T>
            <TextBtn testID="run-photos-settings" label="Open Settings" color={linkColor()} onPress={() => void Linking.openSettings()} />
          </View>
        )}
        {!!err && <T testID="run-error" style={{ color: C.rec, textAlign: 'center' }}>{err}</T>}
      </View>
      <ScrollView style={{ flex: 1 }} contentContainerStyle={{ padding: 16, gap: 12 }}>
        {!B ? (b.rawError ? <Empty text={`Can’t open the set: ${b.rawError}`} /> : <Skeleton lines={4} height={24} />) : !steps.length ? (
          <Empty text="This set has no bits yet. Add a bit, then film it." />
        ) : (
          <BitPane step={steps[run.i]} i={run.i} n={steps.length} view={view} onToggle={() => setView((v) => (v === 'notes' ? 'lines' : 'notes'))} />
        )}
      </ScrollView>
    </Screen>
  );
}
