// A bit's page: the bit is a document (owner 2026-11-16: "a bit is a document, it just has a bunch of ideas for the
// same topic ... it should be as easy as editing a google doc on your phone, with the power of punchable"; SPEC
// 2026-11-16 B11, §4.3, Slice 2). The page opens on his words; a tap anywhere in them puts the caret there and brings
// up the keyboard (lib/BitDoc: one multiline input over the whole text). There is no Write screen and no Save: the
// words are kept on the phone as he types and a version is saved when he leaves the text, the page or the app, or
// pauses, only when the words changed. Saving is quiet (HIG/feedback: he expects it to work): the footnote line reads
// "Saved as v9" for a moment and the Versions row, right under the document, carries the non-timed Undo; only a save
// made as he LEAVES the page says so in a snack (he is no longer looking at the row). While the keyboard is up
// nothing else is on the page: the bar holds Done alone and everything under the document is out of the way.
// Under the document, when he is not typing (SPEC 2026-11-16 §3.1, §4.6; owner on build 28: "when i open a bit i dont
// automatically see other connections"): Around this bit drawn on the page itself (each group that holds something,
// its first two cards, "See all N"; lib/Around AroundInline) and In these sets (the sets by name, Add to a set), both
// from one read, GET /api/jokes/<id>/around, with skeleton rows while it is on its way; then the collapsed rows of
// the 3c page (docs/design/SPEC-2026-11-03-bit-page-3c.md): the Pile and the Versions (inline), On tape (a push).
// The bar: his short name; the sparkle, To a set, More. More › Reorder lines is its own mode (a drag list over the
// document's lines and the gaps between its ideas). Every write is a new version (POST /api/jokes/<id>/versions, the
// old ones stay).
// The older flow is kept behind a preference (Settings › About › "Write button on a bit"; prefs.bitEditor 'classic'):
// the bit as 44-pt lines, one bottom button, Write, and the one-box editor with Save. One of the two is drawn, never
// both.
// Film it's rant half (SPEC 2026-11-03 D10, D11, §2.5, §2.6; build 24): a version is the stage cut or a rant cut (its
// `source`, rules/bitLines cutOf). The body shows the newest STAGE version; the stepper walks every version and says
// the cut; on a rant version a "Rant cut" row sits above the text and a write or Restore writes that cut. The sparkle
// opens the sessions sheet (/session/[bit]) with its two rows, Make it a rant and Pick a hook, on the version shown.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Platform, Share, Text, TextInput, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { decide, undo, useApi, useOnline } from '@/lib/api';
import { useBoards } from '@/lib/boards';
import { addVersion } from '@/lib/bitWork';
import { rant } from '@/lib/rant';
import { isFreshBit, nameFromFirstLine } from '@/rules/bitWorkRules';
import type { SimilarResult } from '@/rules/searchRows';
import * as R from '@/rules/bitLines';
import * as L from '@/rules/lineRules';
import { shortNameOf } from '@/lib/say';
import { C, F, linkColor, stageWord } from '@/lib/theme';
import { haptics } from '@/lib/haptics';
import { getPrefs, onPrefs } from '@/lib/prefs';
import { useRecordings } from '@/lib/recordings';
import { afterWrite } from '@/lib/feedback';
import { Btn, ErrorCard, Icon, IconBtn, MenuItem, MenuSheet, Sheet, SheetHead, Skeleton, T, TextBtn, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { SectionRow } from '@/lib/SectionRow';
import { IdeaLine } from '@/lib/IdeaLine';
import { VersionStepper } from '@/lib/VersionStepper';
import { BitLines, type LineAction } from '@/lib/BitLines';
import { BitDoc, type BitDocHandle, type BitDocState, type SaveWhy } from '@/lib/BitDoc';
import { Pile } from '@/lib/Pile';
import { AroundInline, SetsInline, aroundChanged, aroundPath } from '@/lib/Around';
import * as AR from '@/rules/aroundRules';
import { DragList } from '@/lib/DragList';

type V = R.BitVersion;
type Note = { id: string; text?: string | null; date?: string | null; reasons?: unknown[] | null; score?: number | null };
/** A row of Reorder mode: a line, or the gap between two ideas; `k` keeps its identity through the drag. */
type OrderRow = L.Row & { k: string };

/** The stages he can set from More › Stage (Retire is its own, destructive item). */
const STAGE_MENU: [string, string][] = [['idea', 'Idea'], ['to_try', 'To try'], ['testing', 'Testing'], ['working', 'Working']];
const enc = encodeURIComponent;
const native = Platform.OS !== 'web';
const QUIET: BitDocState = { typing: false, pending: false, note: null };

/** The versions as the page reads them (rules/bitLines versionsOf; the camera reads the same list). */
const versionsOf = (d: any): V[] => R.versionsOf(d);

export default function BitPage() {
  const p = useLocalSearchParams<{ id: string; edit?: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const boards = useBoards();
  const online = useOnline();
  const d = useApi<any>(`/api/jokes/${enc(id)}`, { poll: 30000 });
  const sim = useApi<SimilarResult>(`/api/jokes/${enc(id)}/similar?limit=5`);
  const cand = useApi<{ history?: { notes?: Note[] } }>(`/api/merge/candidates?joke=${enc(id)}`, { cache: false });
  // everything tied to the bit, and the sets it is in (one read; the half sheet reads the same path from the cache)
  const ar = useApi<AR.Around>(aroundPath(id));
  // which editor this phone draws: the document (default) or the older Write button (a preference, the fallback)
  const [classic, setClassic] = useState(() => getPrefs().bitEditor === 'classic');
  useEffect(() => onPrefs((x) => setClassic(x.bitEditor === 'classic')), []);
  // the document: its handle, what it says about itself, and the page's scroll view it keeps the caret visible in
  const doc = useRef<BitDocHandle>(null);
  const [docState, setDocState] = useState<BitDocState>(QUIET);
  const scrollRef = useRef<any>(null);
  const scrollY = useRef(0);
  const scrollH = useRef(0);
  const scrollWatch = useMemo(() => ({
    onScroll: (y: number) => { scrollY.current = y; },
    onHeight: (h: number) => { scrollH.current = h; doc.current?.follow(); },
    // a drag never puts the keyboard away under his thumb mid-sentence; it follows the finger down, as in Notes
    keyboardDismissMode: 'interactive' as const,
  }), []);
  // the last write from a card's sheet: its non-timed Undo sits on the Around row (HIG.md §7)
  const [lastAround, setLastAround] = useState<{ undo: () => Promise<void> } | null>(null);
  // which version is shown: null = the newest stage version (home), 'rant' = the newest rant cut (after a rant write),
  // a number = the one he stepped to
  const [pos, setPos] = useState<number | 'rant' | null>(null);
  const [selRaw, setSel] = useState<number | null>(null);
  const [rename, setRename] = useState(false);
  const [idea, setIdea] = useState(false);
  const [menu, setMenu] = useState<MenuItem[] | null>(null);
  // the older editor only: the text being edited (a new version on Save) and where its caret starts
  const [editing, setEditing] = useState<string | null>(null);
  const [editSel, setEditSel] = useState<{ start: number; end: number } | undefined>(undefined);
  const [order, setOrder] = useState<OrderRow[] | null>(null);   // Reorder mode: the document's rows being reordered
  const [pileOpen, setPileOpen] = useState<boolean | null>(null);
  const [versionsOpen, setVersionsOpen] = useState(false);
  // the last write's undo handle: "v9 · just now · Undo" in the Versions row (non-timed, HIG.md §7)
  const [lastWrite, setLastWrite] = useState<{ indexes: number[]; n: number } | null>(null);
  // "Saved as v9" in the footnote slot for a moment after the document saved itself (no snack, no haptic)
  const [savedNote, setSavedNote] = useState<string | null>(null);
  const savedTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(savedTimer.current), []);
  // opened from Bits "+": a fresh document, the keyboard up before anything else
  const [openedFresh, setOpenedFresh] = useState(false);
  useEffect(() => {
    if (p.edit !== '1' || !d.data || openedFresh) return;
    setOpenedFresh(true);
    if (classic && editing == null) setEditing('');
  }, [p.edit, d.data, openedFresh, editing, classic]);
  const [saving, setSaving] = useState(false);
  const versions = useMemo(() => versionsOf(d.data), [d.data]);
  const reloadAll = useCallback(() => { d.reload(); sim.reload(); cand.reload(); ar.reload(); }, [d.reload, sim.reload, cand.reload, ar.reload]); // eslint-disable-line react-hooks/exhaustive-deps
  // a card's sheet or the picker wrote something about this bit: read again; a new version shows its Undo on the
  // Versions row, any other write on the Around row
  const versionCount = useRef(0);
  useEffect(() => { versionCount.current = versions.length; });
  useEffect(() => aroundChanged.subscribe((c) => {
    if (c.bit !== id) return;
    if (c.indexes) { setPos(null); setSel(null); setLastWrite({ indexes: c.indexes, n: versionCount.current + 1 }); }
    setLastAround(c.undo ? { undo: c.undo } : null);
    reloadAll();
  }), [id, reloadAll]);
  // voice memos on this phone that are on this bit: started here, or filed here (SPEC 2026-10-31 §6)
  const recs = useRecordings();
  const memos = useMemo(() => recs.filter((r) => r.kind === 'memo' && (r.bit?.id === id || r.filed_on?.id === id)), [recs, id]);
  // filmed takes read from this bit (Film it, SPEC 2026-11-03 §2.7): counted on the On tape row; their words go up as
  // an idea filed here, and that note is NOT a scrap in the pile (the words he said live on the On tape row, §12 Q6)
  const takes = useMemo(() => recs.filter((r) => r.kind === 'film' && r.bit?.id === id), [recs, id]);
  const takeNotes = useMemo(() => new Set(takes.map((t) => t.merge_id).filter(Boolean) as string[]), [takes]);
  const J = d.data?.joke;
  // a fresh bit (lib/newBit) carries a marker title until its first save names it; the page says "New bit" meanwhile
  const fresh = isFreshBit(J?.title);
  const title = fresh ? 'New bit' : (J?.title || '').split(' (')[0] || J?.title || 'Bit';
  const shortName = fresh ? 'New bit' : shortNameOf(J?.title || '', J?.short_name);
  const home = R.homeIndex(versions);
  const newestRant = R.newestOf(versions, 'rant');
  const vi = Math.min(typeof pos === 'number' ? pos : pos === 'rant' && newestRant >= 0 ? newestRant : home, Math.max(0, versions.length - 1));
  const v = versions[vi];
  const vCut: R.Cut = v?.cut || 'stage';
  const src = R.sourceFor(vCut, 'write');
  const lines = useMemo(() => R.linesOf(v?.text), [v?.text]);
  // what every version of this cut says: a draft on the phone that one of them already says is not newer words
  const said = useMemo(() => versions.filter((x) => x.cut === vCut).map((x) => x.text), [versions, vCut]);
  const punch = useMemo(() => R.cleanPunch(v?.punch, lines.length), [v?.punch, lines.length]);
  // reading = an older version of its cut: the text is not editable, Restore is on
  const reading = versions.length > 0 && !R.isNewestOfCut(versions, vi);
  const scraps = useMemo(() => R.pileScraps((cand.data?.history?.notes || []).filter((n: Note) => !takeNotes.has(n.id)), memos), [cand.data, memos, takeNotes]);
  const barOn = R.barEnabled(lines, J?.summary) && !fresh;
  const typing = !classic && docState.typing;
  // the page's own writes (a reorder, a line from the pile, a restore, Cut): online, on the newest version, nothing else open
  const canWrite = online && !reading && editing == null && order == null && !typing;
  // the one line above the bottom: an older version being read, else why his words are not a version yet
  const footnote = R.writeReason({ vi, count: versions.length, online: true, newest: !reading }) || (classic ? null : docState.note || savedNote);
  // a selection outlives nothing: stepping clears it (onStep), a write clears it (wrote), a shorter version drops it
  const sel = selRaw != null && selRaw < lines.length ? selRaw : null;
  // an empty bit opens its Pile row on its one real action (frame d); otherwise the rows start closed
  const pileIsOpen = pileOpen ?? (!!d.data && lines.length === 0);
  /** The document as it stands: what he typed (the document owns it), else the version shown. */
  const docNow = () => (!classic && !reading ? doc.current?.text() : undefined) ?? v?.text ?? '';

  /** Every write lands as a new version: the snack's timed Undo and the row's non-timed one share the indexes. */
  const wrote = (indexes: number[], msg: string | null, cut: R.Cut = 'stage') => {
    setPos(cut === 'rant' ? 'rant' : null);
    setSel(null);
    setLastWrite({ indexes, n: versionCount.current + 1 });
    reloadAll();
    if (msg) ui.snack(msg, async () => { await undo(indexes); doc.current?.undone(); setLastWrite(null); reloadAll(); });
  };
  const undoLast = async () => {
    if (!lastWrite) return;
    try { await undo(lastWrite.indexes); doc.current?.undone(); setLastWrite(null); reloadAll(); ui.snack('Undone'); } catch (e: any) { ui.snack(`Couldn't undo: ${e.message}`); }
  };
  /** The document saved a version by itself. Quiet while he is on the page (HIG/feedback: a save is expected to
   *  work): the footnote reads "Saved as v9" for a moment and the Versions row under the document has the Undo. Only a
   *  save made as he leaves the page is a snack, because the row is no longer in front of him. */
  const docSaved = (indexes: number[], why: SaveWhy) => {
    const word = fresh ? 'Saved' : `Saved as v${versionCount.current + 1}${vCut === 'rant' ? ' · rant' : ''}`;
    if (why !== 'leave') {
      setSavedNote(word);
      clearTimeout(savedTimer.current);
      savedTimer.current = setTimeout(() => setSavedNote(null), 4000);
    }
    wrote(indexes, why === 'leave' ? word : null, vCut);
  };

  /** The older editor's Save. */
  const saveEdit = async () => {
    const t = (editing || '').trim();
    if (!t || saving) return;
    if (t === (v?.text || '').trim()) { setEditing(null); return; }
    setSaving(true);
    try {
      const r = await addVersion(id, t, 'edited on the bit page', false, src);
      let indexes = r.indexes;
      if (fresh) {
        // a fresh document takes its name from its first line (the first seven words, like a typed jot): rules/bitWorkRules
        const name = nameFromFirstLine(t);
        try { const w = await decide({ target: 'node', action: 'edit', id, after: { title: name }, reason: 'named from its first line' }); indexes = [...indexes, ...w.indexes]; } catch { /* the version is saved either way */ }
      }
      setEditing(null);
      setEditSel(undefined);
      haptics.success();
      wrote(indexes, openedFresh ? 'Saved' : `Saved as v${versions.length + 1}${vCut === 'rant' ? ' · rant' : ''}`, vCut);
    } catch (e: any) {
      ui.snack(`Couldn’t save: ${e.message}`);
    } finally {
      setSaving(false);
    }
  };
  /** The older editor's Write: one box over the whole bit; `at` = the line the caret starts on (the second tap on a line). */
  const openWrite = (at?: number) => {
    if (!canWrite) return;
    const text = v?.text || '';
    setEditSel(at == null ? undefined : { start: R.lineOffset(text, at), end: R.lineOffset(text, at) });
    setEditing(text);
  };
  /** One of the page's own writes: the whole document as a new version; the document on the page follows it. */
  const writeText = async (make: () => string, reason: string, msg: string) => {
    try {
      // a save of his own typing that is still on its way lands first: this write is made from, and on top of, it
      await doc.current?.save('done');
      const text = make();
      const r = await addVersion(id, text, reason, false, src);
      doc.current?.adopt(text, r.unit);
      haptics.success();
      wrote(r.indexes, msg, vCut);
    } catch (e: any) {
      ui.snack(`Couldn’t save: ${e.message}`);
    }
  };
  // the older editor's Cut under a line: that line goes, the gaps between his ideas stay
  const cut = (i: number) => {
    if (lines.length < 2) return ui.snack('That is the only line');
    void writeText(() => { let n = -1; return L.textOf(L.rowsOf(v?.text).filter((r) => r.kind !== 'line' || ++n !== i)); }, 'cut a line on the bit page', 'Line cut');
  };
  const startReorder = () => {
    if (lines.length < 2 || !canWrite) return;
    setSel(null);
    setOrder(L.rowsOf(docNow()).map((r, i) => ({ ...r, k: `${i}` })));
  };
  const finishReorder = () => {
    const next = L.textOf(order || []);
    setOrder(null);
    if (!L.sameDoc(next, L.textOf(L.rowsOf(docNow())))) void writeText(() => next, 'reordered the lines', 'Reordered');
  };
  // a scrap goes in after the line the caret was last on (the older editor: the selected line), else at the end
  const addScrap = (s: R.Scrap) => void writeText(() => L.applyEdit(docNow(), { op: 'insert', at: classic ? sel : doc.current?.caretLine() ?? null, words: s.words }), 'added from the pile', 'Added');
  const restore = () => { if (v) void writeText(() => L.docText(v.text), `restored ${v.label}`, `Restored ${v.label} as v${versions.length + 1}`); };
  const toSet = () => afterWrite(() => boards.add(id, title), (ok) => ok, haptics);
  // the bar's To a set: the picker, with a tick on every set the bit is already in (SPEC 2026-11-16 §4.5)
  const pickSets = () => boards.pick(id, shortName);
  const undoAround = async () => {
    if (!lastAround) return;
    try { await lastAround.undo(); setLastAround(null); reloadAll(); ui.snack('Undone'); } catch (e: any) { ui.snack(`Couldn't undo: ${e.message}`); }
  };
  // a memo started here is filed on this bit (lib/recordings: POST /api/merge/attach after the idea)
  const recordMemo = () => router.push({ pathname: '/record', params: { kind: 'memo', bit: id, title: shortName } } as any);
  // Film it (SPEC 2026-11-03 §2.1): the front camera with the newest version's lines on the band; phone only
  const filmIt = () => router.push(`/film/${enc(id)}` as any);
  // Rant on this… (SPEC 2026-11-06 §12.7): a topic named for the bit, the bit kept on it, the topic opened in Rant
  const [ranting, setRanting] = useState(false);
  const rantOn = async () => {
    if (ranting) return;
    setRanting(true);
    try {
      const t = await rant.addTopic(shortName || title);
      // the topic opens with the bit on it, or says it is not there yet
      try { await rant.mark(t.topic.id, id, 'kept'); } catch { ui.snack('The topic is made, but this bit didn’t keep on it: + it there.'); }
      router.push({ pathname: '/topic/[topic]', params: { topic: t.topic.id } } as any);
    } catch {
      ui.snack(online ? 'Couldn’t make the topic.' : 'Can’t reach Punchable. Nothing changed.');
    } finally {
      setRanting(false);
    }
  };
  // the sessions sheet on the version shown (its unit; the typed-idea stand-in has none, so the sparkle is off there);
  // off too while words he typed are not a version yet (a session reads the saved version)
  const canAsk = online && lines.length > 0 && !!v && v.key !== 'idea' && editing == null && order == null && !typing && !(!classic && docState.pending);
  const openSessions = () => {
    if (!canAsk || !v) return;
    router.push({ pathname: '/session/[bit]', params: { bit: id, unit: v.key, cut: v.cut, n: String(versions.length), title: shortName } } as any);
  };
  const share = () => Share.share({ message: [title, docNow()].join('\n\n').trim(), title }).catch(() => ui.snack('Sharing is not available here'));
  const retire = async () => {
    try {
      const w = await decide({ target: 'node', action: 'edit', id, after: { stage: 'retired' } });
      d.reload();
      ui.snack('Retired: resting, not gone', async () => { await undo(w.indexes); d.reload(); });
    } catch (e: any) {
      ui.snack(`Couldn't retire: ${e.message}`);
    }
  };
  const setStage = async (stage: string) => {
    if (stage === J?.stage) return;
    try {
      const w = await decide({ target: 'node', action: 'edit', id, after: { stage } });
      d.reload();
      ui.snack(`Now ${stageWord(stage).toLowerCase()}`, async () => { await undo(w.indexes); d.reload(); });
    } catch (e: any) {
      ui.snack(`Couldn't set the stage: ${e.message}`);
    }
  };
  const saveShort = async (s: string) => {
    setRename(false);
    try {
      const w = await decide({ target: 'node', action: 'edit', id, after: { short_name: s.trim() || null } });
      d.reload();
      ui.snack(s.trim() ? `“${s.trim()}” on your set lists` : 'Back to the default name', async () => { await undo(w.indexes); d.reload(); });
    } catch (e: any) {
      ui.snack(`Couldn't rename: ${e.message}`);
    }
  };
  const saveIdea = async (s: string) => {
    setIdea(false);
    if (s.trim() === (J?.summary || '').trim()) return;
    try {
      const w = await decide({ target: 'node', action: 'edit', id, after: { summary: s.trim() }, reason: 'the idea line on the bit page' });
      d.reload();
      ui.snack(s.trim() ? 'Idea saved' : 'Idea cleared', async () => { await undo(w.indexes); d.reload(); });
    } catch (e: any) {
      ui.snack(`Couldn't save the idea: ${e.message}`);
    }
  };

  if (d.rawError && !d.data) return <Screen title="Bit" back><ErrorCard error={String(d.rawError)} onRetry={d.reload} /></Screen>;
  if (!d.data) return <Screen title="Bit" back><Skeleton lines={6} height={18} testID="bit-skeleton" /></Screen>;

  const aiStage = !!d.data?.ai?.stage_ai && !d.data?.ai?.stage_confirmed;
  const aiIdea = !!d.data?.ai?.summary_ai && !d.data?.ai?.summary_confirmed;
  const nights = sim.data?.performed.length || 0;
  const killed = R.killedCount((sim.data?.performed || []) as { rating?: string | null }[]);
  // what sounds like it is a group of "Around this bit" now (SPEC 2026-11-16 §2.2), so it no longer opens On tape
  const tape = R.tapeMeta(nights, killed, 0, takes.length);
  // collapsed, the row reads the newest stage version (Film it §2.6: "unchanged"); right after a write, "just now"
  const vmeta = { count: versions.length - home, newestDate: versions[home]?.date, durationS: J?.duration_s, estimated: !!J?.duration_estimated, justWrote: !!lastWrite && lastWrite.n === versions.length };
  const canUndoWrite = !!lastWrite && vmeta.justWrote && !reading;
  // the older editor's line actions (Move · Cut). Move is not shown on a one-line bit (a greyed control that leads
  // nowhere is not rendered, 3c §14).
  const lineActions: LineAction[] | null = reading || !online || editing != null ? null : [
    ...(lines.length > 1 ? [{ label: 'Move', testID: 'line-move', onPress: startReorder }] : []),
    { label: 'Cut', testID: 'line-cut', onPress: (i: number) => cut(i) },
  ];
  // the two sections under the document are drawn while the read is on its way too (skeleton rows), so the rows
  // under them do not jump when it lands; gone only when the read failed with nothing cached (a server without the route)
  const aroundShown = !!ar.data || !ar.rawError;
  // a card writes into the saved document: not offline, and not over words he typed that no version holds yet
  const cardsCanAct = online && !reading && order == null && !(!classic && docState.pending);
  const pileRow = (
    <SectionRow testID="row-pile" kind="inline" label="Pile" meta={String(scraps.length)} open={pileIsOpen} onPress={() => setPileOpen(!pileIsOpen)}>
      <Pile testID="pile" scraps={scraps} recs={memos} lines={lines} onAdd={addScrap} />
    </SectionRow>
  );
  // More, in frequency order, the destructive item last (§3.8); the same list on the web as a sheet
  const webMenu = (): MenuItem[] => [
    { label: 'Reorder lines', onPress: startReorder, disabled: !canWrite || lines.length < 2, testID: 'menu-reorder' },
    { label: 'Record a memo', onPress: recordMemo, disabled: true },
    { label: `Add to ${boards.current?.title || 'the current set list'}`, onPress: () => void toSet(), disabled: !barOn || !boards.current },
    { label: 'Rant on this…', onPress: () => void rantOn(), disabled: !online, testID: 'menu-rant-on' },
    { label: 'Add to a set…', onPress: pickSets, disabled: !barOn || !online, testID: 'menu-sets' },
    { label: 'Merge with…', onPress: () => router.push('/merge' as any) },
    { label: 'Share…', onPress: () => void share() },
    { label: 'Rename short name…', onPress: () => setRename(true) },
    { label: 'Stage…', onPress: () => setMenu(STAGE_MENU.map(([k, w]) => ({ label: `${w}${J?.stage === k ? ' ✓' : ''}`, onPress: () => void setStage(k) }))) },
    { label: 'Retire', onPress: retire, destructive: true },
  ];
  const done = () => doc.current?.blur();

  return (
    <Screen title={shortName} back testID="bitpage" maxWidth={760} offlineAt="bottom" footnote={footnote}
      offlineNote={classic ? undefined : 'Offline · kept on this iPhone'} scrollRef={scrollRef} scrollWatch={classic ? undefined : scrollWatch}
      right={native ? undefined : typing ? (
        <Btn small v="primary" testID="bit-done" label="Done" onPress={done} />
      ) : (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
          {classic && <Btn small v="primary" testID="bit-write" label="Write" disabled={!canWrite} onPress={() => openWrite()} />}
          <IconBtn testID="bit-ask" label="Ask Claude" disabled={!canAsk} onPress={openSessions}><Icon name="star" size={20} color={canAsk ? C.amber : C.dim} /></IconBtn>
          <IconBtn testID="bit-more" label="More" onPress={() => setMenu(webMenu())}><Icon name="more" size={22} color={C.amber} /></IconBtn>
        </View>
      )}>
      {native && typing && (
        // the keyboard is up: Done alone, trailing (HIG/toolbars; Notes does the same). It puts the keyboard away,
        // which saves a version if the words changed.
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button variant="prominent" accessibilityLabel="Done" accessibilityHint="Puts the keyboard away" onPress={done}>Done</Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}
      {native && !typing && (
        // HIG/toolbars: simple symbols (no circle borders), the important few in the bar, the rest under More
        <Stack.Toolbar placement="right">
          {/* Ask Claude: the sessions sheet (Film it's two rows until 3c Slice 4 adds the rest); off with its reason */}
          <Stack.Toolbar.Button icon="sparkles" disabled={!canAsk} accessibilityLabel="Ask Claude" accessibilityHint={!online ? 'Offline' : !lines.length ? 'Nothing to read' : 'Make it a rant, or pick a hook'} onPress={openSessions} />
          <Stack.Toolbar.Button icon="text.badge.plus" disabled={!barOn || !online} accessibilityLabel="Add to a set" accessibilityHint={online ? 'Tick the sets it belongs in' : 'Offline'} onPress={pickSets} />
          <Stack.Toolbar.Menu icon="ellipsis" accessibilityLabel="More">
            <Stack.Toolbar.MenuAction icon="arrow.up.arrow.down" disabled={!canWrite || lines.length < 2} onPress={startReorder}>Reorder lines</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="mic" onPress={recordMemo}>Record a memo</Stack.Toolbar.MenuAction>
            {/* Film it sits with the two "make something from this bit" items; greyed with its one-word reason when there is nothing to read */}
            <Stack.Toolbar.MenuAction icon="video" disabled={!lines.length} subtitle={lines.length ? undefined : 'nothing to read'} onPress={filmIt}>Film it…</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="megaphone" disabled={!online} subtitle={online ? undefined : 'offline'} onPress={() => void rantOn()}>Rant on this…</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.Menu inline>
              <Stack.Toolbar.MenuAction icon="arrow.triangle.merge" onPress={() => router.push('/merge' as any)}>Merge with…</Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="square.and.arrow.up" onPress={() => void share()}>Share…</Stack.Toolbar.MenuAction>
            </Stack.Toolbar.Menu>
            <Stack.Toolbar.Menu inline>
              <Stack.Toolbar.MenuAction icon="pencil" onPress={() => setRename(true)}>Rename short name…</Stack.Toolbar.MenuAction>
              <Stack.Toolbar.Menu icon="flag">
                Stage
                {STAGE_MENU.map(([k, w]) => (
                  <Stack.Toolbar.MenuAction key={k} isOn={J?.stage === k} subtitle={J?.stage === k && aiStage ? 'Set by the AI' : undefined} onPress={() => void setStage(k)}>{w}</Stack.Toolbar.MenuAction>
                ))}
              </Stack.Toolbar.Menu>
            </Stack.Toolbar.Menu>
            <Stack.Toolbar.Menu inline>
              <Stack.Toolbar.MenuAction icon="archivebox" destructive onPress={retire}>Retire</Stack.Toolbar.MenuAction>
            </Stack.Toolbar.Menu>
          </Stack.Toolbar.Menu>
        </Stack.Toolbar>
      )}
      {native && classic && (
        // the older editor only (HIG/toolbars: one prominent action, trailing). The document needs no button: he taps his words.
        <Stack.Toolbar placement="bottom">
          <Stack.Toolbar.Spacer />
          {/* label only: UIKit draws a bar item that has an image as the image alone, and the word is the HIG-standard prominent action */}
          <Stack.Toolbar.Button variant="prominent" disabled={!canWrite} accessibilityLabel="Write" accessibilityHint={footnote || (online ? undefined : 'Offline')} onPress={() => openWrite()}>Write</Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}

      {/* what sits above the document never changes when the keyboard comes up: a tap must land where he aimed it */}
      <View style={{ gap: 2 }}>
        {/* HIG/typography Title 1 (28 pt) for the full title, only when it says more than the bar's short name */}
        {!fresh && title !== shortName && <Text testID="bit-title" accessibilityRole="header" maxFontSizeMultiplier={1.8} style={{ fontFamily: F.g7, fontSize: 28, lineHeight: 34, letterSpacing: -0.6, color: C.text }}>{title}</Text>}
        {/* a new bit has one placeholder, the document's own; the idea line comes once the bit has a name */}
        {!fresh && <IdeaLine testID="bit-idea" summary={J?.summary} ai={aiIdea} onEdit={() => setIdea(true)} />}
      </View>

      {editing != null ? (
        <View testID="bit-edit" style={{ padding: 12, borderRadius: 16, backgroundColor: C.card, gap: 10, borderWidth: 1, borderColor: C.amber }}>
          <TextInput testID="bit-edit-input" value={editing} onChangeText={(t) => { setEditing(t); if (editSel) setEditSel(undefined); }} multiline autoFocus textAlignVertical="top" accessibilityLabel="The bit's text"
            selection={editSel} onSelectionChange={() => { if (editSel) setEditSel(undefined); }}
            placeholder={fresh ? 'Write the bit. The first line names it.' : undefined} placeholderTextColor={C.dim}
            style={{ minHeight: 160, fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text }} />
          <View style={{ flexDirection: 'row', gap: 8, alignItems: 'center', justifyContent: 'flex-end' }}>
            <Btn small v="ghost" testID="bit-edit-cancel" label="Cancel" onPress={() => { setEditing(null); setEditSel(undefined); }} />
            <Btn small v="primary" testID="bit-edit-save" label={saving ? 'Saving…' : 'Save'} disabled={saving || !(editing || '').trim()} onPress={saveEdit} />
          </View>
        </View>
      ) : order ? (
        <View testID="bit-reorder" style={{ gap: 4 }}>
          <View style={{ flexDirection: 'row', alignItems: 'center', minHeight: 44 }}>
            <TextBtn testID="reorder-cancel" label="Cancel" color={linkColor()} onPress={() => setOrder(null)} />
            <T v="semi" style={{ flex: 1, textAlign: 'center' }} accessibilityRole="header">Reorder</T>
            <TextBtn testID="reorder-done" label="Done" color={linkColor()} onPress={finishReorder} />
          </View>
          <DragList items={order} keyOf={(x) => x.k} testID="reorder-list"
            onReorder={(from, to) => setOrder((o) => (o ? (L.moveRow(o, from, to) as OrderRow[]) : o))}
            render={(x, _i, handle) => (
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4, minHeight: 44, borderBottomWidth: 1, borderBottomColor: C.hairline }}>
                {handle}
                {x.kind === 'line'
                  ? <Text maxFontSizeMultiplier={R.PROSE_CAP} style={{ flex: 1, minWidth: 0, fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text, paddingVertical: 9 }}>{x.text}</Text>
                  // the blank line between two ideas is a row of its own: drag it to split an idea somewhere else, or a line across it
                  : <Text maxFontSizeMultiplier={R.ROW_CAP} style={{ flex: 1, minWidth: 0, fontFamily: F.semi, fontSize: 13, lineHeight: 18, color: C.mut2 }}>Blank line</Text>}
              </View>
            )} />
        </View>
      ) : (
        <>
          {vCut === 'rant' && (
            // a word, not a colour (Film it §2.6): the version shown is the rant cut, and a write writes that cut
            <View testID="bit-rant-cut" accessible accessibilityLabel={`Rant cut, ${v?.label}`} style={{ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 8 }}>
              <View style={{ paddingHorizontal: 8, minHeight: 24, borderRadius: 6, borderWidth: 1, borderColor: C.border, justifyContent: 'center' }}>
                <Text maxFontSizeMultiplier={R.ROW_CAP} style={{ fontFamily: F.semi, fontSize: 13, color: C.text }}>Rant cut</Text>
              </View>
            </View>
          )}
          {classic ? (
            lines.length
              ? <BitLines testID="bit-lines" lines={lines} punch={punch} selected={sel} onSelect={setSel} onOpenAt={(i) => openWrite(i)} actions={lineActions} />
              : <Text testID="bit-empty" maxFontSizeMultiplier={R.PROSE_CAP} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.mut2, paddingVertical: 8 }}>Nothing written yet.</Text>
          ) : reading ? (
            // an older version: read, select and copy; not typed into (Restore makes it the newest)
            <Text testID="bit-reading" selectable maxFontSizeMultiplier={R.PROSE_CAP} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text2 }}>{v?.text || ''}</Text>
          ) : (
            // keyed by the cut: the stage document and the rant cut are two documents, each with its own draft
            <BitDoc key={vCut} ref={doc} testID="bit-doc" bit={id} cut={vCut} unit={v?.key ?? null} saved={v?.text || ''} said={said} fresh={fresh} autoFocus={p.edit === '1'} online={online}
              noteOn={!online || !!footnote} onSaved={docSaved} onFailed={() => ui.snack('Couldn’t save. Trying again.')} onState={setDocState}
              scroll={{ ref: scrollRef, y: scrollY, h: scrollH }} />
          )}
        </>
      )}

      {/* everything under the document is out of the way while the keyboard is up: nothing sits between him and typing */}
      {!typing && (
        <View testID="bit-rows" style={{ borderTopWidth: 1, borderTopColor: C.hairline }}>
          {/* first under the document: its Undo is the Undo of the save he just made (shown from the first save on) */}
          {(versions.length > 1 || canUndoWrite) && (
            <SectionRow testID="row-versions" kind="inline" label="Versions" meta={vi !== home ? R.stepperLabel(vi, versions.length, v?.date, v) : R.versionsMeta(vmeta)}
              accessibilityLabel={vi !== home ? `Versions, reading ${R.stepperValue(vi, versions.length, v?.date, v)}` : R.versionsLabel(vmeta)}
              open={versionsOpen} onPress={() => setVersionsOpen((o) => !o)}
              right={canUndoWrite ? <TextBtn testID="row-versions-undo" label="Undo" color={linkColor()} accessibilityLabel="Undo the last write" onPress={() => void undoLast()} /> : undefined}
              extraActions={canUndoWrite ? [{ label: 'Undo the last write', onPress: () => void undoLast() }] : undefined}>
              <VersionStepper testID="stepper" versions={versions} vi={vi} reading={reading} onStep={(i) => { setPos(i); setSel(null); }} onRestore={restore} durationS={J?.duration_s} estimated={!!J?.duration_estimated} />
            </SectionRow>
          )}
          {/* an empty bit opens on its one real action, the Pile: it leads there (the two sections below it hold nothing yet) */}
          {lines.length === 0 && pileRow}
          {aroundShown && (
            <AroundInline testID="around" bit={id} around={ar.data || null} text={v?.text || ''} canAct={cardsCanAct}
              right={lastAround ? <TextBtn testID="row-around-undo" label="Undo" color={linkColor()} accessibilityLabel="Undo the last change around this bit" onPress={() => void undoAround()} /> : undefined} />
          )}
          {aroundShown && <SetsInline testID="sets" sets={ar.data ? ar.data.sets : null} canAdd={barOn && online} onAdd={pickSets} />}
          {lines.length > 0 && pileRow}
          {!!tape && (
            <SectionRow testID="row-tape" kind="push" label="On tape" meta={tape} onPress={() => router.push(`/joke/${enc(id)}/tape` as any)} />
          )}
        </View>
      )}

      <Sheet visible={rename} onClose={() => setRename(false)} label="Short name" testID="bit-rename-sheet">
        <SheetHead title="Short name" onCancel={() => setRename(false)} testID="bit-rename" />
        <ShortName value={shortName} title={J?.title || ''} onSave={saveShort} />
      </Sheet>
      <Sheet visible={idea} onClose={() => setIdea(false)} label="Idea" testID="bit-idea-sheet">
        {idea && <IdeaForm value={J?.summary || ''} onCancel={() => setIdea(false)} onSave={saveIdea} />}
      </Sheet>
      <MenuSheet visible={!!menu} items={menu || []} onClose={() => setMenu(null)} title={title} testID="bit-menu" />
    </Screen>
  );
}

