// Voice memos and taped sets kept on this iPhone (SPEC 2026-10-31 §2-§3, §7). The audio stays in
// Documents/recordings (backed up with the phone; never uploaded); one small index beside it
// (Documents/recordings/index.json) holds a row per recording: file NAME (the container path changes across
// updates), when, how long, which bit or set list, the state, the words. The queue:
//   - transcribe on the phone, one at a time, memos first (lib/transcribe.native); never while a recording runs; the
//     screen stays awake while it works (its own keep-awake tag); the app going to the background aborts the run and
//     puts it back (it restarts from zero when Punchable is in front again: the package cannot resume at an offset);
//   - send the words: a memo -> POST /api/ideas (filed on the bit it clearly belongs to by the server, or on the bit
//     it was started from: POST /api/merge/undo + /api/merge/attach); a set, or a memo over 4000 characters -> POST
//     /api/sets/transcript. No signal, a 5xx or a 401 -> kept with the backoff in rules/uploadsCore; a refusal ->
//     failed with the audio kept. Every decision is in rules/memoRules (tested); this file is the wiring.
// The paid cloud backup (later) syncs these same files and rows.
import { useEffect, useState } from 'react';
import { AppState } from 'react-native';
import * as FS from 'expo-file-system/legacy';
import { activateKeepAwakeAsync, deactivateKeepAwake } from 'expo-keep-awake';
import { ApiError, post, setOnline } from './api';
import { handle401 } from './auth';
import { isRecording, onRecording } from './audioBusy';
import { stopLocal } from './localPlay';
import { transcribeFile } from './transcribe';
import type { TranscribeRun } from './transcribe';
import type { Recording } from './recorderBase';
import { isPermanent } from '@/rules/uploadsCore';
import {
  KEEP_AWAKE_TRANSCRIBE, afterRefusal, afterRequeue, afterSendFailure, dueToSend, filingSteps, ideaBody, isSafeFileName,
  nextToTranscribe, parseIndex, retry, routeOf, safeFileName, setBody, textOf,
} from '@/rules/memoRules';
import type { LocalRec, RecKind } from '@/rules/memoRules';

export type { LocalRec } from '@/rules/memoRules';
export const recordingsSupported = true;

let recs: LocalRec[] = [];
let loaded: Promise<void> | null = null;
let started = false;
let run: { id: string; t: TranscribeRun; byUs: 'bg' | 'rec' | null } | null = null;
let sending = false;
let timer: ReturnType<typeof setTimeout> | null = null;
let pumpTimer: ReturnType<typeof setTimeout> | null = null;
let lastPersist = 0;
let writing: Promise<void> = Promise.resolve();
const listeners = new Set<(r: LocalRec[]) => void>();
const notify = () => listeners.forEach((f) => f(recs));

const dir = () => `${FS.documentDirectory}recordings/`;
const indexPath = () => `${dir()}index.json`;
/** The file:// URI of a row's audio, rebuilt from documentDirectory every time (never stored). */
export function uriOf(r: Pick<LocalRec, 'file'>): string {
  return dir() + r.file;
}

async function load() {
  if (!loaded) {
    loaded = (async () => {
      try {
        await FS.makeDirectoryAsync(dir(), { intermediates: true }).catch(() => {});
        const p = indexPath();
        if ((await FS.getInfoAsync(p)).exists) recs = parseIndex(JSON.parse(await FS.readAsStringAsync(p)));
      } catch {
        recs = [];
      }
      notify();
    })();
  }
  return loaded;
}
/** Writes run one after another, each with the rows as they are when it starts, so an older snapshot can never land
 *  after a newer one (and lose an idea id) [review]. A failed write keeps the rows in memory; the next one tries again. */
function persist(): Promise<void> {
  lastPersist = Date.now();
  writing = writing.then(() => FS.writeAsStringAsync(indexPath(), JSON.stringify(recs))).catch(() => {});
  return writing;
}
function set(id: string, f: (r: LocalRec) => LocalRec, save = true) {
  recs = recs.map((r) => (r.id === id ? f(r) : r));
  notify();
  if (save) void persist();
}
const find = (id: string) => recs.find((r) => r.id === id);

// ------------------------------------------------------------------ transcribe
const awake = (on: boolean) => (on ? activateKeepAwakeAsync(KEEP_AWAKE_TRANSCRIBE) : deactivateKeepAwake(KEEP_AWAKE_TRANSCRIBE)).catch(() => {});

