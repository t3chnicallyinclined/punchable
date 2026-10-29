// Bits (MixBits, docs/UX_2026-10.md): every bit as a list, a status dot (kills · working · fresh · resting) and how
// long it runs; a search box that also takes a question (a question goes to the conversation); Bits · Notes · Map;
// "N pairs look like the same idea → Review" when the merge judge found some. One list request, virtualized.
// A search also asks /api/search for the taped moments that match ("In your sets": lib/MomentRow), so a word he half
// remembers saying on stage comes back with where and when, not only the bits that carry it.
// 2026-10-29 (owner: "like Google Docs"): the list opens on what he touched last (sort=recent, the server's
// `touched` timestamp: his last add/edit/confirm, else the newest version, night or first day); Performed and
// Strongest stay as choices in the toolbar's sort menu (HIG/pull-down-buttons: a sort is a menu, not a segmented
// control, which is for views). "+" in the navigation bar makes a new bit (the composer, like Tonight's "+").
import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { FlatList, PixelRatio, Platform, Pressable, Text, TextInput, View } from 'react-native';
import { Stack, router } from 'expo-router';
import type { SearchBarProps } from 'react-native-screens';
import { ApiError, JokeSummary, get, useApi } from '@/lib/api';
import { haptics } from '@/lib/haptics';
import { fmtDur } from '@/lib/laughs';
import { VoiceMemos } from '@/lib/memos';
import { MomentRow } from '@/lib/MomentRow';
import type { SearchResult } from '@/lib/searchRows';
import { isQuestion, shortNameOf } from '@/lib/say';
import { C, F } from '@/lib/theme';
import { Avatar, Btn, Empty, ErrorCard, Icon, IconBtn, Seg, Skeleton, T, TextBtn, useRefresh, useWide } from '@/lib/ui';
import { getPrefs, setPrefs } from '@/lib/prefs';
import { mergeInto, pickMain, undoMerge } from '@/lib/bitWork';
import { actionSheet } from '@/lib/actionSheet';
import { useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

type View_ = 'bits' | 'notes';
type Sort = 'recent' | 'performed' | 'strongest';
const SORT_WORD: Record<Sort, string> = { recent: 'Recent', performed: 'Performed', strongest: 'Strongest' };
const SORTS: Sort[] = ['recent', 'performed', 'strongest'];
const native = Platform.OS !== 'web';
type Status = 'kills' | 'working' | 'fresh' | 'resting';
const STATUSES: Status[] = ['kills', 'working', 'fresh', 'resting'];
// read at render so the Light / Dark switch reaches the dots (theme.ts tokens with light variants)
const dotOf = (st: Status) => ({ kills: C.statusKills, working: C.statusWorking, fresh: C.statusFresh, resting: C.statusResting }[st]);
const WORD: Record<Status, string> = { kills: 'Kills', working: 'Working', fresh: 'Fresh', resting: 'Resting' };
// a glyph next to the dot, so the status is never colour alone (HIG)
const GLYPH: Record<Status, string> = { kills: '★', working: '●', fresh: '○', resting: '–' };
// row height scales with the OS text size (fixed 64 clipped at large text)
const ROW_H = Math.round(64 * Math.min(1.6, Math.max(1, PixelRatio.getFontScale())));

/** One word from his tapes and his stage: kills (big laughs, more than once) · working (on stage) · fresh (never tried) · resting (retired). */
export function statusOf(j: JokeSummary): Status {
  if (j.stage === 'retired') return 'resting';
  if ((j.hit_sets || 0) >= 2 || (j.avg_vs_room != null && j.avg_vs_room >= 1.3 && (j.sets || 0) >= 2)) return 'kills';
  if ((j.sets || 0) > 0 || j.stage === 'working') return 'working';
  return 'fresh';
}
function subOf(j: JokeSummary, st: Status) {
  const bits = [WORD[st]];
  if (j.sets) bits.push(`${j.sets} set${j.sets === 1 ? '' : 's'}`);
  if (j.versions > 1) bits.push(`${j.versions} versions`);
  if (!j.sets) bits.push('never performed');
  return bits.join(' · ');
}

const Row = memo(function Row({ j, selecting, selected, onToggle }: { j: JokeSummary; selecting?: boolean; selected?: boolean; onToggle?: (id: string) => void }) {
  const st = statusOf(j);
  const open = () => (selecting && onToggle ? onToggle(j.id) : router.push(`/joke/${encodeURIComponent(j.id)}` as any));
  return (
    <Pressable testID={`bit-${j.id}`} accessibilityRole={selecting ? 'checkbox' : 'button'} accessibilityState={selecting ? { checked: !!selected } : undefined}
      accessibilityLabel={`${j.short_title || j.title}. ${subOf(j, st)}. ${selecting ? (selected ? 'Selected' : 'Select') : 'Open'}`} onPress={open}
      style={(s: any) => ({ height: ROW_H, flexDirection: 'row', alignItems: 'center', gap: 12, paddingHorizontal: 4, borderBottomWidth: 1, borderBottomColor: C.hairline, backgroundColor: selected ? C.card2 : s.hovered ? C.card2 : 'transparent' })}>
      <View style={{ width: 18, alignItems: 'center' }} accessibilityLabel={selecting ? undefined : WORD[st]}>
        {selecting
          ? <View style={{ width: 20, height: 20, borderRadius: 10, borderWidth: 2, borderColor: selected ? C.amber : C.border, backgroundColor: selected ? C.amber : 'transparent', alignItems: 'center', justifyContent: 'center' }}>{selected && <Text style={{ color: C.bg, fontSize: 12, fontFamily: F.g7 }}>✓</Text>}</View>
          : <Text maxFontSizeMultiplier={1.2} style={{ fontSize: 12, color: dotOf(st), fontFamily: F.g7 }}>{GLYPH[st]}</Text>}
      </View>
      <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
        <Text numberOfLines={1} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.semi, fontSize: 16, color: C.text }}>{j.short_title || shortNameOf(j.title)}</Text>
        <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>{subOf(j, st)}</Text>
      </View>
      <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.medium, fontSize: 14, color: C.mut2, fontVariant: ['tabular-nums'] }}>{j.duration_s != null && j.sets ? fmtDur(j.duration_s) : j.duration_s != null ? `~${fmtDur(j.duration_s)}` : '—'}</Text>
    </Pressable>
  );
});

