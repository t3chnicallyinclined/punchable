// Working set (UX_SPEC 4.5, 6): what's my set right now, how is it doing night over night, what do I try tonight.
import React, { useMemo, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { Alignment, BitHistory, WSBitRow, WSView, creative, useCreative } from '@/lib/creative';
import { DragList, move } from '@/lib/DragList';
import { fmt1, fmtDur, fmtShortDate, fmtVs, stabilityWord, vsBand } from '@/lib/laughs';
import { MiniStrip, PerfectedMeter } from '@/lib/metrics';
import { C, F } from '@/lib/theme';
import { AIBadge, Btn, Card, Empty, ErrorCard, FixtureBanner, Icon, IconBtn, List, Sheet, T, TextBtn, s, useUI, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TextInput } from 'react-native';

const DONE_KEY = 'pk.tryDone.v1';
function readDone(): Record<string, boolean> {
  try { return JSON.parse((globalThis as any).localStorage?.getItem(DONE_KEY) || '{}'); } catch { return {}; }
}

export default function WorkingSetPage() {
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const wide = useWide();
  const v = useCreative<WSView>(`ws:${id}`, () => creative.workingset(id));
  const n = useCreative<{ nights: Alignment[] }>(`nights:${id}`, () => creative.nights(id));
  const [order, setOrder] = useState<string[] | null>(null);
  const [saveSheet, setSaveSheet] = useState(false);
  const [bit, setBit] = useState<WSBitRow | null>(null);
  const [showVersions, setShowVersions] = useState(false);
  const [done, setDone] = useState<Record<string, boolean>>(readDone);

  const nights = useMemo(() => (n.data?.nights || []).filter((a) => a.totals.bits_performed > 0).sort((a, b) => a.date.localeCompare(b.date)), [n.data]);
  if (v.error && !v.data) return <Screen title="Working set" back><ErrorCard error={v.error} onRetry={v.reload} /></Screen>;
  if (!v.data) return <Screen title="Working set" back><Empty text="Loading the Working set…" /></Screen>;
  const W = v.data;
  const ws = W.working_set;
  const rowsById = new Map(W.bits.map((b) => [b.joke_id, b]));
  const ids = order || W.bits.map((b) => b.joke_id);
  const rows = ids.map((j) => rowsById.get(j)!).filter(Boolean);
  const planned = rows.reduce((a, b) => a + b.target_s, 0);
  const over = planned - W.version.target_s;
  const locked = rows.filter((b) => b.locked);
  const lockedCv = (() => {
    const xs = W.perfected.bits.filter((b) => locked.some((l) => l.joke_id === b.joke_id) && b.cv_laugh_s_per_min != null);
    if (!xs.length) return null;
    return { cv: xs.reduce((a, b) => a + (b.cv_laugh_s_per_min || 0), 0) / xs.length, n: Math.min(5, Math.min(...xs.map((b) => b.nights))) };
  })();
  const word = lockedCv ? stabilityWord(lockedCv.cv, lockedCv.n) : 'nothing locked yet';
  const strip = (jid: string, k = 5) => nights.slice(-k).map((a) => {
    const b = a.bits.find((x) => x.joke_id === jid);
    return b && b.performed ? vsBand(b.vs_room ?? null) : null;
  });
  const last3 = nights.slice(-3).reverse();

  const lock = async (b: WSBitRow, line?: string) => {
    const was = line ? b.locked_lines.some((l) => l.text === line) : b.locked;
    try {
      const nv = await creative.lock(id, { joke_id: b.joke_id, ...(line ? { line } : {}), unlock: was });
      if ((nv as any)?.bits) v.setData(nv as WSView); else v.reload();
      ui.snack(`${was ? 'Unlocked' : 'Locked'} ${line ? 'the line' : b.title.split(' (')[0]}`, async () => {
        const r = await creative.lock(id, { joke_id: b.joke_id, ...(line ? { line } : {}), unlock: !was });
        if ((r as any)?.bits) v.setData(r as WSView); else v.reload();
      });
    } catch (e: any) {
      ui.snack(`Couldn't lock: ${e.message}`);
    }
  };
  const tick = (k: string) => {
    const nd = { ...done, [k]: !done[k] };
    setDone(nd);
    try { (globalThis as any).localStorage?.setItem(DONE_KEY, JSON.stringify(nd)); } catch {}
  };

  const header = (
    <View style={{ gap: 6 }}>
      <T v="eb" style={{ color: C.amber }}>★ Working set · v{ws.current}</T>
      <T v="h1" testID="ws-title">{ws.title}</T>
      <T v="mut">{W.version.note}</T>
      <T testID="ws-length" style={{ fontSize: 15 }}>{fmtDur(planned)} planned / {fmtDur(W.version.target_s)} light{Math.abs(over) >= 5 ? `  ${over > 0 ? '▲' : '▼'} ${fmtDur(Math.abs(over))} ${over > 0 ? 'long' : 'short'}` : ''}</T>
      <PerfectedMeter testID="perfected" segments={rows.map((b) => ({ key: b.joke_id, s: b.target_s, locked: b.locked }))} lockedShare={planned ? locked.reduce((a, b) => a + b.target_s, 0) / planned : 0}
        word={`${locked.length} of ${rows.length} locked · ${word}`} />
    </View>
  );

  const lastNights = (
    <Card testID="last-nights" style={{ gap: 6 }}>
      <View style={s.row}><T v="eb" style={{ flex: 1 }}>Last {last3.length} {last3.length === 1 ? 'night' : 'nights'}</T><AIBadge /></View>
      {!last3.length && <T v="mut">Perform it and import the video; we’ll line it up.</T>}
      {last3.map((a, i) => {
        const prev = nights[nights.length - 2 - i];
        const up = prev && a.totals.ws_laugh_s_per_min != null && prev.totals.ws_laugh_s_per_min != null ? a.totals.ws_laugh_s_per_min - prev.totals.ws_laugh_s_per_min : null;
        return (
          <Pressable key={a.set} testID={`night-${i}`} accessibilityRole="button" onPress={() => router.push(`/set/${encodeURIComponent(`set:${a.set.replace(/^set:/, '')}`)}` as any)}
            style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44 }}>
            <T v="num" style={{ width: 70, fontSize: 13 }}>{fmtShortDate(a.date)}</T>
            <T style={{ flex: 1 }} numberOfLines={1}>{a.venue_label}</T>
            <T v="num" style={{ fontSize: 14 }}>{fmt1(a.totals.ws_laugh_s_per_min)}{up != null && Math.abs(up) >= 0.3 ? (up > 0 ? ' ▲' : ' ▼') : ''}</T>
            <T v="small">on plan {a.order.in_order.length}/{a.order.planned.length}</T>
          </Pressable>
        );
      })}
      {nights.length > 0 && nights.length < 3 && <T v="small">{nights.length} {nights.length === 1 ? 'night' : 'nights'} · early read</T>}
    </Card>
  );

  const bitRow = (b: WSBitRow, i: number, handle?: React.ReactNode) => {
    const last = b.last_night;
    const open = b.notes.filter((t) => !done[`${t.joke_id}|${t.kind}`]).length;
    return (
      <View testID={`ws-bit-${i}`} style={{ flexDirection: 'row', alignItems: 'center', gap: 6, minHeight: 60, borderTopWidth: i ? 1 : 0, borderTopColor: C.border }}>
        {handle}
        <T v="num" style={{ width: 18, fontSize: 14, color: C.mut }}>{i + 1}</T>
        <Pressable testID={`ws-open-${i}`} style={{ flex: 1, minWidth: 0, minHeight: 44, justifyContent: 'center', gap: 3, paddingVertical: 6 }} accessibilityRole="button" onPress={() => setBit(b)}>
          <T v="semi" numberOfLines={1}>{b.title.split(' (')[0]}</T>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
            <T v="small">{fmtDur(b.target_s)}{last?.time_s != null ? ` · last ${fmtDur(last.time_s)}${last.time_s > b.target_s * 1.15 ? ' ▲ long' : ''}` : ''}</T>
            <MiniStrip testID={`strip-${i}`} bands={strip(b.joke_id)} size={12} />
          </View>
        </Pressable>
        <View style={{ width: 28, alignItems: 'center' }}>
          {b.locked ? <Icon name="lock" size={16} color={C.amber} /> : open ? <T v="small">{open}</T> : null}
        </View>
        {!order && !wide && (
          <View>
            <IconBtn testID={`ws-up-${i}`} label="Move up" disabled={i === 0} onPress={() => setOrder(move(ids, i, i - 1))}><Icon name="up" size={16} color={C.mut} /></IconBtn>
          </View>
        )}
      </View>
    );
  };

  const bitsBlock = (
    <Card testID="ws-bits" style={{ gap: 0 }}>
      <View style={[s.row, { marginBottom: 6 }]}>
        <T v="eb" style={{ flex: 1 }}>The bits, in order</T>
        <T v="small">last {Math.min(5, nights.length)} nights ▸</T>
      </View>
      <DragList items={rows} keyOf={(b) => b.joke_id} onReorder={(f, t) => setOrder(move(ids, f, t))} render={(b, i, h) => bitRow(b, i, h)} />
      {order && (
        <View testID="unsaved-order" style={[s.inner, { marginTop: 10, gap: 8 }]}>
          <T v="semi">New order, not saved</T>
          <View style={{ flexDirection: 'row', gap: 8 }}>
            <Btn testID="save-version" small v="primary" label={`Save as v${ws.current + 1}`} onPress={() => setSaveSheet(true)} />
            <Btn small v="ghost" label="Undo order" onPress={() => setOrder(null)} />
          </View>
        </View>
      )}
      <T v="small" style={{ marginTop: 8 }}>Cells: full ≥ 1.3× the room · two-thirds 0.8–1.3× · one-third under 0.8× · empty = not performed.</T>
    </Card>
  );

  const tries = W.try_tonight;
  const tryBlock = (
    <Card testID="try-tonight" style={{ gap: 4 }}>
      <View style={s.row}><T v="eb" style={{ flex: 1 }}>Try tonight ({tries.length})</T><AIBadge /></View>
      {!tries.length && <T v="mut">Nothing to try: every bit is locked or holding.</T>}
      {tries.map((t, i) => {
        const k = `${t.joke_id}|${t.kind}`;
        return (
          <Pressable key={k} testID={`try-${i}`} accessibilityRole="checkbox" accessibilityState={{ checked: !!done[k] }} aria-checked={!!done[k]} onPress={() => tick(k)} style={{ flexDirection: 'row', gap: 10, minHeight: 48, alignItems: 'flex-start', paddingVertical: 6 }}>
            <Text style={{ fontSize: 18, color: done[k] ? C.amber : C.mut, width: 22 }}>{done[k] ? '☑' : '☐'}</Text>
            <View style={{ flex: 1 }}>
              <T style={{ fontSize: 15, textDecorationLine: done[k] ? 'line-through' : 'none', color: done[k] ? C.mut : C.text }}>{t.text}</T>
              {t.low_n && <T v="small">early read</T>}
            </View>
          </Pressable>
        );
      })}
    </Card>
  );

  const grid = wide && nights.length ? (
    <Card testID="heat-grid" style={{ gap: 6 }}>
      <T v="eb">Night by night</T>
      <View style={{ flexDirection: 'row', gap: 4, paddingLeft: 150 }}>{nights.slice(-8).map((a) => <T key={a.set} v="small" style={{ width: 34, textAlign: 'center' }}>{a.date.slice(2, 4)}</T>)}</View>
      {rows.map((b) => (
        <View key={b.joke_id} style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
          <T numberOfLines={1} style={{ width: 146, fontSize: 13 }}>{b.title.split(' (')[0]}</T>
          <MiniStrip bands={strip(b.joke_id, 8)} size={30} />
        </View>
      ))}
    </Card>
  ) : null;

  return (
    <Screen title="Working set" back testID="workingset"
      dock={<Btn testID="open-setlist" label="Set list for tonight" v="primary" onPress={() => router.push(`/setlist/${encodeURIComponent(id)}` as any)} />}>
      <FixtureBanner show={v.fixture || n.fixture} what="GET /api/workingsets/{id}, /nights" />
      {header}
      {wide ? (
        <View style={{ flexDirection: 'row', gap: 20, alignItems: 'flex-start' }}>
          <View style={{ flex: 3, gap: 16, minWidth: 0 }}>{bitsBlock}{grid}</View>
          <View style={{ flex: 2, gap: 16, minWidth: 0 }}>{lastNights}{tryBlock}</View>
        </View>
      ) : (<>{lastNights}{bitsBlock}{tryBlock}</>)}
      <Card style={{ gap: 4 }} testID="versions-history">
        <TextBtn testID="versions-toggle" label={`${showVersions ? '▾' : '▸'} Versions (${ws.versions.length})`} color={C.mut} onPress={() => setShowVersions((x) => !x)} />
        {showVersions && ws.versions.slice().reverse().map((ver) => <T key={ver.v} v="small">v{ver.v} · {fmtShortDate(ver.created_at)} · {ver.note} · {ver.bits.length} bits</T>)}
      </Card>
      <SaveVersionSheet visible={saveSheet} onClose={() => setSaveSheet(false)} next={ws.current + 1} onSave={async (note) => {
        setSaveSheet(false);
        try {
          const nv = await creative.newVersion(id, note, ids);
          setOrder(null);
          if ((nv as any)?.bits) v.setData(nv as WSView); else v.reload();
          ui.snack(`Saved v${ws.current + 1}`);
        } catch (e: any) { ui.snack(`Couldn't save: ${e.message}`); }
      }} />
      <BitSheet b={bit} wsId={id} onClose={() => setBit(null)} onLock={lock} />
    </Screen>
  );
}

