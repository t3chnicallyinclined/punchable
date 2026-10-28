// Write (SIMPLE.md, the home): a blank box. Type or say an idea and his own related bits answer first (verbatim,
// dated, proven on stage first, 3 + "more"). "Check this" shows facts with their sources. His riff lines stack under
// the idea, each saved as an idea in his notebook and one ＋ from a board. The AI's ideas stay folded, dashed, drafts.
// Always opens blank; "pick up where you left off" is a line, not a restored draft.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Linking, Platform, Pressable, Text, TextInput, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { post, undo, useApi } from '@/lib/api';
import { AskAnswer, BoardListItem, TodayFeed, creative, useCreative } from '@/lib/creative';
import { useBoards } from '@/lib/boards';
import { BitCard, Facts, Related, Riff, draft, isQuestion, monthYear, quoteText, simple } from '@/lib/simple';
import { C, F } from '@/lib/theme';
import { WorkerLine } from '@/lib/worker';
import { AIBadge, AddBtn, Avatar, Btn, Card, Eb, Icon, T, TextBtn, s, useUI, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

type Line = { key: string; text: string; id?: string; indexes?: number[]; state: 'saving' | 'saved' | 'error'; into?: string };

// ---- voice: the browser's speech recognition when it has one; on the phone, the voice-note recorder
function useDictation(onText: (t: string) => void) {
  const rec = useRef<any>(null);
  const [on, setOn] = useState(false);
  const Ctor = Platform.OS === 'web' ? ((globalThis as any).SpeechRecognition || (globalThis as any).webkitSpeechRecognition) : null;
  const supported = !!Ctor;
  const toggle = useCallback(() => {
    if (!Ctor) return;
    if (rec.current) { rec.current.stop(); return; }
    const r = new Ctor();
    r.lang = 'en-US';
    r.interimResults = false;
    r.continuous = false;
    r.onresult = (e: any) => {
      const t = Array.from(e.results || []).map((x: any) => x[0]?.transcript || '').join(' ').trim();
      if (t) onText(t);
    };
    r.onend = () => { rec.current = null; setOn(false); };
    r.onerror = () => { rec.current = null; setOn(false); };
    rec.current = r;
    setOn(true);
    try { r.start(); } catch { rec.current = null; setOn(false); }
  }, [Ctor, onText]);
  useEffect(() => () => rec.current?.stop?.(), []);
  return { supported, on, toggle };
}

function MicBtn({ testID, onText }: { testID: string; onText: (t: string) => void }) {
  const ui = useUI();
  const d = useDictation(onText);
  if (Platform.OS === 'web' && !d.supported) return null; // voice is optional on the web
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={d.on ? 'Stop listening' : 'Say it'}
      onPress={() => (d.supported ? d.toggle() : ui.quickVoiceNote())}
      style={{ width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center', backgroundColor: d.on ? C.amber : 'transparent' }}>
      <Icon name="mic" size={20} color={d.on ? C.bg : C.amber} />
    </Pressable>
  );
}

/** One of his bits: title · date, his line (solid, amber edge), one stage line in words, and ＋. */
export function BitRow({ b, testID, onAdd }: { b: BitCard; testID: string; onAdd: () => void }) {
  return (
    <View testID={testID} style={{ flexDirection: 'row', gap: 10, alignItems: 'flex-start', paddingVertical: 10, borderTopWidth: 1, borderTopColor: C.border }}>
      <Pressable accessibilityRole="button" accessibilityLabel={`${b.title}. Open its map`} onPress={() => router.push(`/joke/${encodeURIComponent(b.joke_id)}` as any)} style={{ flex: 1, minWidth: 0, gap: 4 }}>
        <T v="semi" numberOfLines={1}>{b.title}{b.date ? <Text style={{ fontFamily: F.body, color: C.mut }}> · {monthYear(b.date)}</Text> : null}</T>
        {!!b.quote && <View style={s.punch}><T numberOfLines={4} style={{ fontSize: 15 }}>{quoteText(b)}</T></View>}
        {!!b.stage_line && <T v="small" style={{ color: b.proven ? C.green : C.mut }}>{b.stage_line}</T>}
      </Pressable>
      <AddBtn testID={`${testID}-add`} onPress={onAdd} label={`Add ${b.title} to a board`} />
    </View>
  );
}

export default function Write() {
  const ui = useUI();
  const boards = useBoards();
  const wide = useWide();
  const p = useLocalSearchParams<{ idea?: string }>();
  const [idea, setIdea] = useState(p.idea ? String(p.idea) : '');
  const [q, setQ] = useState('');
  const [more, setMore] = useState(false);
  const [facts, setFacts] = useState<{ state: 'asking' | 'done' | 'error'; r?: Facts; error?: string } | null>(null);
  const [ask, setAsk] = useState<{ q: string; state: 'asking' | 'done' | 'error'; a?: AskAnswer & { status?: string } } | null>(null);
  const [lines, setLines] = useState<Line[]>([]);
  const [line, setLine] = useState('');
  const [aiOpen, setAiOpen] = useState(false);
  const [riff, setRiff] = useState<{ state: 'asking' | 'done' | 'error'; r?: Riff; error?: string } | null>(null);
  const [usedDrafts, setUsedDrafts] = useState<Set<string>>(new Set());
  const lineRef = useRef<any>(null);

  // "Riff on this" from a joke seeds the box (derived from the param, not restored from a draft)
  const [seededFrom, setSeededFrom] = useState(p.idea ? String(p.idea) : '');
  if (p.idea && String(p.idea) !== seededFrom) {
    setSeededFrom(String(p.idea));
    setIdea(String(p.idea));
  }
  useEffect(() => {
    const t = setTimeout(() => {
      const nq = idea.trim();
      if (nq === q) return;
      setQ(nq);
      setMore(false); setFacts(null); setRiff(null); setAiOpen(false);
    }, 450);
    return () => clearTimeout(t);
  }, [idea, q]);

  const rel = useApi<Related>(q.length >= 3 ? `/api/notebook/related?q=${encodeURIComponent(q)}&cap=10` : null, { cache: false });
  const blank = !idea.trim() && !lines.length;

  // empty state: the board he was on, one old idea that fits, new set videos
  const bl = useCreative<{ boards: BoardListItem[] }>(blank ? 'boards' : null, creative.boards);
  const feed = useCreative<TodayFeed>(blank ? 'today' : null, creative.today);
  const curId = boards.current?.id;
  const boardList = bl.data?.boards;
  const lastBoard = (boardList || []).find((b) => b.id === curId) || boardList?.[0] || null;
  const gem = (feed.data?.serendipity || [])[0];

  const pickBoard = (jokeId: string, title: string) => boards.pick(jokeId, title);

  const saveLine = async (text: string, draft = false) => {
    const t = text.trim();
    if (!t) return;
    const key = `${Date.now()}-${lines.length}`;
    setLines((xs) => [...xs, { key, text: t, state: 'saving' }]);
    setLine('');
    try {
      const w = await simple.saveIdea(t, idea.trim() || t, draft);
      const att = w.attach?.decision === 'attach' && w.attach.into ? w.attach : null;
      // filed as a note of an existing bit (the merge module): ＋ sends that bit to a board
      setLines((xs) => xs.map((x) => (x.key === key ? { ...x, id: att ? att.into : w.id, indexes: w.indexes, state: 'saved', into: att?.title } : x)));
      if (att) {
        ui.snack(`Added to ${(att.title || 'that bit').split(' (')[0]}`, async () => {
          await post('/api/merge/undo', { merge_id: att.id });
          setLines((xs) => xs.map((x) => (x.key === key ? { ...x, id: w.id, into: undefined } : x)));
        });
      } else {
        ui.snack('Saved to your notebook', async () => {
          await undo(w.indexes);
          setLines((xs) => xs.filter((x) => x.key !== key));
        });
      }
    } catch (e: any) {
      setLines((xs) => xs.map((x) => (x.key === key ? { ...x, state: 'error' } : x)));
      ui.snack(`Couldn't save the line: ${e.message}`);
    }
  };

  const checkThis = async () => {
    setFacts({ state: 'asking' });
    try {
      const r = await draft<Facts>(() => simple.facts(q || idea.trim()), () => setFacts({ state: 'done', r: { status: 'queued', facts: [] } }));
      setFacts({ state: 'done', r });
    } catch (e: any) { setFacts({ state: 'error', error: e.message }); }
  };
  const runAsk = async () => {
    const question = idea.trim();
    setAsk({ q: question, state: 'asking' });
    try {
      const a = await draft<any>(() => creative.ask(question), () => setAsk({ q: question, state: 'done', a: { status: 'queued' } }));
      setAsk({ q: question, state: 'done', a });
    } catch { setAsk({ q: question, state: 'error' }); }
  };
  const openAi = async () => {
    const open = !aiOpen;
    setAiOpen(open);
    if (!open || riff?.state === 'done') return;
    setRiff({ state: 'asking' });
    try {
      const r = await draft<Riff>(() => simple.riff(idea.trim(), lines.map((l) => l.text)), () => setRiff({ state: 'done', r: { status: 'queued', ideas: [] } }));
      setRiff({ state: 'done', r });
    } catch (e: any) { setRiff({ state: 'error', error: e.message }); }
  };

  const header = (
    <View style={{ flexDirection: 'row', alignItems: 'center', paddingHorizontal: wide ? 32 : 16, paddingTop: 12, paddingBottom: 8, borderBottomWidth: 1, borderBottomColor: C.border, backgroundColor: C.bg }}>
      <T v="h1" style={{ flex: 1 }}>Write</T>
      <Avatar />
    </View>
  );

  const box = (
    <View style={{ flexDirection: 'row', alignItems: 'flex-start', gap: 4, borderWidth: 1, borderColor: idea ? C.amberMid : C.border, borderRadius: 14, backgroundColor: C.surface, paddingLeft: 14, paddingRight: 4, paddingVertical: 4 }}>
      <TextInput testID="write-idea" value={idea} onChangeText={(t) => { setIdea(t); if (ask && t.trim() !== ask.q) setAsk(null); }} multiline
        placeholder="What's the idea?" placeholderTextColor={C.dim} accessibilityLabel="What's the idea?"
        style={{ flex: 1, minHeight: 96, paddingTop: 12, paddingBottom: 12, color: C.text, fontFamily: F.semi, fontSize: 18, lineHeight: 25, textAlignVertical: 'top', outlineStyle: 'none' } as any} />
      <MicBtn testID="write-mic" onText={(t) => setIdea((x) => (x ? `${x.trimEnd()} ${t}` : t))} />
    </View>
  );

  const empty = blank ? (
    <View testID="write-empty" style={{ gap: 16 }}>
      {ui.importCount > 0 && (
        <Pressable testID="write-import" accessibilityRole="button" onPress={() => router.push('/import' as any)} style={[s.inner, s.row, { minHeight: 52 }]}>
          <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: C.amber }} />
          <T v="semi" style={{ flex: 1 }}>{ui.importCount} new set {ui.importCount === 1 ? 'video' : 'videos'}</T>
          <T style={{ color: C.amber }}>Import ›</T>
        </Pressable>
      )}
      {lastBoard && (
        <View style={{ gap: 6 }}>
          <Eb>Pick up where you left off</Eb>
          <Pressable testID="write-pickup" accessibilityRole="button" onPress={() => router.push(`/board/${encodeURIComponent(lastBoard.id)}` as any)} style={[s.inner, s.row, { minHeight: 52 }]}>
            <View style={{ flex: 1, minWidth: 0 }}>
              <T v="semi" numberOfLines={1}>{lastBoard.title}</T>
              <T v="small">Board · {lastBoard.n} {lastBoard.n === 1 ? 'bit' : 'bits'}</T>
            </View>
            <Text style={{ color: C.dim, fontSize: 20 }}>›</Text>
          </Pressable>
        </View>
      )}
      {gem && (
        <View style={{ gap: 6 }} testID="write-gem">
          <Eb>An old idea that fits</Eb>
          <View style={[s.inner, s.row, { alignItems: 'flex-start' }]}>
            <Pressable accessibilityRole="button" onPress={() => router.push(`/joke/${encodeURIComponent(gem.joke_id)}` as any)} style={{ flex: 1, minWidth: 0, gap: 4 }}>
              <T v="semi" numberOfLines={1}>{gem.title.split(' (')[0]}</T>
              {!!gem.evidence?.[0]?.quote && <View style={s.punch}><T numberOfLines={3} style={{ fontSize: 15 }}>{gem.evidence[0].quote}</T></View>}
            </Pressable>
            <AddBtn testID="write-gem-add" onPress={() => pickBoard(gem.joke_id, gem.title.split(' (')[0])} />
          </View>
        </View>
      )}
    </View>
  ) : null;

  const items = rel.data?.items || [];
  const shown = more ? items : items.slice(0, 3);
  const notebook = q.length >= 3 ? (
    <View testID="write-notebook" style={{ gap: 2 }}>
      <Eb>From your notebook</Eb>
      {rel.loading && !rel.data && <T v="mut">Looking through your notebook…</T>}
      {rel.data && !items.length && <T v="mut" testID="write-notebook-none">Nothing in your notebook on this yet. It’s new.</T>}
      {shown.map((b, i) => <BitRow key={b.joke_id} b={b} testID={`nb-${i}`} onAdd={() => pickBoard(b.joke_id, b.title)} />)}
      {items.length > 3 && !more && <TextBtn testID="nb-more" label={`${items.length - 3} more from your notebook ›`} onPress={() => setMore(true)} />}
      {rel.error && !rel.data && <T v="mut">Your notebook loads when you’re online.</T>}
    </View>
  ) : null;

  const question = q.length >= 3 && isQuestion(idea) ? (
    ask && ask.q === idea.trim() ? (
      <Card testID="write-answer" style={{ gap: 8 }}>
        <View style={s.row}><T v="eb" style={{ flex: 1 }}>From your material</T><AIBadge /></View>
        {ask.state === 'asking' && <T v="mut">Looking through your material…</T>}
        {ask.state === 'error' && <T v="mut">The AI didn’t answer. Your own bits are below.</T>}
        {ask.state === 'done' && ask.a?.status === 'queued' && <><T v="mut" testID="write-answer-queued">The writer is on it. The answer shows up here when it’s done.</T><WorkerLine what="The answer" /></>}
        {ask.state === 'done' && ask.a?.status !== 'queued' && (ask.a?.not_found || !ask.a?.answer ? <T>Not in your material.</T> : <T>{ask.a.answer}</T>)}
        {ask.state === 'done' && (ask.a?.sources || []).slice(0, 3).map((src, i) => (
          <View key={i} style={s.punch}><T style={{ fontSize: 15 }}>“{src.quote}”</T><T v="small">{src.title || ''}{src.date ? ` · ${monthYear(src.date)}` : ''}</T></View>
        ))}
      </Card>
    ) : <Btn testID="write-ask" small label="Answer this from my notebook" onPress={runAsk} style={{ alignSelf: 'flex-start' }} />
  ) : null;

  const factsBlock = q.length >= 3 ? (
    <View testID="write-facts" style={{ gap: 8 }}>
      {!facts && <Btn testID="write-check" small label="Check this" onPress={checkThis} style={{ alignSelf: 'flex-start' }} />}
      {facts && <View style={s.row}><T v="eb" style={{ flex: 1 }}>Facts</T><AIBadge /></View>}
      {facts?.state === 'asking' && <T v="mut">Checking…</T>}
      {facts?.state === 'error' && <T v="mut">The fact check didn’t run ({facts.error}).</T>}
      {facts?.state === 'done' && facts.r?.status === 'queued' && <><T v="mut" testID="write-facts-queued">The writer checks facts against their sources. They show up here when it’s done.</T><WorkerLine what="The fact check" /></>}
      {facts?.state === 'done' && (facts.r?.status === 'none' || facts.r?.status === 'provider_error') && <T v="mut">No fact it could back with a source.</T>}
      {facts?.state === 'done' && (facts.r?.facts || []).map((f, i) => (
        <View key={i} testID={`fact-${i}`} style={[s.inner, { gap: 4 }]}>
          <T style={{ fontSize: 15 }}>{f.text}</T>
          <Pressable accessibilityRole="link" onPress={() => Linking.openURL(f.source_url).catch(() => {})} style={{ minHeight: 32, justifyContent: 'center' }}>
            <T v="small" style={{ color: C.amber }}>{f.source_title} ›</T>
          </Pressable>
        </View>
      ))}
    </View>
  ) : null;

  const riffBlock = idea.trim() || lines.length ? (
    <View testID="write-riff" style={{ gap: 8 }}>
      <Eb>Your riff</Eb>
      {lines.map((l, i) => (
        <View key={l.key} testID={`riff-${i}`} style={[s.row, { alignItems: 'flex-start' }]}>
          <View style={[s.punch, { flex: 1, paddingVertical: 4 }]}>
            <T>{l.text}</T>
            {l.state !== 'saved' && <T v="small">{l.state === 'saving' ? 'Saving…' : 'Not saved'}</T>}
            {!!l.into && <T v="small" testID={`riff-${i}-into`}>A note on {l.into.split(' (')[0]}</T>}
          </View>
          {l.id ? <AddBtn testID={`riff-${i}-add`} onPress={() => pickBoard(l.id!, l.text.split(/\s+/).slice(0, 6).join(' '))} label="Add this line to a board" /> : <View style={{ width: 44 }} />}
        </View>
      ))}
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4, borderWidth: 1, borderColor: C.border, borderRadius: 12, paddingLeft: 12, paddingRight: 4, backgroundColor: C.surface }}>
        <TextInput ref={lineRef} testID="riff-input" value={line} onChangeText={setLine} placeholder="next line…" placeholderTextColor={C.dim}
          onSubmitEditing={() => { saveLine(line); setTimeout(() => lineRef.current?.focus(), 50); }} blurOnSubmit={false} returnKeyType="done"
          accessibilityLabel="Next line of your riff" style={{ flex: 1, minHeight: 48, color: C.text, fontFamily: F.body, fontSize: 16, outlineStyle: 'none' } as any} />
        <MicBtn testID="riff-mic" onText={(t) => setLine((x) => (x ? `${x.trimEnd()} ${t}` : t))} />
        {!!line.trim() && <TextBtn testID="riff-save" label="Save" onPress={() => saveLine(line)} />}
      </View>
      {!!idea.trim() && (
        <View testID="write-ai" style={[s.draft, { gap: 8, padding: 0 }]}>
          <Pressable testID="write-ai-toggle" accessibilityRole="button" accessibilityState={{ expanded: aiOpen }} onPress={openAi}
            style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 48, paddingHorizontal: 12 }}>
            <T v="small" style={{ flex: 1, color: C.amber }}>{aiOpen ? '▾' : '▸'} Ideas from the AI · not your words yet</T>
          </Pressable>
          {aiOpen && (
            <View style={{ gap: 8, paddingHorizontal: 12, paddingBottom: 12 }}>
              {riff?.state === 'asking' && <T v="mut">Asking…</T>}
              {riff?.state === 'error' && <T v="mut">The AI didn’t answer ({riff.error}).</T>}
              {riff?.state === 'done' && riff.r?.status === 'queued' && <><T v="mut" testID="write-ai-queued">The writer drafts these. They show up here when it’s done.</T><WorkerLine what="The riff" /></>}
              {riff?.state === 'done' && riff.r?.status === 'no_grounding' && <T v="mut">Write a line first. The AI builds on your words.</T>}
              {riff?.state === 'done' && (riff.r?.ideas || []).filter((d) => !usedDrafts.has(d.text)).map((d, i) => (
                <View key={i} testID={`ai-idea-${i}`} style={{ gap: 6, borderTopWidth: i ? 1 : 0, borderTopColor: C.draftBorder, paddingTop: i ? 8 : 0 }}>
                  <View style={{ alignSelf: 'flex-start', paddingHorizontal: 6, minHeight: 20, borderRadius: 4, backgroundColor: C.addBg, justifyContent: 'center' }}><Text style={{ fontFamily: F.g7, fontSize: 11, color: C.amber }}>Draft</Text></View>
                  <Text maxFontSizeMultiplier={1.5} style={{ fontFamily: F.body, fontSize: 16, lineHeight: 23, color: C.amber }}>{d.text}</Text>
                  <View style={{ flexDirection: 'row', gap: 8 }}>
                    <Btn testID={`ai-idea-${i}-use`} small label="Use as mine" onPress={() => { setUsedDrafts((x) => new Set(x).add(d.text)); saveLine(d.text, true); }} />
                    <Btn testID={`ai-idea-${i}-dismiss`} small v="ghost" label="Dismiss" onPress={() => { setUsedDrafts((x) => new Set(x).add(d.text)); simple.dismissDraft(d.text); }} />
                  </View>
                </View>
              ))}
            </View>
          )}
        </View>
      )}
    </View>
  ) : null;

  return (
    <Screen title="Write" header={header} testID="write">
      {box}
      {question}
      {empty}
      {wide && (q.length >= 3 || lines.length) ? (
        <View style={{ flexDirection: 'row', gap: 24, alignItems: 'flex-start' }}>
          <View style={{ flex: 1, minWidth: 0, gap: 16 }}>{riffBlock}</View>
          <View style={{ flex: 1, minWidth: 0, gap: 16 }}>{notebook}{factsBlock}</View>
        </View>
      ) : (
        <>{notebook}{factsBlock}{riffBlock}</>
      )}
    </Screen>
  );
}
