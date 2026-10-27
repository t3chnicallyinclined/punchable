// Add a joke to a set list (MixAdd): a search box that also takes a question, filters Fits · Kills · Never tried ·
// Memos, each result with why it fits, "+" adds (the board's own add op), a joke already in the set is greyed with
// its position, and "Nothing right? Write a new one here" saves an idea (POST /api/ideas) and adds it.
// The search is debounced (300 ms) and stale answers are dropped (useApi keeps only the latest).
import React, { useEffect, useMemo, useState } from 'react';
import { FlatList, Pressable, Text, TextInput, View } from 'react-native';
import { router } from 'expo-router';
import { JokeSummary, post, useApi } from './api';
import { fmtDur } from './laughs';
import { Memo, MemosResp, memoTitle } from './memos';
import { BitCard, Build, Related, quoteText } from './simple';
import { isQuestion, shortNameOf } from './say';
import { C, F } from './theme';
import { Btn, Chip, Chips, Sheet, T } from './ui';

type Filter = 'fits' | 'kills' | 'never' | 'memos';
export type Pick = { joke_id: string; title: string; short_name: string };
type RowT = { joke_id: string; title: string; short_name: string; sub: string; why?: string | null; duration_s?: number | null; estimated?: boolean };

function fromCard(b: BitCard): RowT {
  return { joke_id: b.joke_id, title: b.title, short_name: b.short_name || shortNameOf(b.title), sub: b.quote ? `“${quoteText(b)}”` : b.stage_line || '', why: b.why || null, duration_s: b.duration_s, estimated: b.estimated };
}
function fromSummary(j: JokeSummary): RowT {
  const line = j.sets ? `${j.hit_sets ? `Laughed in ${j.hit_sets} of ${j.sets}` : `On stage ${j.sets}`} set${j.sets === 1 ? '' : 's'}` : 'never tried';
  return { joke_id: j.id, title: j.short_title || j.title, short_name: j.short_name || shortNameOf(j.title), sub: j.punch ? `“…${j.punch}”` : line, why: j.sets ? line : null, duration_s: j.duration_s, estimated: j.duration_estimated };
}

