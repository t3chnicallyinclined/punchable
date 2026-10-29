// Ask (docs/API_CONTRACT.md "Ask"): a conversation with his own notebook. The API finds his bits, notes and set
// moments; his PC's Claude answers from those only; every quote is checked word for word before it is shown; with
// nothing to go on the answer is "I couldn't find that in your notebook." This file: the client, the header button,
// the conversation and the list of past ones. Screens: app/ask/index.tsx (/ask) and app/ask/[id].tsx (/ask/<id>).
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, Text, TextInput, View } from 'react-native';
import { router } from 'expo-router';
import { get, patch, post } from './api';
import { C, F } from './theme';
import { AIBadge, Btn, Card, Chip, Empty, ErrorCard, List, ListRow, T, s } from './ui';
import { Screen } from './Screen';

export type AskChip = {
  n: number; kind: 'bit' | 'idea' | 'note' | 'moment'; source: string; label: string; sub: string; href: string;
  joke_id?: string | null; set_id?: string | null; start_ms?: number | null; date?: string | null; quote: string | null;
};
export type AskTurn = {
  id: string; q: string; status: 'thinking' | 'done' | 'failed'; asked_at: string; done_at?: string | null;
  answer?: string; found?: boolean; chips?: AskChip[]; dropped_n?: number; model?: string | null; error?: string;
};
/** The AI worker (the server since 2026-10-27): online = a heartbeat in the last 2 minutes; `paused` = over the day's AI budget, with the reason. */
export type Worker = { online: boolean; last_seen: string | null; where: string; name?: string | null; hosted?: boolean; model?: string | null; paused?: string | null; online_window_s?: number };
export type Chat = { id: string; title: string; created_at: string; updated_at: string; turns: AskTurn[]; worker: Worker; /** the bit this conversation is about (the bit page's workbench), if any */ joke?: string | null };
export type ChatListItem = { id: string; title: string; updated_at: string; questions: number; last_q: string | null; thinking: boolean };

const enc = encodeURIComponent;
export const ask = {
  list: () => get<{ chats: ChatListItem[]; worker: Worker }>('/api/ask/chats'),
  start: (question: string) => post<Chat>('/api/ask/chats', { question }),
  /** a conversation about ONE bit: its versions, idea and tapes lead every turn's sources (lib/BitTalk) */
  startAbout: (question: string, joke: string) => post<Chat>('/api/ask/chats', { question, joke }),
  get: (id: string) => get<Chat>(`/api/ask/chats/${enc(id)}`),
  follow: (id: string, question: string) => post<Chat>(`/api/ask/chats/${enc(id)}`, { question }),
  hide: (id: string) => patch<Chat>(`/api/ask/chats/${enc(id)}`, { deleted: true }),
};
export const EXAMPLES = ['What have I written about AI?', 'Which bits killed at The Corner Room?', 'What do I have about my niece?'];

/** The question box: one line, a send button, Enter sends. */
export function AskBox({ onSend, placeholder, busy, testID = 'ask-input', autoFocus, initial = '' }: {
  onSend: (q: string) => void; placeholder: string; busy?: boolean; testID?: string; autoFocus?: boolean; initial?: string;
}) {
  const [q, setQ] = useState(initial);
  const send = () => {
    const t = q.trim();
    if (!t || busy) return;
    onSend(t);
    setQ('');
  };
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 48, borderWidth: 1, borderColor: C.border, borderRadius: 12, paddingLeft: 12, paddingRight: 4, backgroundColor: C.surface2 }}>
      <TextInput testID={testID} value={q} onChangeText={setQ} onSubmitEditing={send} returnKeyType="send" autoFocus={autoFocus} blurOnSubmit={false}
        placeholder={placeholder} placeholderTextColor={C.dim} accessibilityLabel={placeholder} maxLength={1000}
        style={{ flex: 1, minWidth: 0, minHeight: 44, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      <Pressable testID={`${testID}-send`} accessibilityRole="button" accessibilityLabel="Ask" disabled={busy || !q.trim()} onPress={send}
        style={{ height: 40, paddingHorizontal: 14, borderRadius: 10, alignItems: 'center', justifyContent: 'center', backgroundColor: q.trim() && !busy ? C.amber : C.surface, opacity: q.trim() && !busy ? 1 : 0.6 }}>
        <Text style={{ fontFamily: F.g7, fontSize: 14, color: q.trim() && !busy ? C.bg : C.mut }}>Ask</Text>
      </Pressable>
    </View>
  );
}

