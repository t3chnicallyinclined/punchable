// A bit's page, design option 3c (docs/design/SPEC-2026-11-03-bit-page-3c.md; Slice 1, app-only). Mostly his words:
// the bar (his short name; the sparkle, To a set, More), one muted idea line, the bit as 44-pt lines (the punch bold
// with one amber dot, the tags folded), then one collapsed row each for the Pile and the Versions (inline) and On
// tape (a push), and one bottom button, Write. Tap a line for its actions; tap it again and Write opens at that line.
// Two reads kept from the workbench: the joke (/api/jokes/<id>) and what sounds like it (/similar); the pile reads the
// notes attached to the bit (/api/merge/candidates?joke=) and this phone's memos. Every write is a new version
// (POST /api/jokes/<id>/versions, the old ones stay) with Undo in the snack and, non-timed, in the Versions row.
// Slice 1 has no outbox (D4): Write and To a set grey with the offline line until Slice 2. The sparkle waits for
// Slice 4; the Punch action for Slice 3 (line_meta); Sessions and the Claude line are hidden until they exist.
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Platform, Share, Text, TextInput, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { decide, undo, useApi, useOnline } from '@/lib/api';
import { useBoards } from '@/lib/boards';
import { addVersion } from '@/lib/bitWork';
import type { Version } from '@/lib/bitWork';
import { isFreshBit, nameFromFirstLine } from '@/rules/bitWorkRules';
import type { SimilarResult } from '@/rules/searchRows';
import * as R from '@/rules/bitLines';
import { shortNameOf } from '@/lib/say';
import { C, F, linkColor, stageWord } from '@/lib/theme';
import { haptics } from '@/lib/haptics';
import { useRecordings } from '@/lib/recordings';
import { afterWrite } from '@/lib/feedback';
import { Btn, ErrorCard, Icon, IconBtn, MenuItem, MenuSheet, Sheet, SheetHead, Skeleton, T, TextBtn, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { SectionRow } from '@/lib/SectionRow';
import { IdeaLine } from '@/lib/IdeaLine';
import { VersionStepper } from '@/lib/VersionStepper';
import { BitLines, type LineAction } from '@/lib/BitLines';
import { Pile } from '@/lib/Pile';
import { DragList } from '@/lib/DragList';

type V = Version & { punch?: number[] };
type Note = { id: string; text?: string | null; date?: string | null; reasons?: unknown[] | null; score?: number | null };

/** The stages he can set from More › Stage (Retire is its own, destructive item). */
const STAGE_MENU: [string, string][] = [['idea', 'Idea'], ['to_try', 'To try'], ['testing', 'Testing'], ['working', 'Working']];
const enc = encodeURIComponent;
const native = Platform.OS !== 'web';

function versionsOf(d: any): V[] {
  const out: V[] = [];
  for (const g of d?.versions || []) {
    for (const u of g.units || []) {
      if (!u.text) continue;
      // the punch marks ride on the version row once the read carries them (line_meta, Slice 3); nothing today
      out.push({ key: u.id || `${g.date}-${out.length}`, label: '', date: u.date || g.date || null, text: u.text, doc: u.doc_title, punch: u.line_meta?.punch ?? u.punch });
    }
  }
  if (!out.length && d?.joke?.summary && d?.yours) out.push({ key: 'idea', label: '', date: d.joke.first_seen || null, text: d.joke.summary });
  out.sort((a, b) => (b.date || '').localeCompare(a.date || ''));
  return out.map((v, i) => ({ ...v, label: `v${out.length - i}` }));
}

export default function BitPage() {
  const p = useLocalSearchParams<{ id: string; edit?: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const boards = useBoards();
  const online = useOnline();
  const d = useApi<any>(`/api/jokes/${enc(id)}`, { poll: 30000 });
  const sim = useApi<SimilarResult>(`/api/jokes/${enc(id)}/similar?limit=5`);
  const cand = useApi<{ history?: { notes?: Note[] } }>(`/api/merge/candidates?joke=${enc(id)}`, { cache: false });
  const [vi, setVi] = useState(0);
  const [selRaw, setSel] = useState<number | null>(null);
  const [rename, setRename] = useState(false);
  const [idea, setIdea] = useState(false);
  const [menu, setMenu] = useState<MenuItem[] | null>(null);
  const [editing, setEditing] = useState<string | null>(null);   // the text being edited (a new version on Save)
  const [editSel, setEditSel] = useState<{ start: number; end: number } | undefined>(undefined);
  const [order, setOrder] = useState<string[] | null>(null);     // Reorder mode: the lines being reordered
  const [pileOpen, setPileOpen] = useState<boolean | null>(null);
  const [versionsOpen, setVersionsOpen] = useState(false);
  // the last write's undo handle: "v9 · just now · Undo" in the Versions row (non-timed, HIG.md §7)
  const [lastWrite, setLastWrite] = useState<{ indexes: number[]; n: number } | null>(null);
  // opened from Bits "+": a fresh document, the editor open and focused before anything else
  const [openedFresh, setOpenedFresh] = useState(false);
  useEffect(() => {
    if (p.edit === '1' && d.data && !openedFresh && editing == null) { setEditing(''); setOpenedFresh(true); }
  }, [p.edit, d.data, openedFresh, editing]);
  const [saving, setSaving] = useState(false);
  const versions = useMemo(() => versionsOf(d.data), [d.data]);
  const reloadAll = useCallback(() => { d.reload(); sim.reload(); cand.reload(); }, [d.reload, sim.reload, cand.reload]); // eslint-disable-line react-hooks/exhaustive-deps
  // voice memos on this phone that are on this bit: started here, or filed here (SPEC 2026-10-31 §6)
  const recs = useRecordings();
  const memos = useMemo(() => recs.filter((r) => r.kind === 'memo' && (r.bit?.id === id || r.filed_on?.id === id)), [recs, id]);
  const J = d.data?.joke;
  // a fresh bit (lib/newBit) carries a marker title until its first Save names it; the page says "New bit" meanwhile
  const fresh = isFreshBit(J?.title);
  const title = fresh ? 'New bit' : (J?.title || '').split(' (')[0] || J?.title || 'Bit';
  const shortName = fresh ? 'New bit' : shortNameOf(J?.title || '', J?.short_name);
  const v = versions[Math.min(vi, Math.max(0, versions.length - 1))];
  const lines = useMemo(() => R.linesOf(v?.text), [v?.text]);
  const punch = useMemo(() => R.cleanPunch(v?.punch, lines.length), [v?.punch, lines.length]);
  const reading = vi > 0;
  const scraps = useMemo(() => R.pileScraps(cand.data?.history?.notes || [], memos), [cand.data, memos]);
  const barOn = R.barEnabled(lines, J?.summary) && !fresh;
  const canWrite = online && !reading && editing == null && order == null;
  const footnote = R.writeReason({ vi, count: versions.length, online: true });
  // a selection outlives nothing: stepping clears it (onStep), a write clears it (wrote), a shorter version drops it
  const sel = selRaw != null && selRaw < lines.length ? selRaw : null;
  // an empty bit opens its Pile row on its one real action (frame d); otherwise the rows start closed
  const pileIsOpen = pileOpen ?? (!!d.data && lines.length === 0);

  /** Every write lands as a new version: the snack's timed Undo and the row's non-timed one share the indexes. */
  const wrote = (indexes: number[], msg: string) => {
    haptics.success();
    setVi(0);
    setSel(null);
    setLastWrite({ indexes, n: versions.length + 1 });
    reloadAll();
    ui.snack(msg, async () => { await undo(indexes); setLastWrite(null); reloadAll(); });
  };
  const undoLast = async () => {
    if (!lastWrite) return;
    try { await undo(lastWrite.indexes); setLastWrite(null); reloadAll(); ui.snack('Undone'); } catch (e: any) { ui.snack(`Couldn't undo: ${e.message}`); }
  };

  const saveEdit = async () => {
    const t = (editing || '').trim();
    if (!t || saving) return;
    if (t === (v?.text || '').trim()) { setEditing(null); return; }
    setSaving(true);
    try {
      const r = await addVersion(id, t, 'edited on the bit page');
      let indexes = r.indexes;
      if (fresh) {
        // a fresh document takes its name from its first line (the first seven words, like a typed jot): rules/bitWorkRules
        const name = nameFromFirstLine(t);
        try { const w = await decide({ target: 'node', action: 'edit', id, after: { title: name }, reason: 'named from its first line' }); indexes = [...indexes, ...w.indexes]; } catch { /* the version is saved either way */ }
      }
      setEditing(null);
      setEditSel(undefined);
      wrote(indexes, openedFresh ? 'Saved' : `Saved as v${versions.length + 1}`);
    } catch (e: any) {
      ui.snack(`Couldn’t save: ${e.message}`);
    } finally {
      setSaving(false);
    }
  };
  /** Write: today's editor, over the whole bit; `at` = the line the caret starts on (the second tap on a line). */
  const openWrite = (at?: number) => {
    if (!canWrite) return;
    const text = v?.text || '';
    setEditSel(at == null ? undefined : { start: R.lineOffset(text, at), end: R.lineOffset(text, at) });
    setEditing(text);
  };
  const writeLines = async (next: string[], reason: string, msg: string) => {
    try {
      const r = await addVersion(id, R.joinLines(next), reason);
      wrote(r.indexes, msg);
    } catch (e: any) {
      ui.snack(`Couldn’t save: ${e.message}`);
    }
  };
  const cut = (i: number) => {
    const next = R.cutLine(lines, i);
    if (!next) return ui.snack('That is the only line');
    void writeLines(next, 'cut a line on the bit page', 'Line cut');
  };
  const startReorder = () => { if (lines.length > 1 && canWrite) { setSel(null); setOrder(lines); } };
  const finishReorder = () => {
    const next = order || [];
    setOrder(null);
    if (next.join('\n') !== lines.join('\n')) void writeLines(next, 'reordered the lines', 'Reordered');
  };
  const addScrap = (s: R.Scrap) => void writeLines(R.insertAfter(lines, sel, s.words), 'added from the pile', 'Added');
  const restore = () => { if (v) void writeLines(R.linesOf(v.text), `restored ${v.label}`, `Restored ${v.label} as v${versions.length + 1}`); };
  const toSet = () => afterWrite(() => boards.add(id, title), (ok) => ok, haptics);
  // a memo started here is filed on this bit (lib/recordings: POST /api/merge/attach after the idea)
  const recordMemo = () => router.push({ pathname: '/record', params: { kind: 'memo', bit: id, title: shortName } } as any);
  const share = () => Share.share({ message: [title, v?.text || ''].join('\n\n').trim(), title }).catch(() => ui.snack('Sharing is not available here'));
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
  const echoes = (sim.data?.like.moments.length || 0) + (sim.data?.like.bits.length || 0);
  const tape = R.tapeMeta(nights, killed, echoes);
  const vmeta = { count: versions.length, newestDate: versions[0]?.date, durationS: J?.duration_s, estimated: !!J?.duration_estimated, justWrote: !!lastWrite && lastWrite.n === versions.length };
  // the line's actions (Slice 1: Move · Cut; Punch waits for its home in Slice 3, the Claude asks for Slice 4). Move is
  // not shown on a one-line bit (a greyed control that leads nowhere is not rendered, §14).
  const lineActions: LineAction[] | null = reading || !online || editing != null ? null : [
    ...(lines.length > 1 ? [{ label: 'Move', testID: 'line-move', onPress: startReorder }] : []),
    { label: 'Cut', testID: 'line-cut', onPress: (i: number) => cut(i) },
  ];
  // More, in frequency order, the destructive item last (§3.8); the same list on the web as a sheet
  const webMenu = (): MenuItem[] => [
    { label: 'Reorder lines', onPress: startReorder, disabled: !canWrite || lines.length < 2, testID: 'menu-reorder' },
    { label: 'Record a memo', onPress: recordMemo, disabled: true },
    { label: `Add to ${boards.current?.title || 'the current set list'}`, onPress: () => void toSet(), disabled: !barOn || !boards.current },
    { label: 'Pick a set list…', onPress: () => boards.pick(id, title) },
    { label: 'Merge with…', onPress: () => router.push('/merge' as any) },
    { label: 'Share…', onPress: () => void share() },
    { label: 'Rename short name…', onPress: () => setRename(true) },
    { label: 'Stage…', onPress: () => setMenu(STAGE_MENU.map(([k, w]) => ({ label: `${w}${J?.stage === k ? ' ✓' : ''}`, onPress: () => void setStage(k) }))) },
    { label: 'Retire', onPress: retire, destructive: true },
  ];

  return (
    <Screen title={shortName} back testID="bitpage" maxWidth={760} offlineAt="bottom" footnote={footnote}
      right={native ? undefined : (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
          <Btn small v="primary" testID="bit-write" label="Write" disabled={!canWrite} onPress={() => openWrite()} />
          <IconBtn testID="bit-more" label="More" onPress={() => setMenu(webMenu())}><Icon name="more" size={22} color={C.amber} /></IconBtn>
        </View>
      )}>
      {native && (
        // HIG/toolbars: simple symbols (no circle borders), the important few in the bar, the rest under More
        <Stack.Toolbar placement="right">
          {/* Ask Claude arrives with Slice 4; until then the sparkle is present and off (VoiceOver hears why) */}
          <Stack.Toolbar.Button icon="sparkles" disabled accessibilityLabel="Ask Claude" accessibilityHint="Not in this build yet" onPress={() => {}} />
          <Stack.Toolbar.Button icon="text.badge.plus" disabled={!barOn || !online} accessibilityLabel="To a set" accessibilityHint={online ? `Adds it to ${boards.current?.title || 'a set list'}` : 'Offline'} onPress={() => void toSet()} />
          <Stack.Toolbar.Menu icon="ellipsis" accessibilityLabel="More">
            <Stack.Toolbar.MenuAction icon="arrow.up.arrow.down" disabled={!canWrite || lines.length < 2} onPress={startReorder}>Reorder lines</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="mic" onPress={recordMemo}>Record a memo</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.Menu inline>
              <Stack.Toolbar.MenuAction icon="list.bullet" onPress={() => boards.pick(id, title)}>Pick a set list…</Stack.Toolbar.MenuAction>
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
      {native && (
        // HIG/toolbars: one prominent action, trailing. iOS 18+ draws it (expo-router's bottom toolbar); device check 1 (§4.1) decides it stays
        <Stack.Toolbar placement="bottom">
          <Stack.Toolbar.Spacer />
          {/* label only: UIKit draws a bar item that has an image as the image alone, and the word is the HIG-standard prominent action */}
          <Stack.Toolbar.Button variant="prominent" disabled={!canWrite} accessibilityLabel="Write" accessibilityHint={footnote || (online ? undefined : 'Offline')} onPress={() => openWrite()}>Write</Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}

      <View style={{ gap: 2 }}>
        {/* HIG/typography Title 1 (28 pt) for the full title, only when it says more than the bar's short name */}
        {!fresh && title !== shortName && <Text testID="bit-title" accessibilityRole="header" maxFontSizeMultiplier={1.8} style={{ fontFamily: F.g7, fontSize: 28, lineHeight: 34, letterSpacing: -0.6, color: C.text }}>{title}</Text>}
        <IdeaLine testID="bit-idea" summary={J?.summary} ai={aiIdea} onEdit={() => setIdea(true)} />
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
          <DragList items={order.map((text, i) => ({ k: `${i}:${text}`, text }))} keyOf={(x) => x.k} testID="reorder-list"
            onReorder={(from, to) => setOrder((o) => (o ? R.moveLine(o, from, to) : o))}
            render={(x, i, handle) => (
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4, minHeight: 44, borderBottomWidth: 1, borderBottomColor: C.hairline }}>
                {handle}
                <Text maxFontSizeMultiplier={R.PROSE_CAP} style={{ flex: 1, minWidth: 0, fontFamily: R.isPunch(punch, lines.indexOf(x.text)) ? F.bold : F.body, fontSize: 17, lineHeight: 26, color: C.text, paddingVertical: 9 }}>{x.text}</Text>
              </View>
            )} />
        </View>
      ) : lines.length ? (
        <BitLines testID="bit-lines" lines={lines} punch={punch} selected={sel} onSelect={setSel} onOpenAt={(i) => openWrite(i)} actions={lineActions} />
      ) : (
        <Text testID="bit-empty" maxFontSizeMultiplier={R.PROSE_CAP} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.mut2, paddingVertical: 8 }}>Nothing written yet.</Text>
      )}

      <View testID="bit-rows" style={{ borderTopWidth: 1, borderTopColor: C.hairline }}>
        <SectionRow testID="row-pile" kind="inline" label="Pile" meta={String(scraps.length)} open={pileIsOpen} onPress={() => setPileOpen(!pileIsOpen)}>
          <Pile testID="pile" scraps={scraps} recs={memos} lines={lines} onAdd={addScrap} />
        </SectionRow>
        {versions.length > 1 && (
          <SectionRow testID="row-versions" kind="inline" label="Versions" meta={reading ? `v${versions.length - vi} of ${versions.length}` : R.versionsMeta(vmeta)}
            accessibilityLabel={reading ? `Versions, reading ${versions.length - vi} of ${versions.length}` : R.versionsLabel(vmeta)}
            open={versionsOpen} onPress={() => setVersionsOpen((o) => !o)}
            right={lastWrite && vmeta.justWrote && !reading ? <TextBtn testID="row-versions-undo" label="Undo" color={linkColor()} accessibilityLabel="Undo the last write" onPress={() => void undoLast()} /> : undefined}
            extraActions={lastWrite && vmeta.justWrote && !reading ? [{ label: 'Undo the last write', onPress: () => void undoLast() }] : undefined}>
            <VersionStepper testID="stepper" versions={versions} vi={vi} onStep={(i) => { setVi(i); setSel(null); }} onRestore={restore} durationS={J?.duration_s} estimated={!!J?.duration_estimated} />
          </SectionRow>
        )}
        {!!tape && (
          <SectionRow testID="row-tape" kind="push" label="On tape" meta={tape} onPress={() => router.push(`/joke/${enc(id)}/tape` as any)} />
        )}
      </View>

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
