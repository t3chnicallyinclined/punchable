// A set is a page (docs/design/SPEC-2026-11-17-one-notebook.md screen 5): the title is a field at the top (tap and
// type); under it "5 bits · about 7 min · you have 8" (tap it: the slot's length); a numbered list of bits, each with
// his notes under it and its time; a last row he types in: `@` plus letters, or just letters, lists the bits that
// match (rules/setPageRules offers: never one already here) and "New bit called …"; the grip moves a row (hold and
// drag; VoiceOver: Move up / Move down); at the bottom Record this set and Film (the walk-through, /run/<id>). The
// seams between bits are off until ⋯ › Show how the bits connect. What follows is the screen as it was built before:
// Inside a set list (MixBuild, docs/UX_2026-10.md). A set list IS a board: this screen is the board's set builder
// (GET /api/boards/<id>/build) drawn as the paper list: numbered rows with the short name and the time, his bullet
// notes under each (lib/BulletNotes; SPEC 2026-11-16 §3.3: Return adds a note, Backspace on an empty one removes it;
// while he has written none, the bit's own words show muted), and between two rows the seam (rules/setNoteRules
// seamOf: a callback, the usual order, "Same shape twice", or "No bridge yet" with a bridge to draft; tap it to read
// why); the time bar; drag to reorder; "Add a bit · N left" at the bottom. HIG (the audit): tap a row OPENS the bit; trailing swipe = Remove (red, a full swipe
// runs it, Undo in the snackbar) + Swap; long-press = Open · Swap · Rename · Move to top · Move to bottom · Remove;
// the title and the "…" More button rename; "Length…" in the same menu sets the slot (5 · 7 · 10 · 15 · custom,
// PATCH {target_s}; SPEC 2026-10-30: the length a new list is born with is 7, changed here, never asked up front);
// pull to refresh. A trailing 44-pt notes button per row (and "Notes" first
// in the menu) opens the bit's words as a sheet (/bitnotes, SPEC 2026-10-29 §2); the row tap stays the workbench.
// Every edit is one of the board's own per-item ops (add / remove / move): never a whole-list replace, so two
// devices open at once don't clobber each other. Undo in the snackbar is the inverse op (a real write by him).
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { PixelRatio, Platform, Pressable, Share, Text, TextInput, View } from 'react-native';
import { Stack, router, useFocusEffect, useLocalSearchParams } from 'expo-router';
import { actionSheet } from '@/lib/actionSheet';
import { JokeSummary, decide, patch, undo, useApi, useOnline } from '@/lib/api';
import { BarGround } from '@/lib/BarGround';
import { creative } from '@/lib/creative';
import { AddJokeSheet, Pick } from '@/lib/AddJokeSheet';
import { BridgeSheet, Gap } from '@/lib/bridge';
import { BulletNotes } from '@/lib/BulletNotes';
import { DragList } from '@/lib/DragList';
import { haptics } from '@/lib/haptics';
import { stackMeta } from '@/rules/bitLines';
import { mmss } from '@/rules/format';
import { useBoards } from '@/lib/boards';
import { Build, Step } from '@/lib/simple';
import { shortNameOf } from '@/lib/say';
import { stageNotes } from '@/lib/stageNotes';
import { AlsoKept, useTopicOf } from '@/lib/AlsoKept';
import { Seam, seamLabel, seamOf, undoLabel } from '@/rules/setNoteRules';
import * as SP from '@/rules/setPageRules';
import { SwipeRow } from '@/lib/SwipeRow';
import { RowMenu, rowActions } from '@/lib/RowMenu';
import { C, F } from '@/lib/theme';
import { AIBadge, Btn, ErrorCard, Icon, IconBtn, MenuItem, MenuSheet, Sheet, SheetHead, Skeleton, T, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const enc = encodeURIComponent;
const boardPatch = (id: string, body: any) => patch<any>(`/api/boards/${enc(id)}`, body);

const native = Platform.OS !== 'web';
const ios = Platform.OS === 'ios'; // the system context menu owns the long press there
const LENGTHS = [5, 7, 10, 15];
/** The length sheet's state while open: a chip or a typed number of minutes (the typed one wins while it has text). */
type Len = { mins: number; custom: string };
const lenMinutes = (l: Len) => (l.custom.trim() ? Number(l.custom) : l.mins);
const lenValid = (l: Len) => { const m = lenMinutes(l); return Number.isInteger(m) && m >= 1 && m <= 240; };

function Row({ x, i, handle, onOpen, onMenu, items }: { x: Step; i: number; handle: React.ReactNode; onOpen: () => void; onMenu: () => void; items: MenuItem[] }) {
  const name = x.short_name || shortNameOf(x.title);
  return (
    <View testID={`row-${i}`} style={{ borderRadius: 16, paddingVertical: 2 }}>
      <Pressable accessibilityRole="button" accessibilityLabel={`${i + 1}. ${name}, ${mmss(x.duration_s)}. Open the bit; long press for actions`} onPress={onOpen} onLongPress={ios ? undefined : onMenu} delayLongPress={350}
        {...rowActions(items)}
        style={(st: any) => ({ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 48, paddingRight: 4, borderRadius: 16, backgroundColor: st.hovered ? C.card2 : 'transparent' })}>
        {handle}
        <Text maxFontSizeMultiplier={1.4} style={{ width: 22, fontFamily: F.body, fontSize: 17, color: C.mut2, textAlign: 'right' }}>{i + 1}</Text>
        <Text testID={`row-name-${i}`} numberOfLines={1} maxFontSizeMultiplier={1.6} style={{ flex: 1, minWidth: 0, fontFamily: F.semi, fontSize: 18, color: C.text }}>{name}</Text>
        <Text testID={`row-time-${i}`} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 15, color: C.mut2, fontVariant: ['tabular-nums'] }}>{x.estimated ? '~' : ''}{mmss(x.duration_s)}</Text>
        {/* HIG/context-menus: one of a context menu or an edit menu, not both; the phone has the long press (and the row's VoiceOver actions) */}
        {!native && <Pressable testID={`row-more-${i}`} accessibilityRole="button" accessibilityLabel={`More for ${name}`} onPress={onMenu} hitSlop={6} style={{ width: 36, height: 44, alignItems: 'center', justifyContent: 'center', marginRight: -8 }}>
          <Text style={{ color: C.mut2, fontSize: 18, fontFamily: F.g7 }}>…</Text>
        </Pressable>}
      </Pressable>
    </View>
  );
}

