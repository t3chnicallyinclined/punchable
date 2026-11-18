// Notebook: the app's one list (docs/design/SPEC-2026-11-17-one-notebook.md screen 1; owner on build 33: "people are
// used to google docs/notes how can we simplify it but keep it as powerful"). No tabs. Every bit, set and tape, newest
// first, under its day; a search over all of it (the system's field on the phone; a question goes to the
// conversation); a filter All · Bits · Sets · Tapes; a Pinned section (hold a row: Pin); the avatar opens You and
// Settings. At the bottom: the microphone (talk out a new page: a memo, whose words become a page), the count, and the
// pencil (a blank page with the keyboard up; a new set while the filter is on Sets).
// A row is a title, one line under it (a sentence of his from the page: the server's `punch`, an exact substring of
// the newest version; never the app's wording), and a status line only when there is something to say: "New draft
// ready" (GET /api/redlines; the page's own button then opens the draft), "2 marks · 1 tape" (the marks kept on this phone, lib/marks; the
// recordings of it on this phone). A recording kept on this phone draws itself (lib/RecRows: its state, play, Delete).
// What the old tab roots' toolbars held is in ⋯: every bit with sort and merge, every set with its drafts, the inbox,
// the merge review, the joke map, Record a set, Import a recording, Archived sets.
// The reads are the tabs' own: GET /api/jokes (sort=recent; q + mode=hybrid for a search), /api/search (taped
// moments), /api/boards, /api/topics, /api/sets, /api/redlines, /api/tonight/screen. Rules: rules/notebookRules.ts.
import React, { memo, useCallback, useEffect, useMemo, useState } from 'react';
import { Platform, Pressable, SectionList, Text, TextInput, View } from 'react-native';
import { Stack, router, useFocusEffect, useLocalSearchParams } from 'expo-router';
import type { SearchBarProps } from 'react-native-screens';
import { JokeSummary, get, useApi } from '@/lib/api';
import { BarGround } from '@/lib/BarGround';
import { useBoards } from '@/lib/boards';
import { BoardListItem, creative, useCreative } from '@/lib/creative';
import { allDraftsRaw } from '@/lib/draftFiles';
import { haptics } from '@/lib/haptics';
import { VoiceMemos } from '@/lib/memos';
import { MomentRow } from '@/lib/MomentRow';
import { newBit } from '@/lib/newBit';
import { newSetList } from '@/lib/newSetList';
import { getPrefs, onPrefs, setPrefs } from '@/lib/prefs';
import { useRecordings } from '@/lib/recordings';
import { RecRow } from '@/lib/RecRows';
import type { Waiting } from '@/lib/redline';
import { RowMenu, rowActions } from '@/lib/RowMenu';
import { isQuestion } from '@/lib/say';
import { Screen } from '@/lib/Screen';
import { useSets } from '@/lib/sets';
import { C, F, linkColor } from '@/lib/theme';
import { useTonight } from '@/lib/tonight';
import { Avatar, Empty, ErrorCard, Icon, IconBtn, MenuItem, MenuSheet, Seg, Skeleton, T, TextBtn, useRefresh, useUI, useWide } from '@/lib/ui';
import { fmtShortDate, mmss } from '@/rules/format';
import { parseMarkSet } from '@/rules/markRules';
import * as N from '@/rules/notebookRules';
import type { Topic } from '@/rules/rantRules';
import type { SearchResult } from '@/rules/searchRows';
import { splitLists } from '@/rules/setListRules';
import { tapeKind, tapeRows, topicsNotYetSets, type PhoneTape, type ServerTape } from '@/rules/tapesRules';

const native = Platform.OS !== 'web';
const ios = Platform.OS === 'ios'; // the system context menu owns the long press there
const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
/** "Sunday" within the last week, else "Nov 9". */
function tapedWord(at: string, now: Date): string {
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return '';
  return now.getTime() - d.getTime() < 6 * 86400000 ? WEEKDAYS[d.getDay()] : fmtShortDate(at);
}

