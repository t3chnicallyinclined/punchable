// Boards: "Your sets" first (each with its length, run time and one word of health: solid, or how many tough spots),
// then his boards, then the chunks Punchable found. ＋ Set makes a set from scratch, from a board, or a shorter one from
// another set ("make a 5 from my 10"); ＋ New is the board sheet (blank / from a chunk / from a set).
import React, { useState } from 'react';
import { Pressable, TextInput, View } from 'react-native';
import { router } from 'expo-router';
import { get, useApi } from '@/lib/api';
import { BoardListItem, creative, useCreative } from '@/lib/creative';
import { fmtDur, fmtShortDate } from '@/lib/laughs';
import { SetSummary, Trim, lengthWords, mysets } from '@/lib/mysets';
import { useSets } from '@/lib/sets';
import { useBoards } from '@/lib/boards';
import { C, F } from '@/lib/theme';
import { AIBadge, Avatar, Btn, Card, Empty, ErrorCard, FixtureBanner, List, ListRow, Seg, Sheet, T, s, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const LENGTHS = [300, 420, 600, 900];

export default function Boards() {
  const ui = useUI();
  const cur = useBoards();
  const b = useCreative<{ boards: BoardListItem[] }>('boards', creative.boards);
  const ms = useApi<{ sets: SetSummary[] }>('/api/mysets');
  const [sheet, setSheet] = useState(false);
  const [setSheetOpen, setSetSheet] = useState(false);
  const sets = ms.data?.sets || [];
  const fromSet = new Set(sets.map((x) => x.storyboard_id).filter(Boolean));
  const mine = (b.data?.boards || []).filter((x) => x.source !== 'inferred' || fromSet.has(x.id));
  const seeded = (b.data?.boards || []).filter((x) => x.source === 'inferred' && !fromSet.has(x.id));
  return (
    <Screen title="Boards" testID="boards" right={<View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}><Btn testID="new-board" small v="add" label="＋ New" onPress={() => setSheet(true)} /><Avatar /></View>}>
      <FixtureBanner show={b.fixture} what="GET /api/boards" />
      {b.error && !b.data ? <ErrorCard error={b.error} onRetry={b.reload} /> : null}
      <Card style={{ paddingVertical: 4 }} testID="set-list">
        <View style={[s.row, { paddingTop: 8 }]}>
          <T v="eb" style={{ flex: 1 }}>Your sets</T>
          <Btn testID="new-set" small v="add" label="＋ Set" onPress={() => setSetSheet(true)} />
        </View>
        {ms.error && !ms.data ? <T v="mut">Couldn’t load your sets.</T> : null}
        {ms.data && !sets.length && <Empty text="No sets yet. Make a 5, a 7, a 10: ＋ Set." />}
        <List>
          {sets.map((x) => (
            <ListRow key={x.id} testID={`set-${x.id}`}
              title={<T v="semi" numberOfLines={1}>{x.title}</T>}
              sub={`${x.length_words} · runs ${x.runtime} · ${x.n} bit${x.n === 1 ? '' : 's'}${x.made_by === 'Punchable' ? ' · made by Punchable' : ''}`}
              right={<T v="semi" testID={`set-health-${x.id}`} style={{ color: x.tough_n ? C.amber : x.health === 'solid' ? C.green : C.mut, fontSize: 14 }}>{x.health}</T>}
              onPress={() => router.push(`/myset/${encodeURIComponent(x.id)}` as any)} />
          ))}
        </List>
      </Card>
      <Card style={{ paddingVertical: 4 }} testID="board-list">
        <T v="eb" style={{ paddingTop: 8 }}>Your boards</T>
        {!mine.length && <Empty text="Start with a bit you love: ＋ on any bit adds it to a board." />}
        <List>
          {mine.map((x) => (
            <ListRow key={x.id} testID={`board-${x.id}`} title={<View style={s.row}><T v="semi" style={{ flexShrink: 1 }} numberOfLines={1}>{x.title}</T>{cur.current?.id === x.id && <T v="small" style={{ color: C.amber }}>current</T>}</View>}
              sub={`${x.n} bits · ${x.runtime}${x.target_s ? ` of ${fmtDur(x.target_s)}` : ''} · ${fmtShortDate(x.updated)}`} onPress={() => router.push(`/board/${encodeURIComponent(x.id)}` as any)} />
          ))}
        </List>
      </Card>
      {!!seeded.length && (
        <Card style={{ paddingVertical: 4 }} testID="seeded-list">
          <View style={[s.row, { paddingTop: 8 }]}><T v="eb" style={{ flex: 1 }}>Chunks you already have</T><AIBadge /></View>
          <T v="small">Built from bits that share topics and personas. Open one and make it yours.</T>
          <List>
            {seeded.map((x) => (
              <ListRow key={x.id} testID={`board-${x.id}`} title={x.title} sub={`${x.n} bits · ~${x.runtime}`} onPress={() => router.push(`/board/${encodeURIComponent(x.id)}` as any)} />
            ))}
          </List>
        </Card>
      )}
      <NewBoardSheet visible={sheet} onClose={() => setSheet(false)} seeded={seeded} onMade={(id, title) => { cur.setCurrent({ id, title }); b.reload(); ui.snack(`Started ${title}`); router.push(`/board/${encodeURIComponent(id)}` as any); }} />
      <NewSetSheet visible={setSheetOpen} onClose={() => setSetSheet(false)} boards={mine.concat(seeded)} sets={sets}
        onMade={(id, title) => { ms.reload(); ui.snack(`Made ${title}`); router.push(`/myset/${encodeURIComponent(id)}` as any); }} />
    </Screen>
  );
}

