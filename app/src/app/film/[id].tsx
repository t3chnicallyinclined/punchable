// Film it (docs/design/SPEC-2026-11-03-film-it.md §2.2, built on the spike's v1 path, SPIKE-2026-11-03 §7.1): the
// front camera full-screen with his lines in a band directly under the Dynamic Island (nearest the lens), the current
// line at the top, advancing at his pace (rules/prompterRules; Next line and Hold on the dock, tap and long-press on
// the band); the record control from Record (nothing records until the red disc; 3 · 2 · 1 with a tick he feels, then
// the take; the square stops and goes to the REVIEW, where the take plays in the system player; Keep saves the one copy
// to Photos and queues the words; Redo discards; Discard is behind Close). The take is 1080p portrait HEVC with sound,
// the preview mirrored, the file not. A call or the background ends the take into the review ("cut short by a call").
// No haptics while recording (HIG/playing-haptics). The screen stays awake; lib/audioBusy says "recording" for its
// whole life so the transcription queue waits. The state machine is rules/filmRules (tested). Web: one line.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { AccessibilityInfo, AppState, Linking, Platform, Pressable, Text, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { useKeepAwake } from 'expo-keep-awake';
import { useApi } from '@/lib/api';
import { haptics } from '@/lib/haptics';
import { actionSheet } from '@/lib/actionSheet';
import { setRecording } from '@/lib/audioBusy';
import { RecordButton, RecordDock, SideButton } from '@/lib/RecordControl';
import { PromptBand } from '@/lib/Prompter';
import * as Film from '@/lib/film';
import type { CameraModule, Take, VideoModule } from '@/lib/film';
import { shortNameOf } from '@/lib/say';
import { C, DARK, F, linkColor } from '@/lib/theme';
import { Btn, Card, T, TextBtn, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TopBar } from '@/lib/TopBar';
import { cleanPunch, cutChoices, cutWord, filmIndex, linesOf, versionsOf } from '@/rules/bitLines';
import { clockMs } from '@/rules/format';
import * as FR from '@/rules/filmRules';
import type { FilmState } from '@/rules/filmRules';
import * as PR from '@/rules/prompterRules';

const enc = encodeURIComponent;
const native = Platform.OS !== 'web';
/** The one clock's tick (the Stage's 250 ms); the prompter advances on the same tick. */
const TICK_MS = 250;