export function AddJokeSheet({ visible, onClose, boardId, inSet, fits, onPick, left, mode = 'add' }: {
  visible: boolean; onClose: () => void; boardId: string; inSet: { joke_id: string; position: number }[]; fits: BitCard[]; onPick: (p: Pick) => void; left: number | null; mode?: 'add' | 'swap';
}) {
  const [q, setQ] = useState('');
  const [dq, setDq] = useState('');
  const [filter, setFilter] = useState<Filter>('fits');
  const [writing, setWriting] = useState(false);
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setDq(q.trim()), 300);
    return () => clearTimeout(t);
  }, [q]);
  useEffect(() => {
    if (!visible) { setQ(''); setDq(''); setWriting(false); setDraft(''); }
  }, [visible]);
  const question = dq.length >= 3 && isQuestion(dq);
  const rel = useApi<Related>(visible && dq.length >= 2 && !question ? `/api/notebook/related?q=${encodeURIComponent(dq)}&cap=12` : null, { cache: false });
  const kills = useApi<{ items: JokeSummary[] }>(visible && filter === 'kills' && !dq ? '/api/jokes?sort=strongest&limit=30' : null);
  const never = useApi<{ items: JokeSummary[] }>(visible && filter === 'never' && !dq ? '/api/jokes?sort=written&limit=200' : null);
  const memos = useApi<MemosResp>(visible && filter === 'memos' && !dq ? '/api/memos' : null, { cache: false });
  const pos = useMemo(() => new Map(inSet.map((x) => [x.joke_id, x.position])), [inSet]);

  const rows: RowT[] = useMemo(() => {
    if (dq && !question) return (rel.data?.items || []).map(fromCard);
    if (filter === 'fits') return fits.map(fromCard);
    if (filter === 'kills') return (kills.data?.items || []).filter((j) => (j.sets || 0) > 0).map(fromSummary);
    if (filter === 'never') return (never.data?.items || []).filter((j) => !(j.sets || 0)).slice(0, 60).map(fromSummary);
    return (memos.data?.items || []).filter((m: Memo) => m.status === 'done' && m.joke_id && !m.no_speech)
      .map((m: Memo) => ({ joke_id: m.attached_to?.joke_id || m.joke_id!, title: m.attached_to?.title || memoTitle(m), short_name: shortNameOf(m.attached_to?.title || memoTitle(m)), sub: m.text ? `“${m.text.slice(0, 120)}${m.text.length > 120 ? '…' : ''}”` : '', why: `memo${m.date ? ` ${m.date.slice(0, 4)}` : ''}`, duration_s: m.duration_s }));
  }, [dq, question, rel.data, filter, fits, kills.data, never.data, memos.data]);
  const loading = (dq && !question && rel.loading) || (filter === 'kills' && kills.loading && !kills.data) || (filter === 'never' && never.loading && !never.data) || (filter === 'memos' && memos.loading && !memos.data);

  const writeNew = async () => {
    const t = draft.trim();
    if (!t) return;
    setBusy(true);
    try {
      const r = await post<{ id: string; attach?: { decision: string; into?: string; title?: string } }>('/api/ideas', { text: t, reason: 'written in the add-a-joke sheet' });
      const id = r.attach?.decision === 'attach' && r.attach.into ? r.attach.into : r.id;
      const title = r.attach?.decision === 'attach' && r.attach.title ? r.attach.title : t.split(/\s+/).slice(0, 7).join(' ');
      onPick({ joke_id: id, title, short_name: shortNameOf(title) });
    } finally {
      setBusy(false);
    }
  };

  const row = ({ item: r }: { item: RowT }) => {
    const at = pos.get(r.joke_id);
    const inside = at != null;
    return (
      <View testID={`add-row-${r.joke_id}`} style={{ flexDirection: 'row', alignItems: 'center', gap: 12, paddingVertical: 10, borderTopWidth: 1, borderTopColor: C.border, opacity: inside ? 0.45 : 1 }}>
        <Pressable style={{ flex: 1, minWidth: 0, gap: 2 }} accessibilityRole="link" accessibilityLabel={`${r.title}. Open`} onPress={() => router.push(`/joke/${encodeURIComponent(r.joke_id)}` as any)}>
          <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 8 }}>
            <Text numberOfLines={1} style={{ flexShrink: 1, fontFamily: F.semi, fontSize: 16, color: C.text }}>{r.short_name}</Text>
            {r.duration_s != null && <Text style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>{r.estimated ? '~' : ''}{fmtDur(r.duration_s)}</Text>}
          </View>
          {!!r.sub && <Text numberOfLines={2} style={{ fontFamily: F.body, fontSize: 14, color: C.text2 }}>{r.sub}</Text>}
          <Text numberOfLines={1} style={{ fontFamily: F.body, fontSize: 13, color: inside ? C.mut2 : C.amber }}>{inside ? `Already in this set at #${at! + 1}` : r.why || ''}</Text>
        </Pressable>
        <Pressable testID={`add-${r.joke_id}`} accessibilityRole="button" accessibilityLabel={inside ? `${r.title} is already in this set at ${at! + 1}` : `${mode === 'swap' ? 'Swap in' : 'Add'} ${r.title}`}
          disabled={inside} onPress={() => onPick({ joke_id: r.joke_id, title: r.title, short_name: r.short_name })}
          style={{ width: 44, height: 44, borderRadius: 22, backgroundColor: inside ? C.card : C.amber, alignItems: 'center', justifyContent: 'center' }}>
          <Text style={{ fontFamily: F.bold, fontSize: 22, lineHeight: 24, color: inside ? C.mut2 : C.bg }}>{inside ? '✓' : '+'}</Text>
        </Pressable>
      </View>
    );
  };

  return (
    <Sheet visible={visible} onClose={onClose} label={mode === 'swap' ? 'Swap for another joke' : 'Add a joke'} testID="add-sheet">
      <View style={{ flexDirection: 'row', alignItems: 'center', marginBottom: 8 }}>
        <T v="h2" style={{ flex: 1 }}>{mode === 'swap' ? 'Swap it for' : 'Add a joke'}{left != null && mode === 'add' ? <Text style={{ color: C.mut2, fontFamily: F.body, fontSize: 15 }}> · {left > 0 ? `${fmtDur(left)} left` : 'full'}</Text> : null}</T>
        <Btn small v="ghost" label="Done" testID="add-done" onPress={onClose} />
      </View>
      <TextInput testID="add-search" value={q} onChangeText={setQ} placeholder="something about food to close" placeholderTextColor={C.dim} autoFocus
        style={{ minHeight: 48, borderRadius: 24, paddingHorizontal: 16, backgroundColor: C.card, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      {question && (
        <Pressable testID="add-ask" accessibilityRole="button" onPress={() => { onClose(); router.push({ pathname: '/say', params: { text: dq, board: boardId, screen: 'add-a-joke' } } as any); }}
          style={{ marginTop: 8, padding: 12, borderRadius: 14, backgroundColor: C.card2, flexDirection: 'row', alignItems: 'center', gap: 10 }}>
          <T style={{ flex: 1 }}>Ask your material: “{dq}”</T><T style={{ color: C.amber }}>Ask ›</T>
        </Pressable>
      )}
      {!dq && (
        <View style={{ marginVertical: 10 }}>
          <Chips testID="add-filters">
            {([['fits', 'Fits'], ['kills', 'Kills'], ['never', 'Never tried'], ['memos', 'Memos']] as [Filter, string][]).map(([k, l]) => (
              <Chip key={k} testID={`add-filter-${k}`} label={l} on={filter === k} onPress={() => setFilter(k)} />
            ))}
          </Chips>
        </View>
      )}
      {loading ? <T v="mut" style={{ paddingVertical: 12 }}>Looking…</T> : null}
      {!loading && !rows.length && !question && <T v="mut" style={{ paddingVertical: 12 }}>{dq ? 'Nothing in your material matches that.' : filter === 'fits' ? 'Nothing obvious fits yet. Try Kills or search.' : 'Nothing here.'}</T>}
      <FlatList data={rows} keyExtractor={(r) => r.joke_id} renderItem={row} scrollEnabled={false} initialNumToRender={12} />
      <View style={{ marginTop: 14, gap: 8 }}>
        {!writing ? (
          <Pressable testID="add-write" accessibilityRole="button" onPress={() => { setWriting(true); setDraft(dq && !question ? dq : ''); }} style={{ minHeight: 44, justifyContent: 'center' }}>
            <T style={{ color: C.amber }}>Nothing right? Write a new one here ›</T>
          </Pressable>
        ) : (
          <View style={{ gap: 8 }}>
            <TextInput testID="add-draft" value={draft} onChangeText={setDraft} multiline placeholder="The line, as you’d say it" placeholderTextColor={C.dim}
              style={{ minHeight: 72, borderWidth: 1, borderColor: C.border, borderRadius: 10, padding: 12, color: C.text, fontFamily: F.body, fontSize: 16, textAlignVertical: 'top' }} />
            <Btn testID="add-draft-save" v="primary" label={busy ? 'Saving…' : 'Save it and add it'} disabled={!draft.trim() || busy} onPress={writeNew} />
          </View>
        )}
      </View>
    </Sheet>
  );
}
