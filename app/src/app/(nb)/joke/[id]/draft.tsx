// The new draft (docs/design/SPEC-2026-11-17-one-notebook.md screen 4 and §5; owner 2026-11-17: "lets make it a whole
// new draft"). A sheet over the page: Keep mine leading, Use it trailing (HIG/sheets: Cancel and Done). One read:
// GET /api/jokes/<id>/redline (polled while the worker writes), and his page as it stands (GET /api/jokes/<id>).
//   - The whole draft as text, each changed range underlined on a tint (rules/draftRules: lines first, then words
//     inside the lines that differ), with a Mine / New draft switch.
//   - Tap a change: Put mine back (his words there again; tap it again: Use the new one), Try again (one more round on
//     just those words; the next draft replaces this one), and the other takes when he asked for 3 options.
//   - "Anything you didn't mark stays word for word" is kept here: a change on a line he put no mark on opens put
//     back, and the screen says how many.
//   - Use it saves the text he sees as a new version (Undo in the message; his words stay in Versions). Keep mine
//     closes the round (Undo too). Swiping the sheet down decides nothing: the draft waits.
//   - No draft (the checks refused it, or the job failed): why, and "Put the marks back on the page".
// His choices are kept on the phone (lib/putBacks), so the draft opens the way he left it.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { AccessibilityInfo, ActivityIndicator, Platform, Pressable, Text, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { undo, useApi, useOnline } from '@/lib/api';
import { actionSheet } from '@/lib/actionSheet';
import { addVersion } from '@/lib/bitWork';
import { BarGround } from '@/lib/BarGround';
import { haptics } from '@/lib/haptics';
import { markId, setMarks } from '@/lib/marks';
import { clearPutBacks, loadPutBacks, savePutBacks, type PutBacks } from '@/lib/putBacks';
import { redline, useRound } from '@/lib/redline';
import { sendRound } from '@/lib/redlineActions';
import { shortNameOf } from '@/lib/say';
import { Screen } from '@/lib/Screen';
import { C, F, linkColor } from '@/lib/theme';
import { AIBadge, Btn, ErrorCard, Seg, Skeleton, T, TextBtn, useUI } from '@/lib/ui';
import * as R from '@/rules/bitLines';
import * as D from '@/rules/draftRules';
import * as L from '@/rules/lineRules';
import * as M from '@/rules/markRules';

const enc = encodeURIComponent;
const native = Platform.OS !== 'web';
const DOC = { fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text, padding: 0, margin: 0 } as const;