/** One conversation, polled while a question is being answered. */
export function useChat(id: string | null) {
  const [chat, setChat] = useState<Chat | null>(null);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  const load = useCallback(async () => {
    if (!id) return null;
    try {
      const c = await ask.get(id);
      if (alive.current) { setChat(c); setError(null); }
      return c;
    } catch (e: any) {
      if (alive.current) setError(e.message);
      return null;
    }
  }, [id]);
  useEffect(() => {
    alive.current = true;
    load();
    return () => { alive.current = false; };
  }, [load]);
  const thinking = !!chat?.turns.some((t) => t.status === 'thinking');
  useEffect(() => {
    if (!thinking) return;
    let n = 0;
    let timer: any;
    const tick = async () => {
      n += 1;
      await load();
      if (alive.current) timer = setTimeout(tick, n < 20 ? 2500 : 6000); // a long answer: keep checking, more slowly
    };
    timer = setTimeout(tick, 2000);
    return () => clearTimeout(timer);
  }, [thinking, load]);
  return { chat, setChat, error, reload: load, thinking };
}

/** The answer's [1] [2] markers as small amber tappable numbers that open the source. */
function AnswerText({ text, chips, testID }: { text: string; chips: AskChip[]; testID?: string }) {
  const parts = text.split(/(\[\d+\])/g);
  return (
    <Text testID={testID} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 16, lineHeight: 24, color: C.text }}>
      {parts.map((p, i) => {
        const m = p.match(/^\[(\d+)\]$/);
        const ch = m ? chips.find((c) => c.n === Number(m[1])) : null;
        if (!m) return <Text key={i}>{p}</Text>;
        if (!ch) return null;
        return (
          <Text key={i} accessibilityRole="link" accessibilityLabel={`Source ${ch.n}: ${ch.label}`} onPress={() => router.push(ch.href as any)}
            style={{ fontFamily: F.g7, fontSize: 12, color: C.amber }}> {ch.n}</Text>
        );
      })}
    </Text>
  );
}

/** A tappable source: a bit → its joke map; a set moment → that set at that time. */
export function SourceChip({ c }: { c: AskChip }) {
  return (
    <Pressable testID={`ask-chip-${c.n}`} accessibilityRole="link" accessibilityLabel={`${c.label}. ${c.sub}. Open`} onPress={() => router.push(c.href as any)}
      style={(st: any) => ({ borderWidth: 1, borderColor: C.border, borderRadius: 10, padding: 10, gap: 4, backgroundColor: st.hovered || st.pressed ? C.surface2 : C.surface })}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <View style={{ minWidth: 22, height: 22, borderRadius: 11, backgroundColor: C.amberSoft, alignItems: 'center', justifyContent: 'center' }}>
          <Text style={{ fontFamily: F.g7, fontSize: 12, color: C.amber }}>{c.n}</Text>
        </View>
        <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ flex: 1, minWidth: 0, fontFamily: F.semi, fontSize: 14, color: C.text }}>{c.label}</Text>
        <Text style={{ color: C.mut, fontSize: 16 }}>›</Text>
      </View>
      <Text numberOfLines={2} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, color: C.mut }}>{c.sub}</Text>
      {!!c.quote && <Text testID={`ask-quote-${c.n}`} maxFontSizeMultiplier={1.5} style={{ fontFamily: F.body, fontSize: 14, lineHeight: 20, color: C.text2, borderLeftWidth: 3, borderLeftColor: C.amber, paddingLeft: 8 }}>“{c.quote}”</Text>}
    </Pressable>
  );
}

function Thinking({ worker, testID }: { worker?: Worker; testID?: string }) {
  if (worker && !worker.online) {
    return (
      <View testID="ask-offline" style={[s.inner, { gap: 4, borderColor: C.addBorder, backgroundColor: C.warnBg }]}>
        <T style={{ color: C.amber, fontFamily: F.semi }}>{worker.paused ? 'AI paused for today' : 'The writer is offline'}</T>
        <T v="mut">{worker.paused ? `Your question is saved. ${worker.paused}.` : 'Your question is saved. It gets answered when the writer is back, usually under a minute.'}</T>
      </View>
    );
  }
  return (
    <View testID={testID} style={{ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 32 }}>
      <ActivityIndicator color={C.amber} />
      <T v="mut">Thinking… reading your notebook</T>
    </View>
  );
}

export function TurnView({ t, worker, onRetry }: { t: AskTurn; worker?: Worker; onRetry?: (q: string) => void }) {
  return (
    <View testID={`ask-turn-${t.id}`} style={{ gap: 10 }}>
      <View style={{ alignSelf: 'flex-end', maxWidth: '88%', backgroundColor: C.amberSoft, borderRadius: 14, borderBottomRightRadius: 4, paddingHorizontal: 14, paddingVertical: 10 }}>
        <Text testID={`ask-q-${t.id}`} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 16, color: C.text }}>{t.q}</Text>
      </View>
      {t.status === 'thinking' && <Thinking worker={worker} testID="ask-thinking" />}
      {t.status === 'failed' && (
        <Card testID="ask-failed" style={{ gap: 8 }}>
          <T>{t.error || 'Your PC couldn’t answer that one.'}</T>
          {onRetry && <Btn small label="Ask again" testID="ask-retry" onPress={() => onRetry(t.q)} />}
        </Card>
      )}
      {t.status === 'done' && (
        <Card testID={t.found ? 'ask-answer' : 'ask-not-found'} style={{ gap: 12 }}>
          <View style={s.row}><AIBadge /><T v="eb" style={{ flex: 1 }}>{t.found ? 'From your notebook' : 'Not in your notebook'}</T></View>
          <AnswerText text={t.answer || ''} chips={t.chips || []} testID={`ask-text-${t.id}`} />
          {!!t.chips?.length && <View style={{ gap: 8 }}>{t.chips.map((c) => <SourceChip key={c.n} c={c} />)}</View>}
          {!!t.dropped_n && <T v="small" testID="ask-dropped">{t.dropped_n === 1 ? 'One quote was left out: it isn’t word for word in your notebook.' : `${t.dropped_n} quotes were left out: they aren’t word for word in your notebook.`}</T>}
        </Card>
      )}
    </View>
  );
}

