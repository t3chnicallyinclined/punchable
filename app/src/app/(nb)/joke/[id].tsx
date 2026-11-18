// A bit is a page (docs/design/SPEC-2026-11-17-one-notebook.md screens 2 and 3; owner 2026-11-16: "a bit is a
// document ... it should be as easy as editing a google doc on your phone, with the power of punchable"; on build 33:
// "people are used to google docs/notes how can we simplify it but keep it as powerful").
//   - The title is its own field at the top: tap and type; Return goes to the text. Under it the document
//     (lib/BitDoc: one text field over the whole bit; the words are kept on the phone as he types and a version is
//     saved when he leaves the text, the page or the app, or pauses, only when the words changed).
//   - The bar: a quiet "Saved", Undo (the last save: his older words come back), and ⋯ (Versions, Add to a set,
//     Share, then the rest). While the keyboard is up, Done sits beside Undo.
//   - One bar above the keyboard, and at the bottom of the page when the keyboard is down (lib/PageBar): Say it (a
//     memo filed on this bit) · Film it · Punch it up. With words selected the same bar is Funnier · Shorter ·
//     3 options · Note for exactly those words; each is a switch. A marked range stays marked in the text. The marks
//     are kept on the phone (lib/marks) and listed under the document with their notes (lib/MarksList). With marks on
//     the page the bar is the sentence "N marks on this page. Anything you didn't mark stays word for word." and
//     Get a new draft: his typing is saved as a version, then one redline round goes out (lib/redlineActions).
//     Punch it up with nothing marked asks which: this paragraph, or the whole page. The round's state is that same
//     button (Writing…, New draft ready), which opens the draft (joke/[id]/draft).
//   - Under the document, when he is not typing: his marks, then Around this bit and In these sets from one read
//     (GET /api/jokes/<id>/around), the Pile, On tape; Versions when he opened it from ⋯ or is reading an older one.
// Every write is a new version (POST /api/jokes/<id>/versions; the old ones stay).
// A version is the stage cut or a rant cut (rules/bitLines cutOf): the page shows the newest stage version; marks
// live on that document only. The older flow is kept behind a preference (Settings › This device › "Write button on a
// bit"; prefs.bitEditor 'classic'): the bit as 44-pt lines, one bottom button, Write, and the one-box editor with
// Save, with none of the above. One of the two is drawn, never both.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Keyboard, Platform, Share, Text, TextInput, View } from 'react-native';
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
import { getPrefs, onPrefs, setPrefs } from '@/lib/prefs';
import { useRecordings } from '@/lib/recordings';
import { Btn, ErrorCard, Icon, IconBtn, MenuItem, MenuSheet, Sheet, SheetHead, Skeleton, T, TextBtn, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { SectionRow } from '@/lib/SectionRow';
import { VersionStepper } from '@/lib/VersionStepper';
import { BitLines, type LineAction } from '@/lib/BitLines';
import { BitDoc, type BitDocHandle, type BitDocState, type SaveWhy } from '@/lib/BitDoc';
import { Pile } from '@/lib/Pile';
import { AroundInline, SetsInline, aroundChanged, aroundPath } from '@/lib/Around';
import * as AR from '@/rules/aroundRules';
import { DragList } from '@/lib/DragList';
import { PAGE_BAR_H, PAGE_NOTE_H, PageBar, type PageBarHandle, type Target } from '@/lib/PageBar';
import { MarksList } from '@/lib/MarksList';
import { markId, peekMarks, useMarks } from '@/lib/marks';
import { useRound } from '@/lib/redline';
import { sendRound } from '@/lib/redlineActions';
import { actionSheet } from '@/lib/actionSheet';
import { announce } from '@/lib/announce';
import * as MR from '@/rules/markRules';
import * as PG from '@/rules/pageRules';
import { togglePin } from '@/rules/notebookRules';

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
  // which editor this phone draws: the page (default) or the older Write button (a preference, the fallback)
  const [classic, setClassic] = useState(() => getPrefs().bitEditor === 'classic');
  // pinned to the top of Notebook (kept on this device; the row's long press does the same)
  const pinKey = `bit:${id}`;
  const [pinned, setPinned] = useState(() => getPrefs().pins.includes(pinKey));
  useEffect(() => onPrefs((x) => { setClassic(x.bitEditor === 'classic'); setPinned(x.pins.includes(pinKey)); }), [pinKey]);
  const pinIt = () => { haptics.selection(); setPrefs({ pins: togglePin(getPrefs().pins, pinKey) }); };
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
  // marking is selecting: his marks not sent yet (kept on the phone), the bar that makes them, the round on the bit
  const marks = useMarks(id, null);
  const pageBar = useRef<PageBarHandle>(null);
  // the document's last selection: the bar is mounted after the tap that focused it and starts from this
  const lastSel = useRef<{ sel: { start: number; end: number }; text: string } | null>(null);
  // whether the page holds words right now (the document says so as he types; null until it has spoken)
  const [typedWords, setTypedWords] = useState<boolean | null>(null);
  // the mark whose note is being typed (the bar's Note, or a tap on a mark under the page)
  const [noteFor, setNoteFor] = useState<string | null>(null);
  const rr = useRound(id);
  const round = rr.data?.round || null;
  const rstate = MR.roundState(round);
  const [sending, setSending] = useState(false);
  const docStateRef = useRef<BitDocState>(QUIET);
  useEffect(() => { docStateRef.current = docState; }, [docState]);
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
  // the last write's undo handle: the bar's Undo (non-timed, HIG.md §7)
  const [lastWrite, setLastWrite] = useState<{ indexes: number[]; n: number } | null>(null);
  // opened from the pencil: a fresh page, the keyboard up before anything else
  const [openedFresh, setOpenedFresh] = useState(false);
  useEffect(() => {
    if (p.edit !== '1' || !d.data || openedFresh) return;
    setOpenedFresh(true);
    if (classic && editing == null) setEditing('');
  }, [p.edit, d.data, openedFresh, editing, classic]);
  const [saving, setSaving] = useState(false);
  // he typed a title himself: the first save no longer names the page from its first line
  const [titled, setTitled] = useState(false);
  const versions = useMemo(() => versionsOf(d.data), [d.data]);
  const reloadAll = useCallback(() => { d.reload(); sim.reload(); cand.reload(); ar.reload(); }, [d.reload, sim.reload, cand.reload, ar.reload]); // eslint-disable-line react-hooks/exhaustive-deps
  // a card's sheet or the picker wrote something about this bit: read again; a new version's Undo is the bar's, any
  // other write's sits on the Around row
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
  // a fresh bit (lib/newBit) carries a marker title until its first save names it; the page shows an empty title meanwhile
  const fresh = isFreshBit(J?.title);
  const title = fresh ? '' : (J?.title || '').split(' (')[0] || J?.title || '';
  const shortName = fresh ? 'New page' : shortNameOf(J?.title || '', J?.short_name);
  const home = R.homeIndex(versions);
  const newestRant = R.newestOf(versions, 'rant');
  const vi = Math.min(typeof pos === 'number' ? pos : pos === 'rant' && newestRant >= 0 ? newestRant : home, Math.max(0, versions.length - 1));
  const v = versions[vi];
  const vCut: R.Cut = v?.cut || 'stage';
  const src = R.sourceFor(vCut, 'write');
  const lines = useMemo(() => R.linesOf(v?.text), [v?.text]);
  // what every version of this cut says: a draft on the phone that one of them already says is not newer words
  const said = useMemo(() => versions.filter((x) => x.cut === vCut).map((x) => x.text), [versions, vCut]);
  const punchMarks = useMemo(() => R.cleanPunch(v?.punch, lines.length), [v?.punch, lines.length]);
  // reading = an older version of its cut: the text is not editable, Restore is on
  const reading = versions.length > 0 && !R.isNewestOfCut(versions, vi);
  const scraps = useMemo(() => R.pileScraps((cand.data?.history?.notes || []).filter((n: Note) => !takeNotes.has(n.id)), memos), [cand.data, memos, takeNotes]);
  const barOn = R.barEnabled(lines, J?.summary) && !fresh;
  const typing = !classic && docState.typing;
  // marks live on the newest stage document (the rant cut and older versions are other texts)
  const marksHere = !classic && !reading && vCut === 'stage';
  const nMarks = marksHere ? marks.marks.length : 0;
  // the page's bar: on the keyboard while he types (the phone), in the shell's bottom slot otherwise and on the web
  const barShown = !classic && !reading && order == null;
  const typingBar = native && typing && barShown;
  // the page's own writes (a reorder, a line from the pile, a restore, Cut): online, on the newest version, nothing else open
  const canWrite = online && !reading && editing == null && order == null && !typing;
  // the one line above the bottom: an older version being read, else why his words are not a version yet
  const footnote = R.writeReason({ vi, count: versions.length, online: true, newest: !reading }) || (classic ? null : docState.note);
  // the quiet word in the bar (HIG/feedback: status shown passively; he expects a save to work)
  const savedWord = classic || reading ? '' : PG.savedWord({ pending: docState.pending, note: docState.note, typed: typing, versions: versions.length });
  // a selection outlives nothing: stepping clears it (onStep), a write clears it (wrote), a shorter version drops it
  const sel = selRaw != null && selRaw < lines.length ? selRaw : null;
  // an empty bit opens its Pile row on its one real action (frame d); otherwise the rows start closed
  const pileIsOpen = pileOpen ?? (!!d.data && lines.length === 0);
  /** The document as it stands: what he typed (the document owns it), else the version shown. */
  const docNow = () => (!classic && !reading ? doc.current?.text() : undefined) ?? v?.text ?? '';

  /** Every write lands as a new version: the snack's timed Undo and the bar's non-timed one share the indexes. */
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
  /** The document saved a version by itself. Quiet while he is on the page: the bar reads "Saved" and its Undo takes
   *  the save back. Only a save made as he leaves the page is a snack, because the bar is no longer in front of him. */
  const docSaved = (indexes: number[], why: SaveWhy) => {
    const word = fresh ? 'Saved' : `Saved as v${versionCount.current + 1}${vCut === 'rant' ? ' · rant' : ''}`;
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
  // Add to a set: the picker, with a tick on every set the bit is already in (SPEC 2026-11-16 §4.5)
  const pickSets = () => boards.pick(id, shortName);
  const undoAround = async () => {
    if (!lastAround) return;
    try { await lastAround.undo(); setLastAround(null); reloadAll(); ui.snack('Undone'); } catch (e: any) { ui.snack(`Couldn't undo: ${e.message}`); }
  };
  // Say it: a memo started here is filed on this bit (lib/recordings: POST /api/merge/attach after the idea)
  const sayIt = () => { doc.current?.blur(); ui.withMic(() => router.push({ pathname: '/record', params: { kind: 'memo', bit: id, title: shortName } } as any)); };
  // Film it (SPEC 2026-11-03 §2.1): the front camera with the newest version's lines on the band; phone only
  const filmIt = () => { doc.current?.blur(); router.push(`/film/${enc(id)}` as any); };
  // Rant on this… (SPEC 2026-11-06 §12.7): a topic named for the bit, the bit kept on it, the topic opened
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
  // the sessions sheet on the version shown (its unit; the typed-idea stand-in has none, so it is off there); off too
  // while words he typed are not a version yet (a session reads the saved version)
  const canAsk = online && lines.length > 0 && !!v && v.key !== 'idea' && editing == null && order == null && !typing && !(!classic && docState.pending);
  const openSessions = () => {
    if (!canAsk || !v) return;
    router.push({ pathname: '/session/[bit]', params: { bit: id, unit: v.key, cut: v.cut, n: String(versions.length), title: shortName } } as any);
  };
  const share = () => Share.share({ message: [title, docNow()].join('\n\n').trim(), title: title || shortName }).catch(() => ui.snack('Sharing is not available here'));
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
  /** The title he typed: the page's name from now on (quiet, like a save; the snack only when it failed). */
  const saveTitle = async (s: string) => {
    const t = s.replace(/\s+/g, ' ').trim();
    if (!t || t === title) return;
    setTitled(true);
    try {
      await decide({ target: 'node', action: 'edit', id, after: { title: t }, reason: 'the title on the page' });
      d.reload();
    } catch (e: any) {
      ui.snack(online ? `Couldn’t rename the page: ${e.message}` : 'Offline. The title was not saved.');
    }
  };

  /** Get a new draft: his typing is saved as a version first (the marks are on the words he sees), then one round on
   *  that version. A draft already waiting is replaced, so that asks first. One send at a time from the first tap. */
  const sendBusy = useRef(false);
  const sendMarks = async () => {
    if (sendBusy.current || !marksHere) return;
    if (!online) return ui.snack('Offline. Your marks wait on this iPhone.');
    sendBusy.current = true;
    setSending(true);
    const done = () => { sendBusy.current = false; setSending(false); };
    let unit: string | null = null;
    try {
      await doc.current?.save('done');
      unit = doc.current?.unit() ?? null;
      // a page that is still only its typed idea has no version to mark: its words become one now
      if (unit === 'idea') {
        const words = L.docText(doc.current?.text() ?? '');
        const r = await addVersion(id, words, 'saved before a new draft', false, src);
        doc.current?.adopt(words, r.unit);
        unit = r.unit;
        reloadAll();
      }
    } catch {
      unit = null;
    }
    if (!unit) {
      done();
      // the note as it is now (a save that found the words too long says so while it ran)
      const note = docStateRef.current.note;
      return ui.snack(note ? `${note}. Your marks wait here.` : 'Your words are still saving. Try again in a moment.');
    }
    // the words as the server holds them (the route strips the blank ends; the marks are found by their words anyway)
    const text = L.docText(doc.current?.text() ?? v?.text ?? '');
    const go = async () => {
      // the marks as they are now: a note typed a moment ago is in them (never a render's older copy)
      try {
        const ok = await sendRound(ui, id, { unit }, text, peekMarks(id, null));
        if (ok) void rr.reload();
      } finally {
        done();
      }
    };
    if (rstate === 'ready') {
      actionSheet({
        title: 'Get a new draft?', message: 'The draft waiting on this page is replaced by the new one.',
        options: [{ label: 'Get a new draft', onPress: () => void go() }, { label: 'Cancel', cancel: true, onPress: done }],
      });
    } else void go();
  };
  // VoiceOver hears the round change (accessibilityLiveRegion is Android only); never the first read after the page opens
  const lastRound = useRef<MR.RoundState | null>(null);
  useEffect(() => {
    if (!rr.data) return;
    if (lastRound.current != null && lastRound.current !== rstate && rstate !== 'none') {
      announce(MR.roundLine(round));
    }
    lastRound.current = rstate;
  }, [rstate, round, rr.data]);
  /** A tap on Funnier, Shorter, 3 options or Note with words selected (rules/pageRules: each is a switch). */
  const onAsk = (ask: PG.Ask, target: Target) => {
    const text = doc.current?.text() ?? '';
    // the web: a click on the bar has already taken the cursor out of the text, and no selection event follows, so
    // the bar lets go of the words once it has acted on them (the footer and its button are then in sight)
    if (!native) setTimeout(() => pageBar.current?.select({ start: 0, end: 0 }, ''), 0);
    if (ask === 'note') {
      const r = PG.markForNote(text, peekMarks(id, null), target, markId());
      if (r.error || !r.mark) { haptics.warning(); ui.snack(r.error || 'Those words could not be marked'); return; }
      haptics.selection();
      void marks.replace(r.marks);
      doc.current?.blur();
      setNoteFor(r.mark.id);
      return;
    }
    const r = PG.toggleAsk(text, peekMarks(id, null), target, ask, markId());
    announce(r.said);
    if (r.error) { haptics.warning(); ui.snack(r.error); return; }
    haptics.selection();
    void marks.replace(r.marks);
  };
  /** The note sheet closed: a mark that asks for nothing and says nothing goes with it. */
  const closeNote = (note?: string) => {
    const mid = noteFor;
    setNoteFor(null);
    if (!mid) return;
    const now = peekMarks(id, null);
    const m = now.find((x) => x.id === mid);
    if (m) void marks.replace(PG.saveNote(now, mid, note ?? m.note));
  };
  const removeMark = (mid: string) => { setNoteFor(null); void marks.replace(MR.removeMark(peekMarks(id, null), mid)); };
  const openDraft = () => { doc.current?.blur(); router.push(`/joke/${enc(id)}/draft` as any); };
  // the bar's main button (rules/pageRules punch): send the marks, open the draft, or ask what to punch up
  const pstate = PG.punch({ marks: nMarks, round: rstate, online, sending, empty: !marksHere || !(typedWords ?? !!(v?.text || '').trim()) });
  const punchIt = () => {
    if (pstate.does === 'open') return openDraft();
    if (pstate.does === 'send') return void sendMarks();
    if (pstate.does !== 'choose') return;
    const t = PG.punchTargets(docNow(), doc.current?.caret() ?? null);
    const go = (target: Target) => {
      const r = PG.toggleAsk(docNow(), peekMarks(id, null), target, 'funnier', markId());
      if (r.error) return ui.snack(r.error);
      void marks.replace(r.marks).then(() => sendMarks());
    };
    const whole = t.whole && t.whole.text !== t.paragraph?.text ? t.whole : null;
    const options = [
      ...(t.paragraph ? [{ label: 'This paragraph', onPress: () => go(t.paragraph as Target) }] : []),
      ...(whole ? [{ label: 'The whole page', onPress: () => go(whole) }] : []),
    ];
    if (!options.length) return ui.snack('Select the words you want punched up first.');
    if (native) {
      actionSheet({ title: 'Punch it up', message: 'A whole new draft comes back. Anything you didn’t mark stays word for word.', options: [...options, { label: 'Cancel', cancel: true }] });
    } else setMenu(options);
  };
  // the keyboard's top in this screen, for the bar that sits on it (lib/KeyboardBar's pattern)
  const root = useRef<any>(null);
  const [kb, setKb] = useState(0);
  // the bar rides the keyboard only once its top is known; with a hardware keyboard there is none, and the bar stays
  // in the page's bottom slot while he types (iOS dev review)
  const barOnKeys = typingBar && kb > 0;
  useEffect(() => {
    if (!native) return;
    const show = Keyboard.addListener('keyboardWillChangeFrame', (e) => {
      root.current?.measureInWindow((_x: number, y: number, _w: number, h: number) => setKb(Math.max(0, y + h - e.endCoordinates.screenY)));
    });
    const hide = Keyboard.addListener('keyboardWillHide', () => setKb(0));
    return () => { show.remove(); hide.remove(); };
  }, []);

  if (d.rawError && !d.data) return <Screen title="Page" back><ErrorCard error={String(d.rawError)} onRetry={d.reload} /></Screen>;
  if (!d.data) return <Screen title="Page" back><Skeleton lines={6} height={18} testID="bit-skeleton" /></Screen>;

  const aiStage = !!d.data?.ai?.stage_ai && !d.data?.ai?.stage_confirmed;
  const nights = sim.data?.performed.length || 0;
  const killed = R.killedCount((sim.data?.performed || []) as { rating?: string | null }[]);
  // what sounds like it is a group of "Around this bit" now (SPEC 2026-11-16 §2.2), so it no longer opens On tape
  const tape = R.tapeMeta(nights, killed, 0, takes.length);
  // collapsed, the row reads the newest stage version (Film it §2.6: "unchanged"); right after a write, "just now"
  const vmeta = { count: versions.length - home, newestDate: versions[home]?.date, durationS: J?.duration_s, estimated: !!J?.duration_estimated, justWrote: !!lastWrite && lastWrite.n === versions.length };
  const canUndoWrite = !!lastWrite && vmeta.justWrote && !reading;
  // the older editor's line actions (Move · Cut). Move is not shown on a one-line bit.
  const lineActions: LineAction[] | null = reading || !online || editing != null ? null : [
    ...(lines.length > 1 ? [{ label: 'Move', testID: 'line-move', onPress: startReorder }] : []),
    { label: 'Cut', testID: 'line-cut', onPress: (i: number) => cut(i) },
  ];
  // the two sections under the document are drawn while the read is on its way too (skeleton rows), so the rows
  // under them do not jump when it lands; gone only when the read failed with nothing cached (a server without the route)
  const aroundShown = !!ar.data || !ar.rawError;
  // a card writes into the saved document: not offline, and not over words he typed that no version holds yet
  const cardsCanAct = online && !reading && order == null && !(!classic && docState.pending);
  const versionsShown = versionsOpen || vi !== home;
  const pileRow = (
    <SectionRow testID="row-pile" kind="inline" label="Pile" meta={String(scraps.length)} open={pileIsOpen} onPress={() => setPileOpen(!pileIsOpen)}>
      <Pile testID="pile" scraps={scraps} recs={memos} lines={lines} onAdd={addScrap} />
    </SectionRow>
  );
  const showVersions = () => { doc.current?.blur(); setVersionsOpen(true); };
  // ⋯ in the mockup's order (Versions, Add to a set, Share), then the rest by how often, the destructive item last;
  // the same list on the web as a sheet
  const webMenu = (): MenuItem[] => [
    { label: versions.length > 1 ? `Versions · ${versions.length}` : 'Versions', onPress: showVersions, disabled: versions.length < 2, testID: 'menu-versions' },
    { label: 'Add to a set…', onPress: pickSets, disabled: !barOn || !online, testID: 'menu-sets' },
    { label: 'Share…', onPress: () => void share() },
    { label: pinned ? 'Unpin from the top' : 'Pin to the top', onPress: pinIt, testID: 'menu-pin' },
    { label: 'Reorder lines', onPress: startReorder, disabled: !canWrite || lines.length < 2, testID: 'menu-reorder' },
    { label: 'Make it a rant, pick a hook…', onPress: openSessions, disabled: !canAsk, testID: 'menu-ask' },
    { label: 'Idea…', onPress: () => setIdea(true), testID: 'menu-idea' },
    { label: 'Rant on this…', onPress: () => void rantOn(), disabled: !online, testID: 'menu-rant-on' },
    { label: 'Merge with…', onPress: () => router.push('/merge' as any) },
    { label: 'Rename short name…', onPress: () => setRename(true) },
    { label: 'Stage…', onPress: () => setMenu(STAGE_MENU.map(([k, w]) => ({ label: `${w}${J?.stage === k ? ' ✓' : ''}`, onPress: () => void setStage(k) }))) },
    { label: 'Retire', onPress: retire, destructive: true },
  ];
  const done = () => doc.current?.blur();
  const barNote = !online ? 'Offline · kept on this iPhone' : docState.note;
  const bar = (onKeys: boolean) => (
    <PageBar ref={pageBar} testID="page-bar" marks={marksHere ? marks.marks : []} text={docNow} punch={pstate}
      onSay={sayIt} onFilm={filmIt} onPunch={punchIt} onAsk={onAsk} filmOff={!native ? 'Filming is on the iPhone app' : !lines.length && !typing ? 'Nothing to read yet' : null}
      note={onKeys ? barNote : null} footer={!onKeys && !typing && nMarks > 0}
      // only the bar on the keyboard starts from the document's last selection: once the keyboard is down nothing is
      // selected on screen, and a remembered range would hide the footer behind four buttons (iOS dev review)
      seed={onKeys && marksHere ? () => lastSel.current : undefined} />
  );

  return (
    <View ref={root} collapsable={false} style={{ flex: 1 }}>
    <Screen title={shortName} back backLabel="Notebook" testID="bitpage" maxWidth={760} offlineAt={barOnKeys ? 'none' : 'bottom'} footnote={footnote}
      bar={barShown && !barOnKeys ? bar(false) : undefined}
      // the bar's centre is the quiet "Saved" (the page's name is the big title under it), never a button
      nativeHeader={classic ? undefined : { headerTitle: () => <Text testID="page-saved" maxFontSizeMultiplier={1.3} accessibilityLabel={savedWord || undefined} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>{savedWord}</Text> }}
      offlineNote={classic ? undefined : 'Offline · kept on this iPhone'} scrollRef={scrollRef} scrollWatch={classic ? undefined : scrollWatch}
      right={native ? undefined : (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
          {!classic && !!savedWord && <Text testID="page-saved" style={{ fontFamily: F.body, fontSize: 13, color: C.mut2, paddingRight: 6 }}>{savedWord}</Text>}
          {classic && <Btn small v="primary" testID="bit-write" label="Write" disabled={!canWrite} onPress={() => openWrite()} />}
          {!classic && <IconBtn testID="page-undo" label="Undo the last save" disabled={!canUndoWrite} onPress={() => void undoLast()}><Icon name="undo" size={22} color={canUndoWrite ? C.amber : C.dim} /></IconBtn>}
          {typing
            ? <Btn small v="primary" testID="bit-done" label="Done" onPress={done} />
            : <IconBtn testID="bit-more" label="More: versions, add to a set, share" onPress={() => setMenu(webMenu())}><Icon name="more" size={22} color={C.amber} /></IconBtn>}
        </View>
      )}>
      {native && typing && (
        // the keyboard is up: Undo, and Done trailing and prominent (HIG/toolbars; Notes does the same). Done puts the
        // keyboard away, which saves a version if the words changed.
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button icon="arrow.uturn.backward" disabled={!canUndoWrite} accessibilityLabel="Undo the last save" accessibilityHint="Your words from before it come back" onPress={() => void undoLast()} />
          <Stack.Toolbar.Button variant="prominent" accessibilityLabel="Done" accessibilityHint="Puts the keyboard away" onPress={done}>Done</Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}
      {native && !typing && (
        // HIG/toolbars: simple symbols, the important few in the bar, the rest under More (the mockup's Undo and ⋯)
        <Stack.Toolbar placement="right">
          {!classic && <Stack.Toolbar.Button icon="arrow.uturn.backward" disabled={!canUndoWrite} accessibilityLabel="Undo the last save" accessibilityHint="Your words from before it come back" onPress={() => void undoLast()} />}
          <Stack.Toolbar.Menu icon="ellipsis" accessibilityLabel="More: versions, add to a set, share">
            <Stack.Toolbar.MenuAction icon="clock.arrow.circlepath" disabled={versions.length < 2} subtitle={versions.length > 1 ? String(versions.length) : undefined} onPress={showVersions}>Versions</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="text.badge.plus" disabled={!barOn || !online} subtitle={online ? undefined : 'offline'} onPress={pickSets}>Add to a set…</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="square.and.arrow.up" onPress={() => void share()}>Share…</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon={pinned ? 'pin.slash' : 'pin'} onPress={pinIt}>{pinned ? 'Unpin from the top' : 'Pin to the top'}</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.Menu inline>
              <Stack.Toolbar.MenuAction icon="arrow.up.arrow.down" disabled={!canWrite || lines.length < 2} onPress={startReorder}>Reorder lines</Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="sparkles" disabled={!canAsk} subtitle={!online ? 'offline' : undefined} onPress={openSessions}>Make it a rant, pick a hook…</Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="megaphone" disabled={!online} subtitle={online ? undefined : 'offline'} onPress={() => void rantOn()}>Rant on this…</Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="arrow.triangle.merge" onPress={() => router.push('/merge' as any)}>Merge with…</Stack.Toolbar.MenuAction>
            </Stack.Toolbar.Menu>
            <Stack.Toolbar.Menu inline>
              <Stack.Toolbar.MenuAction icon="lightbulb" onPress={() => setIdea(true)}>Idea…</Stack.Toolbar.MenuAction>
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
        // the older editor only (HIG/toolbars: one prominent action, trailing). The page needs no button: he taps his words.
        <Stack.Toolbar placement="bottom">
          <Stack.Toolbar.Spacer />
          <Stack.Toolbar.Button variant="prominent" disabled={!canWrite} accessibilityLabel="Write" accessibilityHint={footnote || (online ? undefined : 'Offline')} onPress={() => openWrite()}>Write</Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}

      {/* what sits above the document never changes when the keyboard comes up: a tap must land where he aimed it */}
      <PageTitle key={id} testID="bit-title" value={title} editable={online && !reading && order == null} onSave={(t) => void saveTitle(t)} onNext={() => doc.current?.focus()} />

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
              ? <BitLines testID="bit-lines" lines={lines} punch={punchMarks} selected={sel} onSelect={setSel} onOpenAt={(i) => openWrite(i)} actions={lineActions} />
              : <Text testID="bit-empty" maxFontSizeMultiplier={R.PROSE_CAP} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.mut2, paddingVertical: 8 }}>Nothing written yet.</Text>
          ) : reading ? (
            // an older version: read, select and copy; not typed into (Restore makes it the newest)
            <Text testID="bit-reading" selectable maxFontSizeMultiplier={R.PROSE_CAP} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text2 }}>{v?.text || ''}</Text>
          ) : (
            // keyed by the cut: the stage document and the rant cut are two documents, each with its own draft
            <BitDoc key={vCut} ref={doc} testID="bit-doc" bit={id} cut={vCut} unit={v?.key ?? null} saved={v?.text || ''} said={said} fresh={fresh && !titled} autoFocus={p.edit === '1'} online={online}
              noteOn={!barOnKeys && (!online || !!footnote)} onSaved={docSaved} onFailed={() => ui.snack('Couldn’t save. Trying again.')} onState={setDocState}
              scroll={{ ref: scrollRef, y: scrollY, h: scrollH }} marks={marksHere ? marks.marks : undefined} barH={barOnKeys ? PAGE_BAR_H + (barNote ? PAGE_NOTE_H : 0) : 0}
              onSelection={(s, text) => { lastSel.current = { sel: s, text }; setTypedWords(!!text.trim()); if (marksHere) pageBar.current?.select(s, text); }} />
          )}
        </>
      )}

      {/* his marks, in the page's order, with their notes; the sheet a mark opens lives here too (the bar's Note opens it) */}
      {marksHere && (nMarks > 0 || !!noteFor) && (
        <View testID="bit-marks" style={{ display: typing && !noteFor ? 'none' : 'flex', gap: 2 }}>
          {nMarks > 0 && <T v="eb" accessibilityRole="header">{MR.markCount(nMarks)}</T>}
          <MarksList testID="marks" text={docNow} marks={marks.marks} editing={noteFor} onEdit={(mid) => (mid ? setNoteFor(mid) : closeNote())}
            onSave={(_mid, note) => closeNote(note)} onRemove={removeMark} />
        </View>
      )}

      {/* everything under the document is out of the way while the keyboard is up: nothing sits between him and typing */}
      {!typing && (
        <View testID="bit-rows" style={{ borderTopWidth: 1, borderTopColor: C.hairline }}>
          {versionsShown && (
            <SectionRow testID="row-versions" kind="inline" label="Versions" meta={vi !== home ? R.stepperLabel(vi, versions.length, v?.date, v) : R.versionsMeta(vmeta)}
              accessibilityLabel={vi !== home ? `Versions, reading ${R.stepperValue(vi, versions.length, v?.date, v)}` : R.versionsLabel(vmeta)}
              open onPress={() => { setVersionsOpen(false); setPos(null); }}>
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
      <MenuSheet visible={!!menu} items={menu || []} onClose={() => setMenu(null)} title={title || shortName} testID="bit-menu" />
    </Screen>
    {barOnKeys && (
      // the page's bar on the keyboard while he types: a plain view at the keyboard's top (never an InputAccessoryView);
      // it carries the page's one line of status meanwhile (one line above the keys, never two)
      <View style={{ position: 'absolute', left: 0, right: 0, bottom: kb, zIndex: 8 }}>{bar(true)}</View>
    )}
    </View>
  );
}

/** The page's title: its own field, typed straight into (HIG/typography Title 1). Return goes on to the text; leaving
 *  the field with other words in it renames the page. Not a controlled echo of the server: what he types stays put. */
function PageTitle({ value, editable, onSave, onNext, testID }: { value: string; editable: boolean; onSave: (v: string) => void; onNext: () => void; testID?: string }) {
  const [v, setV] = useState(value);
  // one save per title: blur can come twice (Return, then the focus moving on)
  const sent = useRef(value);
  // the server's title changes under the page (his own rename landing; the first save naming a new page from its
  // first line): the field takes it only while he is not typing in it, and is never remounted (a remount would drop
  // the keyboard and what he typed: iOS dev review)
  const [inField, setInField] = useState(false);
  const [seen, setSeen] = useState(value);
  if (value !== seen) {
    setSeen(value);
    if (!inField) setV(value);
  }
  // only when the server's title changes: re-running it when he leaves the field would forget the title just sent,
  // and the second blur of a Return would save it twice (iOS dev review, second look)
  useEffect(() => { if (!inField) sent.current = value; }, [value]); // eslint-disable-line react-hooks/exhaustive-deps
  const leave = () => {
    setInField(false);
    const t = v.replace(/\s+/g, ' ').trim();
    // left empty: the field shows the title the page has (it may have been named while he was in the field)
    if (!t) { setV(value); return; }
    if (t === sent.current) return;
    sent.current = t;
    onSave(t);
  };
  return (
    <TextInput testID={testID} value={v} onChangeText={(t) => setV(t.replace(/\n/g, ' '))} editable={editable} multiline scrollEnabled={false}
      submitBehavior="submit" blurOnSubmit returnKeyType="next" onFocus={() => setInField(true)} onSubmitEditing={() => { leave(); onNext(); }} onBlur={leave}
      accessibilityLabel="Title" accessibilityHint="Renames the page when you leave the field" maxFontSizeMultiplier={1.8} maxLength={160}
      placeholder="Title" placeholderTextColor={C.dim}
      style={[{ fontFamily: F.g7, fontSize: 28, lineHeight: 34, letterSpacing: -0.6, color: C.text, padding: 0, margin: 0 }, native ? null : ({ outlineStyle: 'none' } as any)]} />
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
