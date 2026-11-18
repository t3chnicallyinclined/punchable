// (2026-10: the old set builder, kept at /builder/<id>; /board/<id> now opens the set list, docs/UX_2026-10.md.)
// A board, and the set builder (SIMPLE.md): the chain in order with a target length (10–12 min). Runtime adds up
// from his tapes (estimates marked "~ est."), each transition is said in words and the weak ones are flagged with an
// optional bridge draft (labelled draft), "Could come next" picks from his notebook at each step, and
// "Make this a set" adds it to Your sets (creative/mysets.py; a set is a working set). Health numbers and ratios are not shown.
import React, { useCallback, useEffect, useState } from 'react';
import { Platform, Pressable, Text, TextInput, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { decide, undo, useApi } from '@/lib/api';
import { BoardView, Bridge, creative, useCreative } from '@/lib/creative';
import { DragList, move } from '@/lib/DragList';
import { mmss, monthYear } from '@/rules/format';
import { useBoards } from '@/lib/boards';
import { mysets } from '@/lib/mysets';
import { Build, Step, quoteText } from '@/lib/simple';
import { C, F } from '@/lib/theme';
import { AddBtn, Btn, Card, Empty, ErrorCard, FixtureBanner, Icon, IconBtn, List, ListRow, Sheet, T, TextBtn, s, useUI, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { BridgeSheet } from '@/lib/bridge';

/** Target bands: the top of the band is the board's target_s; the band is 2 minutes wide (1 under 8 min). */
const TARGETS = [300, 420, 720, 1020, 1320, 1800];
const band = (t: number) => `${Math.round((t - (t >= 480 ? 120 : 60)) / 60)}–${Math.round(t / 60)} min`;
import type { Gap } from '@/lib/bridge';

export default function SetBuilder() {
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const boards = useBoards();
  const wide = useWide();
  const v = useCreative<BoardView>(`board:${id}`, () => creative.board(id));
  const b = useApi<Build>(`/api/boards/${encodeURIComponent(id)}/build`);
  const [bridge, setBridge] = useState<Gap | null>(null);
  const [picker, setPicker] = useState(false);
  const [promote, setPromote] = useState(false);
  const [exportS, setExportS] = useState(false);
  const [rename, setRename] = useState(false);
  const [targetSheet, setTargetSheet] = useState(false);
  useEffect(() => {
    if (v.data?.board) boards.setCurrent({ id, title: v.data.board.title });
  }, [v.data?.board?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  const apply = useCallback(async (fn: () => Promise<BoardView>, msg: string, undoFn?: () => Promise<BoardView>) => {
    try {
      const nv = await fn();
      if (nv?.health) v.setData(nv);
      else v.reload();
      b.reload();
      ui.snack(msg, undoFn ? async () => { const r = await undoFn(); if (r?.health) v.setData(r); else v.reload(); b.reload(); } : undefined);
    } catch (e: any) {
      ui.snack(`Couldn't save: ${e.message}`);
    }
  }, [v, b, ui]);

  if ((v.error && !v.data) || (b.error && !b.data)) return <Screen title="Set" back><ErrorCard error={String(v.error || b.error)} onRetry={() => { v.reload(); b.reload(); }} /></Screen>;
  if (!v.data || !b.data) return <Screen title="Set" back><Empty text="Opening the set…" /></Screen>;
  const { board } = v.data;
  const B = b.data;
  const ids = board.joke_ids;
  const steps = B.steps;
  const titleOf = (j: string) => steps.find((x) => x.joke_id === j)?.title || '';

  const reorder = (from: number, to: number) => {
    const before = ids.slice();
    apply(() => creative.patchBoard(id, { joke_ids: move(ids, from, to) }), `Moved ${steps[from]?.title}`, () => creative.patchBoard(id, { joke_ids: before }));
  };
  const remove = (jid: string, title: string) => {
    const before = ids.slice();
    apply(() => creative.patchBoard(id, { remove: { joke_id: jid } }), `Removed ${title}`, () => creative.patchBoard(id, { joke_ids: before }));
  };
  const add = (jid: string, title: string) => {
    const before = ids.slice();
    apply(() => creative.addItem(id, jid), `Added ${title}`, () => creative.patchBoard(id, { joke_ids: before }));
  };
  const setTarget = (t: number) => {
    const old = board.target_s;
    apply(() => creative.patchBoard(id, { target_s: t }), `Aiming for ${band(t)}`, () => creative.patchBoard(id, { target_s: old }));
  };

  const statusColor = B.status === 'on' ? C.green : B.status === 'over' ? C.redSoft : C.mut;
  const summary = (
    <View style={{ gap: 4 }}>
      <T v="eb">{B.working_set ? '★ Your working set' : 'Set'}</T>
      <T v="h1" testID="board-title">{board.title}</T>
      <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 8, flexWrap: 'wrap' }}>
        <T v="num" testID="build-runtime" style={{ fontSize: 22 }}>{B.runtime}</T>
        <Pressable testID="target-edit" accessibilityRole="button" accessibilityLabel={`Aiming for ${band(B.board.target_s)}. Change`} onPress={() => setTargetSheet(true)} style={{ minHeight: 44, justifyContent: 'center' }}>
          <T v="mut" testID="build-target" style={{ textDecorationLine: 'underline' }}>of {band(B.board.target_s)}</T>
        </Pressable>
        <T v="semi" testID="build-status" style={{ color: statusColor, fontSize: 15 }}>{B.status === 'on' ? 'on target' : B.status === 'over' ? 'over' : B.status === 'short' ? 'short' : ''}</T>
      </View>
      {B.estimated_n > 0 && <T v="small" testID="build-est">Times marked ~ are estimates from the written words.</T>}
    </View>
  );

  const targets = (
    <Sheet visible={targetSheet} onClose={() => setTargetSheet(false)} label="How long is the set?" testID="target-sheet">
      <T v="h2" style={{ marginBottom: 10 }}>How long is the set?</T>
      <View testID="targets" style={{ flexDirection: 'row', gap: 6, flexWrap: 'wrap' }}>
        {TARGETS.map((t) => <Btn key={t} testID={`target-${t}`} small v={B.board.target_s === t ? 'primary' : 'plain'} label={band(t)} onPress={() => { setTargetSheet(false); setTarget(t); }} />)}
      </View>
    </Sheet>
  );

  const chain = (
    <View testID="chain">
      {!steps.length && <Empty text="Start with a bit you love." />}
      <DragList testID="chain-list" items={steps} keyOf={(x) => x.joke_id} onReorder={reorder}
        render={(x: Step, i, handle) => {
          const t = x.transition_in;
          return (
            <View>
              {t && (
                <Pressable testID={`transition-${i - 1}`} accessibilityRole="button" disabled={!t.weak}
                  accessibilityLabel={`${t.words}${t.weak ? '. Get a bridge idea' : ''}`} onPress={() => t.weak && setBridge({ from: t.from, to: t.to, words: t.words })}
                  style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 40, paddingLeft: 18 }}>
                  <View style={{ width: 3, alignSelf: 'stretch', backgroundColor: t.weak ? C.redSoft : t.kind === 'proven' ? C.green : C.amber, borderRadius: 2 }} />
                  <Text maxFontSizeMultiplier={1.4} numberOfLines={1} style={{ flex: 1, fontFamily: F.body, fontSize: 13, color: t.weak ? C.redSoft : t.kind === 'proven' ? C.green : C.amber }}>
                    {t.weak ? '○ ' : '● '}{t.words}
                  </Text>
                  {t.weak && <Text style={{ fontFamily: F.semi, fontSize: 13, color: C.amber, paddingRight: 8 }}>Bridge idea ›</Text>}
                </Pressable>
              )}
              <View testID={`card-${i}`} style={[s.card, { padding: 10, flexDirection: 'row', alignItems: 'center', gap: 6 }]}>
                {handle}
                <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
                  <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
                    <T v="num" style={{ fontSize: 14, color: C.mut }}>{i + 1}</T>
                    <Pressable style={{ flexShrink: 1 }} onPress={() => router.push(`/joke/${encodeURIComponent(x.joke_id)}` as any)} accessibilityRole="link">
                      <T v="semi" numberOfLines={1}>{x.title}</T>
                    </Pressable>
                    <T v="small" testID={`dur-${i}`}>{x.estimated ? `~${mmss(x.duration_s)} est.` : mmss(x.duration_s)}</T>
                  </View>
                  {!!x.quote && <T v="small" numberOfLines={1} style={{ color: C.text2 }}>{quoteText(x)}</T>}
                  {!!x.stage_line && <T v="small" numberOfLines={2} style={{ color: x.proven ? C.green : C.mut }}>{x.stage_line}</T>}
                </View>
                <View>
                  <IconBtn testID={`up-${i}`} label={`Move ${x.title} up`} disabled={i === 0} onPress={() => reorder(i, i - 1)}><Icon name="up" size={18} color={C.mut} /></IconBtn>
                  <IconBtn testID={`down-${i}`} label={`Move ${x.title} down`} disabled={i === steps.length - 1} onPress={() => reorder(i, i + 1)}><Icon name="down" size={18} color={C.mut} /></IconBtn>
                </View>
                <IconBtn testID={`remove-${i}`} label={`Remove ${x.title}`} onPress={() => remove(x.joke_id, x.title)}><Icon name="trash" size={18} color={C.mut} /></IconBtn>
              </View>
            </View>
          );
        }} />
      <Pressable testID="drop-zone" accessibilityRole="button" onPress={() => setPicker(true)}
        style={{ marginTop: 10, minHeight: 56, borderWidth: 1.5, borderStyle: 'dashed', borderColor: C.borderStrong, borderRadius: 12, alignItems: 'center', justifyContent: 'center' }}>
        <T v="mut">Find a bit to add</T>
      </Pressable>
    </View>
  );

  const next = (
    <Card testID="best-next" style={{ gap: 4 }}>
      <T v="eb">Could come next · from your notebook</T>
      {!B.next.length && <T v="mut">Nothing obvious yet. Add a bit you love.</T>}
      {B.next.slice(0, wide ? 5 : 3).map((x, i) => (
        <View key={x.joke_id} testID={`best-${i}`} style={{ flexDirection: 'row', alignItems: 'center', gap: 10, paddingVertical: 8, borderTopWidth: i ? 1 : 0, borderTopColor: C.border }}>
          <Pressable style={{ flex: 1, minWidth: 0, gap: 2 }} accessibilityRole="button" onPress={() => router.push(`/joke/${encodeURIComponent(x.joke_id)}` as any)}>
            <T v="semi" numberOfLines={1}>{x.title}{x.date ? <Text style={{ fontFamily: F.body, color: C.mut }}> · {monthYear(x.date)}</Text> : null}</T>
            {!!x.why && <T v="small" numberOfLines={2}>{x.why}</T>}
            {!!x.stage_line && <T v="small" numberOfLines={1} style={{ color: x.proven ? C.green : C.mut }}>{x.stage_line}</T>}
          </Pressable>
          <AddBtn testID={`best-add-${i}`} onPress={() => add(x.joke_id, x.title)} label={`Add ${x.title}`} />
        </View>
      ))}
      {B.notes.map((n) => <T key={n.rule} v="small" testID={`note-${n.rule}`} style={{ marginTop: 6 }}>{n.text}</T>)}
    </Card>
  );

  const actions = (
    <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap' }}>
      {B.working_set
        ? <>
          <Btn testID="open-set" small v="add" label="Open the set ›" onPress={() => router.push(`/myset/${encodeURIComponent(B.working_set!)}` as any)} />
          <Btn testID="open-ws" small label="Perform & track ›" onPress={() => router.push(`/workingset/${encodeURIComponent(B.working_set!)}` as any)} />
        </>
        : <Btn testID="promote" small v="add" label="Make this a set" onPress={() => setPromote(true)} disabled={!steps.length} />}
      <Btn testID="export" small label="Set list" onPress={() => setExportS(true)} />
      <Btn small v="ghost" label="Duplicate" onPress={async () => {
        const nv = await creative.createBoard({ title: `${board.title} (copy)`, joke_ids: ids, target_s: board.target_s });
        router.push(`/board/${encodeURIComponent(nv.board.id)}` as any);
      }} />
    </View>
  );

  return (
    <Screen title="Set" back testID="storyboard" right={<IconBtn label="Rename" onPress={() => setRename(true)}><Icon name="notes" size={20} /></IconBtn>}>
      <FixtureBanner show={v.fixture} what="GET/PATCH /api/boards/{id}" />
      {summary}
      {targets}
      {actions}
      {wide ? (
        <View style={{ flexDirection: 'row', gap: 20, alignItems: 'flex-start' }}>
          <View style={{ flex: 3, minWidth: 0 }}>{chain}</View>
          <View style={{ flex: 2, gap: 12, minWidth: 0 }}>{next}</View>
        </View>
      ) : (<>{chain}{next}</>)}

      <BridgeSheet key={bridge ? `${bridge.from}>${bridge.to}` : 'none'} gap={bridge} boardId={id} titleOf={titleOf} onClose={() => setBridge(null)} />
      <BitPicker key={picker ? 'open' : 'closed'} visible={picker} onClose={() => setPicker(false)} onPick={(j) => { setPicker(false); add(j.id, j.title.split(' (')[0]); }} exclude={ids} />
      <PromoteSheet visible={promote} onClose={() => setPromote(false)} boardId={id} runtime={B.runtime_s} target={B.board.target_s} onDone={b.reload} />
      <Sheet visible={exportS} onClose={() => setExportS(false)} label="Set list" testID="export-sheet">
        <T v="h2">Set list</T>
        <View style={[s.inner, { marginVertical: 10 }]}><Text selectable testID="setlist-text" style={{ fontFamily: F.g6, fontSize: 18, lineHeight: 28, color: C.text }}>{v.data.setlist?.text}</Text></View>
        <View style={{ flexDirection: 'row', gap: 8 }}>
          <Btn testID="setlist-copy" label="Copy" v="primary" style={{ flex: 1 }} onPress={async () => { try { await (globalThis as any).navigator?.clipboard?.writeText(v.data!.setlist.text); ui.snack('Set list copied'); } catch { ui.snack('Copy isn’t available here'); } }} />
          <Btn label="Perform" style={{ flex: 1 }} onPress={() => { setExportS(false); router.push(`/setlist/${encodeURIComponent(id)}` as any); }} />
          {Platform.OS === 'web' && <Btn label="Download .txt" style={{ flex: 1 }} onPress={() => {
            const a = document.createElement('a');
            a.href = URL.createObjectURL(new Blob([v.data!.setlist.text], { type: 'text/plain' }));
            a.download = `${board.title.replace(/[^\w-]+/g, '_')}.txt`;
            a.click();
          }} />}
        </View>
      </Sheet>
      <Sheet visible={rename} onClose={() => setRename(false)} label="Rename" testID="rename-sheet">
        <RenameForm title={board.title} onSave={(t) => { setRename(false); const old = board.title; apply(() => creative.patchBoard(id, { title: t }), `Renamed to ${t}`, () => creative.patchBoard(id, { title: old })); }} />
      </Sheet>
    </Screen>
  );
}