export default function DraftPage() {
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const online = useOnline();
  const rr = useRound(id);
  const d = useApi<any>(`/api/jokes/${enc(id)}`);
  const versions = useMemo(() => R.versionsOf(d.data), [d.data]);
  const mine = L.docText(versions[R.homeIndex(versions)]?.text || '');
  const J = d.data?.joke;
  const title = (J?.title || '').split(' (')[0] || shortNameOf(J?.title || '', J?.short_name);
  const round = rr.data?.round || null;
  const state = M.roundState(round);
  const draft = round?.result?.draft || '';
  const job = round?.job || null;
  const [view, setView] = useState<'draft' | 'mine'>('draft');
  const [busy, setBusy] = useState(false);
  // the change he tapped (its buttons sit under its paragraph)
  const [open, setOpen] = useState<number | null>(null);
  // every changed range of the draft; the 3 options sit on the change that holds them
  const hs = useMemo(() => D.hunks(mine, draft, round?.result?.options), [mine, draft, round?.result?.options]);
  // which changes sit on a line he marked. A round on a draft (Try again) and a draft made from older words cannot be
  // judged from here, so nothing more is put back by itself there (what he put back before stays back)
  const marked = useMemo(() => D.onMarkedLines(mine, hs, round?.marks || [], !!round?.stale || !!round?.on), [mine, hs, round]);
  const [kept, setKept] = useState<PutBacks | null>(null);
  useEffect(() => { let on = true; void loadPutBacks(id).then((k) => { if (on) setKept(k); }); return () => { on = false; }; }, [id]);
  const [choices, setChoices] = useState<D.Choice[]>([]);
  const [auto, setAuto] = useState<boolean[]>([]);
  const keysOf = (cs: readonly D.Choice[], want: (c: D.Choice, i: number) => boolean) => hs.filter((_, i) => want(cs[i] ?? 0, i)).map((h) => D.hunkKey(mine, h));
  const remember = (cs: readonly D.Choice[], au: readonly boolean[]) => {
    const back = keysOf(cs, (c) => c === 'mine');
    const used = keysOf(cs, (c, i) => c !== 'mine' && !!au[i]);
    // the copy this screen reads from follows what is kept: the next draft (Try again) opens from what he chose in
    // this sitting, never from what the phone held when the sheet opened (iOS dev review)
    setKept({ job, back: new Set(back), used: new Set(used) });
    void savePutBacks(id, job, back, used);
  };
  // the draft opens the way he left it; a new draft keeps what he put back
  const opened = useRef('');
  useEffect(() => {
    if (!kept || state !== 'ready' || !job || !d.data) return;
    const sig = `${job}|${hs.length}|${mine.length}`;
    if (opened.current === sig) return;
    opened.current = sig;
    const o = D.initialChoices(mine, hs, marked, kept.back, kept.job === job ? kept.used : undefined);
    setChoices(o.choices);
    setAuto(o.auto);
    setOpen(null);
    remember(o.choices, o.auto);
  }, [kept, state, job, hs, mine, marked, d.data]); // eslint-disable-line react-hooks/exhaustive-deps
  const text = useMemo(() => D.compose(mine, draft, hs, choices), [mine, draft, hs, choices]);
  const paras = useMemo(() => D.paragraphs(mine, draft, hs, choices), [mine, draft, hs, choices]);
  const lines = D.changesLines(choices, auto);
  const choose = (i: number, c: D.Choice) => {
    haptics.selection();
    const next = hs.map((_, k) => (k === i ? c : choices[k] ?? 0));
    setChoices(next);
    remember(next, auto);
    AccessibilityInfo.announceForAccessibilityWithOptions(c === 'mine' ? 'Your words are back' : 'The new wording is in', { queue: true });
  };
  // VoiceOver hears the round change: writing, then ready (never the first read)
  const lastState = useRef<M.RoundState | null>(null);
  useEffect(() => {
    if (!rr.data) return;
    if (lastState.current != null && lastState.current !== state && state !== 'none') {
      AccessibilityInfo.announceForAccessibilityWithOptions(M.roundLine(round) || '', { queue: true });
    }
    lastState.current = state;
  }, [state, round, rr.data]);

  const back = () => (router.canGoBack() ? router.back() : router.replace(`/joke/${enc(id)}` as any));

  /** Try again: one more round on just those words of the draft; the next draft replaces this one. (Two taps at
   *  once are one round: the server answers the job already waiting.) */
  const tryAgain = async (i: number) => {
    const t = hs[i] ? D.againTarget(draft, hs[i]) : null;
    if (!t || !job || busy) return;
    setBusy(true);
    try {
      const ok = await sendRound(ui, id, { on: job }, draft, [{ id: markId(), type: 'change', text: t.text, at: t.at, note: '', intent: 'again' }]);
      if (ok) { setOpen(null); void rr.reload(); }
    } finally {
      setBusy(false);
    }
  };

  /** Keep mine: the round closes, his page is untouched (Undo brings the draft back). No question first: nothing is lost. */
  const keepMine = async (word = 'Kept yours') => {
    if (!round || busy) return;
    setBusy(true);
    try {
      await redline.resolve(id, round.job, 'discarded');
      await setMarks(id, round.job, []);
      ui.snack(word, async () => { await redline.resolve(id, round.job, 'open'); });
      back();
    } catch (e: any) {
      ui.snack(`Couldn’t close the draft: ${e.message}`);
    } finally {
      setBusy(false);
    }
  };

  const use = () => {
    if (!round || busy) return;
    const plan = M.planForUse(round);
    if (plan === 'none') return;
    // every change put back: there is nothing new to save, so this is Keep mine
    if (L.sameDoc(text, mine)) return void keepMine('Nothing changed: your page is as you wrote it');
    const go = async () => {
      setBusy(true);
      try {
        const v = await addVersion(id, text, `used a new draft (round ${round.n})`, true, 'redline');
        await redline.resolve(id, round.job, 'used', v.unit).catch(() => {});
        await setMarks(id, round.job, []);
        await clearPutBacks(id);
        haptics.success();
        ui.snack(`Saved as v${versions.length + 1}. Yours is in Versions`, async () => {
          await undo(v.indexes);
          await redline.resolve(id, round.job, 'open').catch(() => {});
        });
        back();
      } catch (e: any) {
        ui.snack(`Couldn’t save the draft: ${e.message}`);
      } finally {
        setBusy(false);
      }
    };
    if (plan === 'confirm') {
      actionSheet({
        title: 'Use this draft over your newer words?',
        message: 'You saved the page after asking for this draft. Your newer words stay in Versions.',
        options: [{ label: 'Use it', onPress: () => void go() }, { label: 'Cancel', cancel: true }],
      });
    } else void go();
  };

  // the round's marks back on his page as it is now, to ask again (no draft came, or he saved newer words)
  const putBack = async () => {
    if (!round) return;
    const again = M.marksAgain(mine, round, () => markId());
    await setMarks(id, null, again.marks);
    // this round is closed (a draft, if there was one, is not lost: Undo opens it again)
    await redline.resolve(id, round.job, 'discarded').catch(() => {});
    const lost = again.lost ? ` ${M.markCount(again.lost)} on words you changed could not come back.` : '';
    ui.snack(`Your marks are back on the page.${lost}`, async () => { await redline.resolve(id, round.job, 'open'); });
    back();
  };

  // a sheet always has a way out (HIG/modality), while it loads and when it failed too
  const closer = native ? (
    <Stack.Toolbar placement="left">
      <Stack.Toolbar.Button accessibilityLabel="Close" onPress={back}>Close</Stack.Toolbar.Button>
    </Stack.Toolbar>
  ) : null;
  if (rr.rawError && !rr.data) return <Screen title="New draft" back backLabel="Close">{closer}<ErrorCard error={String(rr.rawError)} onRetry={rr.reload} /></Screen>;
  if (!rr.data || !d.data) return <Screen title="New draft" back backLabel="Close">{closer}<Skeleton lines={6} height={18} testID="draft-skeleton" /></Screen>;

  const ready = state === 'ready';
  // Use it saves the text on screen: not before his choices (what is put back) are in place
  const canUse = online && !busy && ready && choices.length === hs.length;
  const answers = round?.result?.answers || [];

  /** The buttons of the tapped change, under its paragraph (44-pt pills; HIG/context-menus: never only in a menu). */
  const pills = (i: number) => {
    const h = hs[i];
    const c = choices[i] ?? 0;
    const pill = (key: string, label: string, onPress: () => void, o: { lit?: boolean; off?: boolean; hint?: string } = {}) => (
      <Pressable key={key} testID={`draft-${key}-${i}`} accessibilityRole="button" accessibilityLabel={label} accessibilityHint={o.hint} accessibilityState={{ selected: !!o.lit, disabled: !!o.off }} disabled={o.off} onPress={onPress}
        style={({ pressed }) => ({ minHeight: 44, paddingHorizontal: 14, borderRadius: 22, borderWidth: 1, borderColor: o.lit ? C.amber : C.borderStrong, backgroundColor: o.lit ? C.amberPill : pressed ? C.card2 : 'transparent', justifyContent: 'center', opacity: o.off ? 0.45 : 1 })}>
        <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: o.lit ? F.semi : F.body, fontSize: 15, color: C.text }}>{label}</Text>
      </Pressable>
    );
    return (
      <View testID={`draft-pills-${i}`} style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8, paddingTop: 8, paddingBottom: 6 }}>
        {c === 'mine'
          ? pill('usenew', 'Use the new one', () => choose(i, 0))
          : pill('back', 'Put mine back', () => choose(i, 'mine'))}
        {c !== 'mine' && !!D.againTarget(draft, h) && pill('again', busy ? 'Asking…' : 'Try again', () => void tryAgain(i), { off: !online || busy, hint: online ? 'Asks for another take on these words; the next draft replaces this one' : 'Offline' })}
        {c !== 'mine' && (h.options || []).map((_, k) => pill(`option${k}`, `Option ${k + 1}`, () => choose(i, k), { lit: c === k, hint: `Take ${k + 1} of ${h.options?.length}` }))}
      </View>
    );
  };

  let body: React.ReactNode;
  if (state === 'none') {
    body = <T v="mut" testID="draft-none">No draft is waiting on this page.</T>;
  } else if (state === 'waiting') {
    body = (
      <View testID="draft-waiting" style={{ gap: 12, alignItems: 'flex-start' }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
          <ActivityIndicator color={C.amber} />
          <Text maxFontSizeMultiplier={R.ROW_CAP} style={{ fontFamily: F.semi, fontSize: 17, color: C.text }}>Writing your new draft…</Text>
        </View>
        <T v="mut">You can close this. The draft waits on the page and in your notebook.</T>
      </View>
    );
  } else if (state === 'refused' || state === 'failed') {
    body = (
      <View testID="draft-refused" style={{ gap: 12 }}>
        <Text maxFontSizeMultiplier={R.PROSE_CAP} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 24, color: C.text }}>
          {state === 'failed' ? `Couldn’t make a draft${round?.error ? `: ${round.error}` : '.'}` : round?.result?.why || 'No draft this time.'}
        </Text>
        <Btn testID="draft-putback" v="primary" label="Put the marks back on the page" onPress={() => void putBack()} />
      </View>
    );
  } else {
    body = (
      <View style={{ gap: 14 }}>
        {/* HIG/segmented-controls: two views of one thing; the mockup's order */}
        <Seg<'mine' | 'draft'> testID="draft-seg" fit items={[['mine', 'Mine'], ['draft', 'New draft']]} value={view} onChange={(v) => { haptics.selection(); setOpen(null); setView(v); }} />
        {round?.stale && (
          <View testID="draft-stale" style={{ gap: 6, padding: 12, borderRadius: 12, backgroundColor: C.card }}>
            <Text maxFontSizeMultiplier={R.PROSE_CAP} style={{ fontFamily: F.body, fontSize: 15, lineHeight: 21, color: C.text }}>
              You saved the page after asking for this draft. It was made from the earlier words.
            </Text>
            <TextBtn testID="draft-again" label="Put my marks on what I saved" color={linkColor()} onPress={() => void putBack()} />
          </View>
        )}
        <Text testID="draft-title" accessibilityRole="header" maxFontSizeMultiplier={1.8} style={{ fontFamily: F.g7, fontSize: 28, lineHeight: 34, letterSpacing: -0.6, color: C.text }}>{title}</Text>
        {view === 'mine' ? (
          <Text testID="draft-mine" selectable maxFontSizeMultiplier={R.PROSE_CAP} style={DOC}>{mine}</Text>
        ) : (
          <View testID="draft-doc">
            <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, paddingBottom: 8 }}>
              <AIBadge />
              <Text maxFontSizeMultiplier={R.ROW_CAP} style={{ flex: 1, fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>Claude’s draft. Underlined words are new.</Text>
            </View>
            {paras.map((para, k) => {
              if (!para.runs.length) return <View key={k} style={{ height: 26 }} />;
              const changed = para.runs.some((r) => r.hunk != null);
              // VoiceOver: one element per paragraph, "Changed." first, each change's buttons as its actions (a span
              // inside a paragraph is not reliably reached: iOS dev review)
              const acts = changed ? para.ends.flatMap((i) => {
                const was = D.brief(mine.slice(hs[i].a0, hs[i].a1) || 'nothing here');
                return (choices[i] ?? 0) === 'mine'
                  ? [{ name: `usenew:${i}`, label: `Use the new one, in place of: ${was}` }]
                  : [{ name: `back:${i}`, label: `Put mine back: ${was}` }, ...(D.againTarget(draft, hs[i]) && online && !busy ? [{ name: `again:${i}`, label: 'Try again on this change' }] : []),
                    ...(hs[i].options || []).map((o, k) => ({ name: `option:${i}:${k}`, label: `Option ${k + 1}: ${D.brief(o, 60)}` }))];
              }) : undefined;
              // a change can be one short word: the whole paragraph is the target too (HIG/buttons: 44 pt), opening its
              // first change; a tap on a change itself picks that one
              const first = para.runs.find((r) => r.hunk != null)?.hunk ?? null;
              return (
                <Pressable key={k} accessible={false} disabled={first == null} hitSlop={{ top: 9, bottom: 9 }} onPress={() => { if (first != null) { haptics.selection(); setOpen(open === first ? null : first); } }}>
                  <Text testID={`draft-para-${k}`} maxFontSizeMultiplier={R.PROSE_CAP} style={DOC}
                    accessibilityLabel={changed ? `Changed. ${para.runs.map((r) => r.text).join('')}` : undefined}
                    accessibilityActions={acts} onAccessibilityAction={acts ? (e) => {
                      const [what, n, k] = e.nativeEvent.actionName.split(':');
                      const i = Number(n);
                      if (what === 'back') choose(i, 'mine'); else if (what === 'usenew') choose(i, 0); else if (what === 'again') void tryAgain(i); else if (what === 'option') choose(i, Number(k));
                    } : undefined}>
                    {para.runs.map((r, j) => (r.hunk == null ? r.text : (
                      // a change: underlined on a tint (the new wording), or dotted with no tint (his words, put back); never colour alone
                      <Text key={j} testID={`draft-change-${r.hunk}`} suppressHighlighting onPress={() => { haptics.selection(); setOpen(open === r.hunk ? null : r.hunk); }}
                        style={r.kind === 'new'
                          ? { backgroundColor: C.amberPill, color: C.text, fontFamily: open === r.hunk ? F.bold : F.body, textDecorationLine: 'underline', textDecorationStyle: open === r.hunk ? 'double' : 'solid', textDecorationColor: C.amber }
                          : { textDecorationLine: 'underline', textDecorationStyle: 'dotted', textDecorationColor: C.mut2, fontFamily: open === r.hunk ? F.bold : F.body, backgroundColor: open === r.hunk ? C.card : 'transparent' }}>{r.text}</Text>
                    )))}
                  </Text>
                  {open != null && para.ends.includes(open) && pills(open)}
                </Pressable>
              );
            })}
          </View>
        )}
        {!!answers.length && view === 'draft' && (
          // a round sent by an older build may carry answers to questions: still shown
          <View testID="draft-answers" style={{ gap: 12 }}>
            <T v="eb" accessibilityRole="header">Your questions</T>
            {answers.map((a) => (
              <View key={a.item} style={{ gap: 4 }}>
                <Text maxFontSizeMultiplier={R.ROW_CAP} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>On “{(a.quote || '').slice(0, 80)}”: {a.question}</Text>
                <View style={{ flexDirection: 'row', gap: 6, alignItems: 'flex-start' }}>
                  <AIBadge style={{ marginTop: 3 }} />
                  <Text maxFontSizeMultiplier={R.PROSE_CAP} style={{ flex: 1, fontFamily: F.body, fontSize: 16, lineHeight: 23, color: C.text }}>{a.text}</Text>
                </View>
              </View>
            ))}
          </View>
        )}
      </View>
    );
  }

  // the mockup's footer: what changed, and that nothing of his is lost
  const footer = ready ? (
    <BarGround testID="draft-footer">
      <View style={{ paddingHorizontal: 20, paddingTop: 10, paddingBottom: 6, gap: 4 }}>
        <Text testID="draft-count" maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 14, lineHeight: 19, color: C.text, textAlign: 'center' }}>{view === 'mine' ? 'This is your page as you wrote it.' : lines.main}</Text>
        {view === 'draft' && !!lines.also && <Text testID="draft-also" maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2, textAlign: 'center' }}>{lines.also}</Text>}
        <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2, textAlign: 'center' }}>Your draft is kept in Versions either way. You can keep typing in the new one.</Text>
      </View>
    </BarGround>
  ) : undefined;

  return (
    <Screen title="New draft" back backLabel="Close" testID="draftpage" maxWidth={760} bar={footer}
      right={native ? undefined : ready ? (
        <View style={{ flexDirection: 'row', gap: 6 }}>
          <Btn small v="ghost" testID="draft-keep" label="Keep mine" disabled={busy} onPress={() => void keepMine()} />
          <Btn small v="primary" testID="draft-use" label="Use it" disabled={!canUse} onPress={use} />
        </View>
      ) : undefined}>
      {native && (
        // a sheet's bar (HIG/sheets): the way out leading, the one key action trailing and prominent
        <Stack.Toolbar placement="left">
          {ready
            ? <Stack.Toolbar.Button accessibilityLabel="Keep mine" accessibilityHint="Closes the draft; your page stays as it is" disabled={busy} onPress={() => void keepMine()}>Keep mine</Stack.Toolbar.Button>
            : <Stack.Toolbar.Button accessibilityLabel="Close" onPress={back}>Close</Stack.Toolbar.Button>}
        </Stack.Toolbar>
      )}
      {native && ready && (
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button variant="prominent" accessibilityLabel="Use it" accessibilityHint={online ? 'Saves what you see as the newest version; yours stays in Versions' : 'Offline'} disabled={!canUse} onPress={use}>Use it</Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}
      {body}
    </Screen>
  );
}
