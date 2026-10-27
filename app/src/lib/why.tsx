// The AI presentation system (UX_SPEC 3.4, P0-2): one Why sheet for every AI claim.
// Claim · one-line reason · evidence quotes with their source, each with Open (the doc paragraph) or ▶ Hear (the
// audio moment, from 4 s before) · confidence % (only here) · who said so · and the only Confirm / Reject in the
// app, phrased as answers: "This is right" / "Not this" / "Change…".
import React, { createContext, useCallback, useContext, useMemo, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { router } from 'expo-router';
import { get } from './api';
import { playSegment } from './media';
import { C, F, fmtMs } from './theme';
import { confWord } from './laughs';
import { AIBadge, Btn, Sheet, T, Yours, s, useUI } from './ui';

export type WhyEvidence = { node: string; quote: string; start_ms?: number | null; field?: string; jokeId?: string; label?: string };
export type WhyAnswer = () => Promise<void> | void;
export type Why = {
  title: string;
  claim: string;
  reason?: string | null;
  confidence?: number | null;
  who?: 'ai' | 'yours' | 'measured';
  whoLine?: string;
  evidence?: WhyEvidence[];
  detail?: React.ReactNode;
  change?: { label: string; run: () => void };
  right?: WhyAnswer;
  not?: WhyAnswer;
  testID?: string;
};
type Ctx = { why: (w: Why) => void; close: () => void };
const WhyCtx = createContext<Ctx | null>(null);
export const useWhy = () => {
  const c = useContext(WhyCtx);
  if (!c) throw new Error('useWhy outside WhyProvider');
  return c;
};

// ---- evidence → an audio moment
const analyses = new Map<string, Promise<any>>();
export function analysisOf(setId: string) {
  if (!analyses.has(setId)) analyses.set(setId, get<any>(`/api/sets/${encodeURIComponent(setId)}/analysis`).catch((e) => { analyses.delete(setId); throw e; }));
  return analyses.get(setId)!;
}
const norm = (t: string) => t.toLowerCase().replace(/[^a-z0-9 ]+/g, ' ').replace(/\s+/g, ' ').trim();
/** Where a quote was said: a punchline's own timestamp if it is one, else its place in the row's transcript. */
export async function momentOf(ev: WhyEvidence): Promise<{ audio: string; at_ms: number; set: string } | null> {
  let set: string | null = null;
  let idx: number | null = null;
  let at: number | null = ev.start_ms ?? null;
  const seg = ev.node.match(/^seg:(.+)#(\d+)$/);
  const perf = ev.node.match(/^perf:(.+)@(\d+)$/);
  if (seg) { set = `set:${seg[1]}`; idx = Number(seg[2]); }
  else if (perf) { set = `set:${perf[1]}`; at = at ?? Number(perf[2]); }
  if (!set) return null;
  const a = await analysisOf(set);
  const audio = a?.audio?.opus;
  if (!audio) return null;
  const rows: any[] = a.rows || [];
  const row = idx != null ? rows.find((r) => r.idx === idx) : rows.find((r) => at != null && r.start_ms <= at && at <= r.end_ms);
  if (row && at == null) {
    const q = norm(ev.quote);
    const pl = (row.punchlines || []).find((p: any) => norm(p.text).includes(q) || q.includes(norm(p.text)));
    if (pl) at = pl.at_ms;
    else if (typeof row.transcript === 'string' && row.transcript) {
      const i = norm(row.transcript).indexOf(q);
      const frac = i >= 0 ? i / Math.max(1, norm(row.transcript).length) : 0;
      at = Math.round(row.start_ms + frac * (row.end_ms - row.start_ms));
    } else at = row.start_ms;
  }
  return at == null ? null : { audio, at_ms: at, set };
}
export async function hearEvidence(ev: WhyEvidence, key: string) {
  const m = await momentOf(ev);
  if (!m) throw new Error('No audio for this moment');
  (globalThis as any).__punchableLastEvidence = { node: ev.node, at_ms: m.at_ms };
  await playSegment(m.audio, Math.max(0, m.at_ms - 4000), m.at_ms + 8000, key);
  return m;
}
/** Open the doc paragraph (written versions) or the set at that moment. */
export function openEvidence(ev: WhyEvidence) {
  const unit = ev.node.match(/^unit:/);
  if (unit && ev.jokeId) return router.push(`/bit/${encodeURIComponent(ev.jokeId)}?tab=versions&unit=${encodeURIComponent(ev.node)}` as any);
  const seg = ev.node.match(/^seg:(.+)#(\d+)$/);
  if (seg) return router.push(`/set/${encodeURIComponent(`set:${seg[1]}`)}?row=${seg[2]}` as any);
  const perf = ev.node.match(/^perf:(.+)@(\d+)$/);
  if (perf) return router.push(`/set/${encodeURIComponent(`set:${perf[1]}`)}?at=${perf[2]}` as any);
  if (ev.node.startsWith('joke:')) return router.push(`/joke/${encodeURIComponent(ev.node)}` as any);
}
export function evidenceSource(ev: WhyEvidence) {
  if (ev.label) return ev.label;
  if (ev.node.startsWith('unit:')) return 'your doc';
  if (ev.field) return 'your note';
  const seg = ev.node.match(/^(?:seg|perf):(\d{4})-(\d\d)-\d\d/);
  if (seg) return `on stage · ${seg[1]}`;
  return ev.node.split(':')[0];
}

function EvidenceRow({ ev, i, close }: { ev: WhyEvidence; i: number; close: () => void }) {
  const ui = useUI();
  const [at, setAt] = useState<number | null>(null);
  const audio = /^(seg|perf):/.test(ev.node);
  const doc = /^unit:/.test(ev.node) && !!ev.jokeId;
  return (
    <View testID={`why-ev-${i}`} style={[s.inner, { gap: 8 }]}>
      <T style={{ fontSize: 15 }}>“{ev.quote}”</T>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
        <T v="small" style={{ flex: 1, minWidth: 120 }}>{evidenceSource(ev)}{at != null ? ` · ${fmtMs(at)}` : ''}</T>
        {audio && (
          <Btn testID={`why-hear-${i}`} small label="▶ Hear" onPress={() => hearEvidence(ev, `why-${ev.node}-${i}`).then((m) => setAt(m.at_ms)).catch((e) => ui.snack(e.message))} />
        )}
        {(doc || audio) && <Btn testID={`why-open-${i}`} small v="ghost" label="Open" onPress={() => { close(); openEvidence(ev); }} />}
      </View>
    </View>
  );
}

export function WhyProvider({ children }: { children: React.ReactNode }) {
  const [w, setW] = useState<Why | null>(null);
  const close = useCallback(() => setW(null), []);
  const value = useMemo(() => ({ why: (x: Why) => setW(x), close }), [close]);
  const cw = confWord(w?.confidence);
  const answer = (f?: WhyAnswer) => async () => {
    setW(null);
    await f?.();
  };
  return (
    <WhyCtx.Provider value={value}>
      {children}
      <Sheet visible={!!w} onClose={close} label={w?.title || 'Why'} testID="why-sheet">
        {w && (
          <View style={{ gap: 12 }}>
            <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
              <T v="eb" style={{ flex: 1 }}>{w.title}</T>
              {w.who === 'yours' ? <Yours /> : w.who === 'ai' ? <AIBadge word={cw} /> : null}
            </View>
            <T v="h2" testID="why-claim">{w.claim}</T>
            {!!w.reason && <T style={{ color: C.text2 }} testID="why-reason">{w.reason}</T>}
            {w.detail}
            {!!w.evidence?.length && (
              <View style={{ gap: 8 }}>
                <T v="eb">Evidence</T>
                {w.evidence.map((ev, i) => <EvidenceRow key={`${ev.node}-${i}`} ev={ev} i={i} close={close} />)}
              </View>
            )}
            <T v="small" testID="why-who">
              {w.whoLine || (w.who === 'yours' ? 'You set this.' : w.who === 'measured' ? 'Measured from the tape.' : 'The AI read this.')}
              {w.confidence != null ? ` · ${Math.round(w.confidence * 100)}% sure (${cw})` : ''}
            </T>
            {(w.right || w.not || w.change) && (
              <View style={{ gap: 8, marginTop: 4 }}>
                {w.change && <Btn testID="why-change" label={w.change.label} onPress={() => { setW(null); w.change!.run(); }} />}
                <View style={{ flexDirection: 'row', gap: 8 }}>
                  {w.right && <Btn testID="why-right" label="This is right" v="primary" style={{ flex: 1 }} onPress={answer(w.right)} />}
                  {w.not && <Btn testID="why-not" label="Not this" v="danger" style={{ flex: 1 }} onPress={answer(w.not)} />}
                </View>
              </View>
            )}
            <Btn label="Close" v="ghost" onPress={close} />
          </View>
        )}
      </Sheet>
    </WhyCtx.Provider>
  );
}

/** One line of why under a claim; tap = the Why sheet. Max ~70 chars at rest. */
export function WhyLine({ text, onPress, testID, color = C.mut }: { text: string; onPress?: () => void; testID?: string; color?: string }) {
  const t = text.length > 72 ? `${text.slice(0, 69).trimEnd()}…` : text;
  return (
    <Pressable testID={testID} disabled={!onPress} onPress={onPress} accessibilityRole={onPress ? 'button' : undefined} accessibilityLabel={`${text}${onPress ? '. Why?' : ''}`}
      style={{ minHeight: 24, justifyContent: 'center' }}>
      <Text maxFontSizeMultiplier={1.4} numberOfLines={1} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color }}>{t}{onPress ? <Text style={{ color: C.dim }}>  ›</Text> : null}</Text>
    </Pressable>
  );
}
