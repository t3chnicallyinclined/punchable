// The morning inbox (MixInbox, docs/UX_2026-10.md): the voice memos and jots that need a call, one card at a time
// (play, the words, "sounds like <bit>"), Add as a new version · New bit · Not a joke · Ask; below it "Filed for
// you" (the memos the app attached to a bit on its own) with Undo. Sources: GET /api/suggestions/not-a-joke (the
// "needs a call" cards) and GET /api/memos (filed on their own); the "sounds like" comes from
// GET /api/merge/candidates?joke= for the card that is showing (one call per card, none per list). Changes arrive
// over /api/live; the health poll runs only while that socket is down.
import React, { useEffect, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { router } from 'expo-router';
import { ApiError, decide, get, post, undo, useApi } from '@/lib/api';
import { HearIt } from '@/lib/hear';
import { Memo, MemosResp, memoTitle } from '@/lib/memos';
import { C, F, fmtDate, fmtMs } from '@/lib/theme';
import { AIBadge, Btn, Empty, ErrorCard, List, ListRow, Screen, T, useUI } from '@/lib/ui';

type NotJoke = { joke_id: string; title: string; text: string; kind: string; why: string; confidence: number; memo?: string };
type Like = { joke_id: string; title: string; why?: string } | null;

function Card({ x, memo, onDone }: { x: NotJoke; memo: Memo | null; onDone: (what: string) => void }) {
  const ui = useUI();
  const [like, setLike] = useState<Like | undefined>(undefined);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let alive = true;
    setLike(undefined);
    get<{ group: any; closest?: any[] }>(`/api/merge/candidates?joke=${encodeURIComponent(x.joke_id)}`)
      .then((r) => {
        if (!alive) return;
        const g = r.group;
        const other = g ? g.members.find((m: any) => m.joke_id !== x.joke_id) : (r.closest || [])[0];
        setLike(other ? { joke_id: other.joke_id, title: (other.title || '').split(' (')[0], why: g?.reasons?.[0] || other.why } : null);
      })
      .catch(() => alive && setLike(null));
    return () => { alive = false; };
  }, [x.joke_id]);
  const act = async (what: 'version' | 'new' | 'not' | 'ask') => {
    if (what === 'ask') return router.push({ pathname: '/say', params: { text: x.text.slice(0, 400), joke: x.joke_id, screen: 'inbox' } } as any);
    setBusy(true);
    try {
      if (what === 'version' && like) {
        const r = await post<{ merge_id: string }>('/api/merge', { main: like.joke_id, others: [x.joke_id], reason: 'added as a new version (inbox)' });
        ui.snack(`Added to ${like.title}`, async () => { await post('/api/merge/undo', { merge_id: r.merge_id }); });
      } else if (what === 'new') {
        await post('/api/suggestions/not-a-joke/keep', { joke_id: x.joke_id });
        if (like) post('/api/merge/dismiss', { jokes: [x.joke_id, like.joke_id], reason: 'kept as its own bit (inbox)' }).catch(() => {});
        ui.snack(`Kept “${x.title}” as its own bit`);
      } else {
        const r = await post<{ undo?: { indexes: number[] } }>('/api/suggestions/not-a-joke/hide', { joke_id: x.joke_id });
        ui.snack('Not a joke: hidden', async () => { await undo(r.undo?.indexes || []); });
      }
      onDone(what);
    } catch (e: any) {
      ui.snack(`Couldn't do that: ${e.message}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <View testID={`inbox-card-${x.joke_id}`} style={{ padding: 16, borderRadius: 20, backgroundColor: C.card, gap: 10 }}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <T v="eb" style={{ flex: 1 }}>{memo ? 'Voice memo' : 'Jot'}</T>
        <T v="small">{memo?.recorded_at ? `${fmtDate(memo.recorded_at.slice(0, 10))}` : ''}{memo?.duration_s ? ` · ${fmtMs(memo.duration_s * 1000)}` : ''}</T>
      </View>
      {memo && <HearIt audio={memo.audio} reference={memo.reference} play={{ start_ms: 0, end_ms: (memo.duration_s || 0) * 1000 + 1000 }} k={`inbox:${memo.id}`} sub={memo.duration_s ? fmtMs(memo.duration_s * 1000) : 'the recording'} testID={`inbox-play-${x.joke_id}`} />}
      <Text style={{ fontFamily: F.body, fontSize: 16, lineHeight: 24, color: C.text }}>“{x.text}”</Text>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
        <AIBadge />
        {like === undefined ? <T v="small">Looking for the bit it sounds like…</T>
          : like ? <T v="small" style={{ color: C.text2 }}>Sounds like <Text style={{ fontFamily: F.semi, color: C.text }}>{like.title}</Text>{like.why ? `: ${like.why}` : ', with a new angle.'}</T>
            : <T v="small" style={{ color: C.text2 }}>{x.why || 'Sounds like a note, not a joke.'}</T>}
      </View>
      <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
        <Btn small testID={`inbox-version-${x.joke_id}`} v="primary" label={like ? 'Add as a new version' : 'Add as a new version'} disabled={busy || !like} onPress={() => act('version')} />
        <Btn small testID={`inbox-new-${x.joke_id}`} label="New bit" disabled={busy} onPress={() => act('new')} />
        <Btn small testID={`inbox-not-${x.joke_id}`} label="Not a joke" disabled={busy} onPress={() => act('not')} />
        <Btn small testID={`inbox-ask-${x.joke_id}`} v="ghost" label="Ask" disabled={busy} onPress={() => act('ask')} />
      </View>
    </View>
  );
}

export default function Inbox() {
  const sug = useApi<{ items: NotJoke[]; count: number }>('/api/suggestions/not-a-joke', { cache: false, poll: 30000 });
  const memos = useApi<MemosResp>('/api/memos', { cache: false, poll: 60000 });
  const ui = useUI();
  const [done, setDone] = useState<Set<string>>(new Set());
  const [undone, setUndone] = useState<Set<string>>(new Set());
  const items = (sug.data?.items || []).filter((x) => !done.has(x.joke_id));
  const byJoke = new Map((memos.data?.items || []).filter((m) => m.joke_id).map((m) => [m.joke_id!, m]));
  const filed = (memos.data?.items || []).filter((m) => m.status === 'done' && m.attached_to && !undone.has(m.id)).slice(0, 12);
  const first = items[0] || null;
  const total = sug.data?.count || 0;
  const unfile = async (m: Memo) => {
    try {
      const r = await get<{ attached_to?: { id: string } | null }>(`/api/merge/candidates?joke=${encodeURIComponent(m.joke_id!)}`);
      const aid = r.attached_to?.id;
      if (!aid) return ui.snack('That one was filed by hand; open it to move it.');
      await post('/api/merge/undo', { merge_id: aid });
      setUndone((s) => new Set(s).add(m.id));
      ui.snack('Back as its own idea');
      memos.reload();
    } catch (e: any) {
      ui.snack(e instanceof ApiError ? `Couldn't undo: ${e.message}` : 'Can’t reach Punchable');
    }
  };
  return (
    <Screen title="Inbox" back testID="inbox" maxWidth={760}>
      <View style={{ gap: 2 }}>
        <T v="eb">Since last night</T>
        <T v="mut" testID="inbox-line">{filed.length} filed on their own · <Text style={{ color: C.text }}>{items.length} need a call</Text></T>
      </View>
      {sug.rawError && !sug.data ? <ErrorCard error={sug.rawError} onRetry={sug.reload} /> : null}
      {sug.data && !first && <Empty text="Nothing to sort. Say a bit and it lands here if it isn’t sure." />}
      {first && (
        <View style={{ gap: 8 }}>
          <T v="small" testID="inbox-count">{total - items.length + 1} of {total}</T>
          <Card key={first.joke_id} x={first} memo={byJoke.get(first.joke_id) || null} onDone={() => setDone((s) => new Set(s).add(first.joke_id))} />
          {items.length > 1 && <T v="small" style={{ textAlign: 'center' }}>{items.length - 1} more after this one</T>}
        </View>
      )}
      {!!filed.length && (
        <View style={{ gap: 4 }}>
          <T v="eb">Filed for you</T>
          <List>
            {filed.map((m) => (
              <ListRow key={m.id} testID={`filed-${m.id}`} title={<T v="semi" numberOfLines={1}>{memoTitle(m)} <Text style={{ color: C.mut2 }}>→</Text> {m.attached_to!.title}</T>}
                sub={[m.date ? fmtDate(m.date) : null, m.duration_s ? fmtMs(m.duration_s * 1000) : null].filter(Boolean).join(' · ')}
                chevron={false} onPress={() => router.push(`/joke/${encodeURIComponent(m.attached_to!.joke_id)}` as any)}
                right={<Pressable testID={`filed-undo-${m.id}`} accessibilityRole="button" accessibilityLabel={`Undo: take ${memoTitle(m)} out of ${m.attached_to!.title}`} onPress={() => unfile(m)} style={{ minHeight: 44, minWidth: 44, justifyContent: 'center' }}><T style={{ color: C.amber }}>Undo</T></Pressable>} />
            ))}
          </List>
        </View>
      )}
      {memos.data && !memos.data.available && <T v="small">Voice memos are transcribed on your PC; the ones it filed show here once it reports them.</T>}
    </Screen>
  );
}
