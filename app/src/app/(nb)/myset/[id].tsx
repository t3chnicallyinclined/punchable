// One of his sets (docs/API_CONTRACT.md "Your sets"; engine creative/mysets.py), opened to work on: the bits in order
// with the run time against the set's length, what's working per bit in words (killed / solid / mixed / quiet /
// untested, from his tapes), every tough spot with what he can do about it (swap from his notebook, a tag from his own
// lines, a bridge draft labelled draft, move, cut, add), reorder by drag or ↑ ↓, lock a bit, the history of versions,
// and the last tape that lined up with this set. No ratios, no rates: three numbers on the first view.
import React, { useCallback, useState } from 'react';
import { Pressable, Text, TextInput, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { decide, undo, useApi } from '@/lib/api';
import { Bridge } from '@/lib/creative';
import { DragList, move } from '@/lib/DragList';
import { fmtShortDate, mmss } from '@/rules/format';
import { Action, LABEL_WORD, SetBit, SetView, Spot, Trim, labelColor, lengthWords, mysets } from '@/lib/mysets';
import { draft, quoteText } from '@/lib/simple';
import { C, F } from '@/lib/theme';
import { Btn, Card, Empty, ErrorCard, Icon, IconBtn, List, ListRow, Sheet, T, TextBtn, s, useUI, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const LENGTHS = [300, 420, 600, 900];
type Open = { kind: 'swap' | 'tag' | 'bridge'; spot: Spot; act: Action } | null;

export default function MySet() {
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const wide = useWide();
  const v = useApi<SetView>(`/api/mysets/${encodeURIComponent(id)}`);
  const [open, setOpen] = useState<Open>(null);
  const [picker, setPicker] = useState(false);
  const [trimS, setTrimS] = useState(false);
  const [lenS, setLenS] = useState(false);
  const [rename, setRename] = useState(false);
  const [hist, setHist] = useState(false);

  const apply = useCallback(async (fn: () => Promise<SetView>, msg: string, undoFn?: () => Promise<SetView>) => {
    try {
      const nv = await fn();
      if (nv?.set) v.setData(nv); else v.reload();
      ui.snack(msg, undoFn ? async () => { const r = await undoFn(); if (r?.set) v.setData(r); else v.reload(); } : undefined);
    } catch (e: any) {
      ui.snack(`Couldn't save: ${e.message}`);
    }
  }, [v, ui]);

  if (v.error && !v.data) return <Screen title="Set" back><ErrorCard error={String(v.error)} onRetry={v.reload} /></Screen>;
  if (!v.data) return <Screen title="Set" back><Empty text="Opening the set…" /></Screen>;
  const S = v.data;
  const ids = S.bits.map((b) => b.joke_id);
  const titleOf = (j: string) => S.bits.find((b) => b.joke_id === j)?.title || '';
  const setIds = (next: string[], msg: string) => {
    const before = ids.slice();
    apply(() => mysets.edit(id, { joke_ids: next }), msg, () => mysets.edit(id, { joke_ids: before, note: 'Undo' }));
  };
  const reorder = (from: number, to: number) => setIds(move(ids, from, to), `Moved ${S.bits[from]?.title} to #${to + 1}`);
  const remove = (j: string) => setIds(ids.filter((x) => x !== j), `Cut ${titleOf(j)}`);
  const lock = (b: SetBit) => apply(() => mysets.lock(id, b.joke_id, b.locked), b.locked ? `Unlocked ${b.title}` : `Locked ${b.title}: no more suggestions on it`,
    () => mysets.lock(id, b.joke_id, !b.locked));
  const act = (spot: Spot, a: Action) => {
    if (a.kind === 'swap' || a.kind === 'tag' || a.kind === 'bridge') return setOpen({ kind: a.kind, spot, act: a });
    if (a.kind === 'move') {
      const rest = ids.filter((x) => x !== a.joke_id);
      rest.splice(a.to, 0, a.joke_id);
      return setIds(rest, `Moved ${titleOf(a.joke_id)} to #${rest.indexOf(a.joke_id) + 1}`);
    }
    if (a.kind === 'cut') return remove(a.joke_id);
    if (a.kind === 'add') return setIds([...ids, a.joke_id], `Added ${a.card.title}`);
  };

  const statusColor = S.status === 'on' ? C.green : S.status === 'over' ? C.redSoft : C.mut;
  const toughColor = S.tough_n ? C.amber : C.green;
  const summary = (
    <View style={{ gap: 4 }} testID="set-summary">
      <T v="eb">Set{S.set.made_by === 'Punchable' ? ' · made by Punchable' : ''}</T>
      <T v="h1" testID="set-title">{S.set.title}</T>
      <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 8, flexWrap: 'wrap' }}>
        <T v="num" testID="set-runtime" style={{ fontSize: 22 }}>{S.runtime}</T>
        <Pressable testID="length-edit" accessibilityRole="button" accessibilityLabel={`A ${S.set.length_words} set. Change the length`} onPress={() => setLenS(true)} style={{ minHeight: 44, justifyContent: 'center' }}>
          <T v="mut" style={{ textDecorationLine: 'underline' }}>of {S.set.length_words}</T>
        </Pressable>
        <T v="semi" testID="set-status" style={{ color: statusColor, fontSize: 15 }}>{S.status_words}</T>
      </View>
      <T v="semi" testID="set-health" style={{ color: toughColor }}>{S.tough_n ? S.health : S.health === 'solid' ? 'Solid: no tough spots' : S.health}</T>
      {S.estimated_n > 0 && <T v="small">Times marked ~ are from the written words (not on tape yet).</T>}
    </View>
  );

  const setSpots = !!S.tough.length && (
    <Card testID="set-spots" style={{ gap: 8 }}>
      <T v="eb">The set as a whole</T>
      {S.tough.map((sp, i) => <SpotRow key={`${sp.kind}-${i}`} spot={sp} testID={`set-spot-${sp.kind}`} onAct={act} />)}
    </Card>
  );

  const list = (
    <View testID="set-bits">
      {!S.bits.length && <Empty text="No bits yet. Find one to start." />}
      <DragList testID="set-bit-list" items={S.bits} keyOf={(b) => b.joke_id} onReorder={reorder}
        render={(b: SetBit, i, handle) => {
          const t = b.transition_in;
          const spots = b.tough.filter((x) => x.kind !== 'transition');
          const tspot = b.tough.find((x) => x.kind === 'transition');
          return (
            <View>
              {t && (
                <View testID={`set-transition-${i}`} style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 32, paddingLeft: 18 }}>
                  <View style={{ width: 3, alignSelf: 'stretch', backgroundColor: t.weak ? C.redSoft : t.kind === 'proven' ? C.green : C.amber, borderRadius: 2 }} />
                  <Text maxFontSizeMultiplier={1.4} numberOfLines={1} style={{ flex: 1, fontFamily: F.body, fontSize: 13, color: t.weak ? C.redSoft : t.kind === 'proven' ? C.green : C.amber }}>{t.weak ? '○ ' : '● '}{t.words}</Text>
                </View>
              )}
              {tspot && <View style={{ paddingLeft: 18, paddingBottom: 4 }}><SpotActions spot={tspot} onAct={act} testID={`spot-actions-transition-${i}`} /></View>}
              <View testID={`set-bit-${i}`} style={[s.card, { padding: 10, gap: 6, borderColor: b.locked ? C.addBorder : C.border }]}>
                <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
                  {handle}
                  <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
                    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
                      <T v="num" style={{ fontSize: 14, color: C.mut }}>{i + 1}</T>
                      <Pressable style={{ flexShrink: 1 }} onPress={() => router.push(`/joke/${encodeURIComponent(b.joke_id)}` as any)} accessibilityRole="link">
                        <T v="semi" numberOfLines={1}>{b.title}</T>
                      </Pressable>
                      <T v="small">{b.estimated ? `~${mmss(b.duration_s)}` : mmss(b.duration_s)}</T>
                    </View>
                    <LabelLine testID={`label-line-${i}`} label={b.label} words={b.label_words} />
                  </View>
                  <View>
                    <IconBtn testID={`up-${i}`} label={`Move ${b.title} up`} disabled={i === 0} onPress={() => reorder(i, i - 1)}><Icon name="up" size={18} color={C.mut} /></IconBtn>
                    <IconBtn testID={`down-${i}`} label={`Move ${b.title} down`} disabled={i === S.bits.length - 1} onPress={() => reorder(i, i + 1)}><Icon name="down" size={18} color={C.mut} /></IconBtn>
                  </View>
                  <View>
                    <IconBtn testID={`lock-${i}`} label={b.locked ? `Unlock ${b.title}` : `Lock ${b.title}`} onPress={() => lock(b)}><Icon name={b.locked ? 'lock' : 'unlock'} size={18} color={b.locked ? C.amber : C.mut} /></IconBtn>
                    <IconBtn testID={`remove-${i}`} label={`Cut ${b.title}`} onPress={() => remove(b.joke_id)}><Icon name="trash" size={18} color={C.mut} /></IconBtn>
                  </View>
                </View>
                {b.locked && <T v="small" testID={`locked-${i}`} style={{ color: C.amber }}>Locked: it’s done. No suggestions on it.</T>}
                {spots.map((sp, k) => <SpotRow key={`${sp.kind}-${k}`} spot={sp} testID={`spot-${i}-${sp.kind}`} onAct={act} />)}
              </View>
            </View>
          );
        }} />
      <Pressable testID="set-add" accessibilityRole="button" onPress={() => setPicker(true)}
        style={{ marginTop: 10, minHeight: 56, borderWidth: 1.5, borderStyle: 'dashed', borderColor: C.borderStrong, borderRadius: 12, alignItems: 'center', justifyContent: 'center' }}>
        <T v="mut">Find a bit to add</T>
      </Pressable>
    </View>
  );

  const side = (
    <View style={{ gap: 12 }}>
      <Card testID="set-tapes" style={{ gap: 6 }}>
        <T v="eb">On tape</T>
        <T v="body">{S.tapes.words}</T>
        {S.tapes.n > 1 && <T v="small">{S.tapes.n} recordings line up with this set.</T>}
        <TextBtn testID="open-nights" label="Nights & punch-ups ›" onPress={() => router.push(`/workingset/${encodeURIComponent(id)}` as any)} />
      </Card>
      {!!S.next.length && (
        <Card testID="set-next" style={{ gap: 4 }}>
          <T v="eb">Could come next · from your notebook</T>
          {S.next.slice(0, wide ? 3 : 2).map((x, i) => (
            <View key={x.joke_id} style={{ flexDirection: 'row', alignItems: 'center', gap: 10, paddingVertical: 6, borderTopWidth: i ? 1 : 0, borderTopColor: C.border }}>
              <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
                <T v="semi" numberOfLines={1}>{x.title}</T>
                <T v="small" numberOfLines={1} style={{ color: labelColor(x.label) }}>{x.label_words}</T>
              </View>
              <Btn small v="add" testID={`next-add-${i}`} label="＋ Add" onPress={() => setIds([...ids, x.joke_id], `Added ${x.title}`)} />
            </View>
          ))}
        </Card>
      )}
      <Card testID="set-history" style={{ gap: 4 }}>
        <Pressable testID="history-toggle" accessibilityRole="button" onPress={() => setHist(!hist)} style={{ minHeight: 44, flexDirection: 'row', alignItems: 'center' }}>
          <T v="eb" style={{ flex: 1 }}>History · version {S.set.version}</T>
          <T v="small">{hist ? 'Hide' : 'Show'}</T>
        </Pressable>
        {(hist ? S.versions : S.versions.slice(0, 1)).map((x) => (
          <View key={x.v} testID={`version-${x.v}`} style={{ paddingVertical: 6, borderTopWidth: 1, borderTopColor: C.border, gap: 2 }}>
            <T v="semi">v{x.v} · {fmtShortDate(x.created_at)}{x.by === 'Punchable' ? ' · made by Punchable' : ''}</T>
            <T v="small">{x.note}</T>
            {hist && <T v="small" numberOfLines={2} style={{ color: C.dim }}>{x.titles.join(' · ')}</T>}
          </View>
        ))}
      </Card>
    </View>
  );

  return (
    <Screen title="Set" back testID="myset" right={<IconBtn label="Rename" onPress={() => setRename(true)}><Icon name="notes" size={20} /></IconBtn>}>
      {summary}
      <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap' }}>
        <Btn testID="trim-open" small v="add" label="Make a shorter set" onPress={() => setTrimS(true)} disabled={S.bits.length < 3} />
        {!!S.set.storyboard_id && <Btn small label="The board" onPress={() => router.push(`/board/${encodeURIComponent(S.set.storyboard_id!)}` as any)} />}
      </View>
      {setSpots}
      {wide ? (
        <View style={{ flexDirection: 'row', gap: 20, alignItems: 'flex-start' }}>
          <View style={{ flex: 3, minWidth: 0 }}>{list}</View>
          <View style={{ flex: 2, minWidth: 0 }}>{side}</View>
        </View>
      ) : (<>{list}{side}</>)}

      <SwapSheet open={open?.kind === 'swap' ? open : null} onClose={() => setOpen(null)} onSwap={(a) => { setOpen(null); setIds(ids.map((x) => (x === a.joke_id ? a.with.joke_id : x)), `Swapped in ${a.with.title}`); }} />
      <TagSheet open={open?.kind === 'tag' ? open : null} has={(j) => ids.includes(j)} onClose={() => setOpen(null)}
        onAdd={(a) => { setOpen(null); const at = ids.indexOf(a.joke_id) + 1; const n = ids.slice(); n.splice(at, 0, a.tag.joke_id); setIds(n, `Added ${a.tag.title} after ${titleOf(a.joke_id)}`); }} />
      <BridgeSheet key={open?.kind === 'bridge' ? `${(open.act as any).from}>${(open.act as any).to}` : 'none'} open={open?.kind === 'bridge' ? open : null} setId={id} titleOf={titleOf} onClose={() => setOpen(null)} />
      <BitPicker key={picker ? 'open' : 'closed'} visible={picker} onClose={() => setPicker(false)} exclude={ids} onPick={(j) => { setPicker(false); setIds([...ids, j.id], `Added ${j.title.split(' (')[0]}`); }} />
      <TrimSheet visible={trimS} onClose={() => setTrimS(false)} set={S} />
      <Sheet visible={lenS} onClose={() => setLenS(false)} label="How long is the set?" testID="length-sheet">
        <T v="h2" style={{ marginBottom: 10 }}>How long is the set?</T>
        <View style={{ flexDirection: 'row', gap: 6, flexWrap: 'wrap' }}>
          {[...LENGTHS, 1200, 1800].map((t) => <Btn key={t} testID={`length-${t}`} small v={S.set.target_s === t ? 'primary' : 'plain'} label={lengthWords(t)}
            onPress={() => { setLenS(false); const old = S.set.target_s; apply(() => mysets.edit(id, { target_s: t }), `A ${lengthWords(t)} set now`, () => mysets.edit(id, { target_s: old, note: 'Undo' })); }} />)}
        </View>
      </Sheet>
      <Sheet visible={rename} onClose={() => setRename(false)} label="Rename" testID="rename-sheet">
        <RenameForm title={S.set.title} onSave={(t) => { setRename(false); const old = S.set.title; apply(() => mysets.edit(id, { title: t }), `Renamed to ${t}`, () => mysets.edit(id, { title: old })); }}
          onDelete={async () => { setRename(false); try { await mysets.edit(id, { delete: true }); ui.snack(`Deleted ${S.set.title}`); router.replace('/boards' as any); } catch (e: any) { ui.snack(`Couldn't delete: ${e.message}`); } }} />
      </Sheet>
    </Screen>
  );
}