function RenameForm({ title, onSave }: { title: string; onSave: (t: string) => void }) {
  const [t, setT] = useState(title);
  return (
    <View style={{ gap: 8 }}>
      <T v="h2">Rename</T>
      <TextInput value={t} onChangeText={setT} style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      <Btn label="Save" v="primary" disabled={!t.trim()} onPress={() => onSave(t.trim())} />
    </View>
  );
}

function BitPicker({ visible, onClose, onPick, exclude }: { visible: boolean; onClose: () => void; onPick: (j: { id: string; title: string }) => void; exclude: string[] }) {
  const [q, setQ] = useState('');
  const res = useApi<{ items: any[] }>(visible && q.trim().length >= 2 ? `/api/jokes?q=${encodeURIComponent(q.trim())}&mode=hybrid&limit=10` : null, { cache: false });
  return (
    <Sheet visible={visible} onClose={onClose} label="Add a bit" testID="bit-picker">
      <T v="h2" style={{ marginBottom: 8 }}>Add a bit</T>
      <TextInput testID="bit-search" value={q} onChangeText={setQ} placeholder="Search your notebook" placeholderTextColor={C.dim} autoFocus
        style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      <List>{(res.data?.items || []).filter((j) => !exclude.includes(j.id)).map((j) => <ListRow key={j.id} testID={`bit-pick-${j.id}`} title={j.title.split(' (')[0]} sub={j.punch ? `…${j.punch}` : undefined} onPress={() => onPick(j)} />)}</List>
    </Sheet>
  );
}

