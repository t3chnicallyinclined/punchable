// Sending recordings up (iOS/Android). The file is already in the document directory (recorder.native.ts); the
// queue (`uploads.json` next to it) survives a crash or a dead signal at the venue, and each item goes up through
// expo-file-system's uploadAsync (a native, background-capable transfer with the headers api.ts builds: authHeaders
// + uploadHeaders, plus X-Idempotency-Key = the item's id) on the backoff schedule in rules/uploadsCore. A 401 is
// transient: sendOnce refreshes through the auth machine and tries once more; still 401 = kept, never deleted. The
// file is deleted once the API has it (Punchable keeps the words, not the tape, on the phone).
import { useEffect, useState } from 'react';
import { API_URL, UploadMeta, authHeaders, setOnline, uploadHeaders } from './api';
import { handle401 } from './auth';
import { QueuedUpload, backoffMs, isPermanent, runQueue, sendOnce } from '@/rules/uploadsCore';
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

async function send(item: QueuedUpload): Promise<Sent> {
  const FS = await fs();
  const r = await sendOnce(item, {
    headers: async (it) => ({ ...(await authHeaders()), ...uploadHeaders(it.meta as UploadMeta, it.name, it.type) }),
    put: async (it, headers) => {
      let res: any;
      try {
        res = await FS.uploadAsync(API_URL + '/api/upload', it.uri, {
          httpMethod: 'POST', uploadType: FS.FileSystemUploadType.BINARY_CONTENT, headers, sessionType: FS.FileSystemSessionType.BACKGROUND,
        });
      } catch (e: any) {
        setOnline(false);
        throw Object.assign(new Error(e?.message || 'No connection'), { status: null });
      }
      setOnline(true);
      return { status: res.status, body: res.body };
    },
    refresh: handle401,
  });
  FS.deleteAsync(item.uri, { idempotent: true }).catch(() => {});
  return r as Sent;
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
    if (isPermanent(typeof e?.status === 'number' ? e.status : null)) {
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