const NbRow = memo(function NbRow({ r, pinned, onPin, onMenu }: { r: N.Row; pinned: boolean; onPin: (key: string) => void; onMenu: (r: N.Row) => void }) {
  const items: MenuItem[] = [{ label: pinned ? 'Unpin' : 'Pin', icon: pinned ? 'pin.slash' : 'pin', onPress: () => onPin(r.key) }];
  const row = (
    <Pressable testID={`nb-${r.key}`} accessibilityRole="button"
      accessibilityLabel={[r.title, r.tag, r.line, r.status].filter(Boolean).join('. ')} accessibilityHint={pinned ? 'Pinned. Opens it' : 'Opens it'}
      onPress={() => r.href && router.push(r.href as any)} onLongPress={ios ? undefined : () => onMenu(r)} delayLongPress={350} {...rowActions(items)}
      style={(s: any) => ({ minHeight: 56, paddingVertical: 12, gap: 3, borderBottomWidth: 1, borderBottomColor: C.hairline, backgroundColor: s.pressed || s.hovered ? C.card2 : 'transparent' })}>
      <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 8 }}>
        <Text numberOfLines={1} maxFontSizeMultiplier={1.6} style={{ flex: 1, minWidth: 0, fontFamily: F.semi, fontSize: 17, color: C.text }}>{r.title}</Text>
        {!!r.tag && <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 13, color: linkColor() }}>{r.tag}</Text>}
      </View>
      {!!r.line && <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 15, color: C.mut2 }}>{r.line}</Text>}
      {!!r.status && <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: r.hot ? F.semi : F.body, fontSize: 13, color: r.hot ? linkColor() : C.mut2 }}>{r.status}</Text>}
    </Pressable>
  );
  return r.href ? <RowMenu href={r.href} items={items}>{row}</RowMenu> : row;
});

