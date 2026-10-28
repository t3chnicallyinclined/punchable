// The conversation (MixAsk, docs/UX_2026-10.md): one input; each thing he says is sent to POST /api/say and comes
// back as a question (answered from his material: the chat's turns, with bit cards and "Put it in tonight's set")
// or a bit ("That's a bit, not a question. Saved to <bit>. Undo"). The classification is the app's; "Not a question?
// Save as a bit" / "Not a bit? Ask it" re-send with his `kind` and are recorded as his override.
// Offline: the text is queued on this phone as a bit, shown "saving…", and sent when Punchable is back.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, Text, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { ApiError, patch as apiPatch, post, undo } from '@/lib/api';
import { AskChip, TurnView, ask, useChat } from '@/lib/ask';
import { useBoards } from '@/lib/boards';
import { haptics } from '@/lib/haptics';
import { CommandResult, Queued, SayContext, SayKind, SayResult, flushOutbox, sayOrQueue, useOutbox } from '@/lib/say';
import { SayBar } from '@/lib/SayBar';
import { C, F } from '@/lib/theme';
import { noteWorker } from '@/lib/worker';
import { Btn, T, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

type Entry = { key: string; text: string; how: 'typed' | 'spoken'; state: 'sending' | 'queued' | 'done' | 'error'; result?: SayResult; error?: string; queuedKey?: string; undone?: boolean };

function Said({ e }: { e: Entry }) {
  return (
    <View style={{ alignSelf: 'flex-end', maxWidth: '88%', gap: 3 }}>
      <View style={{ backgroundColor: C.amberSoft, borderRadius: 16, borderBottomRightRadius: 4, paddingHorizontal: 14, paddingVertical: 10 }}>
        <Text testID={`said-${e.key}`} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 16, color: C.text }}>{e.text}</Text>
      </View>
      <Text style={{ alignSelf: 'flex-end', fontFamily: F.body, fontSize: 12, color: C.mut2 }}>
        {e.how === 'spoken' ? 'said out loud' : 'typed'}{e.state === 'queued' ? ' · saving… (no connection; it’s kept on this phone)' : e.state === 'sending' ? ' · …' : ''}
      </Text>
    </View>
  );
}

/** A question: its conversation, polled while the PC thinks; bit cards get "Put it in tonight's set". */
function QuestionEntry({ e, onNotQuestion }: { e: Entry; onNotQuestion: () => void }) {
  const id = e.result?.chat?.id || null;
  const { chat, thinking } = useChat(id);
  const boards = useBoards();
  const ui = useUI();
  const c = chat || e.result?.chat || null;
  const [more, setMore] = useState(false);
  if (!c) return null;
  const turn = c.turns[c.turns.length - 1];
  const jokes = (turn?.chips || []).filter((x: AskChip) => x.joke_id);
  const shown = more ? jokes : jokes.slice(0, 3);
  return (
    <View style={{ gap: 10 }}>
      <View style={{ alignSelf: 'flex-start', maxWidth: '96%', width: '100%', gap: 8 }}>
        {turn && (
          <View style={{ gap: 8 }}>
            {turn.status === 'thinking' && (
              c.worker && !c.worker.online
                ? <View testID="say-offline" style={{ padding: 12, borderRadius: 14, backgroundColor: C.warnBg, gap: 2 }}><T style={{ color: C.amber, fontFamily: F.semi }}>The writer is offline</T><T v="mut">The question is saved. It gets answered when the writer is back, usually under a minute.</T></View>
                : <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}><ActivityIndicator color={C.amber} /><T v="mut">Reading your material…</T></View>
            )}
            {turn.status !== 'thinking' && <TurnViewNoQ t={turn} worker={c.worker} />}
          </View>
        )}
        {!!shown.length && turn?.status === 'done' && (
          <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
            {shown.map((ch: AskChip) => (
              <Btn key={ch.n} small v="add" testID={`say-put-${ch.n}`} label={`Put #${ch.n} in tonight’s set`} onPress={() => boards.add(ch.joke_id!, ch.label)} />
            ))}
            {jokes.length > 3 && !more && <Btn small v="ghost" label="More" onPress={() => setMore(true)} />}
          </View>
        )}
        <Pressable testID={`say-not-question-${e.key}`} accessibilityRole="button" onPress={onNotQuestion} style={{ minHeight: 44, justifyContent: 'center' }}>
          <T v="small" style={{ color: C.amber }}>Not a question? Save it as a bit</T>
        </Pressable>
        {thinking ? null : null}
        {!!ui ? null : null}
      </View>
    </View>
  );
}
/** TurnView without repeating the question bubble (the thread already shows what he said). */
function TurnViewNoQ({ t, worker }: { t: any; worker: any }) {
  return <View style={{ marginTop: -4 }}><TurnView t={{ ...t, q: '' }} worker={worker} /></View>;
}

