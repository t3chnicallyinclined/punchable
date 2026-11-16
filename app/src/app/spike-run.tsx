// Device check 7 (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §4.4): can one screen record sound while he walks
// through a set, and what happens to that sound take when the camera is mounted on top of it? A full-screen modal
// like the stage. He starts a sound take with the shipped recorder (lib/recorder: expo-audio), taps Next through
// three invented bits (plain re-renders: the take is written natively), turns the camera on MUTED (expo-camera adds
// its audio input only when the view is not muted and in video mode), then UNMUTES it (the capture session now wants
// the microphone too: the real question), then stops. Nothing is ever filmed (no recordAsync) and the sound file is
// measured and deleted: it is not a tape, a memo or a set.
//
// What tells him the truth:
//   - the clock and the level, read twice a second. expo-audio's own "recording" flag is its state, not the
//     recorder's, so a take that died quietly would still say recording; a level stuck at 0 while he talks is the sign.
//   - after Stop, the file against the clock: 160 kbps is about 20 kB a second, so a file under 80% of that is short.
// Each step writes a line in the log with the level at that moment. No haptic fires while the microphone is live
// (HIG, Playing haptics). The microphone and camera indicators both show; the page says so. Phone only.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Platform, Text, View } from 'react-native';
import { router } from 'expo-router';
import { useRecorder } from '@/lib/recorder';
import * as Film from '@/lib/film';
import { mmss } from '@/rules/format';
import { C, F, linkColor } from '@/lib/theme';
import { TopBar } from '@/lib/TopBar';
import { Btn, T, TextBtn } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const BITS: [string, string[]][] = [
  ['Roommate', ['labels his food', 'the salt']],
  ['Smart fridge', ['wifi, door notification', 'standing in it', 'roommate tag']],
  ['Landlord text', ['ransom note', '“Come alone”']],
];
/** 160 kbps AAC (rules/recorderLogic SET_RECORDING) is about 20 kB for each second taped. */
const BYTES_PER_S = 20_000;
type Cam = 'off' | 'asking' | 'muted' | 'live' | 'denied' | 'missing';

