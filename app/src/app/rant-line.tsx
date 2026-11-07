// Claude on a line (SPEC 2026-11-06 §12.5): a sheet over the script, nonmodal at the medium detent so the line stays in
// sight. The line quoted; Sharper · Shorter · Angrier · Bigger · Tag after (one tap asks); "Or say what you want".
// "Nothing changes until you pick one." While asking: "Asking Claude…" and the worker line. Then three versions, each
// with the AI badge and Use this · Add after (Tag after: Add it after), and Try again last. Use this replaces the line
// ("Line changed", Undo); the others teach the profile as passed over (the standing learning loop).
// Request: POST /api/rant/line (a `rant_line` job, polled).
import React, { useEffect, useRef, useState } from 'react';
import { ScrollView, Text, TextInput, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { useOnline } from '@/lib/api';
import { passedOver, pollJob, rant, type LineResult } from '@/lib/rant';
import { scripts, useScript } from '@/lib/scriptStore';
import { haptics } from '@/lib/haptics';
import { WorkerLine } from '@/lib/worker';
import { C, F } from '@/lib/theme';
import { AIBadge, Btn, Chip, Chips, T, useUI } from '@/lib/ui';
import { PROSE_CAP, ROW_CAP } from '@/rules/bitLines';
import { applyOption, removeKeys, restoreLine } from '@/rules/scriptRules';

const ASKS: [string, string][] = [['sharper', 'Sharper'], ['shorter', 'Shorter'], ['angrier', 'Angrier'], ['bigger', 'Bigger'], ['tag', 'Tag after']];
type Phase = { k: 'idle' } | { k: 'asking'; waitingFor: string | null } | { k: 'options'; options: string[]; note: string | null } | { k: 'failed'; error: string };

export default function RantLine() {
  const p = useLocalSearchParams<{ topic: string; key: string }>();
  const tid = decodeURIComponent(String(p.topic || ''));
  const ui = useUI();
  const online = useOnline();
  const st = useScript(tid);
  // the line by its key: he can cut or move lines on the script behind this sheet
  const i = st ? st.lines.findIndex((l) => l.key === p.key) : -1;
  const line = i >= 0 ? st?.lines[i] : undefined;
  const [ask, setAsk] = useState<string | null>(null);
  const [say, setSay] = useState('');
  const [phase, setPhase] = useState<Phase>({ k: 'idle' });
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const tag = ask === 'tag';

  const go = async (a: string | null, fresh = false) => {
    if (!online || !st || !line?.text.trim()) return;
    setAsk(a);
    setPhase({ k: 'asking', waitingFor: null });
    const texts = st.lines.map((l) => l.text.trim()).filter(Boolean);
    const at = st.lines.slice(0, i).filter((l) => l.text.trim()).length;
    try {
      const res = await pollJob<LineResult>(await rant.line({ joke: st.rantId, lines: texts, at, ask: a, say: say.trim() || null, fresh }), () => alive.current, (w) => setPhase({ k: 'asking', waitingFor: w }));
      if (!alive.current) return;
      if (!res.ok) { setPhase({ k: 'failed', error: res.error }); return; }
      const opts = (res.result.options || []).map((o) => o.text).filter(Boolean);
      setPhase({ k: 'options', options: opts, note: opts.length ? null : res.result.why || 'Claude has nothing new for this line.' });
    } catch (e: any) {
      if (alive.current) setPhase({ k: 'failed', error: e?.message || 'no answer' });
    }
  };
  const use = (text: string, after: boolean) => {
    const cur = scripts.get(tid);
    const at = cur ? cur.lines.findIndex((l) => l.key === p.key) : -1;
    if (!cur || at < 0) { ui.snack('That line is gone from the rant.'); return; }
    const was = cur.lines[at];
    const r = applyOption(cur.lines, at, text, after);
    const added = after ? r.lines[r.focus].key : null;
    scripts.setLines(tid, r.lines, r.focus);
    if (phase.k === 'options') passedOver(phase.options, text, cur.rantId);
    haptics.success();
    router.back();
    // Undo touches only this line (anything typed since stays)
    ui.snack(after ? 'Added' : 'Line changed', () => {
      const now = scripts.get(tid);
      if (now) scripts.setLines(tid, added ? removeKeys(now.lines, [added]) : restoreLine(now.lines, was));
    });
  };

  return (
    <ScrollView testID="rant-line" style={{ flex: 1, backgroundColor: C.surface }} contentContainerStyle={{ padding: 16, paddingTop: 20, gap: 12 }} keyboardShouldPersistTaps="handled">
      <View style={{ borderLeftWidth: 3, borderLeftColor: C.amber, paddingLeft: 10 }}>
        <Text testID="rant-line-quote" maxFontSizeMultiplier={PROSE_CAP} style={{ fontFamily: F.body, fontSize: 18, lineHeight: 25, color: C.text }}>{line?.text || ''}</Text>
      </View>
      <Chips testID="rant-line-asks">
        {ASKS.map(([k, w]) => <Chip key={k} testID={`ask-${k}`} label={online ? w : `${w} (offline)`} on={ask === k} onPress={online ? () => void go(k) : undefined} />)}
      </Chips>
      <TextInput testID="rant-line-say" value={say} onChangeText={setSay} placeholder="Or say what you want" placeholderTextColor={C.dim}
        editable={online} accessibilityLabel="Tell Claude what you want from this line" returnKeyType="send" submitBehavior="blurAndSubmit" onSubmitEditing={() => void go(ask)}
        maxLength={500} maxFontSizeMultiplier={ROW_CAP} style={{ minHeight: 44, borderRadius: 10, paddingHorizontal: 12, backgroundColor: C.card, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      {!online && <T v="mut">Claude (offline)</T>}
      {phase.k === 'idle' && online && <T v="mut">Nothing changes until you pick one.</T>}
      {phase.k === 'asking' && (
        <View style={{ gap: 4 }}>
          <T v="mut">Asking Claude…</T>
          <WorkerLine what="The line" waitingFor={phase.waitingFor} />
        </View>
      )}
      {phase.k === 'failed' && <T v="mut">{`Claude didn’t answer: ${phase.error}`}</T>}
      {phase.k === 'options' && (
        <View style={{ gap: 10 }}>
          {phase.note && <T v="mut">{phase.note}</T>}
          {phase.options.map((o, k) => (
            <View key={k} testID={`option-${k}`} style={{ borderRadius: 12, padding: 12, gap: 8, backgroundColor: C.card }}>
              <AIBadge style={{ alignSelf: 'flex-start' }} />
              <Text maxFontSizeMultiplier={PROSE_CAP} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 24, color: C.text }}>{o}</Text>
              <View style={{ flexDirection: 'row', gap: 8 }}>
                {tag ? <Btn small testID={`option-${k}-after`} label="Add it after" onPress={() => use(o, true)} /> : (
                  <>
                    <Btn small testID={`option-${k}-use`} label="Use this" onPress={() => use(o, false)} />
                    <Btn small testID={`option-${k}-after`} label="Add after" onPress={() => use(o, true)} />
                  </>
                )}
              </View>
            </View>
          ))}
          <Btn testID="rant-line-again" label="Try again" disabled={!online} onPress={() => void go(ask, true)} />
        </View>
      )}
    </ScrollView>
  );
}