export default function FilmScreen() {
  useKeepAwake();
  const p = useLocalSearchParams<{ id: string; cut?: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const d = useApi<any>(`/api/jokes/${enc(id)}`);          // cached: offline the page he came from already holds it
  const J = d.data?.joke;
  const shortName = shortNameOf(J?.title || '', J?.short_name);
  // the cut being filmed (Film it D10): the one he picked, else the newest rant cut, else the stage cut
  const versions = useMemo(() => versionsOf(d.data), [d.data]);
  const [cutKey, setCutKey] = useState<string | null>(p.cut ? String(p.cut) : null);
  const v = versions[filmIndex(versions, cutKey)];
  const choices = useMemo(() => cutChoices(versions), [versions]);
  const lines = useMemo(() => linesOf(v?.text), [v?.text]);
  const punch = useMemo(() => cleanPunch(v?.punch, lines.length), [v?.punch, lines.length]);
  const wpm = useMemo(() => PR.paceWpm(lines, J?.duration_s), [lines, J?.duration_s]);

  // the native modules, lazily (a binary without them sees "not in this build", never a crash at launch)
  const [cam, setCam] = useState<CameraModule | null | undefined>(undefined);
  const [vid, setVid] = useState<VideoModule | null | undefined>(undefined);
  const [access, setAccess] = useState<Film.Access | null>(null);
  const [free, setFree] = useState<number | null>(null);
  const [codec, setCodec] = useState<'hvc1' | undefined>(undefined);
  const [camReady, setCamReady] = useState(false);
  const [mountErr, setMountErr] = useState<string | null>(null);
  const [camKey, setCamKey] = useState(0);                   // Try again after a mount error remounts the view

  const [state, setState] = useState<FilmState>('idle');
  const stateRef = useRef<FilmState>('idle');                 // the live state for callbacks that land after a render
  useEffect(() => { stateRef.current = state; }, [state]);
  const [count, setCount] = useState(FR.COUNTDOWN_S);
  const [ms, setMs] = useState(0);
  const [prompter, setPrompter] = useState(PR.prompterIdle());
  const [take, setTake] = useState<Take | null>(null);
  const [stopping, setStopping] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [photosOff, setPhotosOff] = useState(false);
  const [player, setPlayer] = useState<any>(null);
  const camRef = useRef<InstanceType<CameraModule['CameraView']> | null>(null);
  const startedAt = useRef(0);
  const weStopped = useRef<false | 'square' | 'bg'>(false);
  const after = useRef<'keep' | 'discard' | null>(null);  // Close while recording: what to do once the file is back

  // the disc's one word: no module or a mount error reads "no camera"; a denied mic keeps the preview (the view runs
  // muted, so the session never asks for an input it cannot have) and greys the disc
  const reason = useMemo(() => (cam === null || mountErr ? 'no camera' : FR.discReason({ camera: cam === undefined || !access || access.camera, mic: !access || access.mic, freeBytes: free })), [cam, mountErr, access, free]);
  const recording = state === 'recording';
  const reviewing = state === 'review' || state === 'saving';

  const leave = () => (router.canGoBack() ? router.back() : router.navigate(`/joke/${enc(id)}` as any));
  const keep = async (t: Take | null = take) => {
    if (!t || FR.nextFilmState(stateRef.current, 'keep') !== 'saving') return;   // a second Keep while saving is a no-op
    stateRef.current = 'saving';
    setState('saving');
    setErr(null);
    try {
      await Film.keepTake(t, { bit: { id, title: shortName || 'this bit' }, script: lines.join('\n') });
      haptics.success();
      setState(FR.nextFilmState('saving', 'kept'));
      ui.snack(`Saved to Photos · ${clockMs(t.durationMs)}`);
      leave();
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
    leave();
  };
  /** recordAsync resolved: the square, maxDuration, a call (the session was taken) or the background (our stop). */
  const onEnded = async (res: { uri: string } | undefined) => {
    if (stateRef.current !== 'recording') return;
    const durationMs = Math.max(0, Date.now() - startedAt.current);
    // the lock's own stop can land before ours: a file that came back while the app is not in front is the background
    const cutShort = FR.cutShortOf(weStopped.current !== false, weStopped.current === 'bg' || AppState.currentState !== 'active', durationMs, FR.MAX_TAKE_S);
    setPrompter((q) => PR.stopPrompter(q));
    setStopping(false);
    if (!res?.uri) {
      setErr('Couldn’t record. Try again.');
      setState('idle');
      setMs(0);
      return;
    }
    try {
      const t = await Film.stashTake(res.uri, durationMs, cutShort);
      setTake(t);
      setMs(durationMs);
      const next = FR.nextFilmState('recording', weStopped.current === 'square' ? 'stop' : 'ended');
      stateRef.current = next;
      setState(next);
      const then = after.current;
      after.current = null;
      if (then === 'discard') { await Film.discardTake(t); setTake(null); ui.snack('Take discarded'); leave(); }
      else if (then === 'keep') void keep(t);
    } catch (e: any) {
      setErr(`Couldn’t keep the file: ${e?.message || e}`);
      setState('idle');
      setMs(0);
    }
  };
  const go = () => {
    if (stateRef.current !== 'countdown' || !camRef.current) return;
    const now = Date.now();
    startedAt.current = now;
    weStopped.current = false;
    setMs(0);
    stateRef.current = 'recording';
    setState('recording');
    setPrompter((q) => PR.startPrompter(q, now, lines, wpm));
    haptics.medium();   // the first frame; nothing hums after this until the take is over
    camRef.current.recordAsync({ codec, maxDuration: FR.MAX_TAKE_S }).then(onEnded).catch((e: any) => {
      setStopping(false);
      setErr(`Couldn’t record: ${e?.message || e}`);
      setPrompter((q) => PR.stopPrompter(q));
      if (stateRef.current === 'recording') { stateRef.current = 'idle'; setState('idle'); }
      setMs(0);
    });
  };
  const arm = () => {
    if (stateRef.current !== 'idle' || reason || !camReady) return;
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
  /** A mount error's Try again: the view is remounted (a fresh session), the error cleared. */
  const remount = () => { setMountErr(null); setCamReady(false); setCamKey((k) => k + 1); };
  const next = () => setPrompter((q) => PR.nextLine(q, Date.now(), lines, wpm));
  const toggleHold = () => setPrompter((q) => PR.toggleHold(q, Date.now(), lines, wpm));
  const redo = async () => {
    if (stateRef.current !== 'review') return;
    const t = take;
    setTake(null);
    setPhotosOff(false);
    setErr(null);
    stateRef.current = 'idle';
    setState('idle');
    setPrompter(PR.prompterIdle());
    setMs(0);
    await Film.discardTake(t);
    // Redo returns to the countdown without a sheet (SPEC D7): the loss is expected
    if (!reason) { setCount(FR.COUNTDOWN_S); setState((s) => FR.nextFilmState(s, 'arm')); }
  };
  // Close: nothing lost while idle; during the count it just stops; with a take recorded and not kept, the sheet
  // (HIG/modality: an action sheet with a save option)
  // the cut picker (§2.2, §3): the newest version of each cut, and "Make a rant cut…" when there is none, which closes
  // the camera and opens the sessions sheet on the bit page (one modal at a time). Only while idle: a take is of one cut.
  const pickCut = () => {
    if (stateRef.current !== 'idle') return;
    const stage = choices.find((c) => c.cut === 'stage');
    actionSheet({ title: shortName, options: [
      // the cut on the band carries a ✓ (the sheet stands in for a pop-up button: the current choice is shown)
      ...choices.map((c) => ({ label: `${c.key === v?.key ? '✓ ' : ''}${c.label}`, onPress: () => { setCutKey(c.key); setPrompter(PR.prompterIdle()); } })),
      // the typed-idea stand-in is no version the server holds, so a bit with no version has nothing to rework
      ...(!choices.some((c) => c.cut === 'rant') && stage && stage.key !== 'idea' ? [{
        label: 'Make a rant cut…',
        onPress: () => router.replace({ pathname: '/session/[bit]', params: { bit: id, unit: stage.key, cut: 'stage', n: String(versions.length), title: shortName, mode: 'rant' } } as any),
      }] : []),
      { label: 'Cancel', cancel: true },
    ] });
  };
  const cutBtn = v && state === 'idle'
    ? <TextBtn testID="film-cut" label={`${cutWord(v.cut)} ▾`} accessibilityLabel={`${v.cut === 'rant' ? 'Rant cut' : 'Stage cut'}, ${v.label}. Change`} color={linkColor()} onPress={pickCut} />
    : <View style={{ width: 66 }} />;

  const close = () => {
    const s = stateRef.current;
    if (s === 'saving') return;
    if (s === 'countdown') { stateRef.current = 'idle'; setState('idle'); return; }
    if (s === 'idle' || s === 'done') return leave();
    actionSheet({ title: 'Leave the take?', message: FR.leaveMessage(ms), options: [
      { label: 'Discard the take', destructive: true, onPress: () => { if (stateRef.current === 'recording') { after.current = 'discard'; stop(); } else void discard(); } },
      { label: 'Keep the take', onPress: () => { if (stateRef.current === 'recording') { after.current = 'keep'; stop(); } else void keep(); } },
      { label: 'Keep going', cancel: true },
    ] });
  };

  // the camera screen owns the microphone and the encoder: the transcription queue waits, no memo plays (lib/audioBusy)
  useEffect(() => {
    if (!native) return;
    setRecording(true);
    const cam = camRef;  // the ref object itself (stable); its current camera is read at cleanup, the one to stop
    return () => {
      setRecording(false);
      try { cam.current?.stopRecording(); } catch {}
    };
  }, []);
  // the module, then the grant (the view mounts only after it: a session started under the prompt fails its
  // permission check once and never recovers [review]), then the codec list (it needs the camera) and the free space
  useEffect(() => {
    if (!native) return;
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
  // the clock and the prompter: one tick, the Stage's 250 ms
  useEffect(() => {
    if (!recording) return;
    const t = setInterval(() => {
      const now = Date.now();
      setMs(now - startedAt.current);
      setPrompter((q) => PR.tickPrompter(q, now, lines, wpm));
    }, TICK_MS);
    return () => clearInterval(t);
  }, [recording, lines, wpm]);
  // the countdown: a selection tick a second, announced; tap to skip; Close cancels it
  useEffect(() => {
    if (state !== 'countdown') return;
    let n = FR.COUNTDOWN_S;
    AccessibilityInfo.announceForAccessibility?.(String(n));
    const t = setInterval(() => {
      n -= 1;
      if (n <= 0) { clearInterval(t); go(); return; }
      haptics.selection();
      setCount(n);
      AccessibilityInfo.announceForAccessibility?.(String(n));
    }, 1000);
    return () => clearInterval(t);
  }, [state]); // eslint-disable-line react-hooks/exhaustive-deps
  // the background ends the take (a camera stops in the background; SPIKE §1 "Background"); a count is cancelled;
  // back in front, the free space is read again (he may have freed some)
  useEffect(() => {
    const sub = AppState.addEventListener('change', (s) => {
      if (s === 'background') {
        if (stateRef.current === 'recording' && !weStopped.current) { weStopped.current = 'bg'; try { camRef.current?.stopRecording(); } catch {} }
        else if (stateRef.current === 'countdown') { stateRef.current = 'idle'; setState('idle'); }
      } else if (s === 'active' && stateRef.current === 'idle') void Film.freeBytes().then(setFree);
    });
    return () => sub.remove();
  }, []);
  // the review: the system player over the take (expo-video, nativeControls, contain, loop, sound on); the audio mode
  // becomes playback first (the speaker, not whatever the capture left), the player is made after the frame and
  // released on Redo, Keep and unmount
  useEffect(() => {
    if (!reviewing || !vid || !take) return;
    let made: any = null;
    let gone = false;
    const t = setTimeout(() => {
      void Film.playbackMode().then(() => {
        if (gone) return;
        try {
          made = vid.createVideoPlayer(take.uri);
          made.loop = true;
          made.play();
          setPlayer(made);
        } catch {
          setPlayer(null);
        }
      });
    }, 0);
    return () => {
      gone = true;
      clearTimeout(t);
      setPlayer(null);
      try { made?.release?.(); } catch {}
    };
  }, [reviewing, vid, take]);

  if (!native || !Film.filmSupported) {
    return (
      <Screen title="Film it" task deep testID="film" header={<TopBar task title="Film it" left={<TextBtn testID="film-close" label="Close" onPress={leave} />} right={<View style={{ width: 66 }} />} />}>
        <T v="mut" style={{ textAlign: 'center' }}>Film it is in the iPhone app.</T>
      </Screen>
    );
  }

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

  return (
    <Screen title="Film it" task deep scroll={false} testID="film" offlineAt="none"
      header={<TopBar task title="Film it" left={<TextBtn testID="film-close" label="Close" onPress={close} />} right={cutBtn} />}
      dock={
        reviewing ? (
          <RecordDock
            left={<SideButton testID="film-redo" label="Redo" disabled={state === 'saving'} onPress={() => void redo()} />}
            right={<SideButton testID="film-keep" prominent label="Keep" disabled={state === 'saving' || photosOff} accessibilityLabel={photosOff ? 'Keep, no Photos' : state === 'saving' ? 'Keep, saving' : 'Keep'} onPress={() => void keep()} />}>
            <View style={{ width: 92, height: 92 }} />
          </RecordDock>
        ) : (
          <RecordDock
            // the left slot holds Hold / Go only while a take runs (idle it is empty, SPEC §2.2: a greyed control would owe a reason)
            left={recording ? <SideButton testID="film-hold" label={PR.holdWord(prompter.held)} tone={prompter.held ? 'amber' : 'plain'} disabled={stopping} onPress={toggleHold} /> : <View style={{ flex: 1 }} />}
            right={<SideButton testID="film-next" prominent={recording} label="Next line" disabled={!lines.length || prompter.i >= lines.length - 1 || stopping} onPress={next} />}>
            <RecordButton testID="film-rec" state={recording ? (stopping ? 'saving' : 'recording') : 'idle'} busy={state === 'countdown' || (opening && !reason)} reason={reason}
              onStart={arm} onStop={stop} onResume={() => {}} stopLabel="Stop and review the take" />
          </RecordDock>
        )
      }>
      <StatusBar hidden={state !== 'done'} />
      <PromptBand testID="film-band" lines={lines} punch={punch} i={prompter.i} running={recording} held={prompter.held} onNext={next} onToggleHold={toggleHold} />
      {/* the preview: 9:16 inside what is left, a 1-pt record ring while taping (a shape and the word, never colour alone) */}
      <View style={{ flex: 1, alignItems: 'center', paddingHorizontal: 16, paddingVertical: 12 }}>
        <View testID="film-preview" style={{ height: '100%', aspectRatio: 9 / 16, maxWidth: '100%', borderRadius: 16, overflow: 'hidden', backgroundColor: C.surface2, borderWidth: 1, borderColor: recording ? C.rec : C.border }}>
          {/* mounted only after the camera grant; a denied mic runs it muted (no audio input asked for) and the disc says "no mic" */}
          {CameraView && !!access?.camera && (
            <CameraView key={camKey} ref={camRef} style={{ flex: 1 }} facing="front" mode="video" mute={!access.mic} mirror={false} videoQuality={FR.VIDEO_QUALITY} videoStabilizationMode="auto"
              active={!reviewing} responsiveOrientationWhenOrientationLocked={false}
              onCameraReady={() => setCamReady(true)} onMountError={(e: { message: string }) => setMountErr(e?.message || 'camera')} />
          )}
          {reviewing && VideoView && player && (
            <VideoView player={player} style={{ position: 'absolute', left: 0, top: 0, right: 0, bottom: 0 }} nativeControls contentFit="contain" />
          )}
          {/* the clock and the status word, inside the preview's top-left (HIG/feedback: near what it describes); the pill
              sits over video, which has no scheme, so its colours are the dark set in both appearances [review] */}
          <View testID="film-clock" pointerEvents="none" style={{ position: 'absolute', left: 10, top: 10, flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 10, paddingVertical: 6, borderRadius: 9999, backgroundColor: DARK.scrim }}>
            {recording && !stopping && <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: DARK.rec }} />}
            <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 15, color: recording && !stopping ? DARK.rec : '#fff', fontVariant: ['tabular-nums'] }}>
              {reviewing ? status : `${clockMs(ms)} · ${status}`}
            </Text>
          </View>
          {state === 'countdown' && (
            <Pressable testID="film-countdown" accessibilityRole="button" accessibilityLabel={`${count}. Skip the countdown`} onPress={go}
              style={{ position: 'absolute', left: 0, top: 0, right: 0, bottom: 0, alignItems: 'center', justifyContent: 'center', backgroundColor: 'rgba(0,0,0,0.5)' }}>
              <Text maxFontSizeMultiplier={1.2} style={{ fontFamily: F.black, fontSize: 84, lineHeight: 90, letterSpacing: -2, color: '#fff', fontVariant: ['tabular-nums'] }}>{count}</Text>
            </Pressable>
          )}
          {(camOff || mountErr) && (
            <View testID="film-camera-off" style={{ position: 'absolute', left: 0, top: 0, right: 0, bottom: 0, alignItems: 'center', justifyContent: 'center', gap: 12, padding: 16 }}>
              <T style={{ textAlign: 'center' }}>{cam === null ? 'Not in this build yet.' : mountErr ? 'Couldn’t open the camera.' : 'Punchable can’t use the camera.'}</T>
              {cam !== null && (mountErr
                ? <Btn testID="film-camera-retry" label="Try again" onPress={remount} />
                : <Btn testID="film-open-settings" label="Open Settings" onPress={() => void Linking.openSettings()} />)}
            </View>
          )}
        </View>
        {photosOff && (
          <View testID="film-photos-off" style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 12, paddingTop: 8 }}>
            <T v="mut">no Photos</T>
            <TextBtn testID="film-photos-settings" label="Open Settings" color={linkColor()} onPress={() => void Linking.openSettings()} />
          </View>
        )}
        {!!err && (
          <Card style={{ borderColor: C.dangerBorder, gap: 8, marginTop: 8, alignSelf: 'stretch' }} testID="film-error">
            <T>{err}</T>
            {reviewing && <Btn small label="Try again" onPress={() => void keep()} />}
          </Card>
        )}
      </View>
    </Screen>
  );
}
