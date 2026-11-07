// Write a rant (docs/design/SPEC-2026-11-06-rant-tab.md §3, slice 1): a full-height form sheet on the phone (HIG/sheets:
// Close leading, the save trailing once there is a rant to save), a page on the web. Opened from the Rant tab's "+"
// (blank) or a topic (filled, `topic` + `topic_id`).
//
//   1 Topic     one typed field, "What's the rant about?" (jots are typed)
//   2 Gather    today's search on the topic (/api/search: his bits, typed ideas, taped moments), close bits ticked;
//               Find more = /api/jokes/<id>/similar from each ticked bit, appended unticked
//   3 Draft     Write it asks the worker (POST /api/rant/draft, a `rant_draft` job, Opus) and polls the job; the lines
//               come back in rant shape: his (word for word from a ticked source, checked on the server, shown plain
//               with "from Self-checkout v8") or new (the AI badge); every line is in until he takes it out (Cut /
//               Skip, then Put back): no review step (the owner's standing rule, over the spec's Keep / Skip each)
//               Hook check on the script under the lines (rules/hookRules, no AI)
//   4 Save rant a new bit, v1 a rant cut (`source: rant_draft`), "from" links to the bits whose lines he kept, the
//               topic marked ranted, skipped lines taught as passed over (lib/rant saveRant); then the bit's page
import React, { useEffect, useRef, useState } from 'react';
import { AccessibilityInfo, Keyboard, Platform, Pressable, ScrollView, Text, TextInput, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { get, useOnline } from '@/lib/api';
import { rant, saveRant } from '@/lib/rant';
import { haptics } from '@/lib/haptics';
import { actionSheet } from '@/lib/actionSheet';
import { WorkerLine } from '@/lib/worker';
import { C, F } from '@/lib/theme';
import { AIBadge, Btn, Sheet, SheetHead, Skeleton, T, useUI } from '@/lib/ui';
import { jobView, savedWord } from '@/rules/sessionRules';
import * as R from '@/rules/rantRules';
import { hookCheck, hookDetail, hookWord } from '@/rules/hookRules';

const enc = encodeURIComponent;
const POLL_MS = 2500;
const POLL_FOR_MS = 4 * 60 * 1000;   // Opus with thinking can take a while; past this the worker line says why
type Result = { status?: string; why?: string | null; lines?: R.DraftLine[]; dropped?: number };
type Draft =
  | { k: 'none' }
  | { k: 'asking'; waitingFor?: string | null }
  | { k: 'failed'; error: string }
  | { k: 'lines'; lines: R.DraftLine[]; note: string | null };

export default function WriteRant() {
  const p = useLocalSearchParams<{ topic?: string; topic_id?: string }>();
  const topicId = typeof p.topic_id === 'string' && p.topic_id ? p.topic_id : null;
  const ui = useUI();
  const online = useOnline();
  const [topic, setTopic] = useState(typeof p.topic === 'string' ? p.topic : '');
  const [asked, setAsked] = useState(typeof p.topic === 'string' ? p.topic.trim() : '');   // the topic Gather searched
  // Gather's rows for the topic they were searched for (a row list for an older topic is not shown)
  const [found, setFound] = useState<{ q: string; hits: R.Hit[] } | null>(null);
  const [more, setMore] = useState(false);
  const hits = asked && found?.q === asked ? found.hits : null;
  const setHits = (f: (h: R.Hit[]) => R.Hit[]) => setFound((x) => (x ? { q: x.q, hits: f(x.hits) } : x));
  const gathering = more || (!!asked && found?.q !== asked);
  const [draft, setDraft] = useState<Draft>({ k: 'none' });
  const [marks, setMarks] = useState<R.Marks>([]);
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [writeNext, setWriteNext] = useState(false);   // Write it was tapped before Gather ran for this topic
  const savedId = useRef<string | null>(null);          // a save that failed half way retries onto the same bit
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const leave = () => (router.canGoBack() ? router.back() : router.replace('/' as any));

  // Gather: today's search on the topic, once per topic he settles on
  useEffect(() => {
    if (!asked) return;
    let live = true;
    get(`/api/search?q=${enc(asked)}&limit=12`)
      .then((r) => { if (live) setFound({ q: asked, hits: R.gatherHits(r) }); })
      .catch(() => { if (live) setFound({ q: asked, hits: [] }); });
    return () => { live = false; };
  }, [asked]);
  const findMore = async () => {
    if (!hits) return;
    setMore(true);
    let out = hits;
    for (const h of hits.filter((x) => x.ticked && x.kind === 'bit')) {
      try {
        const s = await get<any>(`/api/jokes/${enc(h.id)}/similar`);
        out = R.gatherHits({ bits: s?.like?.bits || [], moments: s?.like?.moments || [] }, out);
      } catch {}
    }
    if (alive.current) { setHits(() => out); setMore(false); }
  };
  const tick = (id: string) => { haptics.selection(); setHits((hs) => (hs || []).map((h) => (h.id === id ? { ...h, ticked: !h.ticked } : h))); };

  const write = async (fresh = false) => {
    const t = topic.trim();
    if (!t || !online) return;
    // his material for THIS topic first: Gather runs, then the draft goes (never another topic's ticks, and never
    // none just because the field still had the keyboard)
    if (t !== asked || !hits) {
      Keyboard.dismiss();
      setAsked(t);
      setWriteNext(true);
      return;
    }
    setErr(null);
    setMarks([]);
    setDraft({ k: 'asking' });
    try {
      let view = jobView<Result>(await rant.draft(t, R.tickedIds(hits || []), topicId, fresh));
      const until = Date.now() + POLL_FOR_MS;
      while (view.state === 'waiting' && alive.current) {
        setDraft({ k: 'asking', waitingFor: view.waitingFor });
        if (!view.job || Date.now() > until) { setDraft({ k: 'failed', error: 'still waiting' }); return; }
        await new Promise((r) => setTimeout(r, POLL_MS));
        if (!alive.current) return;
        const jid: string = view.job;
        view = jobView<Result>(await get(`/api/jobs/${enc(jid)}`).catch(() => ({ status: 'queued', id: jid })));
      }
      if (!alive.current) return;
      if (view.state === 'failed') { setDraft({ k: 'failed', error: view.error }); return; }
      if (view.state !== 'done') { setDraft({ k: 'failed', error: 'still waiting' }); return; }
      const r = view.result;
      if (r.status === 'provider_error') { setDraft({ k: 'failed', error: r.why || 'no answer' }); return; }
      setDraft({ k: 'lines', lines: r.lines || [], note: r.status === 'ok' ? null : r.why || 'Claude has no rant for this one.' });
    } catch (e: any) {
      if (alive.current) setDraft({ k: 'failed', error: e?.message || 'no answer' });
    }
  };

  useEffect(() => {
    if (!writeNext || !hits || topic.trim() !== asked) return;
    const go = setTimeout(() => { setWriteNext(false); void write(); }, 0);
    return () => clearTimeout(go);
  }, [writeNext, hits]); // eslint-disable-line react-hooks/exhaustive-deps

  const lines = draft.k === 'lines' ? draft.lines : [];
  const saveable = draft.k === 'lines' && R.canSaveRant(lines, marks);
  const kept = R.assemble(lines, marks);
  const hc = draft.k === 'lines' ? hookCheck(kept, R.hisOfKept(lines, marks)) : null;
  const mark = (i: number, on: boolean) => { haptics.selection(); setMarks((m) => { const n = m.slice(); n[i] = on; return n; }); };
  // HIG/modality: a written rant is not thrown away by a swipe or Close without asking (the swipe is off meanwhile)
  const dirty = draft.k === 'lines' && lines.length > 0;
  const close = () => {
    if (!dirty) return leave();
    actionSheet({ title: 'Leave without saving?', options: [
      { label: 'Discard the rant', destructive: true, onPress: leave },
      { label: 'Keep going', cancel: true },
    ] });
  };
  const save = async () => {
    if (!saveable || saving) return;
    setSaving(true);
    try {
      const r = await saveRant({ topic: topic.trim(), topicId, lines, marks, text: kept.join('\n'), into: savedId.current, onBit: (id) => { savedId.current = id; } });
      haptics.success();
      ui.snack(r.missed.length ? `${savedWord(1, 'rant')}, but ${r.missed.join(' and ')} didn’t save` : savedWord(1, 'rant'));
      leave();
      setTimeout(() => router.push(`/joke/${enc(r.id)}` as any), 350);
    } catch (e: any) {
      const m = `Couldn’t save: ${e.message}`;
      setErr(m);
      ui.snack(m);
      AccessibilityInfo.announceForAccessibility(m);
    } finally {
      if (alive.current) setSaving(false);
    }
  };

  const body = (
    <View style={{ gap: 14 }}>
      <View style={{ gap: 6 }}>
        <T v="eb" accessibilityRole="header">Topic</T>
        <TextInput testID="rant-topic" value={topic} onChangeText={setTopic} placeholder="What’s the rant about?" placeholderTextColor={C.dim}
          autoFocus={!topic} returnKeyType="search" onSubmitEditing={() => setAsked(topic.trim())} onBlur={() => setAsked(topic.trim())}
          accessibilityLabel="What’s the rant about?" maxLength={200} maxFontSizeMultiplier={1.6}
          style={{ minHeight: 48, borderWidth: 1, borderColor: C.border, borderRadius: 10, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 17 }} />
      </View>

      {!!asked && (
        <View testID="rant-gather" style={{ gap: 4 }}>
          <View style={{ flexDirection: 'row', alignItems: 'baseline' }}>
            <T v="eb" accessibilityRole="header" style={{ flex: 1 }}>From your material</T>
            {!!hits?.some((h) => h.ticked) && <T v="small">{hits.filter((h) => h.ticked).length} ticked</T>}
          </View>
          {hits == null && gathering && <Skeleton lines={3} height={18} testID="rant-gather-skeleton" />}
          {hits != null && !hits.length && <T v="mut">Nothing of yours matches yet. Claude writes it from the topic alone.</T>}
          {(hits || []).map((h) => (
            <Pressable key={h.id} testID={`gather-${h.id}`} accessibilityRole="checkbox" accessibilityState={{ checked: h.ticked }}
              accessibilityLabel={`${h.title}${h.sub ? `, ${h.sub}` : ''}`} onPress={() => tick(h.id)}
              style={{ flexDirection: 'row', alignItems: 'center', gap: 12, minHeight: 52, paddingVertical: 6, borderBottomWidth: 1, borderBottomColor: C.hairline }}>
              <View style={{ width: 22, height: 22, borderRadius: 11, borderWidth: 2, borderColor: h.ticked ? C.amber : C.border, backgroundColor: h.ticked ? C.amber : 'transparent', alignItems: 'center', justifyContent: 'center' }}>
                {h.ticked && <Text style={{ color: C.bg, fontSize: 13, fontFamily: F.g7 }}>✓</Text>}
              </View>
              <View style={{ flex: 1, minWidth: 0 }}>
                <Text numberOfLines={2} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.semi, fontSize: 16, color: C.text }}>{h.title}</Text>
                {!!h.sub && <T v="small" numberOfLines={1}>{h.sub}</T>}
              </View>
            </Pressable>
          ))}
          {!!hits?.some((h) => h.ticked && h.kind === 'bit') && (
            <Btn small v="ghost" testID="rant-find-more" label={gathering ? 'Finding…' : 'Find more'} disabled={gathering || !online} onPress={() => void findMore()} style={{ alignSelf: 'flex-start' }} />
          )}
        </View>
      )}

      {draft.k !== 'lines' && (
        <View style={{ gap: 6 }}>
          <Btn v="primary" testID="rant-write" label={draft.k === 'asking' || writeNext ? 'Writing…' : draft.k === 'failed' ? 'Try again' : 'Write it'}
            disabled={!topic.trim() || !online || draft.k === 'asking' || writeNext} onPress={() => void write(draft.k === 'failed')} />
          {!online && <T v="small">offline</T>}
          {draft.k === 'asking' && <WorkerLine what="The rant" waitingFor={draft.waitingFor} />}
          {draft.k === 'failed' && <T v="mut" testID="rant-failed">{`Claude didn’t answer: ${draft.error}`}</T>}
        </View>
      )}

      {draft.k === 'lines' && (
        <View testID="rant-lines" style={{ gap: 2 }}>
          {!!hc && (
            <View testID="rant-hookcheck" accessible accessibilityLabel={`Hook check. ${hookWord(hc).replace(/ s$/, ' seconds')}. ${hookDetail(hc).replace(' · ', ', ')}`} style={{ paddingVertical: 6, gap: 2 }}>
              <Text maxFontSizeMultiplier={1.6} style={{ fontFamily: F.semi, fontSize: 15, color: hc.late ? C.gapText : C.text }}>{hookWord(hc)}</Text>
              <T v="small">{hookDetail(hc)}</T>
            </View>
          )}
          {!!draft.note && <T v="mut">{draft.note}</T>}
          {lines.map((l, i) => <LineRow key={i} l={l} i={i} n={lines.length} mark={marks[i]} onMark={(on) => mark(i, on)} />)}
          <Btn small v="ghost" testID="rant-rewrite" label="Write it again" disabled={!online} onPress={() => void write(true)} style={{ alignSelf: 'flex-start', marginTop: 8 }} />
        </View>
      )}
      {!!err && <T v="mut" testID="rant-save-error" accessibilityRole={'alert' as any}>{err}</T>}
    </View>
  );

  if (Platform.OS === 'web') {
    return (
      <View style={{ flex: 1, backgroundColor: C.bg }}>
        <Sheet visible onClose={close} label="Write a rant" testID="rant-sheet">
          <SheetHead title="Write a rant" onCancel={close} cancelLabel="Close" testID="rant" onDone={() => void save()} doneLabel={saving ? 'Saving…' : 'Save rant'} doneDisabled={!saveable || saving} />
          {body}
        </Sheet>
      </View>
    );
  }
  return (
    <View testID="rant-sheet" style={{ flex: 1, backgroundColor: C.surface }}>
      <Stack.Screen options={{ title: 'Write a rant', gestureEnabled: !dirty }} />
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button accessibilityLabel="Close" onPress={close}>Close</Stack.Toolbar.Button>
      </Stack.Toolbar>
      {/* HIG/sheets: the Done-style button trailing, inactive until there is a rant to save */}
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="prominent" disabled={!saveable || saving} accessibilityLabel="Save rant" onPress={() => void save()}>Save rant</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" keyboardShouldPersistTaps="handled" keyboardDismissMode="interactive" automaticallyAdjustKeyboardInsets
        contentContainerStyle={{ padding: 16, paddingBottom: 48 }}>
        {body}
      </ScrollView>
    </View>
  );
}