/** A command (docs/UX_2026-10.md "Commands"): done with Undo, or the app asks which bit (his one tap = the write). */
function CommandEntry({ e, onChoice, onUndo, onNotCommand, onAskIt }: { e: Entry; onChoice: (jokeId: string) => void; onUndo: () => void; onNotCommand: () => void; onAskIt: () => void }) {
  const c = e.result!.command as CommandResult;
  const boards = useBoards();
  const open = () => c.board && router.push(`/list/${encodeURIComponent(c.board.id)}` as any);
  const link = (label: string, onPress: () => void, testID?: string) => (
    <Pressable testID={testID} accessibilityRole="button" onPress={onPress} style={{ minHeight: 44, justifyContent: 'center' }}><T v="small" style={{ color: C.amber }}>{label}</T></Pressable>
  );
  let body: React.ReactNode;
  if (e.undone) body = <T style={{ color: C.text2 }}>Undone.</T>;
  else if (c.done && c.op === 'new_set') {
    body = <T style={{ color: C.text2 }}>Started <Text style={{ fontFamily: F.semi, color: C.text }} onPress={open}>{c.board?.title}</Text>{c.minutes ? ` · a ${c.minutes}-minute slot` : ''}. Add jokes, or say “add &lt;bit&gt; to tonight”.</T>;
  } else if (c.done && c.already) {
    body = <T style={{ color: C.text2 }}><Text style={{ fontFamily: F.semi, color: C.text }}>{c.joke?.short_name}</Text> is already #{(c.position ?? 0) + 1} on <Text style={{ fontFamily: F.semi, color: C.text }} onPress={open}>{c.board?.title}</Text>.</T>;
  } else if (c.done) {
    body = <T style={{ color: C.text2 }}>Added <Text style={{ fontFamily: F.semi, color: C.text }}>{c.joke?.short_name}</Text> to <Text style={{ fontFamily: F.semi, color: C.text }} onPress={open}>{c.board?.title}</Text> as #{(c.position ?? 0) + 1}{c.created ? ' (a new list for tonight)' : ''}.</T>;
  } else if (c.ask === 'which') {
    body = (
      <View style={{ gap: 8 }}>
        <T style={{ color: C.text2 }}>Which bit?</T>
        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
          {(c.candidates || []).map((x) => <Btn key={x.joke_id} small v="add" testID={`say-choice-${x.joke_id}`} label={x.short_name} onPress={() => onChoice(x.joke_id)} />)}
        </View>
      </View>
    );
  } else {
    body = <T style={{ color: C.text2 }}>No bit called “{c.query}” that I can see.</T>;
  }
  React.useEffect(() => {
    if (c.done && c.board && !e.undone) boards.setCurrent({ id: c.board.id, title: c.board.title });
  }, [c.done, c.board?.id, e.undone]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <View testID={`say-command-${e.key}`} style={{ alignSelf: 'flex-start', maxWidth: '92%', gap: 6, padding: 12, borderRadius: 16, backgroundColor: C.card2 }}>
      {body}
      {!e.undone && (
        <View style={{ flexDirection: 'row', gap: 14, flexWrap: 'wrap' }}>
          {c.done && c.undo && link('Undo', onUndo, `say-undo-${e.key}`)}
          {c.done && c.board && link('Open the list', open, `say-open-${e.key}`)}
          {link('Not a command? Save it as a bit', onNotCommand, `say-not-command-${e.key}`)}
          {link('Ask it', onAskIt)}
        </View>
      )}
    </View>
  );
}

