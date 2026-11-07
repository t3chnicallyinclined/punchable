// The script (SPEC 2026-11-06 §12.4, §12.5): a page like Notes, not a form with a Save button. Pushed from the topic.
// The bar: Back · the topic · sparkle (Draft the rest · Make it a rant · Pick a hook) · ⋯ (Versions · Open in Bits);
// while a line is being written both become Done. The hook line first ("Hook lands at 2.1 s · runs 0:48", tap = Pick
// a hook), then his lines at the stage prose size, each with where it came from under it ("from Free employee v3",
// "· edited", the AI badge, the Yours mark, or nothing for a line he typed). Tap a line: Claude · Punch · Cut · ⋯ under
// it; tap it again: he writes there (Return = a new line under it, Backspace on an empty line joins up). "Next line"
// last. A blank rant: "First line" and Draft it from 3 kept. At the bottom: Material, and Film it once there is a line;
// while writing, the keyboard bar: Material · Claude · Punch · hide keyboard.
// Saving: every settled edit is a local draft (nothing typed is lost); leaving the page, or a minute in the
// background, writes one version if anything changed ("Saved as v4", Undo). Claude never changes a line until he picks.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AppState, Keyboard, Platform, Pressable, Text, TextInput, View } from 'react-native';
import { Stack, router, useFocusEffect, useLocalSearchParams } from 'expo-router';
import { get, undo, useOnline } from '@/lib/api';
import { pollJob, rant, type DraftResult } from '@/lib/rant';
import { readDraft } from '@/lib/scriptDraft';
import { scripts, useScript } from '@/lib/scriptStore';
import { actionSheet } from '@/lib/actionSheet';
import { haptics } from '@/lib/haptics';
import { Icon } from '@/lib/Icon';
import { KEYBOARD_BAR_H, KeyboardBar } from '@/lib/KeyboardBar';
import { Screen } from '@/lib/Screen';
import { WorkerLine } from '@/lib/worker';
import { C, F, linkColor } from '@/lib/theme';
import { AIBadge, Btn, Skeleton, T, Yours, useUI } from '@/lib/ui';
import { PROSE_CAP, ROW_CAP } from '@/rules/bitLines';
import { hookCheck, hookWord } from '@/rules/hookRules';
import { draftFrom, materialRows, type TopicPage } from '@/rules/rantRules';
import * as S from '@/rules/scriptRules';

const native = Platform.OS !== 'web';
const enc = encodeURIComponent;
const BACKGROUND_SAVE_MS = 60_000;