/** One drafted line. His: plain, "from <source>" under it. New: the AI badge. Every line is in until he takes it out:
 *  Cut (his) / Skip (new), then Put back; a line taken out reads struck through and dimmed, and the button's word says
 *  so too (never colour alone). VoiceOver: one element with the action. */
function LineRow({ l, i, n, mark, onMark }: { l: R.DraftLine; i: number; n: number; mark: boolean | undefined; onMark: (on: boolean) => void }) {
  const out = !R.isIn(l, mark);
  return (
    <View testID={`rant-line-${i}`} style={{ paddingVertical: 10, gap: 8, borderBottomWidth: 1, borderBottomColor: C.hairline }}
      accessible accessibilityLabel={R.lineLabel(l, i, n, mark)}
      accessibilityActions={[{ name: 'toggle', label: R.lineAction(l, mark) }]}
      onAccessibilityAction={(e) => { if (e.nativeEvent.actionName === 'toggle') onMark(out); }}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <T v="small" style={{ flex: 1 }}>{l.role}</T>
        {!l.his && <AIBadge />}
      </View>
      <Text maxFontSizeMultiplier={2} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 24, color: out ? C.mut2 : C.text, textDecorationLine: out ? 'line-through' : 'none' }}>{l.text}</Text>
      {l.his && !!l.source_label && <T v="small">from {l.source_label}</T>}
      <View style={{ flexDirection: 'row', gap: 8 }} importantForAccessibility="no-hide-descendants" accessibilityElementsHidden>
        <Btn small testID={`rant-out-${i}`} label={R.lineAction(l, mark)} v="plain" onPress={() => onMark(out)} style={{ alignSelf: 'flex-start' }} />
      </View>
    </View>
  );
}