/** What's working, in one line: the word in its colour, then the rest ("Killed every night · 7 nights",
 *  "Mixed: killed at Chuckle hut '23, quiet at Mabel's '24", "Solid · Worked 8 of 9 nights, killed 4"). */
function LabelLine({ label, words, testID }: { label: SetBit['label']; words: string; testID?: string }) {
  const w = LABEL_WORD[label];
  const same = words.toLowerCase().startsWith(w.toLowerCase());
  const head = same ? words.slice(0, w.length) : w;
  const rest = same ? words.slice(w.length) : ` · ${words}`;
  return (
    <Text testID={testID} numberOfLines={2} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut }}>
      <Text style={{ fontFamily: F.semi, color: labelColor(label) }}>{head}</Text>{rest}
    </Text>
  );
}

/** One tough spot: what's wrong in words, then what he can do. */
function SpotRow({ spot, onAct, testID }: { spot: Spot; onAct: (s: Spot, a: Action) => void; testID?: string }) {
  return (
    <View testID={testID} style={{ gap: 4, paddingTop: 6, borderTopWidth: 1, borderTopColor: C.border }}>
      <View style={{ flexDirection: 'row', gap: 6 }}>
        <Text style={{ fontFamily: F.semi, fontSize: 13, color: C.amber }}>▲</Text>
        <T v="small" style={{ flex: 1, color: C.text2 }}>{spot.words}</T>
      </View>
      <SpotActions spot={spot} onAct={onAct} />
    </View>
  );
}

