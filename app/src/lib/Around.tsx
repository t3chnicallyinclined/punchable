// Around this bit and In these sets (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §3.1, §4.6; Slices 1 and 2): what
// is tied to a bit, drawn on the bit page under the document, and the half sheet that acts on one card.
//
//   AroundInline  (Slice 2; owner on build 28: "when i open a bit i dont automatically see other connections") the
//                 page itself: each group that holds something as a header and its first two cards (rules/aroundRules
//                 inlineGroups), each card with one line of HIS words, the AI badge or "yours", and its one button;
//                 "See all 5" opens the sheet on that group. Skeleton rows while the read is on its way.
//   AroundList    one group's cards (the sheet behind "See all"), or every group (the bit page's older editor, kept
//                 behind a preference). A card: the other thing's name and its facts, one line of HIS words, then the
//                 AI badge or "yours" and the one-line why. A tap opens that card (/around, a native form sheet).
//   SetsList      the sets the bit is in ("bit 2 of 6", his notes when he has some), each a push to the set list; the
//                 last row opens the Add to a set picker (lib/SetPicker).
//   AroundSheet   one card: the why, his lines (tap the one you want when a button writes a line), where a line would
//                 land, and one button per thing he can do; or, opened by "See all", one group's cards first. Every
//                 button is one call (rules/aroundRules planFor) with its Undo; nothing is written until he taps.
//                 "Not connected" is remembered or rejected by the rule in aroundRules, never here. No button is
//                 filled when the first one is a merge (HIG/buttons: people tap a filled button without reading it; a
//                 merge then asks once more in an action sheet).
//   actOn         the one write behind every button, on the page and in the sheet. A button that writes a line edits
//                 the DOCUMENT (rules/lineRules applyEdit), so the blank lines between his ideas stay where they were.
//
// One read: GET /api/jokes/<id>/around (pk-api). The sheet reads the same path through the cache, so it opens with
// what the page shows. After a write the sheet closes (only if it is still there: he may have swiped it away while
// the request ran), the root's snack carries Undo, and `aroundChanged` tells the page under it to read again and to
// keep a non-timed Undo on its row (HIG.md §7).
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { PixelRatio, Platform, Pressable, ScrollView, Text, View } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { patch, post, undo, useApi, useOnline } from './api';
import { addVersion } from './bitWork';
import { bitPending } from './bitDraft';
import { actionSheet } from './actionSheet';
import { haptics } from './haptics';
import { Icon } from './Icon';
import { C, F, linkColor } from './theme';
import { AIBadge, Btn, Sheet, SheetHead, Skeleton, T, Yours, useUI } from './ui';
import * as R from '@/rules/bitLines';
import * as A from '@/rules/aroundRules';
import { applyEdit } from '@/rules/lineRules';

const enc = encodeURIComponent;
const native = Platform.OS !== 'web';

/** The one read behind both rows and the sheet. */
export const aroundPath = (bit: string) => `/api/jokes/${enc(bit)}/around`;

/** What a write from the sheet (or the picker) tells the page under it. */
export type AroundChange = {
  /** The bit it is about. */ bit: string;
  /** A new version was written: the decision indexes its Undo needs (the Versions row's non-timed Undo). */ indexes?: number[];
  /** Any other write's Undo (the Around row's non-timed Undo). */ undo?: () => Promise<void>;
};
const listeners = new Set<(c: AroundChange) => void>();
export const aroundChanged = {
  emit(c: AroundChange) { listeners.forEach((l) => l(c)); },
  subscribe(l: (c: AroundChange) => void) { listeners.add(l); return () => { listeners.delete(l); }; },
};

const send = (c: A.Call) => (c.method === 'POST' ? post<any>(c.path, c.body) : patch<any>(c.path, c.body));

/** The bit's document as a card's button writes into it: the newest stage version, as the page shows it. */
export function homeText(d: any): string {
  const vs = R.versionsOf(d);
  return vs[R.homeIndex(vs)]?.text || '';
}

/** One button's work, on the page or in the sheet: the call (or the new version), then the snack with its Undo and
 *  the word to the page under it. Resolves to null when it was done, or to the reason nothing was (never throws). */