async function pump() {
  await load();
  if (run || isRecording() || AppState.currentState !== 'active') return;
  const now = Date.now();
  const next = nextToTranscribe(recs, now);
  if (!next) {
    // a row backing off after a busy recognizer: come back when it is due
    const waits = recs.filter((r) => r.state === 'queued' && (r.next_at || 0) > now).map((r) => r.next_at);
    if (pumpTimer) clearTimeout(pumpTimer);
    pumpTimer = waits.length ? setTimeout(() => void pump(), Math.max(1000, Math.min(...waits) - now)) : null;
    return;
  }
  const id = next.id;
  set(id, (r) => ({ ...r, state: 'transcribing', progress: 0, error: null }));
  void awake(true);
  const t = transcribeFile(uriOf(next), next.duration_s, (p) => {
    // progress only in memory (and on disk at most every 10 s): a long set sends hundreds of finals
    set(id, (r) => ({ ...r, progress: p, paused: r.paused && p < (r.peak || 0) }), Date.now() - lastPersist > 10_000);
  });
  run = { id, t, byUs: null };
  const res = await t.done;
  const byUs = run?.byUs ?? null;
  run = null;
  void awake(false);
  if (!find(id)) return pump();   // deleted while it ran
  set(id, (r) => {
    switch (res.outcome) {
      case 'done': {
        const text = textOf(res.segs);
        const keepSegs = routeOf(r, text) === 'set';
        return { ...r, state: 'words', text, segments: keepSegs ? res.segs : null, engine: res.engine, progress: 1, paused: false, attempts: 0, next_at: 0, error: null };
      }
      case 'requeue': return afterRequeue(r, Date.now(), byUs, res.error);
      case 'no_speech': return { ...r, state: 'no_speech', progress: 0, paused: false, error: null };
      default: return { ...r, state: res.outcome, error: res.error, progress: 0, paused: false };
    }
  });
  void sendDue();
  void pump();
}

/** The app went to the background, or a recording started: stop the run; it goes back in the queue as ours. */
function interrupt(why: 'bg' | 'rec') {
  if (!run) return;
  run.byUs = why;
  run.t.abort();
}

/** A 409 from an undo or attach that already happened (a lost answer, a kill between the call and the save) is done,
 *  not a refusal: otherwise the row would fail and Try again would repeat the same 409 forever [review]. */
const already = (e: any) => e instanceof ApiError && e.status === 409;
const gone = (e: any) => e instanceof ApiError && e.status === 404;

// ------------------------------------------------------------------ send the words
async function sendOne(r: LocalRec): Promise<LocalRec> {
  const text = r.text || '';
  if (routeOf(r, text) === 'set') {
    const segs = r.segments?.length ? r.segments : [{ text, start: 0, end: Math.max(0, r.duration_s) }];
    await post('/api/sets/transcript', setBody(r, segs, r.engine || 'phone'));
    return { ...r, state: 'sent', segments: null, error: null, attempts: 0 };
  }
  let cur = r;
  if (!cur.idea_id) {
    const res = await post<{ id: string; attach?: { decision?: string; into?: string; id?: string; title?: string } | null }>('/api/ideas', ideaBody(cur, text));
    const att = res.attach;
    cur = { ...cur, idea_id: res.id, ...(att?.decision === 'attach' && att.into && att.id ? { merge_id: att.id, filed_on: { id: att.into, title: att.title || 'a bit' } } : {}) };
    set(cur.id, () => cur);   // the idea exists now: a retry never makes a second one
  }
  const steps = filingSteps(cur.bit?.id, cur.merge_id && cur.filed_on ? { decision: 'attach', into: cur.filed_on.id, id: cur.merge_id } : { decision: 'new' });
  if (steps.undo) {
    try {
      await post('/api/merge/undo', { merge_id: steps.undo });
    } catch (e) {
      if (!already(e)) throw e;
    }
    cur = { ...cur, merge_id: null, filed_on: null };
    set(cur.id, () => cur);
  }
  const bit = cur.bit;
  if (steps.attach && bit) {
    try {
      const a = await post<{ merge_id: string }>('/api/merge/attach', { note: cur.idea_id, into: bit.id, reason: 'voice memo recorded on this bit' });
      cur = { ...cur, merge_id: a.merge_id, filed_on: { id: bit.id, title: bit.title } };
    } catch (e) {
      if (already(e)) cur = { ...cur, merge_id: null, filed_on: { id: bit.id, title: bit.title } };
      else if (gone(e)) cur = { ...cur, bit: null };   // the bit is gone (merged away, deleted): it stays a new idea
      else throw e;
    }
  }
  return { ...cur, state: cur.filed_on ? 'filed' : 'new_idea', segments: null, error: null, attempts: 0 };
}

async function sendDue() {
  await load();
  if (sending) return;
  sending = true;
  try {
    for (const r of dueToSend(recs, Date.now())) {
      if (!find(r.id)) continue;
      set(r.id, (x) => ({ ...x, state: 'sending' }));
      try {
        const done = await sendOne(find(r.id) || r);
        setOnline(true);
        set(r.id, () => done);
      } catch (e: any) {
        const status = e instanceof ApiError ? e.status : null;
        if (status == null) setOnline(false);
        if (status === 401) await handle401().catch(() => false);
        const now = Date.now();
        set(r.id, (x) => (isPermanent(status) ? afterRefusal(x, `${status}: ${e?.message || e}`) : afterSendFailure(x, now, String(e?.message || e))));
      }
    }
  } finally {
    sending = false;
    schedule();
  }
}
function schedule() {
  if (timer) clearTimeout(timer);
  timer = null;
  const waits = recs.filter((r) => r.state === 'words').map((r) => r.next_at);
  if (!waits.length) return;
  timer = setTimeout(() => void sendDue(), Math.max(1000, Math.min(...waits) - Date.now()));
}