function PromoteSheet({ visible, onClose, boardId, runtime, target, onDone }: { visible: boolean; onClose: () => void; boardId: string; runtime: number; target: number; onDone: () => void }) {
  const ui = useUI();
  return (
    <Sheet visible={visible} onClose={onClose} label="Make this a set" testID="promote-sheet">
      <T v="h2">Make this a set</T>
      <T v="mut" style={{ marginVertical: 6 }}>It joins Your sets: you work on it, perform it, and every show you import that looks like it lines up with it. It runs {mmss(runtime)} now, aiming for {band(target)}. The board stays as it is.</T>
      <Btn testID="promote-go" label="Make it a set" v="primary" onPress={async () => {
        onClose();
        try {
          const v = await mysets.fromBoard(boardId, target);
          ui.setWorkingSet({ id: v.set.id, title: v.set.title, sub: `v${v.set.version} · ${v.bits.length} bits · ${mmss(target)}`, target_s: target });
          ui.snack(`${v.set.title} is in Your sets`, undefined, { label: 'Open', run: () => router.push(`/myset/${encodeURIComponent(v.set.id)}` as any) });
          onDone();
        } catch (e: any) {
          ui.snack(`Couldn't make it a set: ${e.message}`);
        }
      }} />
      <TextBtn label="Not now" color={C.mut} onPress={onClose} />
    </Sheet>
  );
}