export async function actOn(ui: { snack: (msg: string, undo?: () => Promise<void> | void) => void }, bit: string, found: { group: A.AroundGroup; item: A.AroundItem }, action: A.Does, text: string, pick = 0): Promise<string | null> {
  const { group, item } = found;
  const plan = A.planFor(bit, group, item, action, R.linesOf(text), pick);
  if (plan.kind === 'none') return plan.why;
  try {
    let change: AroundChange = { bit };
    if (plan.kind === 'version') {
      const r = await addVersion(bit, applyEdit(text, plan.edit), plan.reason);
      change = { bit, indexes: r.indexes };
    } else {
      const answer = await send(plan.call);
      const back = A.undoFor(bit, item, plan.call, answer);
      if (back) change = { bit, undo: async () => { await send(back); } };
    }
    haptics.success();
    aroundChanged.emit(change);
    const undoIt = change.indexes
      ? async () => { await undo(change.indexes!); aroundChanged.emit({ bit }); }
      : change.undo ? async () => { await change.undo!(); aroundChanged.emit({ bit }); } : undefined;
    ui.snack(A.doneWord(item, action), undoIt);
    return null;
  } catch (e: any) {
    return `Couldn’t do that: ${e?.message || 'try again'}`;
  }
}
/** A merge moves versions and tapes: one confirmation first, as On tape asked before (HIG/action-sheets). */
function confirmMerge(item: A.AroundItem, go: () => void) {
  const ask = { title: `Merge “${item.title}” into this bit?`, message: 'Its versions and tapes move under this bit. You can undo it.' };
  if (native) actionSheet({ ...ask, options: [{ label: 'Merge', onPress: go }, { label: 'Cancel', cancel: true }] });
  else go();
}

function Who({ item }: { item: A.AroundItem }) {
  return item.source === 'ai' ? <AIBadge /> : <Yours />;
}

/** The groups under "Around this bit". */
export function AroundList({ around, only, onOpen, testID }: { around: A.Around; /** one group's cards (the sheet behind "See all"): no header, the sheet's title says it */ only?: string; onOpen: (item: A.AroundItem) => void; testID?: string }) {
  const groups = A.shownGroups(around).filter((g) => !only || g.key === only);
  // at large text sizes the facts go under the name, as the section rows do (HIG/typography: a stacked layout)
  const stacked = R.stackMeta(PixelRatio.getFontScale());
  if (!groups.length) {
    return (
      <View testID={testID ? `${testID}-none` : undefined} style={{ minHeight: 48, justifyContent: 'center', borderBottomWidth: 1, borderBottomColor: C.hairline }}>
        <Text maxFontSizeMultiplier={R.ROW_CAP} style={{ fontFamily: F.body, fontSize: 15, lineHeight: 21, color: C.mut2 }}>Nothing is tied to this bit yet.</Text>
      </View>
    );
  }
  return (
    <View testID={testID} style={{ paddingTop: 4, paddingBottom: 8, gap: 12, borderBottomWidth: only ? 0 : 1, borderBottomColor: C.hairline }}>
      {groups.map((g) => (
        <View key={g.key} testID={testID ? `${testID}-${g.key}` : undefined} style={{ gap: 6 }}>
          {!only && <T v="eb" accessibilityRole="header">{g.title}</T>}
          {g.items.map((it) => (
            <Pressable key={it.id} testID={testID ? `${testID}-card-${it.id}` : undefined} accessibilityRole="button"
              accessibilityLabel={[it.title, it.meta, it.words[0], `${it.source === 'ai' ? 'From the AI' : 'Yours'}: ${it.why}`].filter(Boolean).join('. ')}
              accessibilityHint="Shows what you can do with it" onPress={() => onOpen(it)}
              style={(st: any) => ({ minHeight: 56, paddingVertical: 8, paddingHorizontal: 12, borderRadius: 12, gap: 3, backgroundColor: st.pressed || st.hovered ? C.card2 : C.card })}>
              <View style={{ flexDirection: stacked ? 'column' : 'row', alignItems: stacked ? 'flex-start' : 'baseline', gap: stacked ? 0 : 8 }}>
                <Text numberOfLines={stacked ? 2 : 1} maxFontSizeMultiplier={R.ROW_CAP} style={{ flex: stacked ? undefined : 1, minWidth: 0, fontFamily: F.semi, fontSize: 15, lineHeight: 20, color: C.text }}>{it.title}</Text>
                {!!it.meta && <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 12, lineHeight: 16, color: C.mut2 }}>{it.meta}</Text>}
              </View>
              {!!it.words[0] && <Text numberOfLines={2} maxFontSizeMultiplier={R.ROW_CAP} style={{ fontFamily: F.body, fontSize: 15, lineHeight: 21, color: C.text2 }}>{it.words[0]}</Text>}
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
                <Who item={it} />
                <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ flex: 1, minWidth: 0, fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>{it.why}</Text>
              </View>
            </Pressable>
          ))}
        </View>
      ))}
    </View>
  );
}