function SpotActions({ spot, onAct, testID }: { spot: Spot; onAct: (s: Spot, a: Action) => void; testID?: string }) {
  if (!spot.actions.length) return null;
  return (
    <View testID={testID} style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
      {spot.actions.map((a, k) => (
        <Btn key={k} small v={a.kind === 'bridge' ? 'ghost' : 'plain'} testID={`act-${a.kind}`} label={a.kind === 'bridge' ? 'Bridge idea (draft)' : a.words}
          accessibilityLabel={a.words} onPress={() => onAct(spot, a)} />
      ))}
    </View>
  );
}

function SwapSheet({ open, onClose, onSwap }: { open: Open; onClose: () => void; onSwap: (a: Extract<Action, { kind: 'swap' }>) => void }) {
  const a = open?.act as Extract<Action, { kind: 'swap' }> | undefined;
  return (
    <Sheet visible={!!a} onClose={onClose} label="Swap" testID="swap-sheet">
      {a && <>
        <T v="eb">Swap in, from your notebook</T>
        <T v="h2">{a.with.title}</T>
        {!!a.with.quote && <T v="body" style={{ color: C.amber, marginVertical: 6 }}>“{quoteText(a.with)}”</T>}
        <T v="small" style={{ color: labelColor(a.with.label) }}>{a.with.label_words}</T>
        {!!a.with.why && <T v="small">{a.with.why}</T>}
        <View style={{ flexDirection: 'row', gap: 8, marginTop: 12 }}>
          <Btn testID="swap-go" label="Swap it in" v="primary" style={{ flex: 1 }} onPress={() => onSwap(a)} />
          <Btn label="Open the bit" style={{ flex: 1 }} onPress={() => { onClose(); router.push(`/joke/${encodeURIComponent(a.with.joke_id)}` as any); }} />
        </View>
      </>}
    </Sheet>
  );
}

