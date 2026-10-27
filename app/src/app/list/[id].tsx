// Inside a set list (MixBuild, docs/UX_2026-10.md). A set list IS a board: this screen is the board's set builder
// (GET /api/boards/<id>/build) drawn as the paper list: numbered rows with the short name, the first line and the
// time; tap a row for the full text and Open bit · Swap · Rename short name · Remove; "Gap here · add a bridge" where
// the builder flags a weak transition; the time bar; drag to reorder; "Add a joke · N left" at the bottom.
// Every edit is one of the board's own per-item ops (add / remove / move): never a whole-list replace, so two
// devices open at once don't clobber each other. Undo in the snackbar is the inverse op (a real write by him).
import React, { useCallback, useMemo, useState } from 'react';
import { Pressable, Text, TextInput, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { decide, patch, undo, useApi } from '@/lib/api';
import { creative } from '@/lib/creative';
import { AddJokeSheet, Pick } from '@/lib/AddJokeSheet';
import { BridgeSheet, Gap } from '@/lib/bridge';
import { DragList } from '@/lib/DragList';
import { fmtDur } from '@/lib/laughs';
import { useBoards } from '@/lib/boards';
import { Build, Step, quoteText } from '@/lib/simple';
import { shortNameOf } from '@/lib/say';
import { C, F } from '@/lib/theme';
import { Btn, Empty, ErrorCard, Screen, Sheet, T, TextBtn, TopBar, useUI } from '@/lib/ui';

const enc = encodeURIComponent;
const boardPatch = (id: string, body: any) => patch<any>(`/api/boards/${enc(id)}`, body);

/** The full text of a bit, fetched once when its row opens (one call per tap, never per row). */
function useJokeText(id: string | null) {
  const d = useApi<any>(id ? `/api/jokes/${enc(id)}` : null);
  const j = d.data;
  if (!j) return { text: null as string | null, loading: d.loading };
  const units = (j.versions || []).flatMap((v: any) => v.units || []).filter((u: any) => u.text);
  const text = units[0]?.text || j.joke?.summary || null;
  return { text, loading: false };
}

function Row({ x, i, open, onToggle, handle, onOpen, onSwap, onRename, onRemove }: {
  x: Step; i: number; open: boolean; onToggle: () => void; handle: React.ReactNode; onOpen: () => void; onSwap: () => void; onRename: () => void; onRemove: () => void;
}) {
  const t = useJokeText(open ? x.joke_id : null);
  const name = x.short_name || shortNameOf(x.title);
  return (
    <View testID={`row-${i}`} style={{ borderRadius: 16, backgroundColor: open ? C.card : 'transparent', paddingVertical: 4 }}>
      <Pressable accessibilityRole="button" accessibilityState={{ expanded: open }} accessibilityLabel={`${i + 1}. ${name}, ${fmtDur(x.duration_s)}. ${open ? 'Less' : 'More'}`} onPress={onToggle}
        style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 56, paddingRight: 12 }}>
        {handle}
        <Text style={{ width: 22, fontFamily: F.semi, fontSize: 14, color: C.mut2, textAlign: 'right' }}>{i + 1}</Text>
        <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
          <Text testID={`row-name-${i}`} numberOfLines={1} style={{ fontFamily: F.bold, fontSize: 17, color: C.text }}>{name}</Text>
          {!open && !!x.quote && <Text numberOfLines={1} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>“{quoteText(x)}”</Text>}
        </View>
        <Text testID={`row-time-${i}`} style={{ fontFamily: F.medium, fontSize: 14, color: C.mut2, fontVariant: ['tabular-nums'] }}>{x.estimated ? '~' : ''}{fmtDur(x.duration_s)}</Text>
      </Pressable>
      {open && (
        <View testID={`row-open-${i}`} style={{ paddingHorizontal: 16, paddingBottom: 12, gap: 10 }}>
          {t.loading ? <T v="mut">…</T> : <Text style={{ fontFamily: F.body, fontSize: 15, lineHeight: 22, color: C.text2 }}>{t.text || x.quote || 'No written version yet.'}</Text>}
          {!!x.stage_line && <T v="small" style={{ color: x.proven ? C.green : C.mut2 }}>{x.stage_line}</T>}
          <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
            <Btn small testID={`row-openbit-${i}`} label="Open bit" onPress={onOpen} />
            <Btn small testID={`row-swap-${i}`} label="Swap" onPress={onSwap} />
            <Btn small testID={`row-rename-${i}`} label={`Rename “${name}”`} onPress={onRename} />
            <Btn small testID={`row-remove-${i}`} v="danger" label="Remove" onPress={onRemove} />
          </View>
        </View>
      )}
    </View>
  );
}

