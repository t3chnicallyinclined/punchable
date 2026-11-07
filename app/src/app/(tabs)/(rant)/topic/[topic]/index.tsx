// The topic (SPEC 2026-11-06 §12.3): a page of his own words, pushed in the Rant stack (the tab bar stays). The bar:
// Back · the topic · ⋯ (Rename… · Start another rant · Delete topic). The system search field, "Find a bit to add"
// (results replace the page as he types, the same cards; the page has no other add). Then the rant so far (its
// first three lines, its takes), Jot a thought, Kept · N, Found · N (the same N as the tab's row: one server read),
// Find more, Dropped · N with Put back. One prominent button at the bottom: Write the rant / Keep writing.
// Nothing has to be sorted (no review step): a draft draws on Kept, else on Found.
// Requests: GET /api/topics/<id>, GET /api/topics/<id>/material, PATCH /api/topics/<id>, POST /api/ideas.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Platform, Pressable, Text, TextInput, View } from 'react-native';
import { Stack, router, useFocusEffect, useLocalSearchParams } from 'expo-router';
import { ApiError, post, undo, useApi, useOnline } from '@/lib/api';
import { rant, topicsChanged } from '@/lib/rant';
import { scripts } from '@/lib/scriptStore';
import { sayOrQueue, useOutbox } from '@/lib/say';
import { useRecordings } from '@/lib/recordings';
import { actionSheet } from '@/lib/actionSheet';
import { askText } from '@/lib/askText';
import { haptics } from '@/lib/haptics';
import { MaterialCard } from '@/lib/MaterialCard';
import { Icon } from '@/lib/Icon';
import { Screen } from '@/lib/Screen';
import { C, F } from '@/lib/theme';
import { Btn, Eb, Skeleton, T, useUI } from '@/lib/ui';
import { PROSE_CAP, ROW_CAP, linesOf, monthDay } from '@/rules/bitLines';
import { momentHref } from '@/rules/searchRows';
import * as R from '@/rules/rantRules';
import { appendMaterial, fromDoc } from '@/rules/scriptRules';

const native = Platform.OS !== 'web';
const enc = encodeURIComponent;
const SEARCH_SETTLE_MS = 250;
/** Over this many characters the bar cuts the topic and a Title 1 repeats it in full (3c's rule). */
const BAR_CHARS = 24;
const NO_MARKS: Record<string, R.State | null> = {};