/** A card's words, as every card draws them: the name and its facts, one line of his, who said so and why. */
function CardWords({ it, stacked }: { it: A.AroundItem; stacked: boolean }) {
  return (
    <>
      <View style={{ flexDirection: stacked ? 'column' : 'row', alignItems: stacked ? 'flex-start' : 'baseline', gap: stacked ? 0 : 8 }}>
        <Text numberOfLines={stacked ? 2 : 1} maxFontSizeMultiplier={R.ROW_CAP} style={{ flex: stacked ? undefined : 1, minWidth: 0, fontFamily: F.semi, fontSize: 15, lineHeight: 20, color: C.text }}>{it.title}</Text>
        {!!it.meta && <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 12, lineHeight: 16, color: C.mut2 }}>{it.meta}</Text>}
      </View>
      {!!it.words[0] && <Text numberOfLines={2} maxFontSizeMultiplier={R.ROW_CAP} style={{ fontFamily: F.body, fontSize: 15, lineHeight: 21, color: C.text2 }}>{it.words[0]}</Text>}
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
        <Who item={it} />
        <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ flex: 1, minWidth: 0, fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>{it.why}</Text>
      </View>
    </>
  );
}

/** Around this bit, on the page itself (Slice 2): under the document, a header per group that holds something, its
 *  first two cards, and "See all N" when there are more. `around` null = the read is on its way (skeleton rows, so
 *  the rows under it do not jump when it lands). `text` = the document a card's line is written into; `canAct` is
 *  false offline and while words he typed are not in a version yet (a card must not write over them). */
export function AroundInline({ bit, around, text, canAct, right, testID }: { bit: string; around: A.Around | null; text: string; canAct: boolean; /** the non-timed Undo of the last change */ right?: React.ReactNode; testID?: string }) {
  const ui = useUI();
  const [busy, setBusy] = useState<string | null>(null);
  const stacked = R.stackMeta(PixelRatio.getFontScale());
  const head = (
    <View style={{ flexDirection: 'row', alignItems: 'center', minHeight: 44 }}>
      <Text accessibilityRole="header" maxFontSizeMultiplier={R.ROW_CAP} style={{ flex: 1, minWidth: 0, fontFamily: F.semi, fontSize: 17, lineHeight: 22, color: C.text }}>Around this bit</Text>
      {right}
    </View>
  );
  if (!around) {
    return (
      <View testID={testID ? `${testID}-loading` : undefined} style={{ paddingBottom: 12, borderBottomWidth: 1, borderBottomColor: C.hairline }}>
        {head}
        <Skeleton lines={3} height={14} />
      </View>
    );
  }
  const groups = A.inlineGroups(around);
  // nothing tied to the bit: no section at all (the page explains nothing, SPEC 2026-11-03 §3.1), unless an Undo is waiting
  if (!groups.length) return right ? <View testID={testID ? `${testID}-none` : undefined} style={{ borderBottomWidth: 1, borderBottomColor: C.hairline }}>{head}</View> : null;
  const open = (params: Record<string, string>) => router.push({ pathname: '/around', params: { bit, ...params } } as any);
  const press = (group: A.InlineGroup, it: A.AroundItem, act: NonNullable<ReturnType<typeof A.inlineAction>>) => {
    // a card with several lines to choose from: the sheet has the picker
    if (!act.direct) return open({ item: it.id });
    const found = A.findItem(around, it.id);
    if (!found || busy) return;
    const go = async () => {
      setBusy(it.id);
      const why = await actOn(ui, bit, found, act.key, text);
      setBusy(null);
      if (why) ui.snack(why);
    };
    if (act.key === 'merge') confirmMerge(it, () => void go());
    else void go();
  };
  return (
    <View testID={testID} style={{ paddingBottom: 12, gap: 12, borderBottomWidth: 1, borderBottomColor: C.hairline }}>
      {head}
      {groups.map((g) => (
        <View key={g.key} testID={testID ? `${testID}-${g.key}` : undefined} style={{ gap: 6 }}>
          <T v="eb" accessibilityRole="header">{g.title}</T>
          {g.cards.map((it) => {
            const act = A.inlineAction(it);
            return (
              <View key={it.id} testID={testID ? `${testID}-card-${it.id}` : undefined} style={{ borderRadius: 12, backgroundColor: C.card, overflow: 'hidden' }}>
                <Pressable accessibilityRole="button"
                  accessibilityLabel={[it.title, it.meta, it.words[0], `${it.source === 'ai' ? 'From the AI' : 'Yours'}: ${it.why}`].filter(Boolean).join('. ')}
                  accessibilityHint="Shows everything you can do with it" onPress={() => open({ item: it.id })}
                  style={(st: any) => ({ minHeight: 56, paddingTop: 8, paddingBottom: act ? 4 : 8, paddingHorizontal: 12, gap: 3, backgroundColor: st.pressed || st.hovered ? C.card2 : 'transparent' })}>
                  <CardWords it={it} stacked={stacked} />
                </Pressable>
                {!!act && (
                  // the card's one button: a bordered shape, not colour alone (HIG/buttons), never filled (a merge is the
                  // first action of a same-idea card, and it asks once more); VoiceOver hears which card it acts on
                  <View style={{ flexDirection: 'row', paddingHorizontal: 12, paddingBottom: 8 }}>
                    <Btn small testID={testID ? `${testID}-do-${it.id}` : undefined} label={busy === it.id ? 'Working…' : act.label} accessibilityLabel={`${act.label}: ${it.title}`}
                      disabled={!canAct || busy != null} textStyle={{ color: canAct ? linkColor() : C.dim }} onPress={() => press(g, it, act)} />
                  </View>
                )}
              </View>
            );
          })}
          {g.more > 0 && (
            <Pressable testID={testID ? `${testID}-all-${g.key}` : undefined} accessibilityRole="button" accessibilityLabel={`${A.seeAllLabel(g.more)} in ${g.title}`} onPress={() => open({ group: g.key })}
              style={({ pressed }) => ({ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 12, opacity: pressed ? 0.7 : 1 })}>
              <Text maxFontSizeMultiplier={R.ROW_CAP} style={{ flex: 1, fontFamily: F.semi, fontSize: 15, lineHeight: 20, color: linkColor() }}>{A.seeAllLabel(g.more)}</Text>
              <Icon name="chevron" size={16} color={C.dim} />
            </Pressable>
          )}
        </View>
      ))}
    </View>
  );
}

