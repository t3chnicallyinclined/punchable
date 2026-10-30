// Sending recordings up (iOS/Android). The file is already in the document directory (recorder.native.ts); the
// queue (`uploads.json` next to it) survives a crash or a dead signal at the venue, and each item goes up through
// expo-file-system's uploadAsync (a native, background-capable transfer with the same headers api.ts sends) on
// the backoff schedule in uploadsCore. The file is deleted once the API has it (Punchable keeps the words, not
// the tape, on the phone).
import { useEffect, useState } from 'react';
import { API_URL, UploadMeta, setOnline } from './api';
import { getAccessToken } from './auth';
import { QueuedUpload, backoffMs, runQueue } from '@/rules/uploadsCore';
import type { Recording } from './recorder';

export type Sent = { status: string; file?: string };

let queue: QueuedUpload[] = [];
let loaded = false;
let timer: any = null;
const listeners = new Set<(q: QueuedUpload[]) => void>();
const notify = () => listeners.forEach((f) => f(queue));

async function fs(): Promise<any> {
  return import('expo-file-system/legacy');
}
async function queueFile() {
  const FS = await fs();
  return `${FS.documentDirectory}uploads.json`;
}
async function load() {
  if (loaded) return;
  loaded = true;
  try {
    const FS = await fs();
    const p = await queueFile();
    if ((await FS.getInfoAsync(p)).exists) {
      const raw = JSON.parse(await FS.readAsStringAsync(p));
      if (Array.isArray(raw)) queue = raw.filter((x) => x && typeof x.uri === 'string');
    }
  } catch {
    queue = [];
  }
  notify();
  schedule();
}
async function persist() {
  try {
    const FS = await fs();
    await FS.writeAsStringAsync(await queueFile(), JSON.stringify(queue));
  } catch {
    /* the queue stays in memory for this run */
  }
}

function headersFor(item: QueuedUpload, token: string | null): Record<string, string> {
  const m = item.meta as UploadMeta;
  const enc = encodeURIComponent;
  return {
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
    'Content-Type': item.type || 'application/octet-stream',
    'X-Kind': m.kind,
    'X-Filename': enc(item.name),
    ...(m.title ? { 'X-Title': enc(m.title) } : {}),
    ...(m.durationS ? { 'X-Duration-S': String(Math.round(m.durationS)) } : {}),
    ...(m.marksMs?.length ? { 'X-Marks-Ms': JSON.stringify(m.marksMs) } : {}),
    ...(m.recordedAt ? { 'X-Recorded-At': m.recordedAt } : {}),
    ...(m.venue ? { 'X-Venue': enc(m.venue) } : {}),
    ...(m.lat != null && m.lng != null ? { 'X-Lat': String(m.lat), 'X-Lng': String(m.lng) } : {}),
    ...(m.showGroup ? { 'X-Show-Group': enc(m.showGroup) } : {}),
    ...(m.alignWorkingSet ? { 'X-Align-Working-Set': enc(m.alignWorkingSet) } : {}),
  };
}

async function send(item: QueuedUpload): Promise<Sent> {
  const FS = await fs();
  const token = await getAccessToken();
  let r: any;
  try {
    r = await FS.uploadAsync(API_URL + '/api/upload', item.uri, {
      httpMethod: 'POST', uploadType: FS.FileSystemUploadType.BINARY_CONTENT, headers: headersFor(item, token), sessionType: FS.FileSystemSessionType.BACKGROUND,
    });
  } catch (e: any) {
    setOnline(false);
    throw Object.assign(new Error(e?.message || 'No connection'), { status: null });
  }
  setOnline(true);
  let body: any = {};
  try {
    body = JSON.parse(r.body || '{}');
  } catch {}
  if (r.status < 200 || r.status >= 300) throw Object.assign(new Error(body.error || `HTTP ${r.status}`), { status: r.status });
  FS.deleteAsync(item.uri, { idempotent: true }).catch(() => {});
  return body as Sent;
}

function schedule() {
  clearTimeout(timer);
  if (!queue.length) return;
  const next = Math.min(...queue.map((u) => u.nextAt));
  timer = setTimeout(flush, Math.max(250, next - Date.now()));
}

export async function flush() {
  await load();
  const r = await runQueue(queue, send);
  queue = r.queue;
  await persist();
  notify();
  schedule();
  return r;
}

export async function enqueueUpload(rec: Recording, meta: UploadMeta): Promise<Sent & { queued?: boolean; error?: string }> {
  if (!rec.uri) throw new Error('No recording to send');
  await load();
  const id = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  const item: QueuedUpload = { id, uri: rec.uri, name: rec.name, type: rec.type, meta, attempts: 0, nextAt: 0 };
  queue = [...queue, item];
  await persist();
  notify();
  try {
    const r = await send(item);
    queue = queue.filter((u) => u.id !== id);
    await persist();
    notify();
    return r;
  } catch (e: any) {
    if (typeof e?.status === 'number' && e.status >= 400 && e.status < 500 && e.status !== 408 && e.status !== 429) {
      queue = queue.filter((u) => u.id !== id);
      await persist();
      notify();
      throw e;
    }
    queue = queue.map((u) => (u.id === id ? { ...u, attempts: 1, nextAt: Date.now() + backoffMs(0), lastError: String(e?.message || e) } : u));
    await persist();
    notify();
    schedule();
    return { status: 'queued_local', queued: true, error: String(e?.message || e) };
  }
}

export function pendingUploads(): QueuedUpload[] {
  return queue;
}
export function usePendingUploads(): QueuedUpload[] {
  const [q, setQ] = useState(queue);
  useEffect(() => {
    listeners.add(setQ);
    load();
    return () => {
      listeners.delete(setQ);
    };
  }, []);
  return q;
}