function BitEntry({ e, onUndo, onNotBit }: { e: Entry; onUndo: () => void; onNotBit: () => void }) {
  const r = e.result!;
  const to = r.saved_to;
  return (
    <View testID={`say-bit-${e.key}`} style={{ alignSelf: 'flex-start', maxWidth: '92%', gap: 6, padding: 12, borderRadius: 16, backgroundColor: C.card2 }}>
      <T style={{ color: C.text2 }}>
        That’s a bit, not a question.{' '}
        {e.undone ? 'Undone.' : to ? <Text>Saved to <Text style={{ fontFamily: F.semi, color: C.text }} onPress={() => router.push(`/joke/${encodeURIComponent(to.joke_id)}` as any)}>{to.title || 'a bit'}</Text>.</Text> : 'Saved as a new bit.'}
      </T>
      {!e.undone && (
        <View style={{ flexDirection: 'row', gap: 14, flexWrap: 'wrap' }}>
          <Pressable testID={`say-undo-${e.key}`} accessibilityRole="button" onPress={onUndo} style={{ minHeight: 44, justifyContent: 'center' }}><T v="small" style={{ color: C.amber }}>Undo</T></Pressable>
          {r.idea && <Pressable accessibilityRole="button" onPress={() => router.push(`/joke/${encodeURIComponent(to ? to.joke_id : r.idea!.id)}` as any)} style={{ minHeight: 44, justifyContent: 'center' }}><T v="small" style={{ color: C.amber }}>Open</T></Pressable>}
          <Pressable testID={`say-not-bit-${e.key}`} accessibilityRole="button" onPress={onNotBit} style={{ minHeight: 44, justifyContent: 'center' }}><T v="small" style={{ color: C.amber }}>Not a bit? Ask it</T></Pressable>
        </View>
      )}
    </View>
  );
}