export default function SetListScreen() {
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const boards = useBoards();
  const b = useApi<Build>(`/api/boards/${enc(id)}/build`, { poll: 30000 });
  const [open, setOpen] = useState<string | null>(null);
  const [add, setAdd] = useState<null | { mode: 'add' } | { mode: 'swap'; at: number; old: Step }>(null);
  const [gap, setGap] = useState<Gap | null>(null);
  const [rename, setRename] = useState<Step | null>(null);
  const [title, setTitle] = useState(false);
  const B = b.data;
  React.useEffect(() => {
    if (B?.board) boards.setCurrent({ id, title: B.board.title });
  }, [B?.board?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  /** One write, one snackbar, and Undo = the inverse op (also his write). The build is re-read after each. */
  const act = useCallback(async (fn: () => Promise<any>, msg: string, inverse?: () => Promise<any>) => {
    try {
      await fn();
      b.reload();
      ui.snack(msg, inverse ? async () => { await inverse(); b.reload(); } : undefined);
    } catch (e: any) {
      ui.snack(`Couldn't save: ${e.message}`);
      b.reload();
    }
  }, [b, ui]);

  const steps = B?.steps || [];
  const inSet = useMemo(() => steps.map((x, i) => ({ joke_id: x.joke_id, position: i })), [steps]);
  const nameOf = (x: Step) => x.short_name || shortNameOf(x.title);
  const titleOf = (j: string) => { const x = steps.find((y) => y.joke_id === j); return x ? nameOf(x) : ''; };

  const reorder = (from: number, to: number) => {
    if (from === to) return;
    act(() => boardPatch(id, { move: { from, to } }), `Moved ${nameOf(steps[from])} to #${to + 1}`, () => boardPatch(id, { move: { from: to, to: from } }));
  };
  const remove = (x: Step, i: number) => {
    setOpen(null);
    act(() => boardPatch(id, { remove: { joke_id: x.joke_id } }), `Removed ${nameOf(x)}`, () => boardPatch(id, { add: { joke_id: x.joke_id, at: i } }));
  };
  const addJoke = (pk: Pick) => {
    setAdd(null);
    act(() => creative.addItem(id, pk.joke_id), `Added ${pk.short_name}`, () => boardPatch(id, { remove: { joke_id: pk.joke_id } }));
  };
  const swap = (at: number, old: Step, pk: Pick) => {
    setAdd(null);
    setOpen(null);
    act(() => boardPatch(id, { add: { joke_id: pk.joke_id, at }, remove: { joke_id: old.joke_id } }), `Swapped ${nameOf(old)} for ${pk.short_name}`,
      () => boardPatch(id, { add: { joke_id: old.joke_id, at }, remove: { joke_id: pk.joke_id } }));
  };
  const saveShortName = async (x: Step, name: string) => {
    setRename(null);
    const v = name.trim() || null;
    try {
      const w = await decide({ target: 'node', action: 'edit', id: x.joke_id, after: { short_name: v } });
      b.reload();
      ui.snack(v ? `Now “${v}” on the list` : 'Back to the default name', async () => { await undo(w.indexes); b.reload(); });
    } catch (e: any) {
      ui.snack(`Couldn't rename: ${e.message}`);
    }
  };

  if (b.rawError && !B) return <Screen title="Set list" back><ErrorCard error={String(b.rawError)} onRetry={b.reload} /></Screen>;
  if (!B) return <Screen title="Set list" back><Empty text="Opening the set list…" /></Screen>;
  const target = B.board.target_s;
  const left = target ? target - B.runtime_s : null;
  const pct = target ? Math.min(100, (B.runtime_s / target) * 100) : 0;

  const header = (
    <TopBar title={B.board.title} back onBack={() => (router.canGoBack() ? router.back() : router.navigate('/sets' as any))}
      right={<TextBtn testID="stage" label="Stage" onPress={() => router.push(`/stage/${enc(id)}` as any)} />} />
  );

  return (
    <Screen title={B.board.title} testID="setlist-screen" header={header}
      dock={<Btn testID="add-joke" v="primary" label={`Add a joke${left != null ? ` · ${left > 0 ? `${fmtDur(left)} left` : 'full'}` : ''}`} onPress={() => setAdd({ mode: 'add' })} />}>
      <Pressable testID="list-title" accessibilityRole="button" accessibilityLabel={`${B.board.title}. Rename`} onPress={() => setTitle(true)} style={{ gap: 6 }}>
        <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 6 }}>
          <Text testID="list-runtime" style={{ fontFamily: F.black, fontSize: 30, letterSpacing: -0.5, color: C.text, fontVariant: ['tabular-nums'] }}>{fmtDur(B.runtime_s)}</Text>
          {!!target && <Text style={{ fontFamily: F.medium, fontSize: 16, color: C.mut2 }}>/ {fmtDur(target)}</Text>}
          <Text testID="list-status" style={{ marginLeft: 8, fontFamily: F.semi, fontSize: 13, color: B.status === 'on' ? C.green : B.status === 'over' ? C.gapText : C.mut2 }}>{B.status_words}</Text>
        </View>
        <View style={{ height: 6, borderRadius: 3, backgroundColor: C.card, overflow: 'hidden' }}>
          <View style={{ width: `${pct}%`, height: '100%', backgroundColor: B.status === 'over' ? C.gap : C.amber }} />
        </View>
      </Pressable>

      {!steps.length && <Empty text="Empty list. Add a joke below, or say one." />}
      <DragList testID="list-rows" items={steps} keyOf={(x) => x.joke_id} onReorder={reorder}
        render={(x: Step, i, handle) => (
          <View>
            {x.transition_in?.weak && (
              <Pressable testID={`gap-${i}`} accessibilityRole="button" accessibilityLabel={`Gap before ${nameOf(x)}: ${x.transition_in.words}. Add a bridge`}
                onPress={() => setGap({ from: x.transition_in!.from, to: x.transition_in!.to, words: x.transition_in!.words })}
                style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 36, paddingLeft: 74 }}>
                <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: C.gap }} />
                <Text style={{ fontFamily: F.medium, fontSize: 13, color: C.gapText }}>Gap here · add a bridge</Text>
              </Pressable>
            )}
            <Row x={x} i={i} open={open === x.joke_id} onToggle={() => setOpen(open === x.joke_id ? null : x.joke_id)} handle={handle}
              onOpen={() => router.push(`/joke/${enc(x.joke_id)}` as any)} onSwap={() => setAdd({ mode: 'swap', at: i, old: x })}
              onRename={() => setRename(x)} onRemove={() => remove(x, i)} />
          </View>
        )} />
      {!!steps.length && <T v="small" style={{ textAlign: 'center' }}>Hold the grip to drag · tap a row for more</T>}
      {B.notes.map((n) => <T key={n.rule} v="small" testID={`note-${n.rule}`}>{n.text}</T>)}

      <AddJokeSheet visible={!!add} onClose={() => setAdd(null)} boardId={id} inSet={inSet} fits={B.next} left={left} mode={add?.mode || 'add'}
        onPick={(pk) => (add?.mode === 'swap' ? swap(add.at, add.old, pk) : addJoke(pk))} />
      <BridgeSheet key={gap ? `${gap.from}>${gap.to}` : 'none'} gap={gap} boardId={id} titleOf={titleOf} onClose={() => setGap(null)} />
      <Sheet visible={!!rename} onClose={() => setRename(null)} label="Rename short name" testID="rename-sheet">
        {rename && <ShortNameForm x={rename} onSave={(v) => saveShortName(rename, v)} />}
      </Sheet>
      <Sheet visible={title} onClose={() => setTitle(false)} label="Rename the set list" testID="title-sheet">
        <TitleForm title={B.board.title} onSave={(t) => { setTitle(false); const old = B.board.title; act(() => boardPatch(id, { title: t }), `Renamed to ${t}`, () => boardPatch(id, { title: old })); }} />
      </Sheet>
    </Screen>
  );
}

