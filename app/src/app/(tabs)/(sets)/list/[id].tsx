// Inside a set list (MixBuild, docs/UX_2026-10.md). A set list IS a board: this screen is the board's set builder
// (GET /api/boards/<id>/build) drawn as the paper list: numbered rows with the short name, the first line and the
// time; "Gap here · add a bridge" where the builder flags a weak transition; the time bar; drag to reorder; "Add a
// joke · N left" at the bottom. HIG (the audit): tap a row OPENS the bit; trailing swipe = Remove (red, a full swipe
// runs it, Undo in the snackbar) + Swap; long-press = Open · Swap · Rename · Move to top · Move to bottom · Remove;
// the title and the "…" More button rename; pull to refresh. A trailing 44-pt notes button per row (and "Notes" first
// in the menu) opens the bit's words as a sheet (/bitnotes, SPEC 2026-10-29 §2); the row tap stays the workbench.
// Every edit is one of the board's own per-item ops (add / remove / move): never a whole-list replace, so two
// devices open at once don't clobber each other. Undo in the snackbar is the inverse op (a real write by him).
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Platform, Pressable, Share, Text, TextInput, View } from 'react-native';
import { Stack, router, useFocusEffect, useLocalSearchParams } from 'expo-router';
import { actionSheet } from '@/lib/actionSheet';
import { decide, patch, undo, useApi } from '@/lib/api';
import { creative } from '@/lib/creative';
import { AddJokeSheet, Pick } from '@/lib/AddJokeSheet';
import { BridgeSheet, Gap } from '@/lib/bridge';
import { DragList } from '@/lib/DragList';
import { haptics } from '@/lib/haptics';
import { mmss } from '@/rules/format';
import { useBoards } from '@/lib/boards';
import { Build, Step, quoteText } from '@/lib/simple';
import { shortNameOf } from '@/lib/say';
import { stageNotes } from '@/lib/stageNotes';
import { SwipeRow } from '@/lib/SwipeRow';
import { RowMenu, rowActions } from '@/lib/RowMenu';
import { C, F } from '@/lib/theme';
import { Btn, Empty, ErrorCard, Icon, IconBtn, MenuItem, MenuSheet, Sheet, SheetHead, Skeleton, T, TextBtn, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TopBar } from '@/lib/TopBar';

const enc = encodeURIComponent;
const boardPatch = (id: string, body: any) => patch<any>(`/api/boards/${enc(id)}`, body);

const native = Platform.OS !== 'web';
const ios = Platform.OS === 'ios'; // the system context menu owns the long press there

function Row({ x, i, handle, onOpen, onMenu, onNotes, items }: { x: Step; i: number; handle: React.ReactNode; onOpen: () => void; onMenu: () => void; onNotes: () => void; items: MenuItem[] }) {
  const name = x.short_name || shortNameOf(x.title);
  return (
    <View testID={`row-${i}`} style={{ borderRadius: 16, paddingVertical: 4 }}>
      <Pressable accessibilityRole="button" accessibilityLabel={`${i + 1}. ${name}, ${mmss(x.duration_s)}. Open the bit; long press for actions`} onPress={onOpen} onLongPress={ios ? undefined : onMenu} delayLongPress={350}
        {...rowActions(items)}
        style={(st: any) => ({ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 56, paddingRight: 4, borderRadius: 16, backgroundColor: st.hovered ? C.card2 : 'transparent' })}>
        {handle}
        <Text maxFontSizeMultiplier={1.4} style={{ width: 22, fontFamily: F.semi, fontSize: 14, color: C.mut2, textAlign: 'right' }}>{i + 1}</Text>
        <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
          <Text testID={`row-name-${i}`} numberOfLines={1} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.bold, fontSize: 17, color: C.text }}>{name}</Text>
          {!!x.quote && <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>“{quoteText(x)}”</Text>}
          {!!x.stage_line && <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 12, color: x.proven ? C.green : C.mut2 }}>{x.stage_line}</Text>}
        </View>
        <Text testID={`row-time-${i}`} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.medium, fontSize: 14, color: C.mut2, fontVariant: ['tabular-nums'] }}>{x.estimated ? '~' : ''}{mmss(x.duration_s)}</Text>
        {/* HIG/accessibility: the notes have a button, not only a menu item; 44 pt, its own VoiceOver element */}
        <IconBtn testID={`row-notes-${i}`} label={`Notes for ${name}`} onPress={onNotes}><Icon name="lines" size={20} color={C.mut2} /></IconBtn>
        {/* HIG/context-menus: one of a context menu or an edit menu, not both; the phone has the long press (and the row's VoiceOver actions) */}
        {!native && <Pressable testID={`row-more-${i}`} accessibilityRole="button" accessibilityLabel={`More for ${name}`} onPress={onMenu} hitSlop={6} style={{ width: 36, height: 44, alignItems: 'center', justifyContent: 'center', marginRight: -8 }}>
          <Text style={{ color: C.mut2, fontSize: 18, fontFamily: F.g7 }}>…</Text>
        </Pressable>}
      </Pressable>
    </View>
  );
}