export default function Say() {
  const p = useLocalSearchParams<{ text?: string; screen?: string; set?: string; joke?: string; board?: string; focus?: string }>();
  const ui = useUI();
  const [entries, setEntries] = useState<Entry[]>([]);
  const sv = useRef<any>(null);
  const started = useRef(false);
  const queued = useOutbox();
  const context: SayContext = { ...(typeof p.screen === 'string' ? { screen: p.screen } : {}), ...(typeof p.set === 'string' ? { set_id: p.set } : {}), ...(typeof p.joke === 'string' ? { joke_id: p.joke } : {}), ...(typeof p.board === 'string' ? { board_id: p.board } : {}) };
  const patch = (key: string, f: (e: Entry) => Entry) => setEntries((xs) => xs.map((x) => (x.key === key ? f(x) : x)));

  const send = useCallback(async (text: string, how: 'typed' | 'spoken', kind?: SayKind, key?: string, choice?: string) => {
    const k = key || `${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
    if (!key) setEntries((xs) => [...xs, { key: k, text, how, state: 'sending' }]);
    else patch(k, (e) => ({ ...e, state: 'sending', result: undefined, undone: false, error: undefined }));
    try {
      const r = await sayOrQueue(text, { kind, context, choice });
      if ('queued' in r) patch(k, (e) => ({ ...e, state: 'queued', queuedKey: r.queued.key }));
      else {
        noteWorker(r.result.worker);
        if (r.result.kind === 'command' && r.result.command?.done) haptics.success();
        patch(k, (e) => ({ ...e, state: 'done', result: r.result }));
      }
    } catch (e: any) {
      patch(k, (x) => ({ ...x, state: 'error', error: e instanceof ApiError ? e.message : String(e?.message || e) }));
    }
  }, [context.screen, context.set_id, context.joke_id, context.board_id]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    const t = typeof p.text === 'string' ? p.text.trim() : '';
    if (t) send(t, 'typed');
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (queued.length) flushOutbox((q: Queued, r: SayResult) => setEntries((xs) => xs.map((x) => (x.queuedKey === q.key ? { ...x, state: 'done', result: r } : x))));
  }, [queued.length]);
  useEffect(() => { setTimeout(() => sv.current?.scrollToEnd?.({ animated: true }), 80); }, [entries.length, entries.map((e) => e.state).join()]); // eslint-disable-line react-hooks/exhaustive-deps

  const undoBit = async (e: Entry) => {
    const r = e.result!;
    try {
      if (r.undo?.detach) await post('/api/merge/undo', { merge_id: r.undo.detach.merge_id }).catch(() => {});
      if (r.undo?.indexes?.length) await undo(r.undo.indexes);
      patch(e.key, (x) => ({ ...x, undone: true }));
    } catch (err: any) {
      ui.snack(`Couldn't undo: ${err.message}`);
    }
  };
  const notBit = async (e: Entry) => { await undoBit(e); send(e.text, e.how, 'question', e.key); };
  const undoCommand = async (e: Entry) => {
    const u = e.result?.command?.undo;
    try {
      if (u) await apiPatch(u.PATCH, u.body);
      patch(e.key, (x) => ({ ...x, undone: true }));
    } catch (err: any) {
      ui.snack(`Couldn't undo: ${err.message}`);
    }
  };
  const notCommand = async (e: Entry) => { if (e.result?.command?.done && !e.undone) await undoCommand(e); send(e.text, e.how, 'bit', e.key); };
  const commandAsk = async (e: Entry) => { if (e.result?.command?.done && !e.undone) await undoCommand(e); send(e.text, e.how, 'question', e.key); };
  const notQuestion = async (e: Entry) => {
    const id = e.result?.chat?.id;
    if (id) ask.hide(id).catch(() => {});
    send(e.text, e.how, 'bit', e.key);
  };
  const worker = [...entries].reverse().find((e) => e.result?.worker)?.result?.worker || null;

  return (
    <Screen title="Asking" back testID="say" scrollRef={sv} maxWidth={760} bar={<SayBar onSay={(t, how) => send(t, how)} worker={worker} testID="say" autoFocus={p.focus === '1' || !p.text} hint={null} />}>
      {!entries.length && <T v="mut" testID="say-empty">Say a bit, ask a question, or give an order (“new 7 for Friday”, “add the kale bit to tonight”). It knows which.</T>}
      <View testID="say-thread" style={{ gap: 18 }}>
        {entries.map((e) => (
          <View key={e.key} style={{ gap: 10 }}>
            <Said e={e} />
            {e.state === 'error' && <View style={{ alignSelf: 'flex-start', padding: 12, borderRadius: 14, backgroundColor: C.card2, gap: 6 }}><T v="mut">{e.error}</T><Btn small label="Try again" onPress={() => send(e.text, e.how, undefined, e.key)} /></View>}
            {e.state === 'done' && e.result?.kind === 'question' && <QuestionEntry e={e} onNotQuestion={() => notQuestion(e)} />}
            {e.state === 'done' && e.result?.kind === 'bit' && <BitEntry e={e} onUndo={() => undoBit(e)} onNotBit={() => notBit(e)} />}
            {e.state === 'done' && e.result?.kind === 'command' && <CommandEntry e={e} onChoice={(j) => send(e.text, e.how, undefined, e.key, j)} onUndo={() => undoCommand(e)} onNotCommand={() => notCommand(e)} onAskIt={() => commandAsk(e)} />}
          </View>
        ))}
      </View>
    </Screen>
  );
}
