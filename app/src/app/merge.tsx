// Merge review (MixMerge, docs/UX_2026-10.md): the same-idea groups the merge judge found (GET /api/merge/candidates),
// one at a time: both pieces with their words, pick the title (that one is the main), then Merge into one bit
// (POST /api/merge, undo in the snackbar) · Not the same (POST /api/merge/dismiss) · Make it a tag (his tag_of edge,
// a decision line, and the pair is never suggested again).
import React, { useEffect, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { router } from 'expo-router';
import { decide, get, post, useApi } from '@/lib/api';
import { monthYear } from '@/lib/simple';
import { C, F } from '@/lib/theme';
import { AIBadge, Btn, Empty, ErrorCard, Screen, T, useUI } from '@/lib/ui';

type Member = { joke_id: string; title: string; first_date?: string | null; last_date?: string | null; nights?: number };
type Group = { id: string; main: string; main_title: string; main_why?: string; members: Member[]; reasons?: string[] };
const short = (t?: string) => (t || '').split(' (')[0];

/** One piece of the pair: its newest written words, fetched when the card shows (two calls per card, none per list). */
function Piece({ m, picked, onPick }: { m: Member; picked: boolean; onPick: () => void }) {
  const d = useApi<any>(`/api/jokes/${encodeURIComponent(m.joke_id)}`);
  const j = d.data;
  const unit = (j?.versions || []).flatMap((v: any) => v.units || []).find((u: any) => u.text);
  const text = unit?.text || j?.joke?.summary || '';
  const kind = unit?.doc_title ? 'Google Doc' : j?.joke?.from?.set ? 'From a set' : j?.yours ? 'Voice memo / jot' : 'Bit';
  return (
    <View testID={`piece-${m.joke_id}`} style={{ padding: 14, borderRadius: 16, backgroundColor: C.card, gap: 6, borderWidth: 1, borderColor: picked ? C.amber : C.card }}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <T v="small" style={{ flex: 1 }}>{kind}{m.first_date ? ` · ${monthYear(m.first_date)}` : ''}{m.nights ? ` · ${m.nights} set${m.nights === 1 ? '' : 's'}` : ''}</T>
        <Pressable accessibilityRole="link" accessibilityLabel={`Open ${short(m.title)}`} onPress={() => router.push(`/joke/${encodeURIComponent(m.joke_id)}` as any)} hitSlop={6} style={{ minHeight: 44, justifyContent: 'center' }}><T v="small" style={{ color: C.amber }}>Open ›</T></Pressable>
      </View>
      <Text style={{ fontFamily: F.bold, fontSize: 17, color: C.text }}>{short(m.title)}</Text>
      {d.loading && !j ? <T v="mut">…</T> : <Text numberOfLines={5} style={{ fontFamily: F.body, fontSize: 15, lineHeight: 22, color: C.text2 }}>{text ? `“${text}”` : 'No written words yet.'}</Text>}
      <Pressable testID={`call-it-${m.joke_id}`} accessibilityRole="radio" accessibilityState={{ checked: picked }} onPress={onPick} style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44 }}>
        <View style={{ width: 18, height: 18, borderRadius: 9, borderWidth: 2, borderColor: picked ? C.amber : C.borderStrong, backgroundColor: picked ? C.amber : 'transparent' }} />
        <T v="small" style={{ color: picked ? C.text : C.mut2 }}>Call it “{short(m.title)}”</T>
      </Pressable>
    </View>
  );
}

export default function MergeReview() {
  const ui = useUI();
  const [groups, setGroups] = useState<Group[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [i, setI] = useState(0);
  const [main, setMain] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const load = () => get<{ groups: Group[] }>('/api/merge/candidates').then((r) => { setGroups(r.groups || []); setErr(null); }).catch((e) => setErr(e.message));
  useEffect(() => { load(); }, []);
  const g = groups?.[i] || null;
  const mainId = (g && main && g.members.some((m) => m.joke_id === main) ? main : g?.main) || null;
  const next = () => { setMain(null); setI((k) => k + 1); };

  const merge = async () => {
    if (!g || !mainId) return;
    setBusy(true);
    const others = g.members.filter((m) => m.joke_id !== mainId).map((m) => m.joke_id);
    try {
      const r = await post<{ merge_id: string }>('/api/merge', { main: mainId, others, group: g.id, reason: 'merged in the review' });
      const t = short(g.members.find((m) => m.joke_id === mainId)?.title);
      ui.snack(`Merged into ${t}`, async () => { await post('/api/merge/undo', { merge_id: r.merge_id }); load(); });
      next();
    } catch (e: any) {
      ui.snack(`Couldn't merge: ${e.message}`);
    } finally {
      setBusy(false);
    }
  };
  const notSame = async () => {
    if (!g) return;
    setBusy(true);
    try {
      await post('/api/merge/dismiss', { jokes: g.members.map((m) => m.joke_id), reason: 'not the same (review)' });
      ui.snack('Never suggested together again');
      next();
    } catch (e: any) {
      ui.snack(`Couldn't save that: ${e.message}`);
    } finally {
      setBusy(false);
    }
  };
  const makeTag = async () => {
    if (!g || !mainId) return;
    setBusy(true);
    try {
      for (const o of g.members.filter((m) => m.joke_id !== mainId)) {
        await decide({ target: 'edge', action: 'add', after: { src: o.joke_id, dst: mainId, kind: 'tag_of', evidence: [] }, reason: 'a tag, not the same bit (merge review)' });
      }
      await post('/api/merge/dismiss', { jokes: g.members.map((m) => m.joke_id), reason: 'made a tag (review)' });
      ui.snack(`Tagged onto ${short(g.members.find((m) => m.joke_id === mainId)?.title)}`);
      next();
    } catch (e: any) {
      ui.snack(`Couldn't make the tag: ${e.message}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Screen title="Merge" back testID="merge" maxWidth={760}>
      {err && !groups ? <ErrorCard error={err} onRetry={load} /> : null}
      {groups && !groups.length && <Empty text="Nothing looks like the same idea right now." />}
      {groups && groups.length > 0 && !g && <Empty text="That’s all of them."><Btn small label="Back to Bits" onPress={() => router.navigate('/bits' as any)} /></Empty>}
      {g && (
        <View style={{ gap: 14 }}>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
            <T v="eb" style={{ flex: 1 }} testID="merge-count">{i + 1} of {groups!.length}</T>
            <AIBadge />
          </View>
          <Text style={{ fontFamily: F.black, fontSize: 26, letterSpacing: -0.5, color: C.text }}>Same idea?</Text>
          {!!(g.reasons || []).length && <T v="mut">{g.reasons![0]}</T>}
          {!!g.main_why && <T v="small">{g.main_why}</T>}
          {g.members.map((m) => <Piece key={m.joke_id} m={m} picked={m.joke_id === mainId} onPick={() => setMain(m.joke_id)} />)}
          <T v="small">One bit. Every version, every set and the memo come along, word for word. You can split it again any time (Undo).</T>
          <Btn testID="merge-go" v="primary" label={busy ? '…' : 'Merge into one bit'} disabled={busy} onPress={merge} />
          <View style={{ flexDirection: 'row', gap: 8 }}>
            <Btn testID="merge-not-same" style={{ flex: 1 }} label="Not the same" disabled={busy} onPress={notSame} />
            <Btn testID="merge-tag" style={{ flex: 1 }} label="Make it a tag" disabled={busy || g.members.length < 2} onPress={makeTag} />
          </View>
        </View>
      )}
    </Screen>
  );
}