/** The left edge the notes and the seams share with a row's name: the grip (44), a gap (8), the number (22), a gap
 *  (8). At the large text sizes the notes move out to the grip's edge, so a note keeps its width (HIG.md §6). */
const INSET = 82;
const INSET_LARGE = 44;

/** The seam between two bits: one line that says what ties them (or that nothing does); a tap opens why, and what he
 *  can do about it. The mark and the words carry the meaning; the colour only repeats it (HIG/color). */
function SeamRow({ seam, i, inset, before, after, open, onToggle, onBridge }: { seam: Seam; i: number; inset: number; before: string; after: string; open: boolean; onToggle: () => void; onBridge: () => void }) {
  // coral is for the one seam that asks him to move something; the rest are quiet
  const color = seam.tone === 'warn' ? C.gapText : C.mut2;
  return (
    <View>
      <Pressable testID={`seam-${i}`} accessibilityRole="button" accessibilityState={{ expanded: open }} accessibilityLabel={seamLabel(seam, before, after)}
        accessibilityHint={open ? 'Hides why' : 'Shows why'} onPress={onToggle}
        style={({ pressed }) => ({ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44, paddingLeft: inset, opacity: pressed ? 0.6 : 1 })}>
        {seam.mark === 'check' ? <Icon name="check" size={14} color={color} sw={2.6} />
          : seam.mark === 'warn' ? <Icon name="warn" size={14} color={color} sw={2.4} />
            : <View style={{ width: 6, height: 6, marginHorizontal: 4, borderRadius: 3, backgroundColor: C.dim }} />}
        <Text testID={`seam-label-${i}`} maxFontSizeMultiplier={1.4} style={{ flexShrink: 1, fontFamily: F.medium, fontSize: 13, color }}>{seam.label}</Text>
        {/* the app's reading says so before he opens it (HIG/generative-ai: say where AI is used) */}
        {seam.ai && <AIBadge testID={`seam-ai-${i}`} />}
        <Icon name={open ? 'chevronDown' : 'chevron'} size={12} color={C.dim} />
      </Pressable>
      {open && (
        <View testID={`seam-why-${i}`} style={{ marginLeft: inset, marginRight: 8, marginBottom: 8, paddingLeft: 12, paddingVertical: 4, borderLeftWidth: 2, borderLeftColor: C.border, gap: 8 }}>
          {!!seam.detail && <Text maxFontSizeMultiplier={2} style={{ fontFamily: F.body, fontSize: 15, lineHeight: 21, color: C.text2 }}>{seam.detail}</Text>}
          {seam.tone === 'warn' && <T v="small">Hold the grip to move one of them.</T>}
          {seam.bridge && <Btn small testID={`seam-bridge-${i}`} label="Draft a bridge" accessibilityLabel={`Draft a bridge from ${before} to ${after}`} onPress={onBridge} style={{ alignSelf: 'flex-start' }} />}
        </View>
      )}
    </View>
  );
}

