// Sets (MixSets, docs/UX_2026-10.md): HIS set lists as Sharpie-on-paper cards, each joke's short name in order (the
// boards list carries `names`, one request), tonight's first; "Recorded" lists the taped sets. A set list IS a board
// (/list/<id>); "New set list" is the create sheet. The lists the app made (`source` "inferred") are drafts: ONE
// collapsed row at the bottom ("15 drafts from your Insights"), each with "Use this" (an owner write: the list becomes
// his) and Archive (undoable: `{archive: true}` / `{unarchive: true}`); nothing is ever chosen for him
// (docs/design/SPEC-2026-10-29-set-lists-and-stage-notes.md §1). Archived lists live on /archived (the toolbar menu).
import React, { useCallback, useState } from 'react';
import { Platform, Pressable, Text, TextInput, View } from 'react-native';
import { Stack, router } from 'expo-router';
import { BoardListItem, creative, useCreative } from '@/lib/creative';
import { ApiError, get, patch } from '@/lib/api';
import { Build } from '@/lib/simple';
import { actionSheet } from '@/lib/actionSheet';
import { RowMenu, rowActions } from '@/lib/RowMenu';
import { SwipeRow } from '@/lib/SwipeRow';
import { fmtShortDate, mmss } from '@/rules/format';
import { draftsLabel, splitLists } from '@/rules/setListRules';
import { useSets } from '@/lib/sets';
import { useBoards } from '@/lib/boards';
import { getPrefs, setPrefs } from '@/lib/prefs';
import { CreateSheet } from '@/lib/CreateSheet';
import { C, F } from '@/lib/theme';
import { haptics } from '@/lib/haptics';
import { Avatar, Btn, Empty, ErrorCard, FixtureBanner, Icon, List, ListRow, MenuItem, MenuSheet, Seg, Sheet, SheetHead, Skeleton, T, TextBtn, useUI, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

type Seg_ = 'lists' | 'performed';
const PAPER = '#F2EEE4';
const INK = '#1B1B1F';
const MARKER = '#2A2A2E';
const enc = encodeURIComponent;
const ios = Platform.OS === 'ios'; // the system context menu owns the long press there

/** A set list on paper: the title in marker, the short names down the page. */
export function PaperCard({ b, current, onPress, onLongPress, a11y, testID }: {
  b: BoardListItem & { names?: string[] }; current?: boolean; onPress: () => void; onLongPress?: () => void; a11y?: ReturnType<typeof rowActions>; testID?: string;
}) {
  const names = b.names || [];
  const shown = names.slice(0, 6);
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={`${b.title}, ${b.n} jokes, ${b.runtime}. Open`} onPress={onPress} onLongPress={onLongPress} delayLongPress={350} {...a11y}
      style={({ pressed }) => ({ width: '100%', borderRadius: 6, backgroundColor: PAPER, padding: 16, paddingBottom: 18, gap: 8, ...(Platform.OS === 'web' ? { transform: [{ rotate: current ? '-0.6deg' : '0.4deg' }] } : null), opacity: pressed ? 0.9 : 1, shadowColor: '#000', shadowOpacity: 0.5, shadowRadius: 10, shadowOffset: { width: 0, height: 6 } })}>
      <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 8 }}>
        <Text numberOfLines={1} style={{ flex: 1, fontFamily: F.black, fontSize: 20, letterSpacing: -0.3, color: INK }}>{b.title}</Text>
        <Text style={{ fontFamily: F.bold, fontSize: 14, color: MARKER, fontVariant: ['tabular-nums'] }}>{b.runtime}</Text>
      </View>
      <View style={{ gap: 3, paddingLeft: 6, borderLeftWidth: 2, borderLeftColor: '#D8D2C4' }}>
        {shown.map((n, i) => <Text key={i} numberOfLines={1} style={{ fontFamily: F.bold, fontSize: 17, color: MARKER, letterSpacing: -0.2 }}>{n}</Text>)}
        {names.length > shown.length && <Text style={{ fontFamily: F.semi, fontSize: 14, color: '#6B6660' }}>+ {names.length - shown.length} more</Text>}
        {!names.length && <Text style={{ fontFamily: F.semi, fontSize: 14, color: '#6B6660' }}>(empty)</Text>}
      </View>
      <Text style={{ fontFamily: F.medium, fontSize: 12, color: '#6B6660' }}>{current ? 'tonight · ' : ''}{b.target_s ? `${mmss(b.target_s)} slot · ` : ''}{fmtShortDate(b.updated)}</Text>
    </Pressable>
  );
}

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
  const sets = useSets();
  const [seg, setSeg] = useState<Seg_>('lists');
  const [create, setCreate] = useState(false);
  const [menu, setMenu] = useState<{ title: string; items: MenuItem[] } | null>(null);
  const [rename, setRename] = useState<BoardListItem | null>(null);
  const [draftsOpen, setDraftsOpen] = useState(() => getPrefs().draftsOpen);
  const ui = useUI();
  const { his, drafts } = splitLists(b.data?.boards || []);
  // "tonight" for the card order only: the current list when it is his, else his newest; never a write
  const curId = (cur.current && his.some((x) => x.id === cur.current!.id) ? cur.current.id : his[0]?.id) || null;
  const ordered = [...his.filter((x) => x.id === curId), ...his.filter((x) => x.id !== curId)];
  const label = draftsLabel(drafts);
  const refresh = useCallback(() => Promise.all([b.reload(), sets.reload?.()]), [b.reload, sets.reload]);
  const native = Platform.OS !== 'web';
  const openCreate = () => (native ? router.push({ pathname: '/create', params: { start: 'set' } } as any) : setCreate(true));
  const openArchived = () => router.push('/archived' as any);
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
    { label: 'Record', icon: 'record.circle', onPress: () => router.push(`/stage/${enc(x.id)}` as any), testID: `card-stage-${x.id}` },
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
      <TextBtn testID="sets-archived" label="Archived" onPress={openArchived} />
      <Pressable testID="new-setlist" accessibilityRole="button" accessibilityLabel="New set list" onPress={openCreate} style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}><Icon name="plus" size={24} color={C.amber} sw={2.4} /></Pressable>
      <Avatar /></View>}>
      {native && (
        <Stack.Toolbar placement="right">
          {/* HIG/menus: "Archived" is navigation, no ellipsis */}
          <Stack.Toolbar.Menu icon="ellipsis.circle" accessibilityLabel="More">
            <Stack.Toolbar.MenuAction icon="archivebox" onPress={openArchived}>Archived</Stack.Toolbar.MenuAction>
          </Stack.Toolbar.Menu>
          <Stack.Toolbar.Button icon="plus" accessibilityLabel="New set list" onPress={openCreate}>New set list</Stack.Toolbar.Button>
          <Stack.Toolbar.View><Avatar ask={false} /></Stack.Toolbar.View>
        </Stack.Toolbar>
      )}
      {/* a set list is what he is working on; a set exists once it is recorded (owner, 2026-10-29). Equal-width nouns
          (HIG/segmented-controls); the count lives in the header row below, not in the segment. */}
      <Seg<Seg_> testID="sets-seg" items={[['lists', 'Set lists'], ['performed', 'Recorded']]} value={seg} onChange={(v) => { haptics.selection(); setSeg(v); }} />
      {seg === 'lists' ? (
        <>
          <FixtureBanner show={b.fixture} what="GET /api/boards" />
          {b.error && !b.data ? <ErrorCard error={b.error} onRetry={b.reload} /> : null}
          {!b.data && !b.error && <Skeleton lines={4} height={22} testID="sets-skeleton" />}
          {/* HIG/writing: an empty screen says what to do next and gives the button */}
          {b.data && !his.length && <Empty text="No set list yet. Build the first one."><Btn small v="add" label="+ New set list" accessibilityLabel="New set list" onPress={openCreate} /></Empty>}
          <View style={{ flexDirection: wide ? 'row' : 'column', flexWrap: 'wrap', gap: 18, paddingVertical: 6 }}>
            {ordered.map((x) => (
              <View key={x.id} style={{ width: wide ? 300 : '100%' }}>
                <RowMenu href={`/list/${enc(x.id)}`} items={cardItems(x)}>
                  <PaperCard b={x} current={x.id === curId} testID={`setlist-${x.id}`} onPress={() => open(x)} a11y={rowActions(cardItems(x))}
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
        </>
      ) : (
        <View style={{ gap: 6 }}>
          {!!sets.list.length && <T v="small" testID="sets-count">{sets.list.length} {sets.list.length === 1 ? 'set' : 'sets'} on tape</T>}
          {sets.error && !sets.list.length ? <ErrorCard error={sets.error} /> : null}
          {!sets.list.length && !sets.error && <Empty text="No recorded sets yet. Open a set list and tap Record, or record one from ＋." />}
          <List>
            {sets.list.map((x) => (
              <ListRow key={x.id} testID={`performed-${x.id}`} title={<T v="semi" numberOfLines={1}>{x.venue.name}</T>}
                sub={`${fmtShortDate(x.show.date)} · ${mmss(x.card.duration_s)}${x.best ? ` · ${x.best.title.split(' (')[0]}` : ''}`}
                onPress={() => router.push(`/set/${encodeURIComponent(x.id)}` as any)} />
            ))}
          </List>
        </View>
      )}
      {!native && <CreateSheet visible={create} onClose={() => setCreate(false)} start="set" />}
      <MenuSheet visible={!!menu} title={menu?.title} items={menu?.items || []} onClose={() => setMenu(null)} testID="card-menu" />
      <Sheet visible={!!rename} onClose={() => setRename(null)} label="Rename the set list" testID="card-title-sheet">
        <SheetHead title="Rename the set list" onCancel={() => setRename(null)} testID="card-title" />
        {rename && <RenameForm key={rename.id} title={rename.title} onSave={(t) => saveTitle(rename, t)} />}
      </Sheet>
    </Screen>
  );
}