function TagSheet({ open, onClose, onAdd, has }: { open: Open; onClose: () => void; onAdd: (a: Extract<Action, { kind: 'tag' }>) => void; has: (j: string) => boolean }) {
  const a = open?.act as Extract<Action, { kind: 'tag' }> | undefined;
  return (
    <Sheet visible={!!a} onClose={onClose} label="A tag from your notebook" testID="tag-sheet">
      {a && <>
        <T v="eb">A line you wrote that could tag it</T>
        <T v="h2">{a.tag.title}</T>
        {!!a.tag.quote && <T v="body" testID="tag-quote" style={{ color: C.amber, marginVertical: 6 }}>“{quoteText(a.tag)}”</T>}
        {!!a.tag.date && <T v="small">Your notebook · {fmtShortDate(a.tag.date)}</T>}
        <View style={{ flexDirection: 'row', gap: 8, marginTop: 12 }}>
          <Btn testID="tag-add" label={has(a.tag.joke_id) ? 'Already in the set' : 'Put it right after'} v="primary" disabled={has(a.tag.joke_id)} style={{ flex: 1 }} onPress={() => onAdd(a)} />
          <Btn label="Open it" style={{ flex: 1 }} onPress={() => { onClose(); router.push(`/joke/${encodeURIComponent(a.tag.joke_id)}` as any); }} />
        </View>
      </>}
    </Sheet>
  );
}