/** ＋ Set: from scratch (a name and a length), from a board, or a shorter one from another set. */
function NewSetSheet({ visible, onClose, boards, sets, onMade }: { visible: boolean; onClose: () => void; boards: BoardListItem[]; sets: SetSummary[]; onMade: (id: string, title: string) => void }) {
  const ui = useUI();
  const [mode, setMode] = useState<'blank' | 'board' | 'set'>('blank');
  const [title, setTitle] = useState('');
  const [target, setTarget] = useState(600);
  const [mins, setMins] = useState('');
  const [src, setSrc] = useState<SetSummary | null>(null);
  const t = mins.trim() && Number(mins) >= 1 ? Math.round(Number(mins) * 60) : target;
  const prev = useApi<Trim>(visible && mode === 'set' && src ? `/api/mysets/${encodeURIComponent(src.id)}/trim?target_s=${t}` : null, { cache: false });
  const run = async (fn: () => Promise<{ set: { id: string; title: string } }>) => {
    onClose();
    try {
      const v = await fn();
      onMade(v.set.id, v.set.title);
    } catch (e: any) {
      ui.snack(`Couldn't make the set: ${e.message}`);
    }
  };
  const input = { minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 } as const;
  return (
    <Sheet visible={visible} onClose={onClose} label="New set" testID="new-set-sheet">
      <T v="h2" style={{ marginBottom: 8 }}>New set</T>
      <Seg items={[['blank', 'Scratch'], ['board', 'A board'], ['set', 'Shorter']]} value={mode} onChange={setMode} testID="new-set-from" />
      <View style={{ gap: 8, marginTop: 12 }}>
        <T v="small">How long</T>
        <View testID="new-set-lengths" style={{ flexDirection: 'row', gap: 6, flexWrap: 'wrap', alignItems: 'center' }}>
          {LENGTHS.map((x) => <Btn key={x} testID={`new-set-${x}`} small v={!mins.trim() && target === x ? 'primary' : 'plain'} label={lengthWords(x)} onPress={() => { setMins(''); setTarget(x); }} />)}
          <TextInput testID="new-set-mins" value={mins} onChangeText={(x) => setMins(x.replace(/[^0-9.]/g, ''))} placeholder="other" placeholderTextColor={C.dim} keyboardType="numeric"
            accessibilityLabel="Other length in minutes" style={[input, { width: 72 }]} />
        </View>
        {mode === 'blank' && <>
          <TextInput testID="new-set-title" value={title} onChangeText={setTitle} placeholder={`Name it (e.g. ${lengthWords(t)} club set)`} placeholderTextColor={C.dim} style={input} />
          <Btn testID="new-set-create" label="Make the set" v="primary" disabled={!title.trim()} onPress={() => run(() => mysets.create({ title: title.trim(), target_s: t }))} />
        </>}
        {mode === 'board' && <List>{boards.map((x) => <ListRow key={x.id} testID={`new-set-board-${x.id}`} title={x.title} sub={`${x.n} bits · ~${x.runtime}`} onPress={() => run(() => mysets.fromBoard(x.id, t))} />)}</List>}
        {mode === 'set' && !src && <List>{sets.map((x) => <ListRow key={x.id} testID={`new-set-src-${x.id}`} title={x.title} sub={`${x.length_words} · ${x.n} bits`} onPress={() => setSrc(x)} />)}</List>}
        {mode === 'set' && src && (
          <View testID="new-set-trim" style={{ gap: 4 }}>
            <T v="semi">A {lengthWords(t)} from {src.title}</T>
            <T v="small">Keeps the opener and the closer, then the bits that do best on stage.</T>
            {prev.data && <>
              <T v="small" testID="new-set-trim-run">{prev.data.runtime} · {prev.data.status_words}</T>
              <T v="small" numberOfLines={4}>{prev.data.keep.map((k) => k.title).join(' · ')}</T>
              <Btn testID="new-set-trim-go" label="Make it" v="primary" onPress={() => run(() => mysets.fromSet(src.id, t, prev.data!.keep.map((k) => k.joke_id)))} />
            </>}
            <Pressable onPress={() => setSrc(null)} style={{ minHeight: 44, justifyContent: 'center' }}><T v="small" style={{ color: C.amber }}>Pick another set</T></Pressable>
          </View>
        )}
      </View>
    </Sheet>
  );
}