export default function Notebook() {
  const p = useLocalSearchParams<{ show?: string }>();
  const wide = useWide();
  const ui = useUI();
  const cur = useBoards();
  const [filter, setFilter] = useState<N.Filter>(() => N.showOf(p.show));
  // an old tab's URL (/sets …) pops here with its filter (lib/PopTo): the filter follows the URL when it changes
  const [asked, setAsked] = useState(p.show);
  if (p.show !== asked) {
    setAsked(p.show);
    if (p.show) setFilter(N.showOf(p.show));
  }
  const [q, setQ] = useState('');
  const [dq, setDq] = useState('');
  useEffect(() => {
    const t = setTimeout(() => setDq(q.trim()), 300);
    return () => clearTimeout(t);
  }, [q]);
  const question = dq.length >= 3 && isQuestion(dq);
  // the pages: a search is the server's (by meaning and by word), the plain list its "recent" order
  const list = useApi<{ items: JokeSummary[]; total: number }>(question ? null : `/api/jokes?sort=recent&limit=500${dq ? `&q=${encodeURIComponent(dq)}&mode=hybrid` : ''}`, { poll: 60000 });
  // the taped moments that say the same words
  const found = useApi<SearchResult>(dq && !question ? `/api/search?q=${encodeURIComponent(dq)}&limit=6` : null);
  const moments = dq && !question ? found.data?.moments || [] : [];
  const boards = useCreative<{ boards: BoardListItem[] }>('boards', creative.boards, { poll: 30000 });
  // the old Rant tab's topics that no set was made from yet (the archived sets count: an archived one still means "made")
  const tp = useApi<{ topics: Topic[] }>('/api/topics');
  const arch = useApi<{ boards: BoardListItem[] }>(tp.data?.topics?.length ? '/api/boards?archived=1' : null);
  const sets = useSets();
  const recs = useRecordings();
  const drafts = useApi<{ open: Waiting[] }>('/api/redlines', { poll: 60000, cache: false });
  const tonight = useTonight();
  const toSort = tonight.data?.to_sort || 0;
  const [pairs, setPairs] = useState(0);
  useEffect(() => {
    let alive = true;
    get<{ groups: any[] }>('/api/merge/candidates').then((r) => alive && setPairs((r.groups || []).length)).catch(() => {});
    return () => { alive = false; };
  }, []);
  const [pins, setPins] = useState<string[]>(() => getPrefs().pins);
  useEffect(() => onPrefs((x) => setPins(x.pins)), []);
  const togglePin = useCallback((key: string) => { haptics.selection(); setPrefs({ pins: N.togglePin(getPrefs().pins, key) }); }, []);
  // the marks he has not sent, per page (kept on this phone: lib/marks), read again each time the list is in front
  const [markCounts, setMarkCounts] = useState<Record<string, number>>({});
  const reloads = { list: list.reload, boards: boards.reload, sets: sets.reload, drafts: drafts.reload };
  useFocusEffect(useCallback(() => {
    let alive = true;
    void allDraftsRaw('marks').then((raws) => {
      if (!alive) return;
      const next: Record<string, number> = {};
      for (const raw of raws) { const s = parseMarkSet(raw); if (s && !s.on && s.marks.length) next[s.bit] = s.marks.length; }
      setMarkCounts(next);
    });
    void reloads.drafts(); void reloads.boards();
    return () => { alive = false; };
  }, [])); // eslint-disable-line react-hooks/exhaustive-deps
  const [menu, setMenu] = useState<{ title: string; items: MenuItem[] } | null>(null);
  // one function for every row, so a keystroke in the search does not re-draw the rows on screen (memo)
  const rowMenu = useCallback((r: N.Row) => setMenu({ title: r.title, items: [{ label: getPrefs().pins.includes(r.key) ? 'Unpin' : 'Pin', onPress: () => togglePin(r.key) }] }), [togglePin]);
  const now = useMemo(() => new Date(), [list.data, boards.data]); // eslint-disable-line react-hooks/exhaustive-deps

  const rows = useMemo(() => {
    const roundOf = new Map((drafts.data?.open || []).map((w) => [w.joke, w]));
    const tapesOf: Record<string, number> = {};
    const lastTaped: Record<string, string> = {};
    for (const r of recs) {
      const bit = r.bit?.id || r.filed_on?.id;
      if (bit) tapesOf[bit] = (tapesOf[bit] || 0) + 1;
      if (r.board?.id && (!lastTaped[r.board.id] || r.recorded_at > lastTaped[r.board.id])) lastTaped[r.board.id] = r.recorded_at;
    }
    const bits = (list.data?.items || []).map((j) => {
      const w = roundOf.get(j.id);
      return N.bitRow(j as N.BitLike, { round: w?.state ?? null, sentAt: w?.state === 'ready' ? w.sent_at : null, marks: markCounts[j.id] || 0, tapes: tapesOf[j.id] || 0 });
    });
    const all = boards.data?.boards || [];
    const { his } = splitLists(all);
    const setRows = his.map((b) => N.setRow(b, lastTaped[b.id] ? tapedWord(lastTaped[b.id], now) : null));
    const topicRows = topicsNotYetSets(tp.data?.topics || [], [...all, ...(arch.data?.boards || [])]).map(N.topicRow);
    const server: ServerTape[] = sets.list.map((x) => ({ id: x.id, title: x.title || x.venue?.name || 'Set', date: x.show?.date || null }));
    const byId = new Map(sets.list.map((x) => [x.id, x]));
    const tapes = tapeRows(recs as PhoneTape[], server).map((t) => {
      if (t.from === 'phone') return N.tapeRow({ from: 'phone', id: t.rec.id, title: t.rec.title || t.rec.board?.title || tapeKind(t), at: t.at, kind: tapeKind(t) });
      const x = byId.get(t.set.id);
      const line = [tapeKind(t), t.set.date ? fmtShortDate(t.set.date) : null, x ? mmss(x.card.duration_s) : null, x?.best ? x.best.title.split(' (')[0] : null].filter(Boolean).join(' · ');
      return N.tapeRow({ from: 'server', id: t.set.id, title: t.set.title, at: t.at, line });
    });
    return { bits, others: [...setRows, ...topicRows, ...tapes] };
  }, [list.data, boards.data, tp.data, arch.data, sets.list, recs, drafts.data, markCounts, now]);

  // a search keeps the server's order for the pages (by relevance) and reads the other rows' titles and lines here
  const searching = !!dq && !question;
  const nb = useMemo(() => (searching
    ? { pinned: [] as N.Row[], groups: [{ title: 'Found', rows: N.found(rows.bits, rows.others, dq, filter) }], count: 0 }
    : N.notebook([...rows.bits, ...rows.others], { filter, pins, now })), [rows, searching, dq, filter, pins, now]);
  const sections = useMemo(() => [
    ...(nb.pinned.length ? [{ title: 'Pinned', data: nb.pinned }] : []),
    ...nb.groups.filter((g) => g.rows.length).map((g) => ({ title: g.title, data: g.rows })),
  ], [nb]);
  const shown = sections.reduce((n, s) => n + s.data.length, 0);

  const askIt = useCallback(() => router.push({ pathname: '/say', params: { text: dq, screen: 'bits' } } as any), [dq]);
  // the microphone: talk out a new page (a memo recorded here; its words become a page, or are filed on the bit they fit)
  const talk = () => ui.withMic(() => router.push({ pathname: '/record', params: { kind: 'memo' } } as any));
  // the pencil: a blank page with the keyboard up (lib/newBit); a new set while the filter is on Sets (lib/newSetList)
  const makes = N.pencilMakes(filter);
  const [making, setMaking] = useState(false);
  const pencil = async () => {
    if (making) return;
    setMaking(true);
    try {
      if (makes === 'set') {
        if (await newSetList({ titles: (boards.data?.boards || []).map((x) => x.title), setCurrent: cur.setCurrent, snack: ui.snack })) void boards.reload();
      } else await newBit(ui.snack, 'Bits');
    } finally {
      setMaking(false);
    }
  };
  const refreshControl = useRefresh(useCallback(async () => { await Promise.all([reloads.list(), reloads.boards(), reloads.sets?.(), reloads.drafts()]); }, [])); // eslint-disable-line react-hooks/exhaustive-deps

  // ⋯: what the three tab roots' toolbars held (HIG/menus: navigation without an ellipsis, a task that asks more with one)
  const more: MenuItem[] = [
    { label: 'All bits · sort, merge', icon: 'square.grid.2x2', onPress: () => router.push('/allbits' as any), testID: 'nb-allbits' },
    { label: 'All sets · drafts', icon: 'list.bullet.rectangle', onPress: () => router.push('/allsets' as any), testID: 'nb-allsets' },
    { label: toSort ? `Notes to sort · ${toSort}` : 'Notes to sort', icon: 'tray', onPress: () => router.push('/inbox' as any), testID: 'nb-inbox' },
    ...(pairs ? [{ label: `Same idea twice · ${pairs}`, icon: 'arrow.triangle.merge', onPress: () => router.push('/merge' as any), testID: 'nb-merge' }] : []),
    { label: 'Joke map', icon: 'map', onPress: () => router.push('/map' as any), testID: 'nb-map' },
    { label: 'Record a set', icon: 'record.circle', onPress: () => ui.withMic(() => router.push('/record' as any)), testID: 'nb-record' },
    { label: 'Import a recording…', icon: 'square.and.arrow.down', onPress: () => router.push('/import' as any), testID: 'nb-import' },
    { label: 'Archived sets', icon: 'archivebox', onPress: () => router.push('/archived' as any), testID: 'nb-archived' },
  ];

  // HIG/search-fields (phone): the system search bar under the large title; `stacked` so it stays put while the list
  // scrolls (iOS 26 may move an `automatic` one into the bottom bar, where the two buttons are)
  const search: SearchBarProps | undefined = native ? {
    placeholder: 'Search everything you wrote or said', placement: 'stacked', hideWhenScrolling: false, autoCapitalize: 'none', tintColor: C.amber,
    onChangeText: (e) => setQ(e.nativeEvent.text), onSearchButtonPress: () => { if (question) askIt(); }, onCancelButtonPress: () => setQ(''),
  } : undefined;

  const header = (
    <View style={{ gap: 12, paddingBottom: 4 }}>
      {!native && (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44, borderRadius: 12, backgroundColor: C.card, paddingHorizontal: 12 }}>
          <Icon name="search" size={16} color={C.mut2} />
          <TextInput testID="nb-search" value={q} onChangeText={setQ} placeholder="Search everything you wrote or said" placeholderTextColor={C.dim} returnKeyType="search"
            onSubmitEditing={() => question && askIt()} accessibilityLabel="Search everything you wrote or said" maxFontSizeMultiplier={1.4}
            style={[{ flex: 1, minWidth: 0, minHeight: 44, color: C.text, fontFamily: F.body, fontSize: 16 }, { outlineStyle: 'none' } as any]} />
          {!!q && <TextBtn testID="nb-search-clear" label="Clear" onPress={() => setQ('')} />}
        </View>
      )}
      {/* HIG/segmented-controls: four views of one list (the mockup's chips; a chip is not a system control) */}
      <Seg<N.Filter> testID="nb-filter" fit items={N.FILTERS} value={filter} onChange={(v) => { haptics.selection(); setFilter(v); }} />
      {question && (
        <Pressable testID="nb-ask" accessibilityRole="button" onPress={askIt} style={{ minHeight: 44, padding: 12, borderRadius: 14, backgroundColor: C.card2, flexDirection: 'row', alignItems: 'center', gap: 10 }}>
          <Icon name="chat" size={18} color={C.amber} /><T style={{ flex: 1 }}>Ask your material: “{dq}”</T><T style={{ color: linkColor() }}>Ask ›</T>
        </Pressable>
      )}
      {!dq && toSort > 0 && (filter === 'all' || filter === 'bits') && (
        // HIG/lists-and-tables: a disclosure row to the inbox; the count says it, the dot is not the only signal
        <Pressable testID="nb-to-sort" accessibilityRole="button" accessibilityLabel={`${toSort} notes to sort. Open the inbox`} onPress={() => router.push('/inbox' as any)}
          style={{ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 10 }}>
          <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: C.amber }} />
          <Text maxFontSizeMultiplier={1.4} style={{ flex: 1, fontFamily: F.semi, fontSize: 16, color: C.text }}>{toSort} {toSort === 1 ? 'note' : 'notes'} to sort</Text>
          <Icon name="chevron" size={18} color={C.dim} />
        </Pressable>
      )}
      {!!moments.length && (filter === 'all' || filter === 'tapes') && (
        <View testID="nb-moments" style={{ gap: 6 }}>
          <T v="eb">Said on stage</T>
          {moments.map((m, i) => <MomentRow key={m.id} m={m} testID={`nb-moment-${i}`} />)}
        </View>
      )}
    </View>
  );

  const loading = (list.loading && !list.data) || (boards.loading && !boards.data);
  const footer = (
    <View style={{ paddingTop: 8, gap: 12 }}>
      {/* the earlier voice memos the server holds (before memos were recorded in the app): with the tapes */}
      {filter === 'tapes' && !dq && <VoiceMemos />}
    </View>
  );
  const bar = (
    // the mockup's bottom row: talk it out (leading), the count, a new page (trailing, the one filled button)
    <BarGround testID="nb-bar">
      <View style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', paddingHorizontal: 20, paddingTop: 8, paddingBottom: 6 }}>
        <Pressable testID="nb-talk" accessibilityRole="button" accessibilityLabel="Talk it out" accessibilityHint="Records a memo. Its words become a new page" onPress={talk}
          style={({ pressed }) => ({ width: 56, height: 56, borderRadius: 28, alignItems: 'center', justifyContent: 'center', backgroundColor: pressed ? C.card2 : C.card })}>
          {/* the label colour, not red: red means "recording now" (HIG/color) */}
          <Icon name="mic" size={24} color={C.text} />
        </Pressable>
        <Text testID="nb-count" maxFontSizeMultiplier={1.3} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>{searching ? `${shown} found` : N.countLabel(nb.count, filter)}</Text>
        <Pressable testID="nb-new" accessibilityRole="button" accessibilityLabel={makes === 'set' ? 'New set' : 'New page'} accessibilityHint={makes === 'set' ? 'Starts a set and opens it' : 'Opens a blank page with the keyboard up'}
          accessibilityState={{ busy: making }} onPress={() => void pencil()}
          style={({ pressed }) => ({ width: 56, height: 56, borderRadius: 28, alignItems: 'center', justifyContent: 'center', backgroundColor: C.amber, opacity: pressed || making ? 0.8 : 1 })}>
          <Icon name={makes === 'set' ? 'plus' : 'write'} size={24} color={C.bg} sw={2.4} />
        </Pressable>
      </View>
    </BarGround>
  );

  return (
    <Screen title="Notebook" testID="notebook" largeTitle search={search} scroll={false} bar={bar}
      right={native ? undefined : (
        <View style={{ flexDirection: 'row', alignItems: 'center' }}>
          <IconBtn testID="nb-more" label="More" onPress={() => setMenu({ title: 'Notebook', items: more })}><Icon name="more" size={22} color={C.amber} /></IconBtn>
          <Avatar ask={false} />
        </View>
      )}>
      {native && (
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Menu icon="ellipsis" accessibilityLabel="More">
            {more.map((it) => <Stack.Toolbar.MenuAction key={it.label} icon={it.icon as any} onPress={it.onPress}>{it.label}</Stack.Toolbar.MenuAction>)}
          </Stack.Toolbar.Menu>
          {/* You and Settings behind the avatar (SPEC 2026-11-16 Slice 6) */}
          <Stack.Toolbar.View><Avatar ask={false} /></Stack.Toolbar.View>
        </Stack.Toolbar>
      )}
      <SectionList contentInsetAdjustmentBehavior="automatic" sections={sections} keyExtractor={(r) => r.key} stickySectionHeadersEnabled={false}
        renderItem={({ item }) => {
          if (item.rec) {
            const rec = recs.find((x) => x.id === item.rec);
            return rec ? <View testID={`nb-${item.key}`}><RecRow r={rec} /></View> : null;
          }
          return <NbRow r={item} pinned={pins.includes(item.key)} onPin={togglePin} onMenu={rowMenu} />;
        }}
        renderSectionHeader={({ section }) => (
          <Text accessibilityRole="header" maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 13, letterSpacing: 0.4, textTransform: 'uppercase', color: C.mut2, paddingTop: 18, paddingBottom: 6, backgroundColor: C.bg }}>{section.title}</Text>
        )}
        initialNumToRender={16} windowSize={9} keyboardShouldPersistTaps="handled" refreshControl={refreshControl}
        contentContainerStyle={{ paddingHorizontal: wide ? 32 : 20, paddingTop: 12, paddingBottom: 24, maxWidth: wide ? 1120 : 720, width: '100%', alignSelf: 'center' }}
        ListHeaderComponent={header} ListFooterComponent={footer}
        ListEmptyComponent={loading ? <Skeleton lines={8} height={18} testID="nb-skeleton" />
          : list.rawError && !list.data ? <ErrorCard error={list.rawError} onRetry={list.reload} />
          : <Empty text={dq ? 'Nothing matches.' : filter === 'tapes' ? 'Nothing recorded yet. Open a set and tap Record, or tap the microphone.' : filter === 'sets' ? 'No sets yet. Tap + to start one.' : 'Nothing here yet. Tap the pencil to write the first page.'} />}
      />
      <MenuSheet visible={!!menu} title={menu?.title} items={menu?.items || []} onClose={() => setMenu(null)} testID="nb-menu" />
    </Screen>
  );
}