/** /ask/<id>: a whole conversation, the follow-up box pinned to the bottom. */
export function AskThreadScreen({ id }: { id: string }) {
  const { chat, setChat, error, reload, thinking } = useChat(id);
  const [sending, setSending] = useState(false);
  const [sendErr, setSendErr] = useState<string | null>(null);
  const sv = useRef<any>(null);
  const n = chat?.turns.length || 0;
  const last = chat?.turns[n - 1]?.status;
  useEffect(() => {
    if (n) setTimeout(() => sv.current?.scrollToEnd?.({ animated: true }), 60);
  }, [n, last]);
  const follow = async (q: string) => {
    setSending(true);
    setSendErr(null);
    try {
      setChat(await ask.follow(id, q));
    } catch (e: any) {
      setSendErr(e.message);
    } finally {
      setSending(false);
    }
  };
  const dock = chat ? <AskBox testID="ask-follow" placeholder={thinking ? 'Wait for the answer…' : 'Ask a follow-up'} busy={sending || thinking} onSend={follow} /> : undefined;
  return (
    <Screen title={chat?.title || 'Ask'} back testID="ask" dock={dock} scrollRef={sv} maxWidth={760}>
      {error && !chat ? <ErrorCard error={error} onRetry={reload} />
        : !chat ? <View style={{ padding: 24, alignItems: 'center' }}><ActivityIndicator color={C.amber} /></View>
          : (
            <View testID="ask-thread" style={{ gap: 20 }}>
              {chat.turns.map((t) => <TurnView key={t.id} t={t} worker={chat.worker} onRetry={thinking ? undefined : follow} />)}
              {!!sendErr && <T v="mut" testID="ask-send-error">Couldn’t send that: {sendErr}</T>}
            </View>
          )}
    </Screen>
  );
}

/** /ask: a new question, then his past conversations. `first` = a question to ask at once (from Notebook, /ask?q=). */
export function AskHome({ first }: { first?: string }) {
  const [list, setList] = useState<{ chats: ChatListItem[]; worker: Worker } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const started = useRef(false);
  const start = async (q: string) => {
    setBusy(true);
    setErr(null);
    try {
      const c = await ask.start(q);
      router.replace(`/ask/${encodeURIComponent(c.id)}` as any);
    } catch (e: any) {
      setErr(e.message);
      setBusy(false);
    }
  };
  useEffect(() => {
    if (first && !started.current) {
      started.current = true;
      start(first);
      return;
    }
    ask.list().then(setList).catch((e) => setErr(e.message));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <View testID="ask-home" style={{ gap: 16 }}>
      <T v="mut">Ask anything about your bits, your notes and your sets. Answers come only from your own material.</T>
      <AskBox testID="ask-input" placeholder="Ask your notebook" busy={busy} onSend={start} autoFocus={!first} />
      {busy && <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}><ActivityIndicator color={C.amber} /><T v="mut">Finding it in your notebook…</T></View>}
      {!!err && <ErrorCard error={err} />}
      {list && !list.chats.length && !busy && (
        <View style={{ gap: 8 }}>
          <T v="eb">Try</T>
          <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>{EXAMPLES.map((x, i) => <Chip key={x} testID={`ask-example-${i}`} label={x} onPress={() => start(x)} />)}</View>
        </View>
      )}
      {list && !!list.chats.length && (
        <View style={{ gap: 8 }}>
          <T v="eb">Your past questions</T>
          <List>
            {list.chats.map((c) => (
              <ListRow key={c.id} testID={`ask-chat-${c.id}`} title={c.title} sub={`${shortDay(c.updated_at)}${c.questions > 1 ? ` · ${c.questions} questions` : ''}${c.thinking ? ' · thinking…' : ''}`}
                onPress={() => router.push(`/ask/${encodeURIComponent(c.id)}` as any)} />
            ))}
          </List>
        </View>
      )}
      {!list && !err && !first && <Empty text="Loading your past questions…" />}
    </View>
  );
}

const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
function shortDay(iso: string) {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return '';
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return 'Today';
  return `${MON[d.getMonth()]} ${d.getDate()}${d.getFullYear() !== now.getFullYear() ? `, ${d.getFullYear()}` : ''}`;
}