/** A weak transition: an optional AI bridge draft (cached, or queued for the writer on the server), and his own line. */
function BridgeSheet({ open, setId, titleOf, onClose }: { open: Open; setId: string; titleOf: (j: string) => string; onClose: () => void }) {
  const ui = useUI();
  const [state, setState] = useState<Bridge | null>(null);
  const [mine, setMine] = useState('');
  const [busy, setBusy] = useState(false);
  const a = open?.act as Extract<Action, { kind: 'bridge' }> | undefined;
  if (!a) return <Sheet visible={false} onClose={onClose} label="Bridge">{null}</Sheet>;
  const ask = () => {
    setBusy(true);
    draft<Bridge>(() => mysets.bridge(setId, a.from, a.to))
      .then(setState)
      .catch((e) => setState({ from: a.from, to: a.to, draft: true, label: '', status: 'provider_error', text: e.message }))
      .finally(() => setBusy(false));
  };
  const save = async (text: string) => {
    onClose();
    try {
      const w = await decide({ target: 'edge', action: 'add', after: { src: a.from, dst: a.to, kind: 'segue', evidence: [], bridge: text }, reason: `bridge written in a set: ${text.slice(0, 160)}` });
      ui.snack('Saved as your segue', async () => { await undo(w.indexes); });
    } catch (e: any) {
      ui.snack(`Couldn't save: ${e.message}`);
    }
  };
  return (
    <Sheet visible onClose={onClose} label="Bridge" testID="set-bridge-sheet">
      <T v="eb">Between</T>
      <T v="h2">{titleOf(a.from)} → {titleOf(a.to)}</T>
      <T v="mut" style={{ marginVertical: 6 }}>{open!.spot.words}</T>
      {!state && !busy && <Btn testID="set-bridge-ask" small v="add" label="Get a bridge idea (draft)" onPress={ask} style={{ alignSelf: 'flex-start' }} />}
      {busy && <T v="mut">Asking for a bridge…</T>}
      {state?.status === 'queued' && <View testID="set-bridge-queued" style={[s.inner, { gap: 4 }]}><T v="semi">Queued</T><T v="mut">The writer drafts bridges. Come back to it in a minute.</T></View>}
      {state?.status === 'no_grounding' && <T v="mut">Nothing links these two to build a bridge from. Write your own, or move one.</T>}
      {state?.status === 'provider_error' && <T v="mut">The AI didn’t answer. Write your own below.</T>}
      {state?.status === 'ok' && state.text && (
        <View testID="set-bridge-draft" style={[s.draft, { gap: 8, marginVertical: 6 }]}>
          <View style={{ flexDirection: 'row', gap: 6, alignItems: 'center' }}>
            <View style={{ paddingHorizontal: 6, minHeight: 20, borderRadius: 4, backgroundColor: C.addBg, justifyContent: 'center' }}><Text style={{ fontFamily: F.g7, fontSize: 11, color: C.amber }}>Draft</Text></View>
            <T v="small" style={{ flex: 1 }}>not your words yet</T>
          </View>
          <Text style={{ fontFamily: F.body, fontSize: 16, lineHeight: 23, color: C.amber }}>{state.text}</Text>
          <View style={{ flexDirection: 'row', gap: 8 }}>
            <Btn testID="set-bridge-use" small label="Use as mine" onPress={() => save(state.text!)} />
            <Btn small v="ghost" label="Edit" onPress={() => setMine(state.text!)} />
          </View>
        </View>
      )}
      <T v="small" style={{ marginTop: 8 }}>Your own bridge</T>
      <TextInput testID="set-bridge-mine" value={mine} onChangeText={setMine} multiline placeholder="Speaking of…" placeholderTextColor={C.dim}
        style={{ minHeight: 72, borderWidth: 1, borderColor: C.border, borderRadius: 8, padding: 12, color: C.text, fontFamily: F.body, fontSize: 16, textAlignVertical: 'top' }} />
      <Btn label="Save my bridge" v="primary" disabled={!mine.trim()} onPress={() => save(mine.trim())} style={{ marginTop: 8 }} />
    </Sheet>
  );
}