export default function SetListScreen() {
  const p = useLocalSearchParams<{ id: string; rename?: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const boards = useBoards();
  const b = useApi<Build>(`/api/boards/${enc(id)}/build`, { poll: 30000 });
  const topic = useTopicOf(id);   // a set made from a Rant topic (SPEC 2026-11-16 Slice 6): Also kept, Draft it as one piece…
  const [add, setAdd] = useState<null | { mode: 'add' } | { mode: 'swap'; at: number; old: Step }>(null);
  const [gap, setGap] = useState<Gap | null>(null);
  const [rename, setRename] = useState<Step | null>(null);
  const [title, setTitle] = useState(false);
  const [len, setLen] = useState<Len | null>(null);
  const [menu, setMenu] = useState<{ title: string; items: MenuItem[] } | null>(null);
  const [listMenu, setListMenu] = useState(false);
  const [openSeam, setOpenSeam] = useState<string | null>(null);
  // the seams between bits (why two sit together) are drawn only when he asks: the page is the list first
  const [seams, setSeams] = useState(false);
  // the last row: what he is typing (`@par`), and every bit it can find (the notebook's own list, already on the phone)
  const [typed, setTyped] = useState('');
  const allBits = useApi<{ items: JokeSummary[] }>('/api/jokes?sort=recent&limit=500');
  const addInput = useRef<any>(null);
  // the last change made on this screen that can be taken back, for the menu's Undo: no clock on it, unlike the
  // snack's (HIG.md §7; HIG/accessibility: prefer an explicit action to a timed one)
  const [lastUndo, setLastUndo] = useState<{ what: string; run: () => Promise<void> } | null>(null);
  const online = useOnline();
  const inset = stackMeta(PixelRatio.getFontScale()) ? INSET_LARGE : INSET;
  const B = b.data;
  React.useEffect(() => {
    if (B?.board) boards.setCurrent({ id, title: B.board.title });
  }, [B?.board?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  // "Rename" on the snack after a new list (lib/newSetList) navigates here with a fresh `rename` value: open the sheet
  // once per value, once the list is on screen; a cancel is not undone by a reload
  const renamed = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (p.rename && B && renamed.current !== p.rename) { renamed.current = p.rename; setTitle(true); }
  }, [p.rename, B]);
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
      const back = inverse ? async () => { await inverse(); b.reload(); setLastUndo(null); } : undefined;
      setLastUndo(back ? { what: msg, run: back } : null);
      ui.snack(msg, back);
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
  /** A tap on a bit the typing row offers: it goes on the end of the set, the row empties and keeps the cursor. */
  const addOffered = (j: JokeSummary) => {
    setTyped('');
    act(() => creative.addItem(id, j.id), `Added ${SP.nameOf(j)}`, () => boardPatch(id, { remove: { joke_id: j.id } }));
  };
  /** "New bit called …": a page with that title is made and put on the end of the set (he writes it when he opens it). */
  const addNew = (name: string) => {
    setTyped('');
    let made: { id: string; indexes: number[] } | null = null;
    act(async () => {
      const w = await decide({ target: 'node', action: 'add', after: { type: 'joke', title: name, stage: 'idea' }, reason: 'a new bit named on a set' });
      const jid = w.decisions?.[0]?.id;
      if (!jid) throw new Error('no id came back');
      made = { id: jid, indexes: w.indexes };
      await creative.addItem(id, jid);
      void allBits.reload();
    }, `Made “${name}” and added it`, async () => {
      if (!made) return;
      await boardPatch(id, { remove: { joke_id: made.id } });
      await undo(made.indexes);
      void allBits.reload();
    });
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

  if (b.rawError && !B) return <Screen title="Set" back backLabel="Notebook"><ErrorCard error={String(b.rawError)} onRetry={b.reload} /></Screen>;
  if (!B) return <Screen title="Set" back backLabel="Notebook"><Skeleton lines={6} height={22} testID="setlist-skeleton" /></Screen>;
  const target = B.board.target_s;
  const left = target ? target - B.runtime_s : null;
  const pct = target ? Math.min(100, (B.runtime_s / target) * 100) : 0;
  // what the typing row offers for his letters, and the name a new bit would get
  const taken = new Set(steps.map((x) => x.joke_id));
  const found = SP.offers(allBits.data?.items || [], taken, typed);
  const newName = online ? SP.newBitName(allBits.data?.items || [], typed) : null;
  // the walk-through (SPEC 2026-11-16 Slice 5): Record (sound) or Film, one bit at a time with his notes
  const stage = () => router.push({ pathname: '/run/[id]', params: { id, mode: 'sound' } } as any);
  const film = () => router.push({ pathname: '/run/[id]', params: { id, mode: 'video' } } as any);
  // the Rant tab's "Draft a rant from a topic" (§3.6, §7 Q4): the topic's rant script, for a set made from a topic
  const draftOne = () => topic && router.push({ pathname: '/topic/[topic]/script', params: { topic } } as any);

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
      try { await boardPatch(id, { delete: true }); ui.snack(`Deleted ${B.board.title}`); if (router.canGoBack()) router.back(); else router.navigate('/' as any); }
      catch (e: any) { ui.snack(`Couldn't delete: ${e.message}`); }
    } },
    { label: 'Cancel', cancel: true },
  ] });
  // the slot: a chip or a typed number of minutes; one PATCH, Undo puts the old length back (through `act`)
  const openLength = () => setLen({ mins: target && LENGTHS.includes(Math.round(target / 60)) ? Math.round(target / 60) : 7, custom: target && !LENGTHS.includes(Math.round(target / 60)) ? String(Math.round(target / 60)) : '' });
  const saveLength = () => {
    if (!len || !lenValid(len)) return;
    const m = lenMinutes(len);
    setLen(null);
    if (target && m * 60 === target) return;
    act(() => boardPatch(id, { target_s: m * 60 }), `${m} min set`, target ? () => boardPatch(id, { target_s: target }) : undefined);
  };
  const undoLast = async () => {
    const u = lastUndo;
    if (!u) return;
    setLastUndo(null);
    try { await u.run(); haptics.success(); ui.snack('Undone'); } catch (e: any) { ui.snack(`Couldn't undo: ${e.message}`); b.reload(); }
  };
  const listItems: MenuItem[] = [
    { label: undoLabel(lastUndo?.what), onPress: () => void undoLast(), disabled: !lastUndo, testID: 'list-undo' },
    { label: 'Add a bit that fits…', onPress: () => openAdd({ mode: 'add' }), testID: 'list-add-fits' },
    { label: 'Length…', onPress: openLength, testID: 'list-length' },
    { label: seams ? 'Hide how the bits connect' : 'Show how the bits connect', onPress: () => setSeams((v) => !v), testID: 'list-seams' },
    { label: 'Duplicate', onPress: duplicate, testID: 'list-duplicate' },
    { label: 'Share…', onPress: share, testID: 'list-share' },
    ...(topic ? [{ label: 'Draft it as one piece…', onPress: draftOne, testID: 'list-draft-one' }] : []),
    { label: 'Delete', onPress: del, destructive: true, testID: 'list-delete' },
  ];

  const saveTitle = (t: string) => { const old = B.board.title; act(() => boardPatch(id, { title: t }), `Renamed to ${t}`, () => boardPatch(id, { title: old })); };
  // the mockup's bottom: Record this set, and Film beside it. The record dot is red, the button is not: nothing tapes
  // until the disc on the next screen (HIG/color: red means recording)
  const bar = (
    <BarGround testID="set-bar">
      <View style={{ flexDirection: 'row', gap: 10, paddingHorizontal: 20, paddingTop: 10, paddingBottom: 6 }}>
        <Pressable testID="stage" accessibilityRole="button" accessibilityLabel="Record this set" accessibilityHint="Opens the walk-through; nothing tapes until the red disc" onPress={stage}
          style={({ pressed }) => ({ flex: 1, minHeight: 50, borderRadius: 14, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 8, backgroundColor: pressed ? C.card2 : C.card })}>
          <View style={{ width: 12, height: 12, borderRadius: 6, backgroundColor: C.rec }} />
          <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.bold, fontSize: 17, color: C.text }}>Record this set</Text>
        </Pressable>
        {native && (
          <Pressable testID="film" accessibilityRole="button" accessibilityLabel="Film this set" onPress={film}
            style={({ pressed }) => ({ width: 50, minHeight: 50, borderRadius: 14, alignItems: 'center', justifyContent: 'center', backgroundColor: pressed ? C.card2 : C.card })}>
            <Icon name="video" size={22} color={C.text} />
          </Pressable>
        )}
      </View>
    </BarGround>
  );

  return (
    <Screen title={B.board.title} testID="setlist-screen" back backLabel="Notebook" onRefresh={b.reload} bar={bar}
      // the set's name is the big title field under the bar, so the bar itself stays empty (as on a bit's page)
      nativeHeader={{ headerTitle: () => null }}
      right={native ? undefined : <IconBtn testID="list-more" label="More: undo, length, duplicate, share or delete the set" onPress={() => setListMenu(true)}><Icon name="more" size={22} color={C.amber} /></IconBtn>}>
      {native && (
        // HIG/toolbars: the rest in a menu; destructive last. Record and Film are the page's bottom bar.
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Menu icon="ellipsis" accessibilityLabel="More">
            <Stack.Toolbar.MenuAction icon="arrow.uturn.backward" disabled={!lastUndo} onPress={() => void undoLast()}>{undoLabel(lastUndo?.what)}</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="text.badge.plus" onPress={() => openAdd({ mode: 'add' })}>Add a bit that fits…</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="timer" onPress={openLength}>Length…</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="link" isOn={seams} onPress={() => setSeams((v) => !v)}>Show how the bits connect</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="plus.square.on.square" onPress={duplicate}>Duplicate</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="square.and.arrow.up" onPress={share}>Share…</Stack.Toolbar.MenuAction>
            {!!topic && <Stack.Toolbar.MenuAction icon="sparkles" onPress={draftOne}>Draft it as one piece…</Stack.Toolbar.MenuAction>}
            <Stack.Toolbar.MenuAction icon="trash" destructive onPress={del}>Delete</Stack.Toolbar.MenuAction>
          </Stack.Toolbar.Menu>
        </Stack.Toolbar>
      )}
      <View style={{ gap: 4 }}>
        <SetTitle key={`${id}|${B.board.title}`} value={B.board.title} editable={online} autoFocus={title} onSave={(t) => { setTitle(false); saveTitle(t); }} />
        {/* the mockup's line; a tap sets how long he has (HIG/typography: Subheadline) */}
        <Pressable testID="list-summary" accessibilityRole="button" accessibilityLabel={`${SP.summaryLine(steps.length, B.runtime_s, target)}. ${B.status_words}`} accessibilityHint="Sets how long the slot is" onPress={openLength} style={{ minHeight: 44, justifyContent: 'center' }}>
          <Text testID="list-runtime" maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 15, color: B.status === 'over' ? C.gapText : C.mut2 }}>{SP.summaryLine(steps.length, B.runtime_s, target)}</Text>
        </Pressable>
        <View style={{ height: 4, borderRadius: 2, backgroundColor: C.card, overflow: 'hidden' }} accessibilityLabel={`${Math.round(pct)} percent of the slot`}>
          <View style={{ width: `${pct}%`, height: '100%', backgroundColor: B.status === 'over' ? C.gap : C.amber }} />
        </View>
      </View>

      <DragList testID="list-rows" items={steps} keyOf={(x) => x.joke_id} onReorder={reorder}
        render={(x: Step, i, handle) => {
          const seam = seams && i > 0 ? seamOf(x.transition_in) : null;
          return (
          <View>
            {seam && (
              <SeamRow seam={seam} i={i} inset={inset} before={nameOf(steps[i - 1])} after={nameOf(x)} open={openSeam === seam.key}
                onToggle={() => setOpenSeam(openSeam === seam.key ? null : seam.key)}
                onBridge={() => setGap({ from: x.transition_in!.from, to: x.transition_in!.to, words: x.transition_in!.words })} />
            )}
            <SwipeRow testID={`row-swipe-${i}`}
              trailing={{ label: 'Remove', destructive: true, onPress: () => remove(x, i), testID: `row-swipe-remove-${i}` }}
              second={{ label: 'Swap', onPress: () => openAdd({ mode: 'swap', at: i, old: x }), testID: `row-swipe-swap-${i}` }}
              onLongPress={ios ? undefined : () => openMenu(x, i)}>
              <RowMenu href={`/joke/${enc(x.joke_id)}`} items={rowItems(x, i)}>
                <Row x={x} i={i} handle={handle} items={rowItems(x, i)} onOpen={() => router.push(`/joke/${enc(x.joke_id)}` as any)} onMenu={() => openMenu(x, i)} />
              </RowMenu>
            </SwipeRow>
            {/* his notes for this bit in this set, under its row; they travel with the bit when it is dragged */}
            <BulletNotes testID={`notes-${i}`} boardId={id} jokeId={x.joke_id} name={nameOf(x)} step={x} canEdit={online} why="offline" indent={inset}
              onSaved={(notes) => { stageNotes.setNotes(id, x.joke_id, notes); b.reload(); }}
              onUndoable={(what, undo) => setLastUndo(undo ? { what, run: undo } : null)} />
          </View>
          );
        }} />
      {/* the last row: type to add (the mockup's `@par`); the list under it is the bits that match, then a new one */}
      <View testID="add-row" style={{ gap: 6 }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 48, borderBottomWidth: 1, borderBottomColor: C.hairline }}>
          <View style={{ width: 44, alignItems: 'center' }}><Icon name="plus" size={18} color={C.mut2} /></View>
          <Text maxFontSizeMultiplier={1.4} style={{ width: 22, fontFamily: F.body, fontSize: 17, color: C.mut2, textAlign: 'right' }}>{steps.length + 1}</Text>
          <TextInput ref={addInput} testID="add-input" value={typed} onChangeText={setTyped} editable={online} autoCapitalize="none" autoCorrect={false} returnKeyType="done"
            submitBehavior="submit" blurOnSubmit={false} onSubmitEditing={() => { if (found.length) addOffered(found[0]); else if (newName) addNew(newName); }}
            placeholder={online ? (steps.length ? 'Add a bit' : 'Add a bit: type its name') : 'Add a bit · offline'} placeholderTextColor={C.dim}
            accessibilityLabel="Add a bit" accessibilityHint="Type its name; the bits that match are listed under this field" maxFontSizeMultiplier={1.6}
            style={[{ flex: 1, minWidth: 0, minHeight: 44, fontFamily: F.body, fontSize: 18, color: C.text, padding: 0 }, native ? null : ({ outlineStyle: 'none' } as any)]} />
        </View>
        {(found.length > 0 || !!newName) && (
          <View testID="add-offers" style={{ borderRadius: 12, backgroundColor: C.card, marginLeft: inset }}>
            {found.map((j, k) => (
              <Pressable key={j.id} testID={`add-offer-${k}`} accessibilityRole="button" accessibilityLabel={`Add ${SP.nameOf(j)} to this set`} onPress={() => addOffered(j)}
                style={({ pressed }) => ({ minHeight: 44, paddingHorizontal: 14, justifyContent: 'center', borderBottomWidth: 1, borderBottomColor: C.border, backgroundColor: pressed ? C.card2 : 'transparent' })}>
                <Text numberOfLines={1} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 16, color: C.text }}>{SP.nameOf(j)}</Text>
              </Pressable>
            ))}
            {!!newName && (
              <Pressable testID="add-new" accessibilityRole="button" accessibilityLabel={SP.newBitLabel(newName)} accessibilityHint="Makes the bit and adds it to this set" onPress={() => addNew(newName)}
                style={({ pressed }) => ({ minHeight: 44, paddingHorizontal: 14, justifyContent: 'center', backgroundColor: pressed ? C.card2 : 'transparent' })}>
                <Text numberOfLines={1} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 16, color: C.amber }}>{SP.newBitLabel(newName)}</Text>
              </Pressable>
            )}
          </View>
        )}
        <Text testID="add-hint" maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 14, color: C.mut2, paddingTop: 6 }}>{SP.HINT}</Text>
      </View>
      {B.notes.map((n) => <T key={n.rule} v="small" testID={`note-${n.rule}`}>{n.text}</T>)}

      {!native && <AddJokeSheet visible={!!add} onClose={() => setAdd(null)} boardId={id} inSet={inSet} fits={B.next} left={left} mode={add?.mode || 'add'}
        onPick={(pk) => (add?.mode === 'swap' ? swap(add.at, add.old, pk) : addJoke(pk))} />}
      <AlsoKept topic={topic} />
      <BridgeSheet key={gap ? `${gap.from}>${gap.to}` : 'none'} gap={gap} boardId={id} titleOf={titleOf} onClose={() => setGap(null)} />
      <Sheet visible={!!rename} onClose={() => setRename(null)} label="Rename short name" testID="rename-sheet">
        <SheetHead title="Short name" onCancel={() => setRename(null)} testID="rename" />
        {rename && <ShortNameForm x={rename} onSave={(v) => saveShortName(rename, v)} />}
      </Sheet>
      {/* HIG/sheets: a short request for one value; Cancel leading, Done trailing (the kit's Sheet until A5's route sheet lands) */}
      <Sheet visible={!!len} onClose={() => setLen(null)} label="How long is the set?" testID="length-sheet">
        <SheetHead title="How long is the set?" onCancel={() => setLen(null)} onDone={saveLength} doneDisabled={!len || !lenValid(len)} testID="length" />
        {len && (
          <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap', alignItems: 'center' }} accessibilityRole="radiogroup" accessibilityLabel="Set length">
            {LENGTHS.map((m) => <Btn key={m} small testID={`length-${m}`} v={len.mins === m && !len.custom.trim() ? 'primary' : 'plain'} label={`${m} min`} accessibilityLabel={`${m} minutes`} onPress={() => setLen({ mins: m, custom: '' })} />)}
            {/* HIG/virtual-keyboards: a number pad for a number */}
            <TextInput testID="length-custom" value={len.custom} onChangeText={(t) => setLen({ ...len, custom: t })} placeholder="custom" placeholderTextColor={C.dim} keyboardType="number-pad" returnKeyType="done" onSubmitEditing={saveLength} accessibilityLabel="Custom length in minutes" maxLength={3}
              style={{ width: 84, minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 12, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 15 }} />
          </View>
        )}
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
/** The set's title: a field he types straight into (HIG/typography Title 1). Leaving it with other words renames the
 *  set (Undo in the message). `autoFocus` when he came from "Rename" on a new set's message. */
function SetTitle({ value, editable, autoFocus, onSave }: { value: string; editable: boolean; autoFocus?: boolean; onSave: (t: string) => void }) {
  const [v, setV] = useState(value);
  const sent = useRef(value);
  const leave = () => {
    const t = v.replace(/\s+/g, ' ').trim();
    if (!t || t === sent.current) return;
    sent.current = t;
    onSave(t);
  };
  return (
    <TextInput testID="list-title" value={v} onChangeText={(t) => setV(t.replace(/\n/g, ' '))} editable={editable} autoFocus={autoFocus} multiline scrollEnabled={false}
      submitBehavior="blurAndSubmit" blurOnSubmit returnKeyType="done" onBlur={leave} maxLength={120}
      accessibilityLabel="Set title" accessibilityHint="Renames the set when you leave the field" maxFontSizeMultiplier={1.8} placeholder="Title" placeholderTextColor={C.dim}
      style={[{ fontFamily: F.g7, fontSize: 28, lineHeight: 34, letterSpacing: -0.6, color: C.text, padding: 0, margin: 0 }, native ? null : ({ outlineStyle: 'none' } as any)]} />
  );
}
