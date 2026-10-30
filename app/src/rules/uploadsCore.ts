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
/** A 4xx (except 408/429) is a refusal that will not change: drop it and tell him; anything else retries. */
export function isPermanent(status: number | null | undefined): boolean {
  return status != null && status >= 400 && status < 500 && status !== 408 && status !== 429;
}


// The upload retry queue driver (pure: no React Native, so tests/uploadQueue.test.mjs runs it in Node). The IO
// (where the queue is stored, how a file goes up) is injected by lib/uploads.ts.

export type Uploader = (item: QueuedUpload) => Promise<{ status: string; file?: string }>;
export type RunResult = { queue: QueuedUpload[]; sent: QueuedUpload[]; dropped: { id: string; name: string; error: string }[] };

/** Try every due item once. A refusal (4xx) drops it and says why; anything else is backed off and kept. */
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