/** "In these sets" on the page itself (Slice 2): the header, then the sets by name (a tap opens the set) and the row
 *  that opens the picker. `sets` null = the read is on its way. */
export function SetsInline({ sets, onAdd, canAdd, testID }: { sets: A.AroundSet[] | null; onAdd: () => void; canAdd: boolean; testID?: string }) {
  return (
    <View testID={testID ? `${testID}-section` : undefined}>
      <View style={{ minHeight: 44, justifyContent: 'center' }}>
        <Text accessibilityRole="header" maxFontSizeMultiplier={R.ROW_CAP} style={{ fontFamily: F.semi, fontSize: 17, lineHeight: 22, color: C.text }}>In these sets</Text>
      </View>
      {sets ? <SetsList testID={testID} sets={sets} onAdd={onAdd} canAdd={canAdd} /> : <Skeleton lines={2} height={14} style={{ paddingBottom: 12, borderBottomWidth: 1, borderBottomColor: C.hairline }} testID={testID ? `${testID}-loading` : undefined} />}
    </View>
  );
}

/** The rows under "In these sets", then the row that opens the picker. */
export function SetsList({ sets, onAdd, canAdd, testID }: { sets: A.AroundSet[]; onAdd: () => void; canAdd: boolean; testID?: string }) {
  const stacked = R.stackMeta(PixelRatio.getFontScale());
  return (
    <View testID={testID} style={{ borderBottomWidth: 1, borderBottomColor: C.hairline }}>
      {sets.map((s) => {
        const notes = A.setNotesLine(s);
        return (
          <Pressable key={s.board} testID={testID ? `${testID}-${s.board}` : undefined} accessibilityRole="button"
            accessibilityLabel={[s.title, A.setPlace(s), notes].filter(Boolean).join(', ')} accessibilityHint="Opens the set"
            onPress={() => router.push(`/list/${enc(s.board)}` as any)}
            style={({ pressed }) => ({ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 52, paddingVertical: 6, backgroundColor: pressed ? C.card2 : 'transparent' })}>
            <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
              <View style={{ flexDirection: stacked ? 'column' : 'row', alignItems: stacked ? 'flex-start' : 'baseline', gap: stacked ? 0 : 8 }}>
                <Text numberOfLines={stacked ? 2 : 1} maxFontSizeMultiplier={R.ROW_CAP} style={{ flexShrink: 1, fontFamily: F.semi, fontSize: 17, lineHeight: 22, color: C.text }}>{s.title}</Text>
                <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>{A.setPlace(s)}</Text>
              </View>
              {!!notes && <Text numberOfLines={2} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>{notes}</Text>}
            </View>
            <Icon name="chevron" size={16} color={C.dim} />
          </Pressable>
        );
      })}
      <Pressable testID={testID ? `${testID}-add` : undefined} accessibilityRole="button" accessibilityLabel="Add to a set" accessibilityHint={canAdd ? 'Tick the sets it belongs in' : 'Offline'}
        accessibilityState={{ disabled: !canAdd }} disabled={!canAdd} onPress={onAdd}
        style={({ pressed }) => ({ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 48, paddingVertical: 6, opacity: pressed ? 0.7 : 1 })}>
        <Icon name="plus" size={18} color={canAdd ? linkColor() : C.dim} sw={2.4} />
        <Text maxFontSizeMultiplier={R.ROW_CAP} style={{ fontFamily: F.semi, fontSize: 17, lineHeight: 22, color: canAdd ? linkColor() : C.dim }}>Add to a set</Text>
      </Pressable>
    </View>
  );
}