function SaveVersionSheet({ visible, onClose, next, onSave }: { visible: boolean; onClose: () => void; next: number; onSave: (note: string) => void }) {
  const [note, setNote] = useState('');
  return (
    <Sheet visible={visible} onClose={onClose} label={`Save as v${next}`} testID="save-version-sheet">
      <T v="h2">Save as v{next}</T>
      <T v="mut" style={{ marginVertical: 6 }}>What changed? (e.g. swapped Parking meter above Airport)</T>
      <TextInput testID="version-note" value={note} onChangeText={setNote} style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      <Btn testID="version-save" label="Save" v="primary" disabled={!note.trim()} onPress={() => onSave(note.trim())} style={{ marginTop: 8 }} />
    </Sheet>
  );
}

function BitSheet({ b, wsId, onClose, onLock }: { b: WSBitRow | null; wsId: string; onClose: () => void; onLock: (b: WSBitRow, line?: string) => void }) {
  const h = useCreative<BitHistory>(b ? `bit:${wsId}:${b.joke_id}` : null, () => creative.bit(wsId, b!.joke_id));
  const [pu, setPu] = useState<any>(null);
  if (!b) return <Sheet visible={false} onClose={onClose} label="Bit">{null}</Sheet>;
  const H = h.data;
  const lines = [...new Set([...(H?.wording?.effects || []).flatMap((e) => e.examples || []), ...(pu?.drafts || []).map((d: any) => d.target_line)])].slice(0, 6) as string[];
  return (
    <Sheet visible onClose={onClose} label={b.title} testID="bit-sheet">
      <View style={{ gap: 10 }}>
        <View style={s.row}><T v="h2" style={{ flex: 1 }}>{b.title.split(' (')[0]}</T><Btn small v="ghost" label="Open bit" onPress={() => { onClose(); router.push(`/joke/${encodeURIComponent(b.joke_id)}` as any); }} /></View>
        {H?.best_night && <T>Best night: {H.best_night.venue} · {fmtShortDate(H.best_night.date)} · {fmt1(H.best_night.laugh_s_per_min)} laugh-s/min</T>}
        <T v="mut">{H ? `${H.n_nights} ${H.n_nights === 1 ? 'night' : 'nights'} · ${H.low_n ? 'early read' : stabilityWord(H.cv_laugh_s_per_min, H.n_nights)}` : 'Loading…'}</T>
        <Btn testID="lock-bit" label={b.locked ? 'Unlock the bit' : 'Lock the bit'} icon={<Icon name={b.locked ? 'unlock' : 'lock'} size={16} />} onPress={() => { onClose(); onLock(b); }} />
        {!!H?.wording?.effects?.length && (
          <View style={{ gap: 6 }}>
            <T v="eb">What changed and what it did</T>
            {H.wording.effects.slice(0, 3).map((e, i) => (
              <View key={i} style={[s.inner, { gap: 4 }]}>
                <T style={{ fontSize: 14 }}>“{e.examples?.[0]}”</T>
                <T v="small">{e.statement.replace(/[+-]?\d+\.\d{2,}/g, (x) => fmt1(Number(x)))}{e.low_n ? '' : ''}</T>
              </View>
            ))}
            <T v="small">{H.wording.note}</T>
          </View>
        )}
        {!!lines.length && (
          <View style={{ gap: 6 }}>
            <T v="eb">Lock a line</T>
            {lines.map((l, i) => {
              const on = b.locked_lines.some((x) => x.text === l);
              return <Pressable key={i} testID={`lock-line-${i}`} accessibilityRole="button" onPress={() => { onClose(); onLock(b, l); }} style={[s.inner, { flexDirection: 'row', gap: 8, alignItems: 'center', minHeight: 44 }, on && { borderColor: C.amber }]}>
                <Icon name={on ? 'lock' : 'unlock'} size={14} color={on ? C.amber : C.mut} /><T style={{ flex: 1, fontSize: 14 }}>{l}</T>
              </Pressable>;
            })}
          </View>
        )}
        <View style={{ gap: 6 }}>
          <T v="eb">Punch-up drafts</T>
          {!pu && <Btn testID="punchup" small v="add" label="Give me drafts" disabled={b.locked} onPress={async () => { try { setPu(await creative.punchup(wsId, b.joke_id)); } catch (e: any) { setPu({ status: 'provider_error', msg: e.message }); } }} />}
          {b.locked && <T v="small">Locked: no drafts on this bit.</T>}
          {pu?.status === 'queued' && <T v="mut" testID="punchup-queued">Queued for the writer. Drafts appear when they’re ready.</T>}
          {pu?.status === 'locked' && <T v="mut">Locked: no drafts.</T>}
          {(pu?.drafts || []).map((d: any, i: number) => (
            <View key={i} style={[s.draft, { gap: 6 }]}>
              <Text style={{ fontFamily: F.g7, fontSize: 11, color: C.amber }}>Draft · {d.move} · not your words yet</Text>
              <Text style={{ fontFamily: F.body, fontSize: 15, lineHeight: 22, color: C.amber }}>{d.alt}</Text>
            </View>
          ))}
        </View>
      </View>
    </Sheet>
  );
}
