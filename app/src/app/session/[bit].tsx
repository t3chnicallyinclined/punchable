// The sessions sheet on a bit, Film it's two rows only (docs/design/SPEC-2026-11-03-film-it.md §2.5; 3c §3.6; the old
// spec's S2 sheet, S3 tracked changes and S7 rows). Opened from the bit page's sparkle, or from Film it's cut picker
// ("Make a rant cut…", which closes the camera first: one modal at a time). Phone: a native form sheet at the medium
// detent (large on drag), Close leading, the save trailing once a change is kept. Web: the RN Sheet.
//
//   root          Make it a rant · Pick a hook (each "offline" when there is no signal; nothing else is rendered:
//                 the old spec's other modes wait for 3c Slice 4)
//   asking        "Asking Claude…" and the worker line (offline / paused for today)
//   changes       one row per hunk: "Was" struck, "Now" under it, the AI badge; Keep · Skip (44 pt each); the trailing
//                 bar button "Save rant cut" (Make it a rant) or "Save version" (a hook) appears once a change is kept
//   openers       three rows, the AI badge, "Use this one"; footer "Show others" (once). Use this one = one tracked
//                 change, a new line before line 1, through the same Keep → Save path
//
// Claude never edits the bit: Save writes a NEW version (POST /api/jokes/<id>/versions, `draft: true` so the voice
// profile learns what he kept, the source naming the cut: rules/bitLines sourceFor) with Undo in the snack; the hunks
// he left skipped are sent as lines he passed over (the learning loop). The page reloads when the sheet goes.
import React, { useEffect, useRef, useState } from 'react';
import { Platform, Pressable, ScrollView, Text, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { get, undo, useOnline } from '@/lib/api';
import { addVersion, askSession } from '@/lib/bitWork';
import { simple } from '@/lib/simple';
import { haptics } from '@/lib/haptics';
import { WorkerLine } from '@/lib/worker';
import { C, F } from '@/lib/theme';
import { AIBadge, Btn, Sheet, SheetHead, T, useUI } from '@/lib/ui';
import { ROW_CAP, cutOf, sourceFor, type Cut } from '@/rules/bitLines';
import * as S from '@/rules/sessionRules';

type Mode = 'rant' | 'hook';
type Result = { status?: string; why?: string | null; lines?: string[]; hunks?: S.Hunk[]; hooks?: S.Opener[]; cut?: string; unit?: string };
type Phase =
  | { k: 'root' }
  | { k: 'asking'; mode: Mode; waitingFor?: string | null }
  | { k: 'failed'; mode: Mode; error: string }
  | { k: 'changes'; mode: Mode; base: string[]; hunks: S.Hunk[]; note?: string | null }
  | { k: 'openers'; base: string[]; openers: S.Opener[]; rounds: number; note?: string | null };

const POLL_MS = 2500;
const POLL_FOR_MS = 4 * 60 * 1000;   // Opus with thinking can take a while; past this the worker line says why
const ROWS: { mode: Mode; label: string; sub: string }[] = [
  { mode: 'rant', label: 'Make it a rant', sub: 'Claude turns it to face the camera and build; you keep what you like.' },
  { mode: 'hook', label: 'Pick a hook', sub: 'Three openers; pick one or none.' },
];

export default function SessionSheet() {
  const p = useLocalSearchParams<{ bit: string; unit?: string; cut?: string; n?: string; title?: string; mode?: string }>();
  const bit = decodeURIComponent(String(p.bit || ''));
  const unit = String(p.unit || '');
  const baseCut: Cut = cutOf(p.cut === 'rant' ? 'rant' : '');
  const count = Number(p.n) || 0;
  const title = String(p.title || 'Claude');
  const online = useOnline();
  const ui = useUI();
  const [phase, setPhase] = useState<Phase>({ k: 'root' });
  const [kept, setKept] = useState<S.Marks>([]);
  const [saving, setSaving] = useState(false);
  const alive = useRef(true);
  useEffect(() => () => { alive.current = false; }, []);
  const close = () => (router.canGoBack() ? router.back() : router.replace(`/joke/${encodeURIComponent(bit)}` as any));

  /** Ask, then poll the job until it finishes (the sheet stays usable; leaving it stops the polling, the job stays cached). */
  const ask = async (mode: Mode, exclude?: string[], rounds = 1) => {
    if (!unit || !online) return;
    setPhase({ k: 'asking', mode });
    setKept([]);
    try {
      let view = S.jobView<Result>(await askSession(bit, mode, unit, exclude));
      const until = Date.now() + POLL_FOR_MS;
      while (view.state === 'waiting' && alive.current) {
        setPhase({ k: 'asking', mode, waitingFor: view.waitingFor });
        if (!view.job || Date.now() > until) return;
        await new Promise((r) => setTimeout(r, POLL_MS));
        if (!alive.current) return;
        view = S.jobView<Result>(await get(`/api/jobs/${encodeURIComponent(view.job)}`).catch(() => ({ status: 'queued', id: view.state === 'waiting' ? view.job : null })));
      }
      if (!alive.current) return;
      if (view.state === 'failed') { setPhase({ k: 'failed', mode, error: view.error }); return; }
      if (view.state !== 'done') return;
      const r = view.result;
      const base = r.lines || [];
      if (mode === 'rant') setPhase({ k: 'changes', mode, base, hunks: r.hunks || [], note: S.emptyWord('rant', r.status, r.why) });
      else setPhase({ k: 'openers', base, openers: r.hooks || [], rounds, note: S.emptyWord('hook', r.status, r.why) });
    } catch (e: any) {
      if (alive.current) setPhase({ k: 'failed', mode, error: e?.message || 'no answer' });
    }
  };
  // opened to start one (Film it's "Make a rant cut…"): asked once, after the first frame
  useEffect(() => {
    const m = p.mode === 'rant' || p.mode === 'hook' ? p.mode : null;
    if (!m) return;
    const t = setTimeout(() => void ask(m), 0);
    return () => clearTimeout(t);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const mark = (i: number, on: boolean) => {
    haptics.selection();
    setKept((k) => { const n = k.slice(); n[i] = on; return n; });
  };
  const pickOpener = (o: S.Opener) => {
    if (phase.k !== 'openers') return;
    haptics.selection();
    setPhase({ k: 'changes', mode: 'hook', base: phase.base, hunks: [S.hookHunk(o.text)] });
    setKept([true]);
  };
  const changes = phase.k === 'changes' ? phase : null;
  const saveable = !!changes && S.canSave(changes.base, changes.hunks, kept);
  const saveLabel = changes?.mode === 'rant' ? 'Save rant cut' : 'Save version';
  const save = async () => {
    if (!changes || !saveable || saving) return;
    const cut: Cut = changes.mode === 'rant' ? 'rant' : baseCut;
    const lines = S.applyHunks(changes.base, changes.hunks, kept);
    setSaving(true);
    try {
      const r = await addVersion(bit, lines.join('\n'), changes.mode === 'rant' ? 'kept from Make it a rant' : 'a hook from Pick a hook', true, sourceFor(cut, changes.mode));
      // the lines he passed over teach the voice profile too (the learning loop); never a line he kept
      changes.hunks.forEach((h, i) => { if (!kept[i] && h.op !== 'cut' && h.now) simple.dismissDraft(h.now, bit); });
      haptics.success();
      ui.snack(S.savedWord(count + 1, cut), async () => { await undo(r.indexes); });
      close();
    } catch (e: any) {
      ui.snack(`Couldn’t save: ${e.message}`);
    } finally {
      if (alive.current) setSaving(false);
    }
  };

  const body = (
    <View style={{ gap: 12 }}>
      {phase.k === 'root' && (
        <View testID="session-root" style={{ gap: 2 }}>
          {ROWS.map((r) => (
            <Pressable key={r.mode} testID={`session-${r.mode}`} accessibilityRole="button" disabled={!online || !unit}
              accessibilityLabel={r.label} accessibilityHint={online ? r.sub : 'Offline'} accessibilityState={{ disabled: !online || !unit }}
              onPress={() => void ask(r.mode)}
              style={({ pressed }) => ({ minHeight: 60, paddingVertical: 10, gap: 2, justifyContent: 'center', borderBottomWidth: 1, borderBottomColor: C.hairline, opacity: !online ? 0.45 : pressed ? 0.7 : 1 })}>
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
                <Text maxFontSizeMultiplier={ROW_CAP} style={{ flex: 1, fontFamily: F.semi, fontSize: 17, lineHeight: 22, color: C.text }}>{r.label}</Text>
                {!online && <T v="small">offline</T>}
              </View>
              <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.body, fontSize: 14, lineHeight: 19, color: C.mut2 }}>{r.sub}</Text>
            </Pressable>
          ))}
        </View>
      )}
      {phase.k === 'asking' && (
        <View testID="session-asking" style={{ gap: 10 }}>
          <T v="mut" accessibilityRole={'status' as any}>Asking Claude…</T>
          <WorkerLine what={phase.mode === 'rant' ? 'The rant' : 'The hooks'} waitingFor={phase.waitingFor} />
        </View>
      )}
      {phase.k === 'failed' && (
        <View testID="session-failed" style={{ gap: 10 }}>
          <T v="mut">{`Claude didn't answer: ${phase.error}`}</T>
          <Btn small label="Try again" onPress={() => void ask(phase.mode)} />
        </View>
      )}
      {phase.k === 'openers' && (
        <View testID="session-openers" style={{ gap: 2 }}>
          {!!phase.note && <T v="mut">{phase.note}</T>}
          {phase.openers.map((o, i) => (
            <View key={`${phase.rounds}-${i}`} testID={`opener-${i}`} style={{ paddingVertical: 10, gap: 8, borderBottomWidth: 1, borderBottomColor: C.hairline }}>
              <View style={{ flexDirection: 'row', gap: 8, alignItems: 'flex-start' }}>
                <Text maxFontSizeMultiplier={2} style={{ flex: 1, fontFamily: F.body, fontSize: 17, lineHeight: 24, color: C.text }}>{o.text}</Text>
                <AIBadge style={{ marginTop: 3 }} />
              </View>
              <Btn small testID={`opener-use-${i}`} label="Use this one" accessibilityLabel={`Use this one: ${o.text}`} onPress={() => pickOpener(o)} style={{ alignSelf: 'flex-start' }} />
            </View>
          ))}
          {S.canShowOthers(phase.rounds) && (
            <Btn small v="ghost" testID="openers-more" label="Show others" disabled={!online}
              onPress={() => void ask('hook', phase.openers.map((o) => o.text), phase.rounds + 1)} style={{ alignSelf: 'flex-start', marginTop: 6 }} />
          )}
        </View>
      )}
      {changes && (
        <View testID="session-changes" style={{ gap: 2 }}>
          {!!changes.note && <T v="mut">{changes.note}</T>}
          {changes.hunks.map((h, i) => (
            <HunkRow key={i} h={h} i={i} n={changes.hunks.length} kept={kept[i]} onMark={(on) => mark(i, on)} />
          ))}
        </View>
      )}
    </View>
  );
  const saveBtn = saveable ? <Btn small v="primary" testID="session-save" label={saving ? 'Saving…' : saveLabel} disabled={saving} onPress={() => void save()} /> : null;

  if (Platform.OS === 'web') {
    return (
      <View style={{ flex: 1, backgroundColor: C.bg }}>
        <Sheet visible onClose={close} label={title} testID="session-sheet">
          <SheetHead title={title} onCancel={close} cancelLabel="Close" testID="session" />
          {body}
          {!!saveBtn && <View style={{ marginTop: 16 }}>{saveBtn}</View>}
        </Sheet>
      </View>
    );
  }
  return (
    <View testID="session-sheet" style={{ flex: 1, backgroundColor: C.surface }}>
      <Stack.Screen options={{ title }} />
      {/* HIG/sheets: Close leading (it keeps nothing open: a session lives until he leaves it); the save trailing,
          not rendered until a change is kept (3c §3.6: no greyed button with a sentence) */}
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button accessibilityLabel="Close" onPress={close}>Close</Stack.Toolbar.Button>
      </Stack.Toolbar>
      {saveable && (
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button variant="prominent" disabled={saving} accessibilityLabel={saveLabel} onPress={() => void save()}>{saveLabel}</Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ padding: 16, paddingBottom: 48 }}>
        {body}
      </ScrollView>
    </View>
  );
}

/** One tracked change (S3): the old line struck through above the new one, never coloured text; Keep · Skip always
 *  shown (44 pt each), the chosen one filled. VoiceOver: "Change 2 of 4. Removed: …. Added: …, kept", with Keep and Skip
 *  as actions. */
function HunkRow({ h, i, n, kept, onMark }: { h: S.Hunk; i: number; n: number; kept: boolean | undefined; onMark: (on: boolean) => void }) {
  const state = kept === true ? ', kept' : kept === false ? ', skipped' : '';
  return (
    <View testID={`hunk-${i}`} style={{ paddingVertical: 10, gap: 8, borderBottomWidth: 1, borderBottomColor: C.hairline }}
      accessible accessibilityLabel={S.hunkLabel(h, i, n) + state}
      accessibilityActions={[{ name: 'keep', label: 'Keep' }, { name: 'skip', label: 'Skip' }]}
      onAccessibilityAction={(e) => onMark(e.nativeEvent.actionName === 'keep')}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <T v="small" style={{ flex: 1 }}>{h.op === 'add' ? (h.line_no === 0 ? 'New first line' : `New line after ${h.line_no}`) : h.op === 'cut' ? `Cut line ${h.line_no}` : `Line ${h.line_no}`}</T>
        <AIBadge />
      </View>
      {!!h.was && (
        <View style={{ flexDirection: 'row', gap: 6 }}>
          <Text style={{ width: 14, fontFamily: F.g7, fontSize: 15, color: C.mut2 }}>−</Text>
          <Text maxFontSizeMultiplier={2} style={{ flex: 1, fontFamily: F.body, fontSize: 16, lineHeight: 22, color: C.mut2, textDecorationLine: 'line-through' }}>{h.was}</Text>
        </View>
      )}
      {!!h.now && (
        <View style={{ flexDirection: 'row', gap: 6 }}>
          <Text style={{ width: 14, fontFamily: F.g7, fontSize: 15, color: C.text }}>+</Text>
          <Text maxFontSizeMultiplier={2} style={{ flex: 1, fontFamily: F.body, fontSize: 17, lineHeight: 24, color: C.text, textDecorationLine: 'underline' }}>{h.now}</Text>
        </View>
      )}
      {!!h.why && <T v="small">{h.why}</T>}
      <View style={{ flexDirection: 'row', gap: 8 }} importantForAccessibility="no-hide-descendants" accessibilityElementsHidden>
        <Btn small testID={`hunk-keep-${i}`} label="Keep" v={kept === true ? 'primary' : 'plain'} onPress={() => onMark(true)} style={{ flex: 1 }} />
        <Btn small testID={`hunk-skip-${i}`} label="Skip" v={kept === false ? 'primary' : 'plain'} onPress={() => onMark(false)} style={{ flex: 1 }} />
      </View>
    </View>
  );
}
