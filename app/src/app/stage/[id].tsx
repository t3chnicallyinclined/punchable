// On stage (MixStage, docs/UX_2026-10.md; the record control per docs/design/HIG.md §3.9, 2026-10-29): the set list in
// marker text with the current bit lit, the big clock, the bar with the light, and the recorder (web MediaRecorder or,
// on the phone, expo-audio in the background with the screen locked). NOTHING RECORDS UNTIL HE TAPS THE RED DISC
// (lib/RecordControl): before that the clock reads 0:00 and Next bit runs the clock alone. While taping, the disc is a
// red square: tap = stop AND save (Voice Memos; stopping is not loss). Discard is behind Close (top-leading): an action
// sheet Discard / Save the set / Keep going. Saving hands the tape to the upload queue (retried if the venue has no
// signal). A phone call reads "Paused by a call" and the disc reads Resume. The screen stays awake.
import React, { useEffect, useRef, useState } from 'react';
import { Platform, Pressable, Text, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { useKeepAwake } from 'expo-keep-awake';
import { useApi } from '@/lib/api';
import { haptics } from '@/lib/haptics';
import { actionSheet } from '@/lib/actionSheet';
import { isWeb } from '@/lib/media';
import { useRecorder } from '@/lib/recorder';
import { RecordButton, RecordClock, RecordDock, SideButton } from '@/lib/RecordControl';
import { type RecordState, nextRecordState, statusWord } from '@/rules/recorderLogic';
import { enqueueUpload } from '@/lib/uploads';
import { Build } from '@/lib/simple';
import { clockMs } from '@/rules/format';
import { shortNameOf } from '@/lib/say';
import { C, F } from '@/lib/theme';
import { Btn, Empty, Sheet, SheetHead, Skeleton, T, TextBtn, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TopBar } from '@/lib/TopBar';

export default function Stage() {
  useKeepAwake();
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const b = useApi<Build>(`/api/boards/${encodeURIComponent(id)}/build`);
  const [cur, setCur] = useState(0);
  const [ms, setMs] = useState(0);
  const [rec, setRec] = useState<RecordState>('idle');
  const [clockOnly, setClockOnly] = useState<number | null>(null); // Next bit before the disc: the clock runs, nothing tapes
  const [ask, setAsk] = useState(false);                             // the web's Close sheet (the phone uses an action sheet)
  const [lvl, setLvl] = useState(0);
  const [intr, setIntr] = useState(false);
  const [starting, setStarting] = useState(false);                   // the disc was tapped; the microphone is opening
  const recorder = useRecorder();
  const native = Platform.OS !== 'web';
  const B = b.data;
  const target = B?.board.target_s || 0;
  const taping = rec === 'recording' || rec === 'paused' || rec === 'saving';
  const recRef = useRef(rec);                                        // the action sheet's Save reads the live state, not the one it was opened with
  recRef.current = rec;
  const saving = useRef(false);

  useEffect(() => {
    const t = setInterval(() => {
      if (taping) { setMs(recorder.elapsedMs()); setLvl(recorder.level()); setIntr(recorder.interrupted()); }
      else if (clockOnly != null) setMs(Date.now() - clockOnly);
    }, 250);
    return () => clearInterval(t);
  }, [taping, clockOnly, recorder]);
  useEffect(() => () => { if (recorder.state() === 'recording' || recorder.state() === 'paused') recorder.cancel(); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const steps = B?.steps || [];
  const next = () => {
    haptics.selection();
    if (!taping && clockOnly == null) setClockOnly(Date.now());
    setCur((c) => Math.min(steps.length - 1, c + 1));
  };
  const back = () => (router.canGoBack() ? router.back() : router.navigate(`/list/${encodeURIComponent(id)}` as any));
  const start = async () => {
    if (!recorder.supported) return ui.snack(isWeb ? 'This browser can’t record audio.' : 'No microphone available');
    if (starting || recRef.current !== 'idle') return;
    setStarting(true);
    try {
      await recorder.start();
      haptics.medium();
      setClockOnly(null);
      setRec(nextRecordState('idle', 'start'));
    } catch (e: any) {
      ui.snack(e?.message || 'Microphone not available');
    } finally {
      setStarting(false);
    }
  };
  const pause = () => { recorder.pause(); setRec(nextRecordState(rec, 'pause')); };
  const resume = () => { recorder.resume(); setIntr(recorder.interrupted()); setRec(nextRecordState(rec, 'resume')); };
  const save = async () => {
    setAsk(false);
    // one save per take: the square and the sheet's "Save the set" can both land, and a second stop() would hand back
    // a file the first one has already moved
    if (saving.current || nextRecordState(recRef.current, 'stop') !== 'saving') return;
    saving.current = true;
    setRec(nextRecordState(recRef.current, 'stop'));
    try {
      const r = await recorder.stop();
      const res = await enqueueUpload(r, { kind: 'set', filename: r.name, title: B?.board.title || 'Set', durationS: r.durationMs / 1000, recordedAt: new Date(Date.now() - r.durationMs).toISOString(), showGroup: id });
      haptics.success();
      ui.snack(res.queued ? `Saved · ${clockMs(r.durationMs)} · it goes up when there’s signal` : `Saved · ${clockMs(r.durationMs)}`);
      setRec(nextRecordState('saving', 'saved'));
      back();
    } catch (e: any) {
      // the native recorder has stopped either way (the file is still on this phone); "paused" would offer a Resume
      // that cannot resume, so the disc goes back to a fresh take
      setRec(nextRecordState('saving', 'failed'));
      setMs(0);
      ui.snack(`Couldn’t save the set: ${e?.message || e}. The tape is still on this phone.`);
    } finally {
      saving.current = false;
    }
  };
  const discard = () => {
    setAsk(false);
    recorder.cancel();
    setRec('idle');
    ui.snack('Recording discarded');
    back();
  };
  // Close while taping: the loss is the discard, so that is what the sheet is about (HIG/modality)
  const close = () => {
    if (!taping) return back();
    if (!native) return setAsk(true);
    actionSheet({ title: 'Leave the stage?', message: `${clockMs(ms)} on tape.`, options: [
      { label: 'Discard the recording', destructive: true, onPress: discard },
      { label: 'Save the set', onPress: save },
      { label: 'Keep going', cancel: true },
    ] });
  };
  const light = Math.max(0, target - 60);
  const lit = target > 0 && ms >= light * 1000;
  const status = statusWord(rec, intr, clockOnly != null, starting);

  return (
    <Screen title="Record" task deep testID="stage"
      header={<TopBar task title={B?.board.title || ''} left={<TextBtn testID="stage-close" label="Close" onPress={close} />} right={<View style={{ width: 66 }} />} />}
      dock={
        <RecordDock
          left={<SideButton testID="stage-pause" label={rec === 'paused' || (rec === 'recording' && intr) ? 'Paused' : 'Pause'} disabled={rec !== 'recording' || intr} onPress={pause} />}
          right={<SideButton testID="stage-next" prominent label={cur >= steps.length - 1 ? 'Last bit' : 'Next bit'} disabled={!steps.length || cur >= steps.length - 1} onPress={next} />}>
          <RecordButton testID="stage-rec" state={rec} interrupted={intr} busy={starting} onStart={start} onStop={save} onResume={resume} />
        </RecordDock>
      }>
      {native && <Stack.Screen options={{ title: 'Record' }} />}
      {!B ? (b.rawError ? <Empty text={`Can’t open the set list: ${b.rawError}`} /> : <Skeleton lines={5} height={28} />) : (
        <Pressable accessibilityRole="button" accessibilityLabel="Next bit" onPress={next} style={{ gap: 18 }}>
          <RecordClock testID="stage" ms={ms} status={status} targetS={target} level={taping ? lvl : null} interrupted={intr} />
          <View style={{ gap: 6, paddingTop: 4 }}>
            {steps.map((x, i) => (
              <Text key={x.joke_id} testID={`stage-row-${i}`} numberOfLines={1} maxFontSizeMultiplier={1.2}
                style={{ fontFamily: F.black, fontSize: i === cur ? 40 : 30, lineHeight: i === cur ? 46 : 36, letterSpacing: -0.8, color: i === cur ? C.amber : i < cur ? C.tabOff : C.text, textDecorationLine: i < cur ? 'line-through' : 'none' }}>
                {x.short_name || shortNameOf(x.title)}
              </Text>
            ))}
          </View>
          <T v="small" style={{ textAlign: 'center' }}>{rec === 'idle' ? 'Tap the red button to tape the set · tap anywhere for next' : rec === 'recording' ? 'Tap anywhere for next · the square stops and saves' : 'Tap anywhere for next'}{lit ? ' · the light is on' : ''}</T>
        </Pressable>
      )}
      <Sheet visible={ask} onClose={() => setAsk(false)} label="Leave the stage" testID="stage-stop-sheet">
        <SheetHead title="Leave the stage?" onCancel={() => setAsk(false)} cancelLabel="Keep going" testID="stage-stop" />
        <T v="mut" style={{ marginBottom: 12 }}>{clockMs(ms)} on tape.</T>
        <View style={{ gap: 8 }}>
          <Btn testID="stage-save" v="primary" label="Save the set" onPress={save} />
          <Btn testID="stage-discard" v="danger" label="Discard the recording" onPress={discard} />
        </View>
      </Sheet>
    </Screen>
  );
}
