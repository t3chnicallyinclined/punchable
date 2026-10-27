// On stage (MixStage, docs/UX_2026-10.md): the big clock, the bar with the light, the short names in marker text with
// the current one lit, Next bit (tap anywhere), and the recording (the existing web recorder; the phone build shows
// the clock and the list, recording comes with expo-audio). End = stop, upload the tape as a set (processed later),
// back to the set list. The screen stays awake.
import React, { useEffect, useRef, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { useKeepAwake } from 'expo-keep-awake';
import { upload, useApi } from '@/lib/api';
import { Recorder, createRecorder, isWeb, micExplained, micSupported } from '@/lib/media';
import { Build } from '@/lib/simple';
import { shortNameOf } from '@/lib/say';
import { C, F, fmtMs } from '@/lib/theme';
import { Btn, Empty, Screen, T, TextBtn, TopBar, useUI } from '@/lib/ui';

export default function Stage() {
  useKeepAwake();
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const b = useApi<Build>(`/api/boards/${encodeURIComponent(id)}/build`);
  const [cur, setCur] = useState(0);
  const [ms, setMs] = useState(0);
  const [rec, setRec] = useState<'off' | 'on' | 'saving' | 'none'>('none');
  const recorder = useRef<Recorder | null>(null);
  const t0 = useRef(Date.now());
  const B = b.data;
  const target = B?.board.target_s || 0;
  const light = Math.max(0, target - 60); // the light comes on a minute before time

  useEffect(() => {
    const t = setInterval(() => setMs(recorder.current ? recorder.current.elapsedMs() : Date.now() - t0.current), 500);
    return () => clearInterval(t);
  }, []);
  useEffect(() => {
    if (!isWeb || !micSupported() || !micExplained()) { setRec('none'); return; }
    const r = createRecorder();
    recorder.current = r;
    r.start().then(() => setRec('on')).catch(() => { recorder.current = null; setRec('none'); });
    return () => { if (r.state() === 'recording' || r.state() === 'paused') r.cancel(); };
  }, []);

  const steps = B?.steps || [];
  const next = () => setCur((c) => Math.min(steps.length - 1, c + 1));
  const end = async () => {
    const r = recorder.current;
    const back = () => (router.canGoBack() ? router.back() : router.navigate(`/list/${encodeURIComponent(id)}` as any));
    if (!r || r.state() === 'idle') return back();
    setRec('saving');
    const dur = r.elapsedMs();
    const blob = await r.stop();
    try {
      const ext = blob.type.includes('mp4') ? 'm4a' : blob.type.includes('ogg') ? 'ogg' : 'webm';
      await upload(blob, { kind: 'set', filename: `set.${ext}`, title: B?.board.title || 'Set', durationS: dur / 1000, recordedAt: new Date(t0.current).toISOString(), showGroup: id });
      ui.snack(`Set saved (${fmtMs(dur)}) · it gets reviewed after`);
      back();
    } catch (e: any) {
      setRec('on');
      ui.snack(`Upload failed: ${e.message}. Tap End again.`);
      (globalThis as any).__punchableUnsaved = blob;
    }
  };
  const pct = target ? Math.min(100, (ms / (target * 1000)) * 100) : 0;
  const lit = ms >= light * 1000 && target > 0;

  return (
    <Screen title="On stage" task deep testID="stage"
      header={<TopBar task title={rec === 'on' ? '● Recording' : rec === 'saving' ? 'Saving…' : rec === 'off' ? 'Not recording' : 'Clock only'} left={<View style={{ width: 44 }} />} right={<TextBtn testID="stage-end" label="End" color={C.rec} onPress={end} />} />}
      dock={<Btn testID="stage-next" label={cur >= steps.length - 1 ? 'Last bit' : 'Next bit'} v="primary" style={{ minHeight: 60 }} onPress={next} disabled={!steps.length || cur >= steps.length - 1} />}>
      {!B ? <Empty text="…" /> : (
        <Pressable accessibilityRole="button" accessibilityLabel="Next bit" onPress={next} style={{ gap: 18 }}>
          <View style={{ alignItems: 'center', gap: 2 }}>
            <Text testID="stage-clock" style={{ fontFamily: F.black, fontSize: 84, lineHeight: 90, letterSpacing: -2, color: lit ? C.gapText : C.text, fontVariant: ['tabular-nums'] }}>{fmtMs(ms)}</Text>
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
    </Screen>
  );
}