/** "Make a 5 from my 10": pick a length, see what stays and what goes, then make it a new set (this one is kept). */
function TrimSheet({ visible, onClose, set }: { visible: boolean; onClose: () => void; set: SetView }) {
  const ui = useUI();
  const [t, setT] = useState<number | null>(null);
  const prev = useApi<Trim>(visible && t ? `/api/mysets/${encodeURIComponent(set.set.id)}/trim?target_s=${t}` : null, { cache: false });
  const lengths = LENGTHS.filter((x) => x < set.set.target_s);
  const make = async () => {
    if (!prev.data) return;
    onClose();
    try {
      const v = await mysets.fromSet(set.set.id, prev.data.target_s, prev.data.keep.map((k) => k.joke_id));
      ui.snack(`Made ${v.set.title}`);
      router.push(`/myset/${encodeURIComponent(v.set.id)}` as any);
    } catch (e: any) {
      ui.snack(`Couldn't make it: ${e.message}`);
    }
  };
  return (
    <Sheet visible={visible} onClose={onClose} label="Make a shorter set" testID="trim-sheet">
      <T v="h2">Make a shorter set</T>
      <T v="mut" style={{ marginVertical: 6 }}>Keeps your opener and your closer, then the bits that do best on stage. {set.set.title} stays as it is.</T>
      <View testID="trim-lengths" style={{ flexDirection: 'row', gap: 6, flexWrap: 'wrap' }}>
        {(lengths.length ? lengths : LENGTHS).map((x) => <Btn key={x} testID={`trim-${x}`} small v={t === x ? 'primary' : 'plain'} label={`A ${lengthWords(x)}`} onPress={() => setT(x)} />)}
      </View>
      {prev.data && (
        <View testID="trim-preview" style={{ gap: 4, marginTop: 10 }}>
          <T v="semi">{prev.data.runtime} · {prev.data.status_words}</T>
          <List>
            {prev.data.keep.map((k, i) => <ListRow key={k.joke_id} chevron={false} title={`${i + 1}. ${k.title}`} sub={k.label_words} />)}
          </List>
          {!!prev.data.cut.length && <T v="small" testID="trim-cut">Left out: {prev.data.cut.map((k) => k.title).join(' · ')}</T>}
          <Btn testID="trim-go" label={`Make this ${lengthWords(prev.data.target_s)} set`} v="primary" onPress={make} style={{ marginTop: 8 }} />
        </View>
      )}
    </Sheet>
  );
}