export default function Bits() {
  const wide = useWide();
  const [view, setView] = useState<View_>('bits');
  const [sort, setSortState] = useState<Sort>(() => getPrefs().bitsSort);
  const setSort = useCallback((k: Sort) => { setSortState(k); setPrefs({ bitsSort: k }); }, []);
  const [q, setQ] = useState('');
  const [dq, setDq] = useState('');
  const [focused, setFocused] = useState(false);
  const [selecting, setSelecting] = useState(false);            // work off a search list: pick bits, make them one
  const [picked, setPicked] = useState<string[]>([]);
  const ui = useUI();
  const input = useRef<any>(null);
  useEffect(() => {
    const t = setTimeout(() => setDq(q.trim()), 300);
    return () => clearTimeout(t);
  }, [q]);
  const question = dq.length >= 3 && isQuestion(dq);
  // a search is ranked by relevance (the server ignores sort when q is set and mode=hybrid); the plain list by his sort
  const list = useApi<{ items: JokeSummary[]; total: number }>(question ? null : `/api/jokes?sort=${sort}&limit=500${dq ? `&q=${encodeURIComponent(dq)}&mode=hybrid` : ''}`, { poll: 60000 });
  // the taped moments that match the same words (api-dev/search_api.py); the bits above come from the list request
  const found = useApi<SearchResult>(dq && !question ? `/api/search?q=${encodeURIComponent(dq)}&limit=6` : null);
  const moments = dq && !question ? found.data?.moments || [] : [];
  const [pairs, setPairs] = useState<number | null>(null);
  useEffect(() => {
    let alive = true;
    get<{ groups: any[] }>('/api/merge/candidates').then((r) => alive && setPairs((r.groups || []).length)).catch((e) => alive && setPairs(e instanceof ApiError ? 0 : null));
    return () => { alive = false; };
  }, []);
  const items = list.data?.items || [];
  useEffect(() => { if (!dq) { setSelecting(false); setPicked([]); } }, [dq]);
  const toggle = useCallback((jid: string) => setPicked((p) => (p.includes(jid) ? p.filter((x) => x !== jid) : [...p, jid])), []);
  const makeOne = () => {
    const chosen = picked.map((jid) => items.find((x) => x.id === jid)).filter(Boolean) as JokeSummary[];
    const main = pickMain(chosen);
    if (!main || chosen.length < 2) return;
    const others = chosen.filter((x) => x.id !== main.id).map((x) => x.id);
    const go = async () => {
      try {
        const r = await mergeInto(main.id, others, `made one bit from the search “${dq}”`);
        haptics.success();
        setSelecting(false); setPicked([]);
        list.reload();
        ui.snack(`${chosen.length} bits are now “${main.short_name || shortNameOf(main.title)}”`, async () => { await undoMerge(r.merge_id); list.reload(); });
        router.push(`/joke/${encodeURIComponent(main.id)}` as any);
      } catch (e: any) {
        ui.snack(`Couldn’t merge: ${e.message}`);
      }
    };
    if (native) actionSheet({ title: `Make ${chosen.length} bits one?`, message: `They become “${main.short_name || shortNameOf(main.title)}” (the one with the most versions). Every line and tape stays. You can undo it.`, options: [{ label: 'Make them one bit', onPress: go }, { label: 'Cancel', cancel: true }] });
    else go();
  };
  const counts = useMemo(() => {
    const c: Record<Status, number> = { kills: 0, working: 0, fresh: 0, resting: 0 };
    for (const j of items) c[statusOf(j)]++;
    return c;
  }, [items]);
  const askIt = useCallback(() => router.push({ pathname: '/say', params: { text: dq, screen: 'bits' } } as any), [dq]);
  const newBit = useCallback(() => router.push('/new-bit' as any), []);
  const refreshControl = useRefresh(useCallback(() => list.reload(), [list.reload]));
  // HIG/search-fields (phone): the system search bar in the header; it supplies Clear and Cancel and starts as he types
  const search: SearchBarProps | undefined = native ? {
    placeholder: 'Search your bits, or ask', placement: 'automatic', hideWhenScrolling: false, autoCapitalize: 'none', tintColor: C.amber,
    onChangeText: (e) => setQ(e.nativeEvent.text), onSearchButtonPress: () => { if (question) askIt(); }, onCancelButtonPress: () => setQ(''),
  } : undefined;
  // HIG/segmented-controls: Map is a navigation, not a view, so it is a header button, not a third segment
  const mapBtn = native ? (
    <Stack.Toolbar placement="right">
      {/* HIG/toolbars: the primary action ("+") at the trailing end; a pop-up for the sort, labelled with the current choice */}
      <Stack.Toolbar.Button icon="map" accessibilityLabel="Map of all your bits" onPress={() => router.push('/map' as any)}>Map</Stack.Toolbar.Button>
      <Stack.Toolbar.Menu icon="arrow.up.arrow.down" accessibilityLabel={`Sorted by ${SORT_WORD[sort]}. Change the order`}>
        {SORTS.map((k) => <Stack.Toolbar.MenuAction key={k} icon={k === sort ? 'checkmark' : undefined} onPress={() => { haptics.selection(); setSort(k); }}>{SORT_WORD[k]}</Stack.Toolbar.MenuAction>)}
      </Stack.Toolbar.Menu>
      <Stack.Toolbar.Button icon="plus" accessibilityLabel="New bit" onPress={newBit}>New bit</Stack.Toolbar.Button>
    </Stack.Toolbar>
  ) : null;
  const right = native ? undefined : (
    <View style={{ flexDirection: 'row', alignItems: 'center' }}>
      <IconBtn testID="bits-new" label="New bit" onPress={newBit}><Icon name="plus" size={24} color={C.amber} sw={2.4} /></IconBtn>
      <IconBtn testID="bits-map" label="Map of all your bits" onPress={() => router.push('/map' as any)}><Icon name="map" size={22} color={C.amber} /></IconBtn>
      <Avatar />
    </View>
  );

  const header = (
    <View style={{ gap: 10, paddingBottom: 6 }}>
      {!native && <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
        <View style={{ flex: 1, flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 48, borderRadius: 24, backgroundColor: C.card, paddingHorizontal: 14 }}>
          <Icon name="search" size={18} color={C.mut2} />
          <TextInput ref={input} testID="bits-search" value={q} onChangeText={setQ} placeholder="Search your bits, or ask" placeholderTextColor={C.dim} returnKeyType="search"
            onFocus={() => setFocused(true)} onBlur={() => setFocused(false)} clearButtonMode="never"
            onSubmitEditing={() => question && askIt()} accessibilityLabel="Search your bits, or ask" maxFontSizeMultiplier={1.4}
            style={{ flex: 1, minWidth: 0, minHeight: 48, color: C.text, fontFamily: F.body, fontSize: 16 }} />
          {!!q && <Pressable accessibilityRole="button" accessibilityLabel="Clear the search" onPress={() => setQ('')} hitSlop={8} style={{ width: 44, height: 44, marginRight: -10, alignItems: 'center', justifyContent: 'center' }}><Text style={{ color: C.mut2, fontSize: 18 }}>×</Text></Pressable>}
        </View>
        {(focused || !!q) && <TextBtn testID="bits-search-cancel" label="Cancel" onPress={() => { setQ(''); setFocused(false); input.current?.blur?.(); }} />}
      </View>}
      {question && (
        <Pressable testID="bits-ask" accessibilityRole="button" onPress={askIt} style={{ padding: 12, borderRadius: 14, backgroundColor: C.card2, flexDirection: 'row', alignItems: 'center', gap: 10 }}>
          <Icon name="chat" size={18} color={C.amber} /><T style={{ flex: 1 }}>Ask your material: “{dq}”</T><T style={{ color: C.amber }}>Ask ›</T>
        </Pressable>
      )}
      {!!dq && !question && items.length > 1 && (
        <View testID="bits-select-row" style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
          <T v="small" style={{ flex: 1 }}>{selecting ? (picked.length ? `${picked.length} picked` : 'Tap the bits that are one idea') : `${items.length} bits match`}</T>
          {selecting && picked.length >= 2 && <Btn small v="primary" testID="bits-make-one" label={`Make ${picked.length} one bit`} onPress={makeOne} />}
          <TextBtn testID="bits-select" label={selecting ? 'Done' : 'Select'} onPress={() => { haptics.selection(); setSelecting((s) => !s); setPicked([]); }} />
        </View>
      )}
      {!!moments.length && (
        <View testID="bits-moments" style={{ gap: 6 }}>
          <View style={{ flexDirection: 'row', alignItems: 'baseline' }}>
            <T v="eb" style={{ flex: 1 }}>In your sets</T>
            <T v="small">{moments.length === 6 ? 'the closest 6' : `${moments.length} on tape`}</T>
          </View>
          {moments.map((m, i) => <MomentRow key={m.id} m={m} testID={`bits-moment-${i}`} />)}
        </View>
      )}
      <Seg<View_> testID="bits-seg" items={[['bits', 'Bits'], ['notes', 'Notes']]} value={view} onChange={(v) => { haptics.selection(); setView(v); }} />
      {view === 'bits' && !dq && !native && (
        <View testID="bits-sort" style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }} accessibilityRole="radiogroup" accessibilityLabel="Sort">
          <T v="small" style={{ marginRight: 4 }}>Sort</T>
          {SORTS.map((k) => (
            <Pressable key={k} testID={`bits-sort-${k}`} accessibilityRole="radio" accessibilityState={{ selected: k === sort }} onPress={() => { haptics.selection(); setSort(k); }} hitSlop={6}
              style={{ minHeight: 32, paddingHorizontal: 10, borderRadius: 16, justifyContent: 'center', backgroundColor: k === sort ? C.card2 : 'transparent' }}>
              <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: k === sort ? F.semi : F.body, fontSize: 13, color: k === sort ? C.text : C.mut2 }}>{SORT_WORD[k]}</Text>
            </Pressable>
          ))}
        </View>
      )}
      {view === 'bits' && !!pairs && (
        <Pressable testID="merge-banner" accessibilityRole="button" onPress={() => router.push('/merge' as any)}
          style={{ padding: 12, borderRadius: 14, backgroundColor: C.card2, flexDirection: 'row', alignItems: 'center', gap: 10 }}>
          <T style={{ flex: 1 }}>{pairs} {pairs === 1 ? 'pair looks' : 'pairs look'} like the same idea. Merge them into one bit each?</T>
          <T style={{ color: C.amber, fontFamily: F.semi }}>Review ›</T>
        </Pressable>
      )}
    </View>
  );

  if (view === 'notes') {
    return (
      <Screen title="Bits" testID="bits" right={right} largeTitle={native} search={search} onRefresh={list.reload}>
        {mapBtn}
        {header}
        <VoiceMemos />
      </Screen>
    );
  }
  return (
    <Screen title="Bits" testID="bits" right={right} largeTitle={native} search={search} scroll={false}>
      {mapBtn}
      <FlatList contentInsetAdjustmentBehavior="automatic"
        data={items} keyExtractor={(j) => j.id} renderItem={({ item }) => <Row j={item} selecting={selecting} selected={picked.includes(item.id)} onToggle={toggle} />}
        extraData={`${selecting}|${picked.join(',')}`}
        getItemLayout={(_, i) => ({ length: ROW_H, offset: ROW_H * i, index: i })} initialNumToRender={16} windowSize={9} removeClippedSubviews
        keyboardShouldPersistTaps="handled" refreshControl={refreshControl}
        contentContainerStyle={{ paddingHorizontal: wide ? 32 : 16, paddingTop: 12, paddingBottom: 140, maxWidth: wide ? 1120 : 720, width: '100%', alignSelf: 'center' }}
        ListHeaderComponent={header}
        ListEmptyComponent={list.loading && !list.data ? <Skeleton lines={8} height={18} testID="bits-skeleton" /> : list.rawError && !list.data ? <ErrorCard error={list.rawError} onRetry={list.reload} /> : <Empty text={dq ? 'Nothing matches.' : 'No bits yet. Say one.'} />}
        ListFooterComponent={items.length ? (
          <View testID="bits-legend" style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 14, paddingVertical: 14, justifyContent: 'center' }}>
            {STATUSES.map((k) => (
              <View key={k} style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}><View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: dotOf(k) }} /><T v="small">{WORD[k]} · {counts[k]}</T></View>
            ))}
          </View>
        ) : undefined}
      />
    </Screen>
  );
}