/** The half sheet for one card (the route /around?bit=…&item=…). */
export function AroundSheet() {
  const p = useLocalSearchParams<{ bit: string; item?: string; group?: string }>();
  const bit = decodeURIComponent(String(p.bit || ''));
  // opened by "See all": one group's cards first, then the card he taps (in this same sheet: one sheet at a time)
  const groupKey = decodeURIComponent(String(p.group || ''));
  const [picked, setPicked] = useState<string | null>(null);
  const itemId = picked ?? decodeURIComponent(String(p.item || ''));
  const ui = useUI();
  const online = useOnline();
  const ar = useApi<A.Around>(bit ? aroundPath(bit) : null);
  const d = useApi<any>(bit ? `/api/jokes/${enc(bit)}` : null);
  const [pick, setPick] = useState(0);
  const [busy, setBusy] = useState<A.Does | null>(null);
  const [err, setErr] = useState<string | null>(null);
  // the sheet may be gone by the time a request answers (a swipe down while it ran): then nothing is popped
  const alive = useRef(true);
  useEffect(() => () => { alive.current = false; }, []);
  const found = useMemo(() => (itemId ? A.findItem(ar.data, itemId) : null), [ar.data, itemId]);
  const group = useMemo(() => (groupKey ? A.findGroup(ar.data, groupKey) : null), [ar.data, groupKey]);
  // the document a pulled line lands in: the newest stage version, as the page shows it
  const text = useMemo(() => homeText(d.data), [d.data]);
  const lines = useMemo(() => R.linesOf(text), [text]);
  const close = () => (router.canGoBack() ? router.back() : router.replace(`/joke/${enc(bit)}` as any));
  // in a group's list, or on a card reached from it
  const listing = !!groupKey && !itemId;
  const title = listing ? group?.title || 'Around this bit' : found?.item.title || 'Around this bit';
  // the leading button: back to the group's list from a card opened there, else Close
  const back = groupKey && picked ? () => { setPicked(null); setPick(0); setErr(null); } : null;

  // words he typed on the page are still on their way to a version: a button here would write from the older text
  const waiting = bitPending(bit);
  const go = async (action: A.Does) => {
    if (!found || busy || bitPending(bit)) return;
    setBusy(action);
    setErr(null);
    const why = await actOn(ui, bit, found, action, text, pick);
    if (why) {
      // the sheet may be gone by now (a swipe down while the request ran): then the snack says it
      if (alive.current) { setErr(why); setBusy(null); } else ui.snack(why);
      return;
    }
    if (alive.current) { setBusy(null); close(); }
  };
  const tap = (action: A.Does) => {
    if (action !== 'merge' || !found) return void go(action);
    confirmMerge(found.item, () => void go('merge'));
  };

  let body: React.ReactNode;
  if (listing) {
    body = group && ar.data
      ? <AroundList testID="around-group" around={ar.data} only={group.key} onOpen={(it) => { setPicked(it.id); setPick(0); setErr(null); }} />
      : <T v="mut" testID="around-gone">{ar.data ? 'Nothing is left in this group.' : ar.rawError ? 'Can’t reach Punchable.' : 'Loading…'}</T>;
  } else if (!found) {
    body = <T v="mut" testID="around-gone">{ar.data ? 'That is no longer tied to this bit.' : ar.rawError ? 'Can’t reach Punchable.' : 'Loading…'}</T>;
  } else {
    const { item } = found;
    const actions = A.actionsFor(item);
    const writes = actions.some((a) => A.writesLine(a.key));
    const pickable = writes && item.words.length > 1;
    const at = A.anchorIndex(lines, item);
    const uses = actions.some((a) => a.key === 'use');
    body = (
      <View style={{ gap: 14 }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
          <Who item={item} />
          <Text maxFontSizeMultiplier={1.6} style={{ flex: 1, minWidth: 0, fontFamily: F.body, fontSize: 15, lineHeight: 21, color: C.mut2 }}>{[item.why, item.meta].filter(Boolean).join(' · ')}</Text>
        </View>
        <View testID="around-words" accessibilityRole={pickable ? 'radiogroup' : undefined} style={{ gap: 4 }}>
          {item.words.map((w, i) => {
            const on = pickable && i === pick;
            return (
              <Pressable key={i} testID={`around-line-${i}`} disabled={!pickable} accessibilityRole={pickable ? 'radio' : 'text'} accessibilityState={pickable ? { checked: on } : undefined}
                onPress={() => { setPick(i); setErr(null); }}
                style={{ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 8, paddingVertical: 8, paddingHorizontal: 12, borderRadius: 10, borderWidth: 1, borderColor: on ? linkColor() : 'transparent', backgroundColor: on ? C.card2 : C.card }}>
                <Text maxFontSizeMultiplier={R.PROSE_CAP} style={{ flex: 1, minWidth: 0, fontFamily: F.body, fontSize: 17, lineHeight: 24, color: C.text }}>{w}</Text>
                {/* the picked line carries a mark, not a colour alone (HIG.md §1); its place is kept so lines do not shift */}
                {pickable && <View style={{ width: 24, alignItems: 'center' }}>{on && <Icon name="check" size={18} color={linkColor()} sw={2.6} />}</View>}
              </Pressable>
            );
          })}
        </View>
        {writes && (
          <Text testID="around-lands" maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>
            {pickable ? 'Tap the line you want. ' : ''}
            {at == null ? 'It goes at the end of the bit.' : uses ? `It takes the place of “${lines[at]}”` : `It goes after “${lines[at]}”`}
          </Text>
        )}
        {!!err && <Text testID="around-error" accessibilityLiveRegion="polite" maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 14, lineHeight: 20, color: C.redSoft }}>{err}</Text>}
        {!online && <T v="mut" testID="around-offline">Offline · reading only</T>}
        {online && waiting && <T v="mut" testID="around-waiting">Saving what you typed first</T>}
        <View style={{ gap: 8 }}>
          {actions.map((a, i) => (
            <Btn key={a.key} testID={`around-do-${a.key}`} label={busy === a.key ? 'Working…' : a.label} v={a.quiet ? 'ghost' : i === 0 && a.key !== 'merge' ? 'primary' : 'plain'}
              disabled={!online || waiting || busy != null} onPress={() => tap(a.key)} />
          ))}
        </View>
      </View>
    );
  }
  if (!native) {
    return (
      <View style={{ flex: 1, backgroundColor: C.bg }}>
        <Sheet visible onClose={close} label={title} testID="around-sheet">
          <SheetHead title={title} onCancel={back || close} cancelLabel={back ? 'Back' : 'Close'} testID="around" />
          {body}
        </Sheet>
      </View>
    );
  }
  return (
    <View testID="around-sheet" style={{ flex: 1, backgroundColor: C.surface }}>
      <Stack.Screen options={{ title }} />
      {/* HIG/sheets: Close on the leading edge, the standard symbol (HIG/toolbars); no Done (each button is its own action) */}
      <Stack.Toolbar placement="left">
        {back
          ? <Stack.Toolbar.Button icon="chevron.left" accessibilityLabel={`Back to ${group?.title || 'the group'}`} onPress={back} />
          : <Stack.Toolbar.Button icon="xmark" accessibilityLabel="Close" onPress={close} />}
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: 16, paddingBottom: 48 }}>
        {body}
      </ScrollView>
    </View>
  );
}