export default function TopicPage() {
  const p = useLocalSearchParams<{ topic: string }>();
  const tid = decodeURIComponent(String(p.topic || ''));
  const ui = useUI();
  const online = useOnline();
  const pg = useApi<R.TopicPage>(tid ? `/api/topics/${enc(tid)}` : null);
  const page = pg.data;
  const topic = page?.topic;
  const takes = useRecordings();
  const pendingJots = useOutbox().filter((q) => q.context?.screen === `topic:${tid}`);
  // his taps, shown at once; they belong to the page read they were made over (a reload carries them from the server)
  const [marked, setMarked] = useState<{ on: R.TopicPage | null; m: Record<string, R.State | null> }>({ on: null, m: {} });
  const marks = marked.on === page ? marked.m : NO_MARKS;
  const setMarks = (f: (m: Record<string, R.State | null>) => Record<string, R.State | null>) =>
    setMarked((x) => ({ on: page || null, m: f(x.on === page ? x.m : {}) }));
  const [extra, setExtra] = useState<R.Mat[]>([]);
  const [more, setMore] = useState<{ offset: number; done: boolean; busy: boolean }>({ offset: R.FOUND_CAP, done: false, busy: false });
  const [sel, setSel] = useState<string | null>(null);
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [q, setQ] = useState('');
  const [hits, setHits] = useState<{ q: string; rows: R.Mat[] } | null>(null);
  const [jot, setJot] = useState('');
  const jotRef = useRef<any>(null);

  // back from the script or a bit: the page reads again (a new version, a mark written elsewhere)
  useFocusEffect(useCallback(() => { void pg.reload(); }, [tid])); // eslint-disable-line react-hooks/exhaustive-deps

  // the search: results replace the page as he types (HIG/search-fields: "start search immediately")
  useEffect(() => {
    const w = q.trim();
    if (!w) return;
    let live = true;
    const t = setTimeout(() => {
      rant.material(tid, w).then((r) => { if (live) setHits({ q: w, rows: r.rows || [] }); }).catch(() => { if (live) setHits({ q: w, rows: [] }); });
    }, SEARCH_SETTLE_MS);
    return () => { live = false; clearTimeout(t); };
  }, [q, tid]);

  const rows = useMemo(() => R.materialRows(page, marks, extra), [page, marks, extra]);
  const stateOf = (ref: string): R.State | null => (ref in marks ? marks[ref] : rows.kept.some((m) => m.ref === ref) ? 'kept' : rows.dropped.some((m) => m.ref === ref) ? 'dropped' : null);
  const theRant = page?.rants?.[0] || null;
  const rantTakes = R.takesOf(takes as R.Take[], theRant?.id);
  const fail = (what: string, e: any) => ui.snack(e instanceof ApiError ? `${what}: ${e.message}` : 'Can’t reach Punchable. Nothing changed.');

  /** A mark, at once on the page, then on the server; Undo puts the old state back. */
  const mark = async (m: R.Mat, to: R.State | null, word: string) => {
    const was = stateOf(m.ref);
    setMarks((x) => ({ ...x, [m.ref]: to }));
    setSel(null);
    try {
      await rant.mark(tid, m.ref, to);
      ui.snack(word, async () => {
        setMarks((x) => ({ ...x, [m.ref]: was }));
        try { await rant.mark(tid, m.ref, was); } catch (e) { fail('Couldn’t undo it', e); }
      });
    } catch (e) {
      setMarks((x) => ({ ...x, [m.ref]: was }));
      fail('Couldn’t mark it', e);
    }
  };
  const keep = (m: R.Mat) => (stateOf(m.ref) === 'kept' ? mark(m, null, 'Back in Found') : mark(m, 'kept', 'Kept'));
  const drop = (m: R.Mat) => { haptics.warning(); void mark(m, 'dropped', 'Dropped from this topic'); };
  const putBack = (m: R.Mat) => mark(m, null, 'Back in Found');
  const openMat = (m: R.Mat) => {
    setSel(null);
    if (m.kind === 'moment') {
      const h = momentHref(m as any);
      if (h) router.push(h as any);
      return;
    }
    router.push(`/topic/${enc(tid)}/bit/${enc(m.ref)}` as any);
  };
  /** To the rant: all its lines at the end of the script as a new version, with their source; the card is kept too. */
  const toRant = async (m: R.Mat) => {
    setSel(null);
    if (!topic) return;
    const s = scripts.get(tid);
    const base = s ? s.lines : fromDoc(theRant?.lines || []);
    const lines = appendMaterial(base, linesOf(m.words), m.kind === 'jot' ? null : m.ref, R.sourceLabel(m));
    scripts.open({
      topicId: tid, topicText: topic.text, rantId: s?.rantId ?? theRant?.id ?? null, unit: s?.unit ?? theRant?.unit ?? null,
      saved: s?.saved ?? theRant?.lines ?? null, versions: s?.versions ?? theRant?.versions ?? 0, lines, at: lines.length - 1,
    });
    if (stateOf(m.ref) !== 'kept') void rant.mark(tid, m.ref, 'kept').then(() => setMarks((x) => ({ ...x, [m.ref]: 'kept' }))).catch(() => {});
    try {
      const r = await scripts.save(tid);
      haptics.success();
      ui.snack(`Added to the rant · v${r?.v ?? (theRant?.versions || 0) + 1}`, r?.indexes.length ? async () => { await undo(r.indexes); scripts.forget(tid); await pg.reload(); } : undefined);
      void pg.reload();
    } catch {
      ui.snack('Added to the rant on this phone. It saves when you’re back online.');
    }
  };
  const findMore = async () => {
    if (more.busy || more.done || !online) return;
    setMore((x) => ({ ...x, busy: true }));
    try {
      const r = await rant.material(tid, null, more.offset);
      setExtra((x) => [...x, ...(r.rows || [])]);
      setMore({ offset: more.offset + R.FOUND_CAP, done: !r.more, busy: false });
      if (!(r.rows || []).length) ui.snack('Nothing more of yours sounds like this.');
    } catch (e) {
      setMore((x) => ({ ...x, busy: false }));
      fail('Couldn’t look further', e);
    }
  };
  // Jot a thought: a real idea (findable in Bits and search), then a reference on this topic; offline it queues
  // through the say outbox and shows at once (lib/say's flush files it here)
  const addJot = async () => {
    const text = jot.trim();
    if (!text) return;
    setJot('');
    jotRef.current?.focus();
    try {
      if (!online) throw new TypeError('offline');
      const r = await post<{ id: string }>('/api/ideas', { text, reason: `a jot on the topic ${topic?.text || ''}`.slice(0, 200) });
      await rant.jot(tid, r.id);
      haptics.success();
      ui.snack('Jotted', async () => { await rant.mark(tid, r.id, null); await pg.reload(); });
      void pg.reload();
    } catch (e) {
      if (e instanceof ApiError) { setJot(text); fail('Couldn’t jot it', e); return; }
      try {
        await sayOrQueue(text, { kind: 'bit', context: { screen: `topic:${tid}` } });
        ui.snack('Jotted on this phone. It saves when you’re back online.');
      } catch (e2) { setJot(text); fail('Couldn’t jot it', e2); }
    }
  };
  const rename = async () => {
    if (!topic) return;
    const t = await askText('Rename the topic', topic.text);
    if (!t || t === topic.text) return;
    try { await rant.rename(tid, t); void pg.reload(); } catch (e) { fail('Couldn’t rename it', e); }
  };
  const del = async () => {
    try {
      await rant.deleteTopic(tid);
      haptics.warning();
      scripts.forget(tid);
      router.back();
      ui.snack('Topic deleted. Its bits are untouched.', async () => { await rant.undelete(tid); topicsChanged(); });
    } catch (e) { fail('Couldn’t delete it', e); }
  };
  const anotherRant = () => router.push({ pathname: '/topic/[topic]/script', params: { topic: tid, fresh: '1' } } as any);
  const writeIt = () => router.push({ pathname: '/topic/[topic]/script', params: { topic: tid } } as any);
  const menu = () => actionSheet({ options: [
    { label: 'Rename…', onPress: () => void rename() },
    { label: 'Start another rant', onPress: anotherRant },
    { label: 'Delete topic', destructive: true, onPress: () => void del() },
    { label: 'Cancel', cancel: true },
  ] });

  const title = topic?.text || '';
  const long = title.length > BAR_CHARS;
  const loading = !page && !pg.rawError;
  const card = (m: R.Mat, state: R.State | null, key: string) => (
    <MaterialCard key={key} testID={`mat-${m.ref}`} m={m} state={state} selected={sel === m.ref} open={!!open[m.ref]}
      onSelect={(on) => setSel(on ? m.ref : null)} onUnfold={() => setOpen((o) => ({ ...o, [m.ref]: true }))}
      href={m.kind === 'moment' ? momentHref(m as any) || '/' : `/topic/${enc(tid)}/bit/${enc(m.ref)}`}
      acts={{ onKeep: () => void keep(m), onOpen: () => openMat(m), onToRant: () => void toRant(m), onDrop: () => drop(m), onPutBack: () => void putBack(m) }} />
  );
  const keptN = rows.kept.length + pendingJots.length;
  const foundN = rows.found.length;
  const nothing = !!page && !keptN && !foundN && !rows.dropped.length && !theRant;
  const writeLabel = theRant ? 'Keep writing' : 'Write the rant';

  return (
    <Screen title={long ? `${title.slice(0, BAR_CHARS - 1).trimEnd()}…` : title} testID="topic" backLabel="Rant" onRefresh={() => pg.reload()}
      search={native ? { placeholder: 'Find a bit to add', hideWhenScrolling: true, placement: 'stacked', autoCapitalize: 'none', onChangeText: (e: any) => setQ(e.nativeEvent.text || ''), onCancelButtonPress: () => setQ('') } as any : undefined}
      right={native ? undefined : <Btn small label="More" onPress={menu} />}>
      {native && (
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Menu icon="ellipsis" accessibilityLabel="More">
            <Stack.Toolbar.MenuAction icon="pencil" onPress={() => void rename()}>Rename…</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="plus" onPress={anotherRant}>Start another rant</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.Menu inline>
              <Stack.Toolbar.MenuAction icon="trash" destructive onPress={() => void del()}>Delete topic</Stack.Toolbar.MenuAction>
            </Stack.Toolbar.Menu>
          </Stack.Toolbar.Menu>
        </Stack.Toolbar>
      )}
      {native && (
        // HIG/toolbars: one prominent action on the page, trailing
        <Stack.Toolbar placement="bottom">
          <Stack.Toolbar.Spacer />
          <Stack.Toolbar.Button variant="prominent" icon="pencil" accessibilityLabel={writeLabel} onPress={writeIt}>{writeLabel}</Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}
      {long && <Text testID="topic-title" accessibilityRole="header" maxFontSizeMultiplier={1.8} style={{ fontFamily: F.g7, fontSize: 28, lineHeight: 34, letterSpacing: -0.6, color: C.text }}>{title}</Text>}
      {!native && (
        <TextInput testID="topic-search" value={q} onChangeText={setQ} placeholder="Find a bit to add" placeholderTextColor={C.dim} accessibilityLabel="Find a bit to add"
          style={{ minHeight: 44, borderRadius: 10, paddingHorizontal: 12, backgroundColor: C.card, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      )}
      {loading && <Skeleton lines={5} height={20} testID="topic-skeleton" />}
      {pg.rawError && !page ? <T v="mut" testID="topic-error">Can’t reach Punchable: {pg.rawError}</T> : null}

      {hits && q.trim() && hits.q === q.trim() ? (
        <View style={{ gap: 10 }} testID="topic-results">
          <Eb>{hits.rows.length ? `Yours · ${hits.rows.length}` : `Nothing of yours says “${hits.q}”`}</Eb>
          {hits.rows.map((m) => card(m, stateOf(m.ref) ?? (m.state || null), `s-${m.ref}`))}
        </View>
      ) : (
        <>
          {theRant && (
            <Pressable testID="topic-rant" accessibilityRole="button" accessibilityHint="Opens the script" onPress={writeIt}
              style={({ pressed }) => ({ borderRadius: 16, padding: 14, gap: 6, backgroundColor: pressed ? C.card2 : C.card })}>
              <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 13, color: C.mut2 }}>
                {['The rant', `v${theRant.v}`, monthDay(theRant.at), R.runOf(theRant.words)].filter(Boolean).join(' · ')}
              </Text>
              {theRant.lines.slice(0, 3).map((l, i) => (
                <Text key={i} maxFontSizeMultiplier={PROSE_CAP} style={{ fontFamily: l.punch ? F.bold : F.body, fontSize: 17, lineHeight: 24, color: C.text }}>{l.text}</Text>
              ))}
              {theRant.lines.length > 3 && <T v="small">{`+ ${theRant.lines.length - 3} more ${theRant.lines.length - 3 === 1 ? 'line' : 'lines'}`}</T>}
            </Pressable>
          )}
          {theRant && rantTakes.length > 0 && (
            <View style={{ marginTop: -10, paddingHorizontal: 14, borderBottomLeftRadius: 16, borderBottomRightRadius: 16, backgroundColor: C.card }}>
              {rantTakes.map((t) => (
                <Pressable key={t.id} testID={`topic-take-${t.id}`} accessibilityRole="button" onPress={() => router.push(`/take/${enc(t.id)}` as any)}
                  style={{ flexDirection: 'row', alignItems: 'center', minHeight: 44, borderTopWidth: 1, borderTopColor: C.hairline }}>
                  <T v="mut" style={{ flex: 1 }}>{`Filmed ${R.filmedSub(t).split(' · in Photos')[0]}`}</T>
                  <Icon name="chevron" size={16} color={C.dim} />
                </Pressable>
              ))}
            </View>
          )}
          {(page?.rants || []).slice(1).map((r) => (
            <Pressable key={r.id} testID={`topic-rant-${r.id}`} accessibilityRole="button" onPress={() => router.push(`/topic/${enc(tid)}/bit/${enc(r.id)}` as any)}
              style={{ flexDirection: 'row', alignItems: 'center', minHeight: 44 }}>
              <T v="mut" style={{ flex: 1 }}>{`Rant · ${monthDay(r.at)} · v${r.v}`}</T>
              <Icon name="chevron" size={16} color={C.dim} />
            </Pressable>
          ))}

          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 48, borderRadius: 12, paddingHorizontal: 12, backgroundColor: C.card }}>
            <Icon name="write" size={18} color={C.mut2} />
            <TextInput ref={jotRef} testID="topic-jot" value={jot} onChangeText={setJot} placeholder="Jot a thought" placeholderTextColor={C.dim}
              returnKeyType="done" submitBehavior="submit" onSubmitEditing={() => void addJot()} accessibilityLabel="Jot a thought on this topic"
              maxLength={4000} maxFontSizeMultiplier={1.6} style={{ flex: 1, minHeight: 44, color: C.text, fontFamily: F.body, fontSize: 16 }} />
          </View>

          {nothing && <T v="mut" testID="topic-empty">Nothing of yours matches yet. Jot a thought, or search above.</T>}

          {keptN > 0 && <Eb testID="topic-kept">{`Kept · ${keptN}`}</Eb>}
          {pendingJots.map((j) => (
            <View key={j.key} style={{ borderRadius: 14, padding: 12, gap: 4, backgroundColor: C.card }}>
              <T v="small">Jot · saving…</T>
              <Text maxFontSizeMultiplier={PROSE_CAP} style={{ fontFamily: F.body, fontSize: 16, lineHeight: 22, color: C.text }}>{j.text}</Text>
            </View>
          ))}
          {rows.kept.map((m) => card(m, 'kept', `k-${m.ref}`))}

          {foundN > 0 && <Eb testID="topic-found">{`Found · ${R.foundWord(foundN, !!page?.found_more && !extra.length)}`}</Eb>}
          {rows.found.map((m) => card(m, null, `f-${m.ref}`))}
          {(foundN > 0 || keptN > 0) && !more.done && (
            <Btn testID="topic-more" label={!online ? 'Find more (offline)' : more.busy ? 'Looking…' : 'Find more'} disabled={!online || more.busy} onPress={() => void findMore()} />
          )}

          {rows.dropped.length > 0 && <Eb testID="topic-dropped">{`Dropped · ${rows.dropped.length}`}</Eb>}
          {rows.dropped.map((m) => card(m, 'dropped', `d-${m.ref}`))}
        </>
      )}

      {!native && (
        <View style={{ alignItems: 'flex-end' }}>
          <Btn v="primary" testID="topic-write" label={writeLabel} onPress={writeIt} />
        </View>
      )}
    </Screen>
  );
}
