// On stage (MixStage, docs/UX_2026-10.md): the big clock, the bar with the light, the short names in marker text with
// the current one lit, Next bit (tap anywhere), and the recording: the web recorder or, on the phone, expo-audio
// (lib/recorder; background audio, locked screen). The bottom Stop asks "Save the set / Discard / Keep going"
// (HIG: a confirmation before an upload you can't take back). Saving hands the tape to the upload queue (retried
// on the backoff schedule if the venue has no signal). The screen stays awake.
import React, { useEffect, useRef, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { useKeepAwake } from 'expo-keep-awake';
import { useApi } from '@/lib/api';
import { haptics } from '@/lib/haptics';
import { isWeb, micExplained } from '@/lib/media';
import { useRecorder } from '@/lib/recorder';
import { enqueueUpload } from '@/lib/uploads';
import { Build } from '@/lib/simple';
import { shortNameOf } from '@/lib/say';
import { C, F, fmtMs } from '@/lib/theme';
import { Btn, Empty, Sheet, SheetHead, Skeleton, T, useUI } from '@/lib/ui';
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
  const [rec, setRec] = useState<'off' | 'on' | 'saving' | 'none'>('none');
  const [ask, setAsk] = useState(false);
  const recorder = useRecorder();
  const started = useRef(false);
  const t0 = useRef(Date.now());
  const B = b.data;
  const target = B?.board.target_s || 0;
  const light = Math.max(0, target - 60); // the light comes on a minute before time

  useEffect(() => {
    const t = setInterval(() => setMs(rec === 'on' || rec === 'saving' ? recorder.elapsedMs() : Date.now() - t0.current), 500);
    return () => clearInterval(t);
  }, [rec, recorder]);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    if (!recorder.supported || (isWeb && !micExplained())) { setRec('none'); return; }
    recorder.start().then(() => setRec('on')).catch(() => setRec('none'));
    return () => { if (recorder.state() === 'recording' || recorder.state() === 'paused') recorder.cancel(); };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const steps = B?.steps || [];
  const next = () => { haptics.selection(); setCur((c) => Math.min(steps.length - 1, c + 1)); };
  const back = () => (router.canGoBack() ? router.back() : router.navigate(`/list/${encodeURIComponent(id)}` as any));
  const stop = () => {
    if (rec !== 'on') return back();
    haptics.warning();
    setAsk(true);
  };
  const discard = () => {
    setAsk(false);
    recorder.cancel();
    setRec('off');
    ui.snack('Recording discarded');
    back();
  };
  const save = async () => {
    setAsk(false);
    setRec('saving');
    try {
      const r = await recorder.stop();
      const res = await enqueueUpload(r, { kind: 'set', filename: r.name, title: B?.board.title || 'Set', durationS: r.durationMs / 1000, recordedAt: new Date(t0.current).toISOString(), showGroup: id });
      haptics.success();
      ui.snack(res.queued ? `Set saved on this phone (${fmtMs(r.durationMs)}) · it goes up when there’s signal` : `Set saved (${fmtMs(r.durationMs)}) · it gets reviewed after`);
      back();
    } catch (e: any) {
      setRec('off');
      ui.snack(`Couldn’t save the set: ${e.message}`);
    }
  };
  const pct = target ? Math.min(100, (ms / (target * 1000)) * 100) : 0;
  const lit = ms >= light * 1000 && target > 0;
  const status = rec === 'on' ? '● REC' : rec === 'saving' ? 'Saving…' : rec === 'off' ? 'Not recording' : 'Clock only';

  return (
    <Screen title="On stage" task deep testID="stage"
      header={<TopBar task title={status} left={<View style={{ width: 44 }} />} right={<View style={{ width: 44 }} />} />}
      dock={
        <View style={{ flexDirection: 'row', gap: 8 }}>
          <Btn testID="stage-end" label={rec === 'on' ? 'Stop' : 'Done'} v={rec === 'on' ? 'danger' : 'plain'} style={{ flex: 1, minHeight: 60 }} onPress={stop} accessibilityLabel={rec === 'on' ? 'Stop recording' : 'Leave the stage'} />
          <Btn testID="stage-next" label={cur >= steps.length - 1 ? 'Last bit' : 'Next bit'} v="primary" style={{ flex: 2, minHeight: 60 }} onPress={next} disabled={!steps.length || cur >= steps.length - 1} />
        </View>
      }>
      {!B ? (b.rawError ? <Empty text={`Can’t open the set list: ${b.rawError}`} /> : <Skeleton lines={5} height={28} />) : (
        <Pressable accessibilityRole="button" accessibilityLabel="Next bit" onPress={next} style={{ gap: 18 }}>
          <View style={{ alignItems: 'center', gap: 2 }}>
            <Text testID="stage-clock" maxFontSizeMultiplier={1.2} style={{ fontFamily: F.black, fontSize: 84, lineHeight: 90, letterSpacing: -2, color: lit ? C.gapText : C.text, fontVariant: ['tabular-nums'] }}>{fmtMs(ms)}</Text>
            {!!target && <T v="mut">of {fmtMs(target * 1000)} · light at {fmtMs(light * 1000)}</T>}
          </View>
          <View style={{ height: 8, borderRadius: 4, backgroundColor: C.card, overflow: 'hidden' }}>
            <View style={{ width: `${pct}%`, height: '100%', backgroundColor: lit ? C.rec : C.amber }} />
            {!!target && <View style={{ position: 'absolute', left: `${(light / target) * 100}%`, top: 0, bottom: 0, width: 2, backgroundColor: C.text }} />}
          </View>
          <View style={{ gap: 6, paddingTop: 10 }}>
            {steps.map((x, i) => (
              <Text key={x.joke_id} testID={`stage-row-${i}`} numberOfLines={1} maxFontSizeMultiplier={1.2}
                style={{ fontFamily: F.black, fontSize: i === cur ? 40 : 30, lineHeight: i === cur ? 46 : 36, letterSpacing: -0.8, color: i === cur ? C.amber : i < cur ? C.tabOff : C.text, textDecorationLine: i < cur ? 'line-through' : 'none' }}>
                {x.short_name || shortNameOf(x.title)}
              </Text>
            ))}
          </View>
          <T v="small" style={{ textAlign: 'center' }}>Tap anywhere for next{rec === 'on' ? ' · the recording gets reviewed after' : rec === 'none' ? ' · no recording on this device' : ''}</T>
        </Pressable>
      )}
      <Sheet visible={ask} onClose={() => setAsk(false)} label="Stop recording" testID="stage-stop-sheet">
        <SheetHead title="Stop recording?" onCancel={() => setAsk(false)} cancelLabel="Keep going" testID="stage-stop" />
        <T v="mut" style={{ marginBottom: 12 }}>{fmtMs(ms)} on tape. Save it and it gets reviewed after the show.</T>
        <View style={{ gap: 8 }}>
          <Btn testID="stage-save" v="primary" label="Save the set" onPress={save} />
          <Btn testID="stage-discard" v="danger" label="Discard the recording" onPress={discard} />
        </View>
      </Sheet>
    </Screen>
  );
}