export default function SpikeRun() {
  const recorder = useRecorder();
  const [taping, setTaping] = useState(false);
  const [starting, setStarting] = useState(false);
  const [bit, setBit] = useState(0);
  const [now, setNow] = useState({ ms: 0, level: 0, interrupted: false, state: 'idle' });
  const [peak, setPeak] = useState(0);
  const [cam, setCam] = useState<Cam>('off');
  const [camMod, setCamMod] = useState<Film.CameraModule | null>(null);
  const [verdict, setVerdict] = useState<string | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const note = useCallback((s: string) => setLog((l) => [`${new Date().toISOString().slice(11, 19)} ${s}`, ...l].slice(0, 18)), []);
  const level = () => Math.round(recorder.level() * 100);
  const stamp = (what: string) => note(`${what} · clock ${mmss(Math.round(recorder.elapsedMs() / 1000))} · level ${level()}${recorder.interrupted() ? ' · PAUSED BY THE SYSTEM' : ''}`);

  // twice a second: the clock, the level and whether the system paused the take
  useEffect(() => {
    if (!taping) return;
    const t = setInterval(() => {
      const lv = recorder.level();
      setNow({ ms: recorder.elapsedMs(), level: lv, interrupted: recorder.interrupted(), state: recorder.state() });
      setPeak((p) => Math.max(p, lv));
    }, 500);
    return () => clearInterval(t);
  }, [taping, recorder]);
  // leaving with a take running: it is thrown away
  const tapingRef = useRef(false);
  useEffect(() => { tapingRef.current = taping; }, [taping]);
  useEffect(() => () => { if (tapingRef.current) recorder.cancel(); }, [recorder]);

  const start = async () => {
    if (starting || taping) return;
    setStarting(true);
    setVerdict(null);
    try {
      await recorder.start();
      setPeak(0);
      setTaping(true);
      setBit(0);
      note('the sound take started');
    } catch (e: any) {
      note(`the sound take did not start: ${e?.message || 'unknown'}`);
    } finally {
      setStarting(false);
    }
  };
  const next = () => { setBit((b) => (b + 1) % BITS.length); stamp('Next'); };
  const cameraOn = async () => {
    if (cam !== 'off' && cam !== 'denied') return;
    setCam('asking');
    stamp('camera asked for');
    const mod = await Film.loadCamera();
    // he may have stopped or closed while the module loaded or the permission alert was up: no camera without a take
    if (!tapingRef.current) return setCam('off');
    if (!mod) { setCam('missing'); return note('this build has no camera module'); }
    const access = await Film.requestCameraAccess(mod);
    if (!tapingRef.current) return setCam('off');
    if (!access.camera) { setCam('denied'); return note('camera access is off (Settings › Punchable)'); }
    setCamMod(mod);
    setCam('muted');
    stamp('camera mounted, MUTED');
  };
  const unmute = () => { setCam('live'); stamp('camera UNMUTED (it now wants the microphone too)'); };
  const cameraOff = () => { setCam('off'); stamp('camera off'); };
  const stop = async () => {
    if (!taping) return;
    stamp('Stop');
    setTaping(false);
    setCam('off');
    try {
      const r = await recorder.stop();
      const FS: any = await import('expo-file-system/legacy');
      const info = r.uri ? await FS.getInfoAsync(r.uri) : { exists: false, size: 0 };
      const secs = r.durationMs / 1000;
      const expected = secs * BYTES_PER_S;
      const size = info.exists ? Number(info.size) || 0 : 0;
      const whole = info.exists && size >= expected * 0.8;
      const line = !info.exists ? 'NO FILE was written.'
        : `${mmss(Math.round(secs))} on the clock · the file is ${(size / 1e6).toFixed(2)} MB, about ${(expected / 1e6).toFixed(2)} MB expected: ${whole ? 'WHOLE' : 'SHORT (the take was cut)'} · loudest level ${Math.round(peak * 100)}`;
      setVerdict(line);
      note(line);
      // a test take: measured, then gone (it is in no list, and it is not his material)
      if (r.uri) await FS.deleteAsync(r.uri, { idempotent: true }).catch(() => {});
    } catch (e: any) {
      const line = `Stop failed: ${e?.message || 'unknown'}`;
      setVerdict(line);
      note(line);
    }
  };
  const close = () => router.back();

  const header = <TopBar title="Walk-through mock" left={<TextBtn testID="run-close" label="Close" color={linkColor()} onPress={close} />} />;
  if (Platform.OS === 'web') {
    return <Screen title="Walk-through mock" testID="spike-run" task header={header}><T v="mut">This check runs in the iPhone app.</T></Screen>;
  }
  const CameraView = camMod?.CameraView as any;
  const [name, notes] = BITS[bit];
  return (
    <Screen title="Walk-through mock" testID="spike-run" task header={header}>
      <T v="mut">Start, talk the whole time, and do the steps top to bottom. Nothing is filmed and the sound is deleted when you stop. The microphone and camera dots at the top of the phone will both show.</T>
      <View testID="run-status" accessible accessibilityLabel={`Clock ${mmss(Math.round(now.ms / 1000))}, level ${Math.round(now.level * 100)}`} style={{ flexDirection: 'row', alignItems: 'baseline', gap: 12 }}>
        <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.g7, fontSize: 34, color: C.text, fontVariant: ['tabular-nums'] }}>{mmss(Math.round(now.ms / 1000))}</Text>
        <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 15, color: C.mut2 }}>
          {taping ? `level ${Math.round(now.level * 100)} · ${now.interrupted ? 'PAUSED BY THE SYSTEM' : now.state}` : 'not recording'}
        </Text>
      </View>
      <View style={{ height: 8, borderRadius: 4, backgroundColor: C.card2, overflow: 'hidden' }}>
        <View style={{ height: 8, width: `${Math.round(Math.min(1, now.level) * 100)}%`, backgroundColor: taping ? C.red : C.dim }} />
      </View>

      <View testID="run-bit" style={{ padding: 12, borderRadius: 12, backgroundColor: C.card, gap: 4 }}>
        <T v="eb">Bit {bit + 1} of {BITS.length}</T>
        <T v="semi">{name}</T>
        {notes.map((n) => <T key={n} v="mut">· {n}</T>)}
      </View>

      {cam !== 'off' && cam !== 'asking' && cam !== 'denied' && cam !== 'missing' && !!CameraView && (
        <View testID="run-camera" style={{ height: 220, borderRadius: 12, overflow: 'hidden', backgroundColor: C.bgDeep }}>
          <CameraView style={{ flex: 1 }} facing="front" mode="video" mute={cam === 'muted'}
            onCameraReady={() => note(`camera ready (${cam === 'muted' ? 'muted' : 'unmuted'})`)}
            onMountError={(e: { message?: string }) => note(`camera error: ${e?.message || 'unknown'}`)} />
        </View>
      )}

      <View style={{ gap: 8 }}>
        <Btn testID="run-start" label={starting ? 'Starting…' : '1 · Start the sound take'} v="primary" disabled={taping || starting} onPress={() => void start()} />
        <Btn testID="run-next" label="2 · Next (tap it a few times)" disabled={!taping} onPress={next} />
        <Btn testID="run-camera-on" label="3 · Camera on, muted" disabled={!taping || (cam !== 'off' && cam !== 'denied')} onPress={() => void cameraOn()} />
        <Btn testID="run-unmute" label="4 · Unmute the camera" disabled={!taping || cam !== 'muted'} onPress={unmute} />
        <Btn testID="run-camera-off" label="5 · Camera off" disabled={!taping || (cam !== 'muted' && cam !== 'live')} onPress={cameraOff} />
        <Btn testID="run-stop" label="6 · Stop and check the file" disabled={!taping} onPress={() => void stop()} />
      </View>
      {!!verdict && <T v="semi" testID="run-verdict">{verdict}</T>}
      <T v="small">Pass: the clock keeps counting and the level keeps moving after every step, and the last line says WHOLE. If the level drops to 0 or the line says SHORT, say after which step. Log (newest first):</T>
      <View testID="run-log" style={{ gap: 2 }}>
        {log.map((s, i) => <Text key={i} numberOfLines={3} style={{ fontFamily: F.body, fontSize: 12, lineHeight: 16, color: C.mut2 }}>{s}</Text>)}
      </View>
    </Screen>
  );
}