function ShortNameForm({ x, onSave }: { x: Step; onSave: (v: string) => void }) {
  const [v, setV] = useState(x.short_name || shortNameOf(x.title));
  return (
    <View style={{ gap: 8 }}>
      <T v="h2">Short name</T>
      <T v="mut">What you’d write on the paper list. Up to 24 letters. The full title stays “{x.title}”.</T>
      <TextInput testID="shortname-input" value={v} onChangeText={setV} maxLength={24} autoFocus placeholder={shortNameOf(x.title)} placeholderTextColor={C.dim}
        style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.bold, fontSize: 18 }} />
      <View style={{ flexDirection: 'row', gap: 8 }}>
        <Btn testID="shortname-save" label="Save" v="primary" style={{ flex: 1 }} disabled={!v.trim()} onPress={() => onSave(v)} />
        <Btn testID="shortname-default" label="Default" v="ghost" onPress={() => onSave('')} />
      </View>
    </View>
  );
}
function TitleForm({ title, onSave }: { title: string; onSave: (t: string) => void }) {
  const [t, setT] = useState(title);
  return (
    <View style={{ gap: 8 }}>
      <T v="h2">Rename the set list</T>
      <TextInput testID="title-input" value={t} onChangeText={setT} autoFocus style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      <Btn label="Save" v="primary" disabled={!t.trim()} onPress={() => onSave(t.trim())} />
    </View>
  );
}