function NewBoardSheet({ visible, onClose, seeded, onMade }: { visible: boolean; onClose: () => void; seeded: BoardListItem[]; onMade: (id: string, title: string) => void }) {
  const ui = useUI();
  const [mode, setMode] = useState<'blank' | 'chunk' | 'set'>('blank');
  const [title, setTitle] = useState('');
  const [target, setTarget] = useState(600);
  const sets = useSets();
  const make = async (t: string, ids: string[]) => {
    onClose();
    try {
      const v = await creative.createBoard({ title: t, joke_ids: ids, target_s: target });
      onMade(v.board.id, v.board.title);
    } catch (e: any) {
      ui.snack(`Couldn't create the board: ${e.message}`);
    }
  };
  const fromSet = async (id: string, t: string) => {
    const an = await get<any>(`/api/sets/${encodeURIComponent(id)}/analysis`);
    const ids: string[] = [];
    for (const r of an.rows || []) if (r.status === 'known' && r.joke?.id && !ids.includes(r.joke.id)) ids.push(r.joke.id);
    await make(`${t} (from the set)`, ids);
  };
  const fromChunk = async (x: BoardListItem) => {
    const v = await creative.board(x.id);
    await make(x.title, v.board.joke_ids);
  };
  return (
    <Sheet visible={visible} onClose={onClose} label="New board" testID="new-board-sheet">
      <T v="h2" style={{ marginBottom: 8 }}>New board</T>
      <Seg items={[['blank', 'Blank'], ['chunk', 'From a chunk'], ['set', 'From a set']]} value={mode} onChange={setMode} testID="new-from" />
      <View style={{ gap: 8, marginTop: 12 }}>
        <T v="small">Target length (the light)</T>
        <View style={{ flexDirection: 'row', gap: 6, flexWrap: 'wrap' }}>
          {[300, 420, 600, 900, 1200].map((s2) => <Btn key={s2} small v={target === s2 ? 'primary' : 'plain'} label={`${s2 / 60} min`} onPress={() => setTarget(s2)} />)}
        </View>
        {mode === 'blank' && <>
          <TextInput testID="board-title" value={title} onChangeText={setTitle} placeholder="Name it (e.g. Travel chunk)" placeholderTextColor={C.dim}
            style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
          <Btn testID="board-create" label="Create" v="primary" disabled={!title.trim()} onPress={() => make(title.trim(), [])} />
        </>}
        {mode === 'chunk' && <List>{seeded.map((x) => <ListRow key={x.id} testID={`chunk-${x.id}`} title={x.title} sub={`${x.n} bits · ~${x.runtime}`} onPress={() => fromChunk(x)} />)}</List>}
        {mode === 'set' && <List>{sets.list.map((x) => <ListRow key={x.id} testID={`from-set-${x.id}`} title={`${fmtShortDate(x.show.date)} · ${x.venue.name}`} sub={x.title} onPress={() => fromSet(x.id, x.venue.name.replace(/\?$/, ''))} />)}</List>}
      </View>
    </Sheet>
  );
}
