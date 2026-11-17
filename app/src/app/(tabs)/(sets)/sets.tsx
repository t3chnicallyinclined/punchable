// Sets (MixSets, docs/UX_2026-10.md): HIS set lists as cards (lib/SetCard: the name, how many bits and how long, the
// bits' names in order; the boards list carries `names`, one request), tonight's first. What he recorded is on Tapes
// (SPEC 2026-11-16 Slice 6: the Recorded segment moved there). A topic of the old Rant tab that no list was made from
// yet is listed under "From the Rant tab" and opens its old page; once `cargo xtask rants-to-sets` made its list, only
// the list shows (rules/tapesRules). A card's Record and Film open the walk-through (/run/<id>, Slice 5). A set list IS a board
// (/list/<id>); "+" starts today's list at once and opens it (lib/newSetList, SPEC 2026-10-30: one tap to an open
// list, the name and the length are defaults he changes from the list's own menu). The lists the app made (`source` "inferred") are drafts: ONE
// collapsed row at the bottom ("15 drafts from your Insights"), each with "Use this" (an owner write: the list becomes
// his) and Archive (undoable: `{archive: true}` / `{unarchive: true}`); nothing is ever chosen for him
// (docs/design/SPEC-2026-10-29-set-lists-and-stage-notes.md §1). Archived lists live on /archived (the toolbar menu).
// 2026-11-07 (SPEC 2026-11-06 R2, Tonight retired): tonight's list as one bar and Record a set open the tab, above
// the segments: he picks tonight's set here.
import React, { useCallback, useState } from 'react';
import { Platform, Pressable, Text, TextInput, View } from 'react-native';
import { Stack, router } from 'expo-router';
import { BoardListItem, creative, useCreative } from '@/lib/creative';
import { ApiError, get, patch, useApi } from '@/lib/api';
import { Build } from '@/lib/simple';
import { actionSheet } from '@/lib/actionSheet';
import { RowMenu, rowActions } from '@/lib/RowMenu';
import { SwipeRow } from '@/lib/SwipeRow';
import { topicsNotYetSets } from '@/rules/tapesRules';
import type { Topic } from '@/rules/rantRules';
import { draftsLabel, splitLists } from '@/rules/setListRules';
import { SetCard } from '@/lib/SetCard';
import { useBoards } from '@/lib/boards';
import { getPrefs, setPrefs } from '@/lib/prefs';
import { newSetList } from '@/lib/newSetList';
import { C, F } from '@/lib/theme';
import { haptics } from '@/lib/haptics';
import { Avatar, Btn, Empty, ErrorCard, FixtureBanner, Icon, IconBtn, List, ListRow, MenuItem, MenuSheet, Sheet, SheetHead, Skeleton, T, TextBtn, useUI, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TonightCard } from '@/lib/SetBar';
import { useTonight } from '@/lib/tonight';

const enc = encodeURIComponent;
const ios = Platform.OS === 'ios'; // the system context menu owns the long press there

function RenameForm({ title, onSave }: { title: string; onSave: (t: string) => void }) {
  const [t, setT] = useState(title);
  return (
    <View style={{ gap: 8 }}>
      <TextInput testID="card-title-input" value={t} onChangeText={setT} autoFocus clearButtonMode="while-editing" accessibilityLabel="Set list title"
        style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      <Btn label="Save" v="primary" disabled={!t.trim()} onPress={() => onSave(t.trim())} />
    </View>
  );
}

/** A draft the app made: the title and size, "Use this" beside it (its own 44-pt target, so VoiceOver reaches both). */
function DraftRow({ x, items, onOpen, onUse, onMenu }: { x: BoardListItem; items: MenuItem[]; onOpen: () => void; onUse: () => void; onMenu?: () => void }) {
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 56 }}>
      <Pressable testID={`draft-${x.id}`} accessibilityRole="button" accessibilityLabel={`${x.title}, ${x.n} jokes, about ${x.runtime}. Open`} onPress={onOpen} onLongPress={onMenu} delayLongPress={350} {...rowActions(items)}
        style={{ flex: 1, minWidth: 0, paddingVertical: 10 }}>
        <T v="semi" numberOfLines={1}>{x.title}</T>
        <T v="mut">{x.n} jokes · ~{x.runtime}</T>
      </Pressable>
      <TextBtn testID={`draft-use-${x.id}`} label="Use this" accessibilityLabel={`Use this list: ${x.title}`} onPress={onUse} />
    </View>
  );
}

