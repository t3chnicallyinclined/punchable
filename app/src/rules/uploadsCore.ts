// The upload retry queue: rules (backoff, what is due, what is a refusal) and the driver.
export type QueuedUpload = { id: string; uri: string; name: string; type: string; meta: Record<string, unknown>; attempts: number; nextAt: number; lastError?: string | null; bytes?: number };
/** Backoff: 5 s, 30 s, 2 min, 10 min, then every 30 min; never dropped (a lost set costs more than a retry). */
export function backoffMs(attempts: number): number {
  const steps = [5_000, 30_000, 120_000, 600_000];
  return attempts < steps.length ? steps[attempts] : 1_800_000;
}
export function dueUploads(q: QueuedUpload[], now: number): QueuedUpload[] {
  return q.filter((u) => u.nextAt <= now).sort((a, b) => a.nextAt - b.nextAt);
}
export function afterFailure(u: QueuedUpload, now: number, error: string): QueuedUpload {
  const attempts = u.attempts + 1;
  return { ...u, attempts, nextAt: now + backoffMs(attempts - 1), lastError: error.slice(0, 200) };
}
/** A 4xx (except 401/408/429) is a refusal that will not change: drop it and tell him; anything else retries.
 *  401 is transient (docs/ARCHITECTURE.md §7.2): a stale token after a refresh that failed offline is exactly the
 *  "taped at the venue, back online later" case, and a recording is never deleted for it. */
export function isPermanent(status: number | null | undefined): boolean {
  return status != null && status >= 400 && status < 500 && status !== 401 && status !== 408 && status !== 429;
}

// ------------------------------------------------------------------ one attempt, with the 401 rule
/** Every upload carries the queue item's id, so a transfer killed mid-flight cannot land twice on relaunch. */
export const IDEMPOTENCY_HEADER = 'X-Idempotency-Key';
export type PutResult = { status: number; body?: string | null };
export type SendDeps = {
  /** The headers for this item, read fresh on every attempt: the bearer (from the token machine) and the upload's X-* set. */
  headers: (item: QueuedUpload) => Promise<Record<string, string>>;
  /** One HTTP attempt; resolves with the status and body text (a network failure throws, with status null). */
  put: (item: QueuedUpload, headers: Record<string, string>) => Promise<PutResult>;
  /** The auth machine's handle401 (one refresh in flight for every caller): resolves "still signed in?". */
  refresh: () => Promise<boolean>;
};
/**
 * Send one item: on a 401 refresh through the shared path and try ONCE more with fresh headers; a second 401 (or a
 * refresh that says signed out) throws with status 401, which the driver keeps and backs off (never drops). Any
 * other non-2xx throws with its status; 2xx resolves with the server's answer.
 */
export async function sendOnce(item: QueuedUpload, deps: SendDeps): Promise<{ status: string; file?: string }> {
  const attempt = async () => deps.put(item, { ...(await deps.headers(item)), [IDEMPOTENCY_HEADER]: item.id });
  let r = await attempt();
  if (r.status === 401 && (await deps.refresh())) r = await attempt();
  let body: any = {};
  try {
    body = JSON.parse(r.body || '{}');
  } catch {}
  if (r.status === 401) throw Object.assign(new Error(body.error || 'Not signed in; the recording stays on this phone until you are'), { status: 401 });
  if (r.status < 200 || r.status >= 300) throw Object.assign(new Error(body.error || `HTTP ${r.status}`), { status: r.status });
  return body;
}


// The upload retry queue driver (pure: no React Native, so tests/uploadQueue.test.mjs runs it in Node). The IO
// (where the queue is stored, how a file goes up) is injected by lib/uploads.ts.

export type Uploader = (item: QueuedUpload) => Promise<{ status: string; file?: string }>;
export type RunResult = { queue: QueuedUpload[]; sent: QueuedUpload[]; dropped: { id: string; name: string; error: string }[] };

/** Try every due item once. A refusal (4xx, not 401/408/429) drops it and says why; anything else is backed off and kept. */
export async function runQueue(queue: QueuedUpload[], upload: Uploader, now = Date.now()): Promise<RunResult> {
  const sent: QueuedUpload[] = [];
  const dropped: RunResult['dropped'] = [];
  const next = new Map(queue.map((u) => [u.id, u]));
  for (const item of dueUploads(queue, now)) {
    try {
      await upload(item);
      sent.push(item);
      next.delete(item.id);
    } catch (e: any) {
      const status = typeof e?.status === 'number' ? e.status : null;
      if (isPermanent(status)) {
        dropped.push({ id: item.id, name: item.name, error: String(e?.message || e) });
        next.delete(item.id);
      } else {
        next.set(item.id, afterFailure(item, now, String(e?.message || e)));
      }
    }
  }
  return { queue: queue.map((u) => next.get(u.id)).filter(Boolean) as QueuedUpload[], sent, dropped };
}
