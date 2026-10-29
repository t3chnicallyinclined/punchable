// The conversation on a bit's page (2026-10-29): a chat pinned to THIS bit (POST /api/ask/chats {question, joke};
// the API keeps the bit's own versions, idea and tapes in every turn's sources and tells the model which bit "it" is).
// Under each answer: Keep as a line (a new version of this bit, undoable), and for every other bit the answer cites,
// Merge into this bit (undoable). One chat per bit per session (a module map), so leaving and coming back keeps it.
import React, { useCallback, useState } from 'react';
import { View } from 'react-native';
import { AskChip, Chat, TurnView, ask, useChat } from './ask';
import { addVersion, answerLine, mergeInto, undoMerge } from './bitWork';
import { haptics } from './haptics';
import { Btn, T, useUI } from './ui';

const CHATS = new Map<string, string>(); // joke id -> chat id, for this session

export function useBitTalk(jokeId: string, onChanged: () => void) {
  const [chatId, setChatId] = useState<string | null>(() => CHATS.get(jokeId) || null);
  const { chat, setChat, thinking, reload } = useChat(chatId);
  const [sending, setSending] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const ui = useUI();
  const askIt = useCallback(async (q: string) => {
    setSending(true);
    setErr(null);
    try {
      let c: Chat;
      if (chatId) c = await ask.follow(chatId, q);
      else {
        c = await ask.startAbout(q, jokeId);
        CHATS.set(jokeId, c.id);
        setChatId(c.id);
      }
      setChat(c);
    } catch (e: any) {
      setErr(e?.message || 'Could not ask');
      ui.snack(`Couldn’t ask: ${e?.message || e}`);
    } finally {
      setSending(false);
    }
  }, [chatId, jokeId, setChat, ui]);
  return { chat, thinking, sending, err, askIt, reload, onChanged };
}

export function BitTalk({ jokeId, talk, shortName }: { jokeId: string; shortName: string; talk: ReturnType<typeof useBitTalk> }) {
  const ui = useUI();
  const { chat, thinking, askIt, onChanged } = talk;
  const [kept, setKept] = useState<Set<string>>(new Set());
  if (!chat) return null;
  const keep = async (turnId: string, text: string) => {
    const line = answerLine(text);
    if (!line) return;
    try {
      const r = await addVersion(jokeId, line, 'kept from the conversation on this bit', true);
      haptics.success();
      setKept((k) => new Set(k).add(turnId));
      onChanged();
      ui.snack('Kept as a line of this bit', async () => { const { undo } = await import('./api'); await undo(r.indexes); setKept((k) => { const n = new Set(k); n.delete(turnId); return n; }); onChanged(); });
    } catch (e: any) {
      ui.snack(`Couldn’t keep it: ${e.message}`);
    }
  };
  const merge = async (chip: AskChip) => {
    if (!chip.joke_id || chip.joke_id === jokeId) return;
    try {
      const r = await mergeInto(jokeId, [chip.joke_id], `merged from the conversation on ${shortName}`);
      haptics.success();
      onChanged();
      ui.snack(`${chip.label} is now part of this bit`, async () => { await undoMerge(r.merge_id); onChanged(); });
    } catch (e: any) {
      ui.snack(`Couldn’t merge: ${e.message}`);
    }
  };
  return (
    <View testID="bit-talk" style={{ gap: 14 }}>
      <T v="eb">Talking about {shortName}</T>
      {chat.turns.map((t) => {
        const others = (t.chips || []).filter((c) => c.joke_id && c.joke_id !== jokeId);
        const seen = new Set<string>();
        const uniq = others.filter((c) => (seen.has(c.joke_id!) ? false : (seen.add(c.joke_id!), true)));
        return (
          <View key={t.id} style={{ gap: 8 }}>
            <TurnView t={t} worker={chat.worker} onRetry={thinking ? undefined : askIt} />
            {t.status === 'done' && t.found && (
              <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
                <Btn small v={kept.has(t.id) ? 'ghost' : 'add'} testID={`talk-keep-${t.id}`} label={kept.has(t.id) ? 'Kept' : 'Keep as a line'} disabled={kept.has(t.id) || !answerLine(t.answer)} onPress={() => keep(t.id, t.answer || '')} />
                {uniq.slice(0, 3).map((c) => <Btn key={c.joke_id!} small testID={`talk-merge-${t.id}-${c.n}`} label={`Merge “${c.label}” into this`} onPress={() => merge(c)} />)}
              </View>
            )}
          </View>
        );
      })}
    </View>
  );
}