export default function Sets() {
  const wide = useWide();
  const cur = useBoards();
  const b = useCreative<{ boards: BoardListItem[] }>('boards', creative.boards, { poll: 30000 });
  // the Rant tab's topics (Slice 6): each one no list was made from yet, until the migration makes them sets. The
  // archived lists count too: a set he archived still means its topic was made into one.
  const tp = useApi<{ topics: Topic[] }>('/api/topics');
  const arch = useApi<{ boards: BoardListItem[] }>(tp.data?.topics?.length ? '/api/boards?archived=1' : null);
  const fromRant = topicsNotYetSets(tp.data?.topics || [], [...(b.data?.boards || []), ...(arch.data?.boards || [])]);
  const [menu, setMenu] = useState<{ title: string; items: MenuItem[] } | null>(null);
  const [rename, setRename] = useState<BoardListItem | null>(null);
  const [draftsOpen, setDraftsOpen] = useState(() => getPrefs().draftsOpen);
  const ui = useUI();
  const tonight = useTonight();
  const { his, drafts } = splitLists(b.data?.boards || []);
  // "tonight" for the card order only: the current list when it is his, else his newest; never a write
  const curId = (cur.current && his.some((x) => x.id === cur.current!.id) ? cur.current.id : his[0]?.id) || null;
  const ordered = [...his.filter((x) => x.id === curId), ...his.filter((x) => x.id !== curId)];
  const label = draftsLabel(drafts);
  const refresh = useCallback(() => Promise.all([b.reload(), tp.reload()]), [b.reload, tp.reload]); // eslint-disable-line react-hooks/exhaustive-deps
  const native = Platform.OS !== 'web';
  // "+" acts (SPEC 2026-10-30 §2): today's title, 7 minutes, empty, current, open. The titles on this tab are passed so
  // a second list the same day gets "(2)" without another request; the tab re-reads once the list exists.
  const startList = async () => { if (await newSetList({ titles: (b.data?.boards || []).map((x) => x.title), setCurrent: cur.setCurrent, snack: ui.snack })) b.reload(); };
  const recordSet = () => ui.withMic(() => router.push('/record' as any));
  const openArchived = () => router.push('/archived' as any);
  // the ⋯ menu's items, the same words on the phone's toolbar menu and the web's sheet (HIG/menus: "Archived" is navigation, no ellipsis)
  const moreItems: MenuItem[] = [
    { label: 'Record a set', icon: 'record.circle', onPress: recordSet, testID: 'sets-record' },
    { label: 'Archived', icon: 'archivebox', onPress: openArchived, testID: 'sets-archived' },
  ];
  const open = (x: BoardListItem) => { cur.setCurrent({ id: x.id, title: x.title }); router.push(`/list/${enc(x.id)}` as any); };
  const openDraft = (x: BoardListItem) => router.push(`/list/${enc(x.id)}` as any); // a draft is never made current
  const toggleDrafts = () => { haptics.selection(); setDraftsOpen((v) => { setPrefs({ draftsOpen: !v }); return !v; }); };
  /** The server's plain words after "Couldn't …"; off the network nothing was written, and the words say so. */
  const fail = (what: string, e: any) => ui.snack(e instanceof ApiError ? `${what}: ${e.message}` : 'Can’t reach Punchable. Nothing changed.');
  const duplicate = async (x: BoardListItem) => {
    try {
      const built = await get<Build>(`/api/boards/${enc(x.id)}/build`);
      const v = await creative.createBoard({ title: `${x.title} (copy)`, joke_ids: built.steps.map((s) => s.joke_id), target_s: x.target_s || undefined });
      ui.snack(`Duplicated as ${v.board.title}`);
      b.reload();
    } catch (e: any) { ui.snack(`Couldn't duplicate: ${e.message}`); }
  };
  // Archive is not loss (HIG/feedback: no warning; HIG/undo-and-redo: Undo is the inverse op, also his write)
  const archive = async (x: BoardListItem) => {
    try {
      await creative.patchBoard(x.id, { archive: true });
      haptics.success();
      b.reload();
      ui.snack(`Archived ${x.title}`, async () => { await creative.patchBoard(x.id, { unarchive: true }); b.reload(); });
    } catch (e: any) { fail('Couldn’t archive', e); }
  };
  const archiveAll = async () => {
    const done: BoardListItem[] = [];
    let err: any = null;
    for (const d of drafts) {
      try { await creative.patchBoard(d.id, { archive: true }); done.push(d); } catch (e) { err = e; break; }
    }
    b.reload();
    if (!done.length) return fail('Couldn’t archive', err);
    haptics.success();
    const n = done.length;
    const head = err ? `Archived ${n} of ${drafts.length} drafts, then: ${err instanceof ApiError ? err.message : 'no signal'}` : `Archived ${n} ${n === 1 ? 'draft' : 'drafts'}`;
    ui.snack(head, async () => { for (const d of done) await creative.patchBoard(d.id, { unarchive: true }); b.reload(); });
  };
  // "Use this" = an owner update that keeps the title: the server marks the list his (creative/story.py: an owner
  // `update` sets `source` "his"; archive / unarchive never do). There is no server op back to a draft, so the snack
  // offers Open, not Undo (Archive is one tap away on the card).
  const makeHis = async (x: BoardListItem) => {
    try {
      await creative.patchBoard(x.id, { title: x.title });
      haptics.success();
      b.reload();
      ui.snack(`${x.title} is yours now`, undefined, { label: 'Open', testID: 'snack-open', run: () => open(x) });
    } catch (e: any) { fail('Couldn’t make it yours', e); }
  };
  // not undoable, so an action sheet confirms (HIG/pull-down-buttons: a destructive choice gets an action sheet)
  const del = (x: BoardListItem) => actionSheet({ title: `Delete ${x.title}?`, message: 'The bits stay; only this list goes.', options: [
    { label: 'Delete the set list', destructive: true, onPress: async () => {
      try { await patch(`/api/boards/${enc(x.id)}`, { delete: true }); ui.snack(`Deleted ${x.title}`); b.reload(); } catch (e: any) { ui.snack(`Couldn't delete: ${e.message}`); }
    } },
    { label: 'Cancel', cancel: true },
  ] });
  const saveTitle = async (x: BoardListItem, t: string) => {
    setRename(null);
    try { await creative.patchBoard(x.id, { title: t }); ui.snack(`Renamed to ${t}`, async () => { await creative.patchBoard(x.id, { title: x.title }); b.reload(); }); b.reload(); }
    catch (e: any) { ui.snack(`Couldn't rename: ${e.message}`); }
  };
  // HIG/context-menus: the destructive item last; Archive (not loss) sits before it
  const cardItems = (x: BoardListItem): MenuItem[] => [
    { label: 'Open', icon: 'doc.text', onPress: () => open(x), testID: `card-open-${x.id}` },
    { label: 'Record', icon: 'record.circle', onPress: () => router.push({ pathname: '/run/[id]', params: { id: x.id, mode: 'sound' } } as any), testID: `card-stage-${x.id}` },
    ...(native ? [{ label: 'Film', icon: 'video', onPress: () => router.push({ pathname: '/run/[id]', params: { id: x.id, mode: 'video' } } as any), testID: `card-film-${x.id}` }] : []),
    { label: 'Rename…', icon: 'pencil', onPress: () => setRename(x), testID: `card-rename-${x.id}` },
    { label: 'Duplicate', icon: 'plus.square.on.square', onPress: () => duplicate(x), testID: `card-duplicate-${x.id}` },
    { label: 'Archive', icon: 'archivebox', onPress: () => archive(x), testID: `card-archive-${x.id}` },
    { label: 'Delete', icon: 'trash', onPress: () => del(x), destructive: true, testID: `card-delete-${x.id}` },
  ];
  const draftItems = (x: BoardListItem): MenuItem[] => [
    { label: 'Open', icon: 'doc.text', onPress: () => openDraft(x), testID: `draft-open-${x.id}` },
    { label: 'Use this', icon: 'checkmark.circle', onPress: () => makeHis(x), testID: `draft-usethis-${x.id}` },
    { label: 'Archive', icon: 'archivebox', onPress: () => archive(x), testID: `draft-archive-${x.id}` },
  ];
  const groupItems: MenuItem[] = [{ label: 'Archive all drafts', icon: 'archivebox', onPress: archiveAll, testID: 'drafts-archive-all-menu' }];
  return (
    <Screen title="Sets" testID="sets" onRefresh={refresh} largeTitle={native} right={native ? undefined : <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
      <IconBtn testID="sets-more" label="More" onPress={() => setMenu({ title: 'Sets', items: moreItems })}><Icon name="more" size={22} color={C.amber} /></IconBtn>
      <IconBtn testID="new-setlist" label="New set list" onPress={startList}><Icon name="plus" size={24} color={C.amber} sw={2.4} /></IconBtn>
      <Avatar /></View>}>
      {native && (
        <Stack.Toolbar placement="right">
          {/* HIG/toolbars: the primary action ("+") acts, trailing; Record a set and Archived in the ⋯ menu */}
          <Stack.Toolbar.Menu icon="ellipsis.circle" accessibilityLabel="More">
            <Stack.Toolbar.MenuAction icon="record.circle" onPress={recordSet}>Record a set</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="archivebox" onPress={openArchived}>Archived</Stack.Toolbar.MenuAction>
          </Stack.Toolbar.Menu>
          <Stack.Toolbar.Button icon="plus" accessibilityLabel="New set list" accessibilityHint="Starts an empty list named for today" onPress={startList}>New set list</Stack.Toolbar.Button>
          <Stack.Toolbar.View><Avatar ask={false} /></Stack.Toolbar.View>
        </Stack.Toolbar>
      )}
      <View testID="sets-tonight" style={{ gap: 4 }}>
        <TonightCard board={tonight.data?.board} onNew={() => void startList()} />
        <Pressable testID="sets-record-set" accessibilityRole="button" accessibilityLabel="Record a set" accessibilityHint="Opens the recorder; nothing tapes until the red disc"
          onPress={recordSet} style={({ pressed }) => ({ minHeight: 48, flexDirection: 'row', alignItems: 'center', gap: 10, opacity: pressed ? 0.7 : 1 })}>
          <Icon name="record" size={22} color={C.rec} />
          <Text maxFontSizeMultiplier={1.6} style={{ flex: 1, fontFamily: F.semi, fontSize: 16, color: C.text }}>Record a set</Text>
          <Icon name="chevron" size={18} color={C.dim} />
        </Pressable>
      </View>
      <>
          <FixtureBanner show={b.fixture} what="GET /api/boards" />
          {b.error && !b.data ? <ErrorCard error={b.error} onRetry={b.reload} /> : null}
          {!b.data && !b.error && <Skeleton lines={4} height={22} testID="sets-skeleton" />}
          {/* HIG/writing: an empty screen says what to do next; the "+" in the bar is the button (SPEC 2026-10-30 §3) */}
          {b.data && !his.length && <Empty text="No set list yet. Tap + and it starts one for tonight." />}
          <View style={{ flexDirection: wide ? 'row' : 'column', flexWrap: 'wrap', gap: 10, paddingVertical: 6 }}>
            {ordered.map((x) => (
              <View key={x.id} style={{ width: wide ? 300 : '100%' }}>
                <RowMenu href={`/list/${enc(x.id)}`} items={cardItems(x)}>
                  <SetCard b={x} current={x.id === curId} testID={`setlist-${x.id}`} onPress={() => open(x)} a11y={rowActions(cardItems(x))}
                    onLongPress={ios ? undefined : () => { haptics.light(); setMenu({ title: x.title, items: cardItems(x) }); }} />
                </RowMenu>
              </View>
            ))}
          </View>
          {!!drafts.length && (
            <View style={{ gap: 2 }}>
              {/* HIG/disclosure-controls: one row, collapsed by default, remembered per device; the chevron swaps
                  symbols (no rotation: Reduce Motion). Its menu and the group's last row both archive all. */}
              <RowMenu href="/sets" items={groupItems}>
                <Pressable testID="drafts-row" accessibilityRole="button" accessibilityState={{ expanded: draftsOpen }} accessibilityLabel={label} accessibilityHint={draftsOpen ? 'Hides the drafts' : 'Shows the drafts'}
                  onPress={toggleDrafts} onLongPress={ios ? undefined : () => { haptics.light(); setMenu({ title: label, items: groupItems }); }} delayLongPress={350} {...rowActions(groupItems)}
                  style={{ flexDirection: 'row', alignItems: 'center', gap: 12, minHeight: 56, paddingVertical: 10 }}>
                  <View style={{ flex: 1, minWidth: 0 }}>
                    <T v="semi" testID="drafts-label">{label}</T>
                    {draftsOpen && <T v="mut">Lists the app put together from your notes. Use one, or archive them.</T>}
                  </View>
                  <Icon name={draftsOpen ? 'chevronDown' : 'chevron'} size={18} color={C.dim} />
                </Pressable>
              </RowMenu>
              {draftsOpen && (
                <List>
                  {drafts.map((x) => (
                    // HIG/gestures: the swipe reveals Archive (not red: not loss); the menu item is the no-gesture path
                    <SwipeRow key={x.id} testID={`draft-swipe-${x.id}`} trailing={{ label: 'Archive', color: C.tabOff, onPress: () => archive(x), testID: `draft-swipe-archive-${x.id}` }}
                      onLongPress={ios ? undefined : () => { haptics.light(); setMenu({ title: x.title, items: draftItems(x) }); }}>
                      <RowMenu href={`/list/${enc(x.id)}`} items={draftItems(x)}>
                        <DraftRow x={x} items={draftItems(x)} onOpen={() => openDraft(x)} onUse={() => makeHis(x)} />
                      </RowMenu>
                    </SwipeRow>
                  ))}
                  <Pressable testID="drafts-archive-all" accessibilityRole="button" onPress={archiveAll} style={{ minHeight: 44, justifyContent: 'center' }}>
                    <T v="semi" style={{ color: C.amber }}>Archive all drafts</T>
                  </Pressable>
                </List>
              )}
            </View>
          )}
          {/* the Rant tab's topics not yet made into sets (Slice 6): each still opens its page; gone once its set exists */}
          {!!fromRant.length && (
            <View testID="sets-from-rant" style={{ gap: 2, marginTop: 8 }}>
              <T v="eb" accessibilityRole="header">Topics not yet in a set</T>
              <List>
                {fromRant.map((t) => (
                  <ListRow key={t.id} testID={`from-rant-${t.id}`} title={<T v="semi" numberOfLines={2}>{t.text}</T>}
                    sub={[(t.rants?.length || 0) ? `${t.rants!.length} ${t.rants!.length === 1 ? 'rant' : 'rants'}` : null, t.kept ? `${t.kept} kept` : null].filter(Boolean).join(' · ') || 'Topic'}
                    onPress={() => router.push({ pathname: '/topic/[topic]', params: { topic: t.id } } as any)} />
                ))}
              </List>
            </View>
          )}
      </>
      <MenuSheet visible={!!menu} title={menu?.title} items={menu?.items || []} onClose={() => setMenu(null)} testID="card-menu" />
      <Sheet visible={!!rename} onClose={() => setRename(null)} label="Rename the set list" testID="card-title-sheet">
        <SheetHead title="Rename the set list" onCancel={() => setRename(null)} testID="card-title" />
        {rename && <RenameForm key={rename.id} title={rename.title} onSave={(t) => saveTitle(rename, t)} />}
      </Sheet>
    </Screen>
  );
}