export default function Script() {
  const p = useLocalSearchParams<{ topic: string; fresh?: string }>();
  const tid = decodeURIComponent(String(p.topic || ''));
  const freshRant = p.fresh === '1';
  const ui = useUI();
  const online = useOnline();
  const st = useScript(tid);
  const [page, setPage] = useState<TopicPage | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [sel, setSel] = useState<number | null>(null);
  const [editing, setEditing] = useState<number | null>(null);
  const caret = useRef(0);
  const [drafting, setDrafting] = useState<{ waitingFor: string | null } | null>(null);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  // ---- open: the server's rant, or this phone's newer draft of it (silently), or a blank one (Start another rant)
  const load = useCallback(async (reopen: boolean) => {
    try {
      const pg = await get<TopicPage>(`/api/topics/${enc(tid)}`);
      if (!alive.current) return;
      setPage(pg);
      setErr(null);
      const cur = scripts.get(tid);
      const doc = freshRant ? null : pg.rants?.[0] || null;
      if (cur && !reopen) return;
      if (cur && reopen) {
        // back from a session (Pick a hook, Make it a rant) that wrote a version: its lines, his marks carried
        if (doc && doc.unit !== cur.unit && !scripts.dirty(tid) && cur.rantId === doc.id) {
          const lines = S.carryMarks(cur.lines, doc.lines.map((l) => l.text), 'ai');
          const marks = S.toSave(lines).marks;
          scripts.open({ ...cur, unit: doc.unit, versions: doc.versions, saved: S.toSave(lines).lines.map((text, i) => ({ text, ...marks[i] })), lines });
          void rant.marks(tid, doc.unit, marks).catch(() => {});
        }
        return;
      }
      const local = await readDraft(tid);
      const use = S.pickOpen(local, doc?.id || null, doc?.unit || null, doc?.lines || null);
      const lines = use === 'local' && local ? S.fromDoc(local.lines) : S.fromDoc(doc?.lines || []);
      scripts.open({
        topicId: tid, topicText: pg.topic.text, rantId: use === 'local' && local ? local.rant : doc?.id || null,
        unit: doc?.unit || null, saved: doc?.lines || null, versions: doc?.versions || 0, lines, at: lines.length ? lines.length - 1 : null,
      });
    } catch (e: any) {
      if (!alive.current) return;
      setErr(e?.message || 'offline');
      if (!scripts.get(tid)) {
        // offline cold open: this phone's draft, or a blank page; both save once there is a signal
        const local = await readDraft(tid);
        const lines = S.fromDoc(local?.lines || []);
        scripts.open({ topicId: tid, topicText: '', rantId: local?.rant || null, unit: local?.base || null, saved: null, versions: 0, lines, at: null });
      }
    }
  }, [tid, freshRant]);
  useEffect(() => {
    // a fresh rant starts from nothing; any other open re-reads the page state only when there is none
    if (freshRant) scripts.forget(tid);
    const t = setTimeout(() => void load(false), 0);
    return () => clearTimeout(t);
  }, [load]); // eslint-disable-line react-hooks/exhaustive-deps
  useFocusEffect(useCallback(() => { if (scripts.get(tid)) void load(true); }, [load, tid]));

  // ---- saving: Back (leaving the page), a minute in the background
  const save = useCallback(async (quiet = false) => {
    const before = scripts.get(tid);
    try {
      const r = await scripts.save(tid);
      if (!r || quiet) return r;
      ui.snack(`Saved as v${r.v}`, async () => {
        await undo(r.indexes);
        scripts.forget(tid);
      });
      return r;
    } catch {
      if (before && !quiet) ui.snack('Saved on this phone. It saves to Punchable when you’re back online.');
      return null;
    }
  }, [tid, ui]);
  // Back: the page leaves the stack and unmounts (expo-router 58's navigation has no beforeRemove); a tab switch keeps
  // it mounted. Empty lines go, one version is written if anything changed, then the page state goes (the next open
  // and the tab's flush read the server and the local draft)
  const saveRef = useRef(save);
  useEffect(() => { saveRef.current = save; }, [save]);
  useEffect(() => () => {
    const cur = scripts.get(tid);
    if (cur) scripts.setLines(tid, S.dropEmpty(cur.lines));
    void saveRef.current().finally(() => { if (!alive.current && !scripts.dirty(tid)) scripts.forget(tid); });
  }, [tid]);
  // a minute in the background: iOS suspends the JS soon after, so the time is checked when he comes back (and a
  // save is tried at once on the way out, while the JS still runs: the local draft holds the words either way)
  useEffect(() => {
    let left = 0;
    const sub = AppState.addEventListener('change', (s) => {
      if (s === 'background') left = Date.now();
      if (s === 'active' && left && Date.now() - left >= BACKGROUND_SAVE_MS) void save(true);
      if (s === 'active') left = 0;
    });
    return () => sub.remove();
  }, [save]);

  // ---- the keyboard bar sits on the keyboard (its top measured in the window, so the header never offsets it)
  const root = useRef<any>(null);
  const [kb, setKb] = useState(0);
  useEffect(() => {
    const show = Keyboard.addListener(Platform.OS === 'ios' ? 'keyboardWillChangeFrame' : 'keyboardDidShow', (e) => {
      root.current?.measureInWindow((_x: number, y: number, _w: number, h: number) => setKb(Math.max(0, y + h - e.endCoordinates.screenY)));
    });
    const hide = Keyboard.addListener(Platform.OS === 'ios' ? 'keyboardWillHide' : 'keyboardDidHide', () => { setKb(0); setEditing(null); });
    return () => { show.remove(); hide.remove(); };
  }, []);

  const lines = st?.lines || [];
  const set = (next: S.SLine[], at?: number | null) => scripts.setLines(tid, next, at);
  const texts = lines.map((l) => l.text).filter((t) => t.trim());
  const hc = hookCheck(texts, lines.filter((l) => l.text.trim()).map((l) => l.by === 'him'));
  const rows = materialRows(page);
  const from = draftFrom(rows, !!page?.found_more);
  const topicText = st?.topicText || page?.topic?.text || '';

  // ---- line actions
  const startEdit = (i: number) => {
    const key = lines[i]?.key;
    const kept = S.dropEmpty(lines, key);
    const at = Math.max(0, kept.findIndex((l) => l.key === key));
    if (kept.length !== lines.length) set(kept, at);
    setSel(null); setEditing(at); scripts.setAt(tid, at); caret.current = kept[at]?.text.length || 0;
  };
  const stopEdit = () => {
    const kept = S.dropEmpty(lines);
    if (kept.length !== lines.length) set(kept, Math.min(scripts.get(tid)?.at ?? 0, kept.length - 1));
    setEditing(null);
    Keyboard.dismiss();
  };
  const cut = (i: number) => {
    const r = S.cutLine(lines, i);
    set(r.lines, Math.min(i, r.lines.length - 1));
    setSel(null);
    haptics.warning();
    ui.snack('Line cut', () => { const cur = scripts.get(tid); if (cur) set(r.undo(cur.lines)); });
  };
  const punch = (i: number) => { set(S.togglePunch(lines, i)); haptics.selection(); };
  const askClaude = (i: number) => {
    if (!online) return;
    scripts.setAt(tid, i);
    router.push({ pathname: '/rant-line', params: { topic: tid, key: lines[i]?.key || '' } } as any);
  };
  const lineMore = (i: number) => {
    const l = lines[i];
    actionSheet({ options: [
      ...(l?.from ? [{ label: 'Open the source', onPress: () => router.push(l.from!.startsWith('perf:') ? `/set/${enc(l.from!)}` as any : `/topic/${enc(tid)}/bit/${enc(l.from!)}` as any) }] : []),
      ...(i > 0 ? [{ label: 'Move up', onPress: () => { set(S.moveLine(lines, i, -1), i - 1); setSel(i - 1); } }] : []),
      ...(i < lines.length - 1 ? [{ label: 'Move down', onPress: () => { set(S.moveLine(lines, i, 1), i + 1); setSel(i + 1); } }] : []),
      { label: 'Cancel', cancel: true },
    ] });
  };
  const nextLine = () => { const r = S.addLine(lines, lines.length - 1); set(r.lines, r.focus); setSel(null); setEditing(r.focus); };
  const material = () => { if (editing != null) scripts.setAt(tid, editing); router.push(`/material/${enc(tid)}` as any); };

  // ---- Claude on the rant: Draft the rest (only adds), Make it a rant and Pick a hook (today's sessions, on the saved version)
  const draftRest = async () => {
    if (!online || drafting || !topicText) return;
    const sent = S.keepOf(lines);
    setDrafting({ waitingFor: null });
    try {
      const res = await pollJob<DraftResult>(await rant.draft(topicText, tid, sent.texts), () => alive.current, (w) => setDrafting({ waitingFor: w }));
      if (!alive.current) return;
      if (!res.ok) { ui.snack(`Claude didn’t answer: ${res.error}`); return; }
      const r = res.result;
      if (r.status !== 'ok') { ui.snack(r.why || 'Claude has nothing to add here.'); return; }
      const cur = scripts.get(tid);
      const merged = S.mergeDraft(cur?.lines || lines, sent.keys, r.lines || []);
      if (!merged || !merged.added) { ui.snack('Claude has nothing to add here.'); return; }
      set(merged.lines);
      haptics.success();
      ui.snack(`${merged.added} ${merged.added === 1 ? 'line' : 'lines'} added`, () => { const now = scripts.get(tid); if (now) set(S.removeKeys(now.lines, merged.keys)); });
    } catch (e: any) {
      ui.snack(`Couldn’t ask Claude: ${e?.message || 'no answer'}`);
    } finally {
      if (alive.current) setDrafting(null);
    }
  };
  const session = async (mode: 'rant' | 'hook') => {
    if (!online) return;
    const r = await save(true);
    const cur = scripts.get(tid);
    const id = r?.id || cur?.rantId;
    const unit = r?.unit || cur?.unit;
    if (!id || !unit) { ui.snack('Write a line first.'); return; }
    router.push({ pathname: '/session/[bit]', params: { bit: id, unit, cut: 'rant', n: String(cur?.versions || 1), title: topicText, mode } } as any);
  };
  const sparkle = () => actionSheet({ options: [
    { label: lines.length ? 'Draft the rest' : 'Draft it', onPress: () => void draftRest() },
    ...(texts.length ? [{ label: 'Make it a rant', onPress: () => void session('rant') }, { label: 'Pick a hook', onPress: () => void session('hook') }] : []),
    { label: 'Cancel', cancel: true },
  ] });
  const filmIt = async () => {
    const r = await save(true);
    const id = r?.id || scripts.get(tid)?.rantId;
    if (id) router.push(`/film/${enc(id)}` as any);
    else ui.snack('Film it reads the saved rant: it saves when you’re back online.');
  };
  const openVersions = () => { const id = st?.rantId; if (id) router.push(`/topic/${enc(tid)}/bit/${enc(id)}` as any); };
  const openInBits = () => { const id = st?.rantId; if (id) router.push(`/joke/${enc(id)}` as any); };

  // ---- one line being written
  const onChange = (i: number, text: string) => {
    // dictation or a paste may bring a newline: it is a new line, as Return is
    const nl = text.indexOf('\n');
    if (nl >= 0) {
      const r = S.splitAt(lines, i, text.slice(0, nl), text.slice(nl + 1).replace(/\n/g, ' '));
      set(r.lines, r.focus);
      setEditing(r.focus);
      caret.current = 0;
      return;
    }
    set(S.editLine(lines, i, text));
  };
  const onSubmit = (i: number) => {
    const t = lines[i]?.text || '';
    const at = Math.max(0, Math.min(t.length, caret.current));
    const r = S.splitAt(lines, i, t.slice(0, at), t.slice(at));
    set(r.lines, r.focus);
    setEditing(r.focus);
    caret.current = 0;
  };
  const onKey = (i: number, key: string) => {
    if (key !== 'Backspace') return;
    const r = S.joinUp(lines, i);
    if (!r) return;
    set(r.lines, r.focus);
    caret.current = r.lines[r.focus]?.text.length || 0;
    setEditing(r.focus);
  };

  const blank = !texts.length && editing == null;
  const offlineWord = !online ? 'Offline · saving on this phone' : null;
  const footer = editing != null ? (
    <View style={{ position: 'absolute', left: 0, right: 0, bottom: kb, zIndex: 8 }}>
      <KeyboardBar testID="script-kbar">
        <Btn small testID="kbar-material" label="Material" onPress={material} />
        <Btn small testID="kbar-claude" label={online ? 'Claude' : 'Claude (offline)'} disabled={!online} onPress={() => askClaude(editing)} />
        <Btn small testID="kbar-punch" label="Punch" onPress={() => punch(editing)} />
        <View style={{ flex: 1 }} />
        <Pressable testID="kbar-hide" accessibilityRole="button" accessibilityLabel="Hide the keyboard" onPress={stopEdit}
          style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}>
          <Icon name="chevronDown" size={22} color={C.text} />
        </Pressable>
      </KeyboardBar>
    </View>
  ) : null;

  return (
    <View ref={root} style={{ flex: 1 }} collapsable={false}>
      <Screen title={topicText} testID="script" backLabel={topicText || 'Topic'} offlineAt="none"
        right={native ? undefined : editing != null ? <Btn small label="Done" onPress={stopEdit} /> : <Btn small label="Claude" disabled={!online} onPress={sparkle} />}>
        {native && (editing != null ? (
          <Stack.Toolbar placement="right">
            <Stack.Toolbar.Button accessibilityLabel="Done" onPress={stopEdit}>Done</Stack.Toolbar.Button>
          </Stack.Toolbar>
        ) : (
          <Stack.Toolbar placement="right">
            <Stack.Toolbar.Menu icon="sparkles" accessibilityLabel="Ask Claude about the rant">
              <Stack.Toolbar.MenuAction icon="text.badge.plus" disabled={!online || !!drafting || (!from && !texts.length)}
                subtitle={!online ? 'offline' : drafting ? 'drafting…' : !from && !texts.length ? 'nothing to draw on' : `${from ? `From ${from.word}. ` : ''}Adds lines, leaves yours alone.`}
                onPress={() => void draftRest()}>{texts.length ? 'Draft the rest' : 'Draft it'}</Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="flame" disabled={!online || !texts.length} subtitle={!online ? 'offline' : !texts.length ? 'nothing to read' : 'Whole-line changes. Keep or skip each.'} onPress={() => void session('rant')}>Make it a rant</Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="quote.opening" disabled={!online || !texts.length} subtitle={!online ? 'offline' : !texts.length ? 'nothing to read' : 'Three openers. Pick one or none.'} onPress={() => void session('hook')}>Pick a hook</Stack.Toolbar.MenuAction>
            </Stack.Toolbar.Menu>
            <Stack.Toolbar.Menu icon="ellipsis" accessibilityLabel="More">
              <Stack.Toolbar.MenuAction icon="clock.arrow.circlepath" disabled={!st?.rantId} subtitle={st?.rantId ? undefined : 'after the first save'} onPress={openVersions}>Versions</Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="square.grid.2x2" disabled={!st?.rantId} subtitle={st?.rantId ? undefined : 'after the first save'} onPress={openInBits}>Open in Bits</Stack.Toolbar.MenuAction>
            </Stack.Toolbar.Menu>
          </Stack.Toolbar>
        ))}
        {native && editing == null && (
          // HIG/toolbars: Material leading, the one prominent action trailing; Film it is absent until there is a line (never grey)
          <Stack.Toolbar placement="bottom">
            <Stack.Toolbar.Button icon="square.stack.3d.up" accessibilityLabel="Material" onPress={material}>Material</Stack.Toolbar.Button>
            <Stack.Toolbar.Spacer />
            {texts.length > 0 && <Stack.Toolbar.Button variant="prominent" icon="video.fill" accessibilityLabel="Film it" onPress={() => void filmIt()}>Film it</Stack.Toolbar.Button>}
          </Stack.Toolbar>
        )}

        {offlineWord && <T v="small" testID="script-offline">{offlineWord}</T>}
        {!st && !err && <Skeleton lines={4} height={22} testID="script-skeleton" />}

        {hc && (
          <Pressable testID="script-hook" accessibilityRole="button" accessibilityHint={online ? 'Pick a hook' : 'Offline'} disabled={!online} onPress={() => void session('hook')}
            style={({ pressed }) => ({ flexDirection: 'row', alignItems: 'center', gap: 6, minHeight: 44, opacity: pressed ? 0.6 : 1 })}>
            <Text maxFontSizeMultiplier={ROW_CAP} style={{ flex: 1, fontFamily: F.semi, fontSize: 15, color: hc.late ? C.gapText : C.mut2 }}>{`${S.hookLine(hookWord(hc), texts)}${online ? '' : ' (offline)'}`}</Text>
            <Icon name="chevron" size={14} color={C.dim} />
          </Pressable>
        )}

        <View testID="script-lines">
          {lines.map((l, i) => {
            const prov = S.provOf(l);
            if (editing === i) {
              return (
                <View key={l.key} style={{ flexDirection: 'row', gap: 8, paddingVertical: 6, paddingLeft: 5, marginHorizontal: -8, paddingRight: 8, borderRadius: 10, borderLeftWidth: 3, borderLeftColor: C.amber, backgroundColor: C.card2 }}>
                  <View style={{ width: 14, alignItems: 'center', paddingTop: 12 }}>{l.punch && <View style={{ width: 7, height: 7, borderRadius: 4, backgroundColor: C.amber }} />}</View>
                  <TextInput testID={`script-edit-${i}`} value={l.text} autoFocus multiline submitBehavior="submit" returnKeyType="next" scrollEnabled={false}
                    onChangeText={(t) => onChange(i, t)} onSubmitEditing={() => onSubmit(i)} onKeyPress={(e) => onKey(i, e.nativeEvent.key)}
                    onSelectionChange={(e) => { caret.current = e.nativeEvent.selection.start; }}
                    placeholder={i === 0 ? 'First line' : 'Next line'} placeholderTextColor={C.dim} accessibilityLabel={`Line ${i + 1}`}
                    maxFontSizeMultiplier={PROSE_CAP} style={{ flex: 1, minHeight: 44, fontFamily: l.punch ? F.bold : F.body, fontSize: 20, lineHeight: 26, color: C.text, paddingVertical: 8 }} />
                </View>
              );
            }
            const on = sel === i;
            const acts = [
              { label: online ? 'Claude' : 'Claude (offline)', disabled: !online, onPress: () => askClaude(i) },
              { label: 'Punch', onPress: () => punch(i) },
              { label: 'Cut', onPress: () => cut(i) },
            ];
            return (
              <View key={l.key}>
                <Pressable testID={`script-line-${i}`} accessibilityRole="button" accessibilityLabel={S.lineA11y(l, i, lines.length)} accessibilityState={{ selected: on }}
                  accessibilityHint={on ? 'Writes here' : undefined}
                  accessibilityActions={[...acts.filter((a) => !a.disabled), { label: 'More' }].map((a) => ({ name: a.label, label: a.label }))}
                  onAccessibilityAction={(e) => { const n = e.nativeEvent.actionName; if (n === 'More') lineMore(i); else acts.find((a) => a.label === n)?.onPress(); }}
                  onPress={() => { if (on) startEdit(i); else { haptics.selection(); setSel(i); scripts.setAt(tid, i); } }}
                  style={{ flexDirection: 'row', alignItems: 'flex-start', gap: 8, minHeight: 44, paddingVertical: 8, paddingLeft: 5, paddingRight: 8, marginHorizontal: -8, borderRadius: 10, borderLeftWidth: 3, borderLeftColor: on ? C.amber : 'transparent', backgroundColor: on ? C.card2 : 'transparent' }}>
                  <View style={{ width: 14, alignItems: 'center', paddingTop: 10 }}>{l.punch && <View style={{ width: 7, height: 7, borderRadius: 4, backgroundColor: C.amber }} />}</View>
                  <View style={{ flex: 1, minWidth: 0, gap: 3 }}>
                    <Text maxFontSizeMultiplier={PROSE_CAP} style={{ fontFamily: l.punch ? F.bold : F.body, fontSize: 20, lineHeight: 26, color: l.text.trim() ? C.text : C.dim }}>{l.text.trim() ? l.text : 'Empty line'}</Text>
                    {prov.k === 'from' && <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>{S.provWords(prov)}</Text>}
                    {prov.k === 'ai' && <AIBadge style={{ alignSelf: 'flex-start' }} />}
                    {prov.k === 'yours' && <View style={{ alignSelf: 'flex-start' }}><Yours /></View>}
                  </View>
                </Pressable>
                {on && (
                  <View accessibilityLabel="Actions for this line" style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6, paddingLeft: 14, paddingVertical: 4 }}>
                    {acts.map((a) => <Btn key={a.label} small testID={`line-${a.label.split(' ')[0].toLowerCase()}`} label={a.label} disabled={a.disabled} onPress={a.onPress} />)}
                    <Btn small testID="line-more" label="⋯" accessibilityLabel="More for this line" onPress={() => lineMore(i)} />
                  </View>
                )}
              </View>
            );
          })}
        </View>

        {blank ? (
          <View style={{ gap: 14 }}>
            <Pressable testID="script-first" accessibilityRole="button" accessibilityLabel="First line" onPress={nextLine} style={{ minHeight: 44, justifyContent: 'center', paddingLeft: 14 }}>
              <Text maxFontSizeMultiplier={PROSE_CAP} style={{ fontFamily: F.body, fontSize: 20, color: C.dim }}>First line</Text>
            </Pressable>
            <View style={{ alignItems: 'flex-start' }}>
              <Btn testID="script-draft" icon={<Icon name="spark" size={16} color={online && from ? linkColor() : C.dim} />}
                label={!online ? 'Draft it (offline)' : from ? `Draft it from ${from.word}` : 'Draft it (nothing to draw on)'}
                disabled={!online || !from || !!drafting} onPress={() => void draftRest()} />
            </View>
          </View>
        ) : editing == null && (
          <Pressable testID="script-next" accessibilityRole="button" accessibilityLabel="Next line" onPress={nextLine} style={{ minHeight: 44, justifyContent: 'center', paddingLeft: 14 }}>
            <Text maxFontSizeMultiplier={PROSE_CAP} style={{ fontFamily: F.body, fontSize: 20, color: C.dim }}>Next line</Text>
          </Pressable>
        )}

        {drafting && (
          <View testID="script-drafting" style={{ gap: 4 }}>
            <T v="mut">Asking Claude…</T>
            <WorkerLine what="The draft" waitingFor={drafting.waitingFor} />
          </View>
        )}

        {!native && editing == null && (
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
            <Btn testID="script-material" label="Material" onPress={material} />
            <View style={{ flex: 1 }} />
            {texts.length > 0 && <Btn v="primary" testID="script-film" label="Film it" onPress={() => void filmIt()} />}
          </View>
        )}
        {/* room for the keyboard and its bar, so the line being written can scroll above them */}
        {/* the screen's KeyboardAvoidingView pads for the keyboard; this is the bar's room (device check 2 decides it) */}
        {editing != null && <View style={{ height: KEYBOARD_BAR_H + 24 }} />}
      </Screen>
      {footer}
    </View>
  );
}

