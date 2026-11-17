// A new draft of the bit (SPEC 2026-11-16 §3.2 and Slice 3 as built; owner 2026-11-17: "lets make it a whole new
// draft", and on marking up the AI's answer and sending again: "yes possibly"). Pushed from the bit page's round row
// and from Bits' "Waiting for you". One read: GET /api/jokes/<id>/redline (polled while the worker writes), and his
// bit as it stands (GET /api/jokes/<id>).
//   - While the worker writes: "Reading your marks…"; he can leave: the draft waits here and on Bits.
//   - A draft: his questions answered first (the AI badge on each answer, the words they are about), then the draft as
//     a document, its changed lines lightly tinted; Draft · Yours flips between it and his words now. Use this draft
//     makes it a new version (Undo in the snack; his old words stay in Versions); Discard closes the round (Undo too).
//     When he saved newer words after sending, the screen says so and Use asks first: never applied by itself.
//   - Round two: he selects words in the draft and marks them the same way (the bar at the bottom); Send makes the next
//     draft, which replaces this one; the Keeps of this round ride along (the server carries them).
//   - No draft (the checks refused it, or the job failed): why, and "Put the marks back" on his bit to try again.
import React, { useEffect, useMemo, useState } from 'react';
import { AccessibilityInfo, ActivityIndicator, Platform, Text, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { undo, useApi, useOnline } from '@/lib/api';
import { actionSheet } from '@/lib/actionSheet';
import { addVersion } from '@/lib/bitWork';
import { haptics } from '@/lib/haptics';
import { MarkBar, type BarAction } from '@/lib/MarkBar';
import { MarkedDoc } from '@/lib/MarkedText';
import { markId, peekMarks, sendAsked, setMarks, useMarks } from '@/lib/marks';
import { openTray } from '@/lib/MarkTray';
import { redline, useRound } from '@/lib/redline';
import { sendRound, takeElsewhere } from '@/lib/redlineActions';
import { shortNameOf } from '@/lib/say';
import { Screen } from '@/lib/Screen';
import { C, F, linkColor } from '@/lib/theme';
import { AIBadge, Btn, ErrorCard, Seg, Skeleton, T, TextBtn, useUI } from '@/lib/ui';
import * as R from '@/rules/bitLines';
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
  const mine = versions[R.homeIndex(versions)]?.text || '';
  const J = d.data?.joke;
  const shortName = shortNameOf(J?.title || '', J?.short_name);
  const round = rr.data?.round || null;
  const state = M.roundState(round);
  const draft = round?.result?.draft || '';
  const job = round?.job || null;
  const [view, setView] = useState<'draft' | 'mine'>('draft');
  const [sel, setSel] = useState<{ start: number; end: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState<{ word: string; undo?: () => void } | null>(null);
  const tray = useMarks(id, job);
  const tint = useMemo(() => M.changedLines(mine, draft), [mine, draft]);
  const kept = useMemo(() => M.keepsOn(draft, round), [draft, round]);
  const shown = useMemo(() => [...kept, ...tray.marks], [kept, tray.marks]);
  // on a draft he reads (no caret): only selected words can be marked
  const target = view === 'draft' && sel && sel.start !== sel.end ? M.targetOf(draft, sel) : null;
  // a mark made here says so in the bar for a few seconds, with Undo, and to VoiceOver (design review)
  useEffect(() => {
    if (!said) return;
    AccessibilityInfo.announceForAccessibilityWithOptions(said.word, { queue: true });
    const t = setTimeout(() => setSaid(null), 4000);
    return () => clearTimeout(t);
  }, [said]);
  // VoiceOver hears the round change: writing, then ready
  const lastState = React.useRef<M.RoundState | null>(null);
  useEffect(() => {
    if (!rr.data) return;
    if (lastState.current != null && lastState.current !== state && state !== 'none') {
      AccessibilityInfo.announceForAccessibilityWithOptions(M.roundLine(round) || '', { queue: true });
    }
    lastState.current = state;
  }, [state, round, rr.data]);

  const back = () => (router.canGoBack() ? router.back() : router.replace(`/joke/${enc(id)}` as any));

  const sending = React.useRef(false);
  const sendTwo = async () => {
    if (!job || sending.current) return;
    sending.current = true;
    setBusy(true);
    try {
      const ok = await sendRound(ui, id, { on: job }, draft, peekMarks(id, job));
      if (ok) { setSel(null); void rr.reload(); }
    } finally {
      sending.current = false;
      setBusy(false);
    }
  };
  // Send from the tray sheet of this draft
  useEffect(() => sendAsked.subscribe((a) => { if (a.bit === id && a.on && a.on === job) void sendTwo(); }), [id, job, draft, tray.marks]); // eslint-disable-line react-hooks/exhaustive-deps

  const act = (a: BarAction) => {
    if (!target || !job) return;
    if (a === 'elsewhere') return takeElsewhere(ui, id, shortName, target.text);
    const m: M.Mark = { id: markId(), type: a, text: target.text, at: target.at, note: '' };
    const r = M.addMark(draft, peekMarks(id, job), m);
    if (r.error) { haptics.warning(); return setSaid({ word: r.error }); }
    haptics.selection();
    void tray.replace(r.marks);
    if (a === 'keep') setSaid({ word: 'Kept', undo: () => { void tray.replace(peekMarks(id, job).filter((x) => x.id !== m.id)); setSaid({ word: 'Mark removed' }); } });
    else openTray(id, job, m.id);
  };

  const use = () => {
    if (!round || busy) return;
    const plan = M.planForUse(round);
    if (plan === 'none') return;
    const go = async () => {
      setBusy(true);
      try {
        const v = await addVersion(id, draft, `used a redline draft (round ${round.n})`, true, 'redline');
        await redline.resolve(id, round.job, 'used', v.unit).catch(() => {});
        await setMarks(id, round.job, []);
        haptics.success();
        ui.snack(`Saved as v${versions.length + 1}`, async () => {
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
        message: 'You saved the bit after sending these marks. Your newer words stay in Versions.',
        options: [{ label: 'Use this draft', onPress: () => void go() }, { label: 'Cancel', cancel: true }],
      });
    } else void go();
  };

  const discard = () => {
    if (!round || busy) return;
    const go = async () => {
      try {
        await redline.resolve(id, round.job, 'discarded');
        await setMarks(id, round.job, []);
        ui.snack('Draft discarded', async () => { await redline.resolve(id, round.job, 'open'); });
        back();
      } catch (e: any) {
        ui.snack(`Couldn’t discard: ${e.message}`);
      }
    };
    actionSheet({
      title: 'Discard this draft?', message: 'Your bit stays as it is.',
      options: [{ label: 'Discard draft', destructive: true, onPress: () => void go() }, { label: 'Keep it', cancel: true }],
    });
  };

  // the round's marks back on his bit as it is now, to send again (no draft came, or he saved newer words)
  const putBack = async () => {
    if (!round) return;
    const again = M.marksAgain(mine, round, () => markId());
    await setMarks(id, null, again.marks);
    // this round is closed (a draft, if there was one, is not lost: Undo opens it again)
    await redline.resolve(id, round.job, 'discarded').catch(() => {});
    const lost = again.lost ? ` ${M.markCount(again.lost)} on words you changed could not come back.` : '';
    ui.snack(`Your marks are back on the bit.${lost}`, async () => { await redline.resolve(id, round.job, 'open'); });
    back();
  };

  if (rr.rawError && !rr.data) return <Screen title="New draft" back><ErrorCard error={String(rr.rawError)} onRetry={rr.reload} /></Screen>;
  if (!rr.data || !d.data) return <Screen title="New draft" back><Skeleton lines={6} height={18} testID="draft-skeleton" /></Screen>;

  const canUse = online && !busy && state === 'ready';
  const nTwo = tray.marks.length;
  const answers = round?.result?.answers || [];
  const notes = (round?.result?.notes || []).filter((n) => n.was || n.why);

  let body: React.ReactNode;
  if (state === 'none') {
    body = <T v="mut" testID="draft-none">No draft is waiting on this bit.</T>;
  } else if (state === 'waiting') {
    body = (
      <View testID="draft-waiting" style={{ gap: 12, alignItems: 'flex-start' }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
          <ActivityIndicator color={C.amber} />
          <Text accessibilityLiveRegion="polite" maxFontSizeMultiplier={R.ROW_CAP} style={{ fontFamily: F.semi, fontSize: 17, color: C.text }}>Reading your marks…</Text>
        </View>
        <T v="mut">You can leave. The draft waits on this bit and on Bits.</T>
      </View>
    );
  } else if (state === 'refused' || state === 'failed') {
    body = (
      <View testID="draft-refused" style={{ gap: 12 }}>
        <Text maxFontSizeMultiplier={R.PROSE_CAP} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 24, color: C.text }}>
          {state === 'failed' ? `Couldn’t make a draft${round?.error ? `: ${round.error}` : '.'}` : round?.result?.why || 'No draft this time.'}
        </Text>
        <Btn testID="draft-putback" v="primary" label="Put the marks back on the bit" onPress={() => void putBack()} />
      </View>
    );
  } else {
    body = (
      <View style={{ gap: 18 }}>
        {round?.stale && (
          <View testID="draft-stale" style={{ gap: 6, padding: 12, borderRadius: 12, backgroundColor: C.card }}>
            <Text maxFontSizeMultiplier={R.PROSE_CAP} style={{ fontFamily: F.body, fontSize: 15, lineHeight: 21, color: C.text }}>
              You saved the bit after sending these marks. This draft was made from the earlier words.
            </Text>
            <TextBtn testID="draft-again" label="Put my marks on what I saved" color={linkColor()} onPress={() => void putBack()} />
          </View>
        )}
        {!!answers.length && (
          <View testID="draft-answers" style={{ gap: 12 }}>
            <T v="eb" accessibilityRole="header">Your questions</T>
            {answers.map((a) => (
              <View key={a.item} testID={`draft-answer-${a.item}`} style={{ gap: 4 }}>
                <Text maxFontSizeMultiplier={R.ROW_CAP} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>On “{(a.quote || '').slice(0, 80)}”: {a.question}</Text>
                <View style={{ flexDirection: 'row', gap: 6, alignItems: 'flex-start' }}>
                  <AIBadge style={{ marginTop: 3 }} />
                  <Text maxFontSizeMultiplier={R.PROSE_CAP} style={{ flex: 1, fontFamily: F.body, fontSize: 16, lineHeight: 23, color: C.text }}>{a.text}</Text>
                </View>
              </View>
            ))}
          </View>
        )}
        <Seg<'draft' | 'mine'> testID="draft-seg" items={[['draft', 'Draft'], ['mine', 'Yours']]} value={view} onChange={(v) => { haptics.selection(); setSel(null); setView(v); }} />
        {view === 'draft' ? (
          <View style={{ gap: 8 }}>
            <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
              <AIBadge />
              <Text maxFontSizeMultiplier={R.ROW_CAP} style={{ flex: 1, fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>
                Claude’s draft. Tinted lines are new or changed. Select words to mark them again.
              </Text>
            </View>
            <MarkedDoc testID="draft-doc" label="The new draft" text={draft} marks={shown} tint={tint} style={DOC} onSelection={setSel} />
          </View>
        ) : (
          <MarkedDoc testID="draft-mine" label="Your words now" text={mine} marks={[]} style={DOC} />
        )}
        {!!notes.length && view === 'draft' && (
          <View testID="draft-notes" style={{ gap: 6 }}>
            <T v="eb" accessibilityRole="header">What changed</T>
            {notes.map((n) => (
              <Text key={n.item} maxFontSizeMultiplier={R.ROW_CAP} style={{ fontFamily: F.body, fontSize: 14, lineHeight: 20, color: C.mut2 }}>
                {n.was ? `Your “${n.was.slice(0, 60)}”` : 'Changed'}{n.why ? `: ${n.why}` : ''}
              </Text>
            ))}
          </View>
        )}
      </View>
    );
  }

  const markBar = state === 'ready' && view === 'draft' && !!target
    ? <MarkBar testID="draft-markbar" target={target} count={nTwo} flash={said?.word} onUndo={said?.undo || null} onAct={act} onTray={() => job && openTray(id, job)} />
    : undefined;

  return (
    <Screen title="New draft" back testID="draftpage" maxWidth={760} bar={markBar}
      right={native ? undefined : state === 'ready' ? (
        <View style={{ flexDirection: 'row', gap: 6 }}>
          <Btn small v="ghost" testID="draft-discard" label="Discard" disabled={busy} onPress={discard} />
          {nTwo > 0 && <Btn small v="plain" testID="draft-send" label={M.sendLabel(nTwo)} disabled={!online || busy} onPress={() => void sendTwo()} />}
          <Btn small v="primary" testID="draft-use" label="Use this draft" disabled={!canUse} onPress={use} />
        </View>
      ) : undefined}>
      {native && state === 'ready' && !target && (nTwo > 0 ? (
        // round-two marks wait: Send is the one prominent action; Use and Discard in a menu beside it (HIG/toolbars)
        <Stack.Toolbar placement="bottom">
          <Stack.Toolbar.Menu accessibilityLabel="This draft" title="This draft" icon="ellipsis">
            <Stack.Toolbar.MenuAction icon="checkmark" disabled={!canUse} onPress={use}>Use this draft</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="trash" destructive disabled={busy} onPress={discard}>Discard</Stack.Toolbar.MenuAction>
          </Stack.Toolbar.Menu>
          <Stack.Toolbar.Spacer />
          <Stack.Toolbar.Button variant="prominent" accessibilityLabel={M.sendLabel(nTwo)} accessibilityHint="Claude makes the next draft from these marks" disabled={!online || busy} onPress={() => void sendTwo()}>{busy ? 'Sending…' : M.sendLabel(nTwo)}</Stack.Toolbar.Button>
        </Stack.Toolbar>
      ) : (
        // HIG/toolbars: the destructive choice leading, the one prominent action trailing
        <Stack.Toolbar placement="bottom">
          <Stack.Toolbar.Button accessibilityLabel="Discard this draft" disabled={busy} onPress={discard}>Discard</Stack.Toolbar.Button>
          <Stack.Toolbar.Spacer />
          <Stack.Toolbar.Button variant="prominent" accessibilityLabel="Use this draft" accessibilityHint={online ? 'It becomes the newest version; Undo is in the message' : 'Offline'} disabled={!canUse} onPress={use}>Use this draft</Stack.Toolbar.Button>
        </Stack.Toolbar>
      ))}
      {body}
    </Screen>
  );
}
