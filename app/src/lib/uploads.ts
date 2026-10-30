// Sending recordings up (web). A recording goes through POST /api/upload at once; when that fails for a network
// reason it stays in a memory queue (the Blob lives in this tab) and is retried on the backoff schedule in
// uploadsCore until the tab closes. The phone build (uploads.native.ts) keeps the file and the queue on disk.
import { useEffect, useState } from 'react';
import { ApiError, UploadMeta, setOnline, upload } from './api';
import { QueuedUpload, backoffMs, runQueue } from '@/rules/uploadsCore';
import type { Recording } from './recorder';

const blobs = new Map<string, Blob>();
let queue: QueuedUpload[] = [];
let timer: any = null;
const listeners = new Set<(q: QueuedUpload[]) => void>();
const notify = () => listeners.forEach((f) => f(queue));

export type Sent = { status: string; file?: string };

async function send(item: QueuedUpload): Promise<Sent> {
  const b = blobs.get(item.id);
  if (!b) throw Object.assign(new Error('the recording is gone from this tab'), { status: 410 });
  try {
    const r = await upload(b, { ...(item.meta as UploadMeta), filename: item.name });
    setOnline(true);
    return r;
  } catch (e: any) {
    if (e instanceof ApiError) throw Object.assign(e, { status: e.status });
    setOnline(false);
    throw e;
  }
}

function schedule() {
  clearTimeout(timer);
  if (!queue.length) return;
  const next = Math.min(...queue.map((u) => u.nextAt));
  timer = setTimeout(flush, Math.max(250, next - Date.now()));
}

/** Try what is due now. Returns what went up and what the server refused (shown to him by the caller). */
export async function flush() {
  const r = await runQueue(queue, send);
  queue = r.queue;
  r.sent.forEach((u) => blobs.delete(u.id));
  r.dropped.forEach((u) => blobs.delete(u.id));
  notify();
  schedule();
  return r;
}

/** Upload now; on a network failure keep it and retry. Resolves with the server's answer or `{status:'queued_local'}`. */
export async function enqueueUpload(rec: Recording, meta: UploadMeta): Promise<Sent & { queued?: boolean; error?: string }> {
  if (!rec.blob) throw new Error('No recording to send');
  const id = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  blobs.set(id, rec.blob);
  const item: QueuedUpload = { id, uri: '', name: rec.name, type: rec.type, meta, attempts: 0, nextAt: 0, bytes: rec.blob.size };
  try {
    const r = await send(item);
    blobs.delete(id);
    return r;
  } catch (e: any) {
    if (e instanceof ApiError) {
      blobs.delete(id);
      throw e;
    }
    queue = [...queue, { ...item, attempts: 1, nextAt: Date.now() + backoffMs(0), lastError: String(e?.message || e) }];
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
    return () => {
      listeners.delete(setQ);
    };
  }, []);
  return q;
}
