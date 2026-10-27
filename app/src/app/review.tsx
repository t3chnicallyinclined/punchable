// Review (full-screen task): pick a set, step through its performances in order, rate each one.
// Killed / Worked / Meh / Died and "Yes, it <suggested>" are pinned to the bottom.
// Every rating appends: confirm of the automatic match (if still proposed) + the rating on the performance.
import { useEffect, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { Step, decide, get, post, undo, useApi } from '@/lib/api';
import { C, F, RATING, fmtDate, fmtMs, stageInfo } from '@/lib/theme';
import { BAR_WORDS, Badge, Btn, Card, ErrorCard, LaughBars, List, ListRow, Screen, Sheet, T, TextBtn, TopBar, s, useUI } from '@/lib/ui';
import { HearIt } from '@/lib/hear';
import { stopPlayback } from '@/lib/media';

type SetCard = { id: string; title: string; date: string; venue: string; duration_s: number; rated: number; rateable: number; reviewed: boolean; has_audio: boolean };
type Review = { set: SetCard; steps: Step[]; audio: { opus: string; wav: string } | null };

export default function ReviewScreen() {
  const { set } = useLocalSearchParams<{ set?: string }>();
  return set ? <Run setId={String(set)} /> : <Pick />;
}

const close = () => {
  stopPlayback();
  router.canGoBack() ? router.back() : router.navigate('/' as any);
};

function Pick() {
  const { data, error, reload } = useApi<{ sets: SetCard[] }>('/api/sets');
  return (
    <Screen title="Step through" task header={<TopBar title="Pick a set" left={<TextBtn testID="review-close" label="Close" onPress={close} />} right={<View style={{ width: 66 }} />} />} testID="screen-review-pick">
      <T v="h1">Step through a set</T>
      <T v="mut">Optional. The AI has already rated every joke from the laughs; step through only if you want to hear each one and set the rating yourself.</T>
      {error && <ErrorCard error={error} onRetry={reload} />}
      <Card style={{ paddingVertical: 4 }}>
        <List>
          {(data?.sets || []).map((st) => (
            <ListRow key={st.id} testID={`pick-${st.id}`} onPress={() => router.setParams({ set: st.id })}
              title={st.title} sub={`${fmtDate(st.date)} · ${st.rated} of ${st.rateable} rated${st.has_audio ? '' : ' · no audio'}`}
              right={st.rated ? <Badge label={`${st.rated} yours`} fg={C.text2} bg={C.surface2} /> : undefined} />
          ))}
        </List>
      </Card>
    </Screen>
  );
}

function Run({ setId }: { setId: string }) {
  const ui = useUI();
  const { data, error, reload, setData } = useApi<Review>(`/api/sets/${encodeURIComponent(setId)}/review`);
  const [i, setI] = useState<number | null>(null);
  const [done, setDone] = useState(false);
  const [ideas, setIdeas] = useState(0);
  const [ready, setReady] = useState<Record<string, { id: string; title: string; hit_sets: number; stage: string; stage_ai?: boolean }>>({});
  const [promoted, setPromoted] = useState<Record<string, boolean>>({});
  const [span, setSpan] = useState(false);
  const [busy, setBusy] = useState(false);

  // start at the first step not rated yet
  useEffect(() => {
    if (data && i === null) {
      const k = data.steps.findIndex((st) => st.kind === 'joke' && !st.rating);
      setI(k < 0 ? 0 : k);
    }
  }, [data]);
  useEffect(() => () => stopPlayback(), []);

  if (error) return <Screen title="Review" task header={<TopBar title="Review" left={<TextBtn label="Close" onPress={close} />} right={<View style={{ width: 66 }} />} />}><ErrorCard error={error} onRetry={reload} /></Screen>;
  if (!data || i === null) return <Screen title="Review" task header={<TopBar title="Review" left={<TextBtn label="Close" onPress={close} />} right={<View style={{ width: 66 }} />} />}><T v="mut">Loading…</T></Screen>;

  const steps = data.steps;
  const cur = steps[Math.min(i, steps.length - 1)];
  const finished = done || !steps.length;
  const go = (n: number) => {
    stopPlayback();
    setSpan(false);
    if (n >= steps.length) setDone(true);
    else setI(n);
  };
  const refresh = async () => {
    const d = await get<Review>(`/api/sets/${encodeURIComponent(setId)}/review`);
    setData(d);
    return d;
  };

  const rate = async (rating: string) => {
    if (!cur.perf || busy) return;
    setBusy(true);
    try {
      const r = await post<{ indexes: number[]; joke?: { id: string; title: string; stage: string; stage_ai?: boolean; hit_sets: number; ready_for_working: boolean } }>('/api/review/rate', { perf: cur.perf.id, rating });
      // offer Working once it hit in 3 rooms; if the AI already put it there, offer to confirm that instead
      if (r.joke?.ready_for_working && (r.joke.stage !== 'working' || r.joke.stage_ai)) setReady((x) => ({ ...x, [r.joke!.id]: r.joke! }));
      const at = i;
      await refresh();
      ui.snack(`${cur.joke?.title || 'It'}: ${RATING[rating].label.toLowerCase()}`, async () => {
        await undo(r.indexes);
        await refresh();
        setReady((x) => { const y = { ...x }; if (r.joke) delete y[r.joke.id]; return y; });
        setDone(false);
        setI(at);
      });
      go(at + 1);
    } catch (e: any) {
      if (e.status === 409) go(i + 1);
      else ui.snack(e.message);
    } finally {
      setBusy(false);
    }
  };
  const saveIdea = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const r = await post<{ indexes: number[]; id: string }>('/api/ideas', {
        text: cur.punch.text || cur.text,
        from: { set: setId, perf: cur.perf?.id, start_ms: cur.punch.start_ms, end_ms: cur.punch.end_ms },
      });
      setIdeas((n) => n + 1);
      const at = i;
      ui.snack('Saved to Ideas', async () => { await undo(r.indexes); setIdeas((n) => n - 1); setDone(false); setI(at); });
      go(at + 1);
    } catch (e: any) {
      ui.snack(e.message);
    } finally {
      setBusy(false);
    }
  };
  const notThisJoke = async () => {
    if (!cur.perf) return;
    try {
      const r = await post<{ indexes: number[] }>('/api/review/reject-match', { perf: cur.perf.id });
      await refresh();
      ui.snack(`Not ${cur.joke?.title}. Save it as a new line?`, async () => { await undo(r.indexes); await refresh(); });
    } catch (e: any) {
      ui.snack(e.message);
    }
  };

  const rated = steps.filter((st) => st.kind === 'joke' && st.rating).length;
  const offers = Object.values(ready).filter((j) => !promoted[j.id]);
  const kindBadge = cur.kind === 'joke'
    ? { label: stageInfo(cur.joke?.stage).label, fg: stageInfo(cur.joke?.stage).fg, bg: stageInfo(cur.joke?.stage).bg }
    : cur.kind === 'new_line' ? { label: 'New line?', fg: C.blue, bg: C.blueSoft } : { label: 'Crowd work', fg: C.text2, bg: C.surface2 };
  const best = cur.punch.laugh;
  const sug = cur.ai_rating || cur.suggested; // the AI's rating when there is one, else the laugh meter's

  const header = (
    <TopBar task={false} title={finished ? data.set.title : `${Math.min(i + 1, steps.length)} of ${steps.length}`}
      left={<TextBtn testID="review-close" label="Close" onPress={close} />}
      right={!finished ? <TextBtn testID="open-span" label="Span" onPress={() => setSpan(true)} /> : <View style={{ width: 66 }} />} />
  );
  const dock = finished ? (
    <>
      {offers.map((j) => (
        <Btn key={j.id} testID={`promote-${j.id}`} v="green" label={j.stage === 'working' ? `Confirm ${j.title} as Working` : `Move ${j.title} to Working`} style={{ minHeight: 56 }} onPress={async () => {
          try {
            const r = j.stage === 'working'
              ? await decide({ target: 'node', id: j.id, action: 'confirm' })
              : await decide({ target: 'node', id: j.id, action: 'edit', after: { stage: 'working' } });
            setPromoted((x) => ({ ...x, [j.id]: true }));
            ui.snack(`${j.title} moved to Working`, async () => { await undo(r.indexes); setPromoted((x) => ({ ...x, [j.id]: false })); });
          } catch (e: any) { ui.snack(e.message); }
        }} />
      ))}
      <Btn testID="review-done" label="Done" onPress={close} />
    </>
  ) : cur.kind === 'joke' ? (
    <>
      <View style={{ flexDirection: 'row', gap: 6 }}>
        {(['killed', 'worked', 'meh', 'died'] as const).map((r) => (
          <Btn key={r} testID={`rate-${r}`} label={RATING[r].label} disabled={busy} onPress={() => rate(r)}
            style={[{ flex: 1, paddingHorizontal: 0 }, cur.rating === r && { borderColor: C.amber, backgroundColor: C.amberSoft }]} textStyle={{ color: RATING[r].fg }} />
        ))}
      </View>
      {cur.rating
        ? <Btn testID="review-next" v="primary" label={`Keep “${RATING[cur.rating].label}” · Next`} style={{ minHeight: 56 }} onPress={() => go(i + 1)} textStyle={{ fontSize: 17 }} />
        : <Btn testID="review-agree" v="primary" disabled={busy} label={sug === 'meh' ? 'Yes, it was meh' : sug === 'died' ? 'Yes, it died' : `Yes, it ${sug}`} style={{ minHeight: 56 }} onPress={() => rate(sug)} textStyle={{ fontSize: 17 }} />}
    </>
  ) : cur.kind === 'new_line' ? (
    <>
      <Btn testID="review-ignore" label="Ignore" onPress={() => go(i + 1)} />
      <Btn testID="review-save-idea" v="primary" disabled={busy} label="Save to Ideas" style={{ minHeight: 56 }} onPress={saveIdea} textStyle={{ fontSize: 17 }} />
    </>
  ) : (
    <>
      <Btn testID="review-save-line" label="Save a line" onPress={saveIdea} />
      <Btn testID="review-skip-crowd" v="primary" label="OK, don’t count it" style={{ minHeight: 56 }} onPress={() => go(i + 1)} textStyle={{ fontSize: 17 }} />
    </>
  );

  return (
    <Screen title="Review" task dock={dock} testID="screen-review" scrollKey={finished ? -1 : i}
      header={header}>
      <View style={{ flexDirection: 'row', gap: 4 }} accessibilityElementsHidden>
        {steps.map((st, k) => (
          <Pressable key={st.key} onPress={() => { setDone(false); go(k); }} style={{ flex: 1, height: 16, justifyContent: 'center' }} accessibilityLabel={`Go to ${k + 1}`}>
            <View style={{ height: 4, borderRadius: 2, backgroundColor: !finished && k === i ? C.text : st.rating ? C.amber : st.kind !== 'joke' ? C.borderStrong : C.border }} />
          </Pressable>
        ))}
      </View>
      {finished ? (
        <>
          <T v="h1" testID="set-saved">Set saved</T>
          <T v="mut" style={{ fontSize: 16 }}>{rated} jokes rated · crowd work not counted · {ideas ? `${ideas} new idea${ideas > 1 ? 's' : ''}` : 'no new ideas'}</T>
          {offers.map((j) => (
            <Card key={j.id} testID={`offer-${j.id}`} style={{ borderColor: C.greenBorder }}>
              <T v="eb" style={{ color: C.green }}>Ready for Working</T>
              <T style={{ marginTop: 6 }}>{j.title} hit in <T v="num">{j.hit_sets}</T> different rooms. {j.stage === 'working' ? 'The AI already has it in Working. Confirm?' : 'Move to Working?'}</T>
            </Card>
          ))}
          <Card style={{ paddingVertical: 4 }}>
            <List>
              {steps.filter((st) => st.kind === 'joke').map((st) => (
                <ListRow key={st.key} chevron={false} title={<T numberOfLines={1}>{st.joke?.title}</T>}
                  right={<Badge label={st.rating ? RATING[st.rating].label : 'not rated'} bordered fg={st.rating ? RATING[st.rating].fg : C.dim} />} />
              ))}
            </List>
          </Card>
        </>
      ) : (
        <Card testID="review-card" style={{ gap: 12 }}>
          <View style={[s.row, { gap: 8, flexWrap: 'wrap' }]}>
            <Badge label={fmtMs(cur.perf?.start_ms ?? cur.punch.start_ms)} bordered />
            <Badge label={kindBadge.label} fg={kindBadge.fg} bg={kindBadge.bg} />
            {cur.rating && <Badge label={`rated ${RATING[cur.rating].label}`} fg={RATING[cur.rating].fg} bordered />}
          </View>
          <T v="h1" style={{ fontSize: 26, lineHeight: 31 }} testID="review-title">
            {cur.kind === 'joke' ? cur.joke?.title : cur.kind === 'crowd_work' ? (cur.name || 'Crowd work') : 'New line?'}
          </T>
          {cur.kind === 'joke' && cur.name && cur.name !== cur.joke?.title && <T v="small">Heard as “{cur.name}”</T>}
          <View style={s.punch}>
            <Text style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text }}>
              {(() => {
                const segs = cur.punch.segments || [];
                const k = segs.findIndex((x) => x.punch);
                const before = k > 0 ? segs.slice(Math.max(0, k - 2), k).map((x) => x.text).join(' ') : '';
                return <>{before ? `…${before} ` : '…'}<Text style={s.rv}>{cur.punch.text}</Text></>;
              })()}
            </Text>
          </View>
          <View style={[s.row, { gap: 10 }]}>
            <LaughBars bars={cur.max_bars} size={22} />
            <T v="mut" style={{ flex: 1 }} testID="review-laughs">
              {BAR_WORDS[cur.max_bars]}{best ? ` · ${((best.end_ms - best.start_ms) / 1000).toFixed(1)}s` : ''} · {cur.hits} laugh{cur.hits === 1 ? '' : 's'} in this span
            </T>
          </View>
          <LaughStrip step={cur} />
          <HearIt testID="hear-it" audio={data.audio?.opus || null} play={cur.play} k={cur.key}
            sub={`${fmtMs(cur.play.start_ms)}–${fmtMs(cur.play.end_ms)} · the punch and the laugh`} />
          {cur.kind === 'joke' && cur.match && cur.match.bucket === 'proposed' && (
            <View style={[s.row, { flexWrap: 'wrap' }]}>
              <T v="small" style={{ flex: 1, minWidth: 180 }}>Matched automatically: {cur.match.reason}</T>
              <Btn small testID="not-this-joke" label={`Not ${cur.joke?.title?.split(' (')[0]}?`} onPress={notThisJoke} textStyle={{ fontSize: 13 }} />
            </View>
          )}
          {cur.kind === 'new_line' && <T v="small">This span isn’t matched to any joke you have. Save it as an idea, or ignore it.</T>}
          {cur.kind === 'crowd_work' && <T v="small">Crowd work doesn’t count toward joke stats.</T>}
        </Card>
      )}

      <Sheet visible={span} onClose={() => setSpan(false)} label="The whole span" testID="span-sheet">
        <View style={[s.row, { justifyContent: 'space-between' }]}>
          <T v="h2">What you said</T>
          <TextBtn label="Done" onPress={() => setSpan(false)} />
        </View>
        <T v="mut" style={{ marginBottom: 8 }}>{fmtMs(cur?.perf?.start_ms ?? cur?.punch.start_ms)}–{fmtMs(cur?.perf?.end_ms ?? cur?.punch.end_ms)} · punch highlighted · laughs in amber</T>
        <View style={{ gap: 6 }}>
          {(cur?.punch.segments || []).map((g, k) => {
            const l = cur.laughs.find((x) => x.start_ms >= g.end_ms - 300 && x.start_ms <= g.end_ms + 1500);
            return (
              <Text key={k} style={{ fontFamily: F.body, fontSize: 16, lineHeight: 24, color: C.text }}>
                <Text style={{ color: C.dim, fontFamily: F.g6, fontSize: 13 }}>{fmtMs(g.start_ms)}  </Text>
                {g.punch ? <Text style={s.rv}>{g.text}</Text> : g.text}
                {l ? <Text style={{ color: C.amber, fontFamily: F.g7 }}>{`  ${'▮'.repeat(l.bars)}`}</Text> : null}
              </Text>
            );
          })}
          {!cur?.punch.segments?.length && <T>{cur?.text}</T>}
        </View>
        <T v="mut" style={{ marginTop: 12 }}>Laughs are measured by the detector, scored against this room. Nothing saves until you rate.</T>
      </Sheet>
    </Screen>
  );
}

/** Where the laughs fall across the span (the prototype's light bar). */
function LaughStrip({ step }: { step: Step }) {
  const a = step.perf?.start_ms ?? step.play.start_ms;
  const b = (step.perf?.end_ms ?? step.play.end_ms) + 3000;
  const len = Math.max(1, b - a);
  return (
    <View style={{ height: 28, borderRadius: 4, backgroundColor: C.surface2, overflow: 'hidden' }} accessibilityLabel="Laughs across the span">
      {step.laughs.map((l) => (
        <View key={l.id} style={{ position: 'absolute', bottom: 0, left: `${((l.start_ms - a) / len) * 100}%`, width: `${Math.max(0.8, ((l.end_ms - l.start_ms) / len) * 100)}%`, height: `${25 * Math.max(1, l.bars)}%`, backgroundColor: l.bars >= 4 ? C.green : C.amber, borderRadius: 2 }} />
      ))}
      <View style={{ position: 'absolute', top: 0, bottom: 0, left: `${((step.punch.start_ms - a) / len) * 100}%`, width: 2, backgroundColor: C.text }} />
    </View>
  );
}