// ------------------------------------------------------------------ public
/** Once, from the root layout: read the index, run what is waiting, follow the app's state and the recorder. */
export function startRecordings() {
  if (started) return;
  started = true;
  AppState.addEventListener('change', (s) => {
    // only the background stops a run: `inactive` (the permission prompt, Control Center, a banner, Face ID) does not
    // suspend the app, and on-device recognition of a file keeps going [review]
    if (s === 'active') { void pump(); void sendDue(); } else if (s === 'background') interrupt('bg');
  });
  onRecording((on) => (on ? interrupt('rec') : void pump()));
  void load().then(() => { void pump(); void sendDue(); });
}

export function useRecordings(): LocalRec[] {
  const [r, setR] = useState(recs);
  useEffect(() => {
    listeners.add(setR);
    void load().then(() => setR(recs));
    return () => {
      listeners.delete(setR);
    };
  }, []);
  return r;
}

const newId = () => `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;

export type AddMeta = { kind: RecKind; recordedAt: string; bit?: { id: string; title: string } | null; board?: { id: string; title: string } | null; title?: string | null; marksMs?: number[] };
/** A stopped recording (already moved into Documents/recordings by the recorder) becomes a row and joins the queue. */
export async function addRecording(rec: Recording, meta: AddMeta): Promise<LocalRec> {
  await load();
  if (!rec.uri) throw new Error('The recording was not saved');
  let file = rec.name;
  if (rec.uri !== dir() + file || !isSafeFileName(file)) {
    file = safeFileName(new Date().toISOString(), Math.random().toString(36).slice(2), rec.name.split('.').pop() || 'm4a');
    await FS.moveAsync({ from: rec.uri, to: dir() + file });
  }
  const r: LocalRec = {
    id: newId(), kind: meta.kind, file, recorded_at: meta.recordedAt, duration_s: Math.max(0, rec.durationMs / 1000),
    bit: meta.bit || null, board: meta.board || null, title: meta.title || null, state: 'queued', attempts: 0, next_at: 0,
    ...(meta.marksMs?.length ? { marks_ms: meta.marksMs } : {}),
  };
  recs = [r, ...recs];
  notify();
  await persist();
  void pump();
  return r;
}

export function retryRec(id: string) {
  set(id, retry);
  void pump();
  void sendDue();
}

/** Deletes the audio and the row. The words already on the server stay (the caller's sheet says so). */
export async function deleteRec(id: string) {
  const r = find(id);
  if (!r) return;
  if (run?.id === id) { run.byUs = 'rec'; run.t.abort(); }
  stopLocal();
  recs = recs.filter((x) => x.id !== id);
  notify();
  await persist();
  await FS.deleteAsync(uriOf(r), { idempotent: true }).catch(() => {});
}

/** "Not this bit": detach the memo's idea from the bit it was filed on; it stays a new idea. */
export async function notThisBit(id: string) {
  const r = find(id);
  if (!r?.merge_id) return;
  try {
    await post('/api/merge/undo', { merge_id: r.merge_id });
  } catch (e) {
    if (!already(e)) throw e;   // already detached: the same end state
  }
  set(id, (x) => ({ ...x, state: 'new_idea', merge_id: null, filed_on: null, bit: null }));
}

/** "Import from Files…": recordings he picks are copied in under generated names (SPEC §1 trap 3) and transcribed
 *  here as memos (a long one goes as a set transcript). Resolves to how many were added. */
export async function importFiles(): Promise<number> {
  const DP = await import('expo-document-picker');
  const res: any = await DP.getDocumentAsync({ type: ['audio/*'], multiple: true, copyToCacheDirectory: true });
  if (!res || res.canceled || !res.assets?.length) return 0;
  await load();
  let n = 0;
  for (const a of res.assets as any[]) {
    const ext = String(a.name || '').split('.').pop() || 'm4a';
    const file = safeFileName(new Date().toISOString(), Math.random().toString(36).slice(2), ext);
    try {
      await FS.copyAsync({ from: a.uri, to: dir() + file });
    } catch {
      continue;
    }
    const at = typeof a.lastModified === 'number' ? new Date(a.lastModified).toISOString() : new Date().toISOString();
    await addRecording({ uri: dir() + file, name: file, type: a.mimeType || 'audio/mp4', durationMs: 0 }, { kind: 'memo', recordedAt: at, title: String(a.name || '').replace(/\.[^.]+$/, '').slice(0, 120) || null });
    n++;
  }
  return n;
}