function RenameForm({ title, onSave, onDelete }: { title: string; onSave: (t: string) => void; onDelete: () => void }) {
  const [t, setT] = useState(title);
  const [sure, setSure] = useState(false);
  return (
    <View style={{ gap: 8 }}>
      <T v="h2">Rename</T>
      <TextInput testID="set-rename" value={t} onChangeText={setT} style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      <Btn label="Save" v="primary" disabled={!t.trim()} onPress={() => onSave(t.trim())} />
      {sure
        ? <Btn testID="set-delete-sure" label="Delete this set (its history stays in your log)" v="danger" onPress={onDelete} />
        : <TextBtn testID="set-delete" label="Delete this set…" color={C.redSoft} onPress={() => setSure(true)} />}
    </View>
  );
}

function BitPicker({ visible, onClose, onPick, exclude }: { visible: boolean; onClose: () => void; onPick: (j: { id: string; title: string }) => void; exclude: string[] }) {
  const [q, setQ] = useState('');
  const res = useApi<{ items: any[] }>(visible && q.trim().length >= 2 ? `/api/jokes?q=${encodeURIComponent(q.trim())}&mode=hybrid&limit=10` : null, { cache: false });
  return (
    <Sheet visible={visible} onClose={onClose} label="Add a bit" testID="set-bit-picker">
      <T v="h2" style={{ marginBottom: 8 }}>Add a bit</T>
      <TextInput testID="set-bit-search" value={q} onChangeText={setQ} placeholder="Search your notebook" placeholderTextColor={C.dim} autoFocus
        style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      <List>{(res.data?.items || []).filter((j) => !exclude.includes(j.id)).map((j) => <ListRow key={j.id} testID={`set-pick-${j.id}`} title={j.title.split(' (')[0]} sub={j.punch ? `…${j.punch}` : undefined} onPress={() => onPick(j)} />)}</List>
    </Sheet>
  );
}