function ShortName({ value, title, onSave }: { value: string; title: string; onSave: (v: string) => void }) {
  const [v, setV] = useState(value);
  return (
    <View style={{ gap: 8 }}>
      <T v="mut">What you’d write on the paper list. Up to 24 letters.</T>
      <TextInput testID="bit-shortname-input" value={v} onChangeText={setV} maxLength={24} autoFocus placeholder={shortNameOf(title)} placeholderTextColor={C.dim} clearButtonMode="while-editing"
        style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.bold, fontSize: 18 }} />
      <View style={{ flexDirection: 'row', gap: 8 }}>
        <Btn testID="bit-shortname-save" label="Save" v="primary" style={{ flex: 1 }} disabled={!v.trim()} onPress={() => onSave(v)} />
        <Btn label="Default" v="ghost" onPress={() => onSave('')} />
      </View>
    </View>
  );
}

/** The idea line's editor: one field, Cancel leading and Done trailing (HIG/sheets). */
function IdeaForm({ value, onCancel, onSave }: { value: string; onCancel: () => void; onSave: (v: string) => void }) {
  const [v, setV] = useState(value);
  return (
    <View style={{ gap: 8 }}>
      <SheetHead title="Idea" onCancel={onCancel} onDone={() => onSave(v)} testID="bit-idea-form" />
      <TextInput testID="bit-idea-input" value={v} onChangeText={setV} multiline autoFocus accessibilityLabel="The idea" placeholder="Idea" placeholderTextColor={C.dim}
        style={{ minHeight: 88, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, paddingVertical: 10, color: C.text, fontFamily: F.body, fontSize: 17, lineHeight: 24 }} />
    </View>
  );
}