export default function SetListScreen() {
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const boards = useBoards();
  const b = useApi<Build>(`/api/boards/${enc(id)}/build`, { poll: 30000 });
  const [add, setAdd] = useState<null | { mode: 'add' } | { mode: 'swap'; at: number; old: Step }>(null);
  const [gap, setGap] = useState<Gap | null>(null);
  const [rename, setRename] = useState<Step | null>(null);
  const [title, setTitle] = useState(false);
  const [menu, setMenu] = useState<{ title: string; items: MenuItem[] } | null>(null);
  const [listMenu, setListMenu] = useState(false);
  const B = b.data;
  React.useEffect(() => {
    if (B?.board) boards.setCurrent({ id, title: B.board.title });
  }, [B?.board?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  // the Add-a-joke sheet is its own route on the phone and writes the op itself: re-read when this screen is back in front
  const firstFocus = useRef(true);
  useFocusEffect(useCallback(() => {
    if (firstFocus.current) { firstFocus.current = false; return; }
    b.reload();
  }, [b.reload])); // eslint-disable-line react-hooks/exhaustive-deps
  type AddMode = null | { mode: 'add' } | { mode: 'swap'; at: number; old: Step };
  const openAdd = (m: AddMode) => {
    if (!native) return setAdd(m);
    const params = m?.mode === 'swap' ? { board: id, swap: '1', at: String(m.at), old: m.old.joke_id } : { board: id };
    router.push({ pathname: '/add/[board]', params } as any);
  };

  /** One write, one snackbar, and Undo = the inverse op (also his write). The build is re-read after each. */
  const act = useCallback(async (fn: () => Promise<any>, msg: string, inverse?: () => Promise<any>, tone: 'success' | 'warning' = 'success') => {
    try {
      await fn();
      haptics[tone]();
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
  // the notes sheet reads the build from lib/stageNotes (no fetch); no onNext here, so its footer opens the bit
  const openNotes = (i: number) => {
    if (!B) return;
    stageNotes.set({ boardId: id, title: B.board.title, steps, index: i, onNext: null });
    router.push('/bitnotes' as any);
  };
  useEffect(() => () => stageNotes.clear(id), [id]);

  const reorder = (from: number, to: number) => {
    if (from === to) return;
    act(() => boardPatch(id, { move: { from, to } }), `Moved ${nameOf(steps[from])} to #${to + 1}`, () => boardPatch(id, { move: { from: to, to: from } }));
  };
  const remove = (x: Step, i: number) => {
    act(() => boardPatch(id, { remove: { joke_id: x.joke_id } }), `Removed ${nameOf(x)}`, () => boardPatch(id, { add: { joke_id: x.joke_id, at: i } }), 'warning');
  };
  const addJoke = (pk: Pick) => {
    setAdd(null);
    act(() => creative.addItem(id, pk.joke_id), `Added ${pk.short_name}`, () => boardPatch(id, { remove: { joke_id: pk.joke_id } }));
  };
  const swap = (at: number, old: Step, pk: Pick) => {
    setAdd(null);
    act(() => boardPatch(id, { add: { joke_id: pk.joke_id, at }, remove: { joke_id: old.joke_id } }), `Swapped ${nameOf(old)} for ${pk.short_name}`,
      () => boardPatch(id, { add: { joke_id: old.joke_id, at }, remove: { joke_id: pk.joke_id } }));
  };
  const saveShortName = async (x: Step, name: string) => {
    setRename(null);
    const v = name.trim() || null;
    try {
      const w = await decide({ target: 'node', action: 'edit', id: x.joke_id, after: { short_name: v } });
      haptics.success();
      b.reload();
      ui.snack(v ? `Now “${v}” on the list` : 'Back to the default name', async () => { await undo(w.indexes); b.reload(); });
    } catch (e: any) {
      ui.snack(`Couldn't rename: ${e.message}`);
    }
  };
  // HIG/menus: an ellipsis where more input follows; the destructive item last (HIG/context-menus)
  const rowItems = (x: Step, i: number): MenuItem[] => [
    { label: 'Notes', icon: 'text.alignleft', onPress: () => openNotes(i), testID: `row-notes-menu-${i}` },
    { label: 'Open the bit', icon: 'doc.text', onPress: () => router.push(`/joke/${enc(x.joke_id)}` as any), testID: `row-openbit-${i}` },
    { label: 'Swap…', icon: 'arrow.triangle.swap', onPress: () => openAdd({ mode: 'swap', at: i, old: x }), testID: `row-swap-${i}` },
    { label: 'Rename short name…', icon: 'pencil', onPress: () => setRename(x), testID: `row-rename-${i}` },
    { label: 'Move to top', icon: 'arrow.up.to.line', onPress: () => reorder(i, 0), disabled: i === 0, testID: `row-top-${i}` },
    { label: 'Move to bottom', icon: 'arrow.down.to.line', onPress: () => reorder(i, steps.length - 1), disabled: i === steps.length - 1, testID: `row-bottom-${i}` },
    { label: 'Remove from this list', icon: 'trash', onPress: () => remove(x, i), destructive: true, testID: `row-remove-${i}` },
  ];
  const openMenu = (x: Step, i: number) => {
    haptics.light();
    setMenu({ title: nameOf(x), items: rowItems(x, i) });
  };

  if (b.rawError && !B) return <Screen title="Set list" back backLabel="Sets"><ErrorCard error={String(b.rawError)} onRetry={b.reload} /></Screen>;
  if (!B) return <Screen title="Set list" back backLabel="Sets"><Skeleton lines={6} height={22} testID="setlist-skeleton" /></Screen>;
  const target = B.board.target_s;
  const left = target ? target - B.runtime_s : null;
  const pct = target ? Math.min(100, (B.runtime_s / target) * 100) : 0;
  const addLabel = `Add a joke${left != null ? ` · ${left > 0 ? `${mmss(left)} left` : 'full'}` : ''}`;
  const stage = () => router.push(`/stage/${enc(id)}` as any);

  // the list's own menu: Rename… · Duplicate · Share… (the plain-text list, HIG/activity-views) · Delete (not undoable: an action sheet confirms)
  const share = async () => {
    const text = [B.board.title, ...steps.map((x, i) => `${i + 1}. ${nameOf(x)}`)].join('\n');
    try { await Share.share({ message: text, title: B.board.title }); } catch { ui.snack('Sharing is not available here'); }
  };
  const duplicate = async () => {
    try {
      const v = await creative.createBoard({ title: `${B.board.title} (copy)`, joke_ids: steps.map((x) => x.joke_id), target_s: target || undefined });
      boards.setCurrent({ id: v.board.id, title: v.board.title });
      ui.snack(`Duplicated as ${v.board.title}`);
      router.push(`/list/${enc(v.board.id)}` as any);
    } catch (e: any) { ui.snack(`Couldn't duplicate: ${e.message}`); }
  };
  const del = () => actionSheet({ title: `Delete ${B.board.title}?`, message: 'The bits stay; only this list goes.', options: [
    { label: 'Delete the set list', destructive: true, onPress: async () => {
      try { await boardPatch(id, { delete: true }); ui.snack(`Deleted ${B.board.title}`); if (router.canGoBack()) router.back(); else router.navigate('/sets' as any); }
      catch (e: any) { ui.snack(`Couldn't delete: ${e.message}`); }
    } },
    { label: 'Cancel', cancel: true },
  ] });
  const listItems: MenuItem[] = [
    { label: 'Rename…', onPress: () => setTitle(true), testID: 'list-rename' },
    { label: 'Duplicate', onPress: duplicate, testID: 'list-duplicate' },
    { label: 'Share…', onPress: share, testID: 'list-share' },
    { label: 'Delete', onPress: del, destructive: true, testID: 'list-delete' },
  ];

  const header = native ? undefined : (
    <TopBar title={B.board.title} back backLabel="Sets" onBack={() => (router.canGoBack() ? router.back() : router.navigate('/sets' as any))}
      right={<View style={{ flexDirection: 'row', alignItems: 'center' }}>
        <Pressable testID="list-more" accessibilityRole="button" accessibilityLabel="More: rename, duplicate, share or delete the set list" onPress={() => setListMenu(true)} style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}><Text style={{ color: C.amber, fontSize: 20, fontFamily: F.g7 }}>…</Text></Pressable>
        <TextBtn testID="stage" label="Record" onPress={stage} />
      </View>} />
  );

  return (
    <Screen title={B.board.title} testID="setlist-screen" header={header} backLabel="Sets" onRefresh={b.reload}
      dock={native ? undefined : <Btn testID="add-joke" v="primary" label={addLabel} onPress={() => openAdd({ mode: 'add' })} />}>
      {native && (
        <>
          {/* HIG/toolbars: one prominent action (Record: the record glyph, never play, which means playback) on the trailing side; the rest in a menu; destructive last */}
          <Stack.Toolbar placement="right">
            <Stack.Toolbar.Menu icon="ellipsis.circle" accessibilityLabel="More">
              <Stack.Toolbar.MenuAction icon="pencil" onPress={() => setTitle(true)}>Rename…</Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="plus.square.on.square" onPress={duplicate}>Duplicate</Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="square.and.arrow.up" onPress={share}>Share…</Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="trash" destructive onPress={del}>Delete</Stack.Toolbar.MenuAction>
            </Stack.Toolbar.Menu>
            <Stack.Toolbar.Button icon="record.circle" variant="prominent" accessibilityLabel="Record this set" onPress={stage}>Record</Stack.Toolbar.Button>
          </Stack.Toolbar>
          {/* the dock becomes a bottom toolbar inside the tab's stack, so the tab bar stays visible (HIG/tab-bars) */}
          <Stack.Toolbar placement="bottom">
            <Stack.Toolbar.Spacer />
            <Stack.Toolbar.Button variant="prominent" accessibilityLabel={addLabel} onPress={() => openAdd({ mode: 'add' })}>{addLabel}</Stack.Toolbar.Button>
            <Stack.Toolbar.Spacer />
          </Stack.Toolbar>
        </>
      )}
      <View style={{ gap: 6 }}>
        {!native && <Pressable testID="list-title" accessibilityRole="button" accessibilityLabel={`${B.board.title}. Rename the set list`} onPress={() => setTitle(true)} style={{ minHeight: 44, justifyContent: 'center' }}>
          <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.black, fontSize: 24, letterSpacing: -0.4, color: C.text }}>{B.board.title} <Text style={{ color: C.amber, fontSize: 16, fontFamily: F.medium }}>Rename</Text></Text>
        </Pressable>}
        <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 6 }}>
          <Text testID="list-runtime" maxFontSizeMultiplier={1.3} style={{ fontFamily: F.black, fontSize: 30, letterSpacing: -0.5, color: C.text, fontVariant: ['tabular-nums'] }}>{mmss(B.runtime_s)}</Text>
          {!!target && <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.medium, fontSize: 16, color: C.mut2 }}>/ {mmss(target)}</Text>}
          <Text testID="list-status" maxFontSizeMultiplier={1.4} style={{ marginLeft: 8, fontFamily: F.semi, fontSize: 13, color: B.status === 'on' ? C.green : B.status === 'over' ? C.gapText : C.mut2 }}>{B.status_words}</Text>
        </View>
        <View style={{ height: 6, borderRadius: 3, backgroundColor: C.card, overflow: 'hidden' }} accessibilityLabel={`${Math.round(pct)} percent of the slot`}>
          <View style={{ width: `${pct}%`, height: '100%', backgroundColor: B.status === 'over' ? C.gap : C.amber }} />
        </View>
      </View>

      {!steps.length && <Empty text="Empty list. Add a joke below, or say one." />}
      <DragList testID="list-rows" items={steps} keyOf={(x) => x.joke_id} onReorder={reorder}
        render={(x: Step, i, handle) => (
          <View>
            {x.transition_in?.weak && (
              <Pressable testID={`gap-${i}`} accessibilityRole="button" accessibilityLabel={`Gap before ${nameOf(x)}: ${x.transition_in.words}. Add a bridge`}
                onPress={() => setGap({ from: x.transition_in!.from, to: x.transition_in!.to, words: x.transition_in!.words })}
                style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44, paddingLeft: 74 }}>
                <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: C.gap }} />
                <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.medium, fontSize: 13, color: C.gapText }}>Gap here · add a bridge</Text>
              </Pressable>
            )}
            <SwipeRow testID={`row-swipe-${i}`}
              trailing={{ label: 'Remove', destructive: true, onPress: () => remove(x, i), testID: `row-swipe-remove-${i}` }}
              second={{ label: 'Swap', onPress: () => openAdd({ mode: 'swap', at: i, old: x }), testID: `row-swipe-swap-${i}` }}
              onLongPress={ios ? undefined : () => openMenu(x, i)}>
              <RowMenu href={`/joke/${enc(x.joke_id)}`} items={rowItems(x, i)}>
                <Row x={x} i={i} handle={handle} items={rowItems(x, i)} onOpen={() => router.push(`/joke/${enc(x.joke_id)}` as any)} onMenu={() => openMenu(x, i)} onNotes={() => openNotes(i)} />
              </RowMenu>
            </SwipeRow>
          </View>
        )} />
      {!!steps.length && <T v="small" style={{ textAlign: 'center' }}>{native ? 'Hold the grip to drag · tap a row to open it · the lines button reads its notes · hold a row for Swap, Rename, Move, Remove' : 'Hold the grip to drag · tap a row to open it · the lines button reads its notes · … for Swap, Rename, Move, Remove'}</T>}
      {B.notes.map((n) => <T key={n.rule} v="small" testID={`note-${n.rule}`}>{n.text}</T>)}

      {!native && <AddJokeSheet visible={!!add} onClose={() => setAdd(null)} boardId={id} inSet={inSet} fits={B.next} left={left} mode={add?.mode || 'add'}
        onPick={(pk) => (add?.mode === 'swap' ? swap(add.at, add.old, pk) : addJoke(pk))} />}
      <BridgeSheet key={gap ? `${gap.from}>${gap.to}` : 'none'} gap={gap} boardId={id} titleOf={titleOf} onClose={() => setGap(null)} />
      <Sheet visible={!!rename} onClose={() => setRename(null)} label="Rename short name" testID="rename-sheet">
        <SheetHead title="Short name" onCancel={() => setRename(null)} testID="rename" />
        {rename && <ShortNameForm x={rename} onSave={(v) => saveShortName(rename, v)} />}
      </Sheet>
      <Sheet visible={title} onClose={() => setTitle(false)} label="Rename the set list" testID="title-sheet">
        <SheetHead title="Rename the set list" onCancel={() => setTitle(false)} testID="title" />
        <TitleForm title={B.board.title} onSave={(t) => { setTitle(false); const old = B.board.title; act(() => boardPatch(id, { title: t }), `Renamed to ${t}`, () => boardPatch(id, { title: old })); }} />
      </Sheet>
      <MenuSheet visible={!!menu} title={menu?.title} items={menu?.items || []} onClose={() => setMenu(null)} testID="row-menu" />
      <MenuSheet visible={listMenu} title={B.board.title} items={listItems} onClose={() => setListMenu(false)} testID="list-menu" />
    </Screen>
  );
}

function ShortNameForm({ x, onSave }: { x: Step; onSave: (v: string) => void }) {
  const [v, setV] = useState(x.short_name || shortNameOf(x.title));
  return (
    <View style={{ gap: 8 }}>
      <T v="mut">What you’d write on the paper list. Up to 24 letters. The full title stays “{x.title}”.</T>
      <TextInput testID="shortname-input" value={v} onChangeText={setV} maxLength={24} autoFocus placeholder={shortNameOf(x.title)} placeholderTextColor={C.dim} clearButtonMode="while-editing" accessibilityLabel="Short name"
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
      <TextInput testID="title-input" value={t} onChangeText={setT} autoFocus clearButtonMode="while-editing" accessibilityLabel="Set list title" style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      <Btn label="Save" v="primary" disabled={!t.trim()} onPress={() => onSave(t.trim())} />
    </View>
  );
}
