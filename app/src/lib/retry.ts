// A small retry for native calls that fail transiently (the keychain while the device is locking, a file write
// during a backgrounding). No React Native imports: tests/retry.test.mjs runs it in Node.
export type RetryOpts = { tries?: number; delayMs?: number; sleep?: (ms: number) => Promise<void> };

const wait = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** Runs `f` up to `tries` times (default 3), waiting `delayMs` (default 150, doubling) between tries. Throws the last error. */
export async function retry<T>(f: () => Promise<T>, opts: RetryOpts = {}): Promise<T> {
  const tries = Math.max(1, opts.tries ?? 3);
  const sleep = opts.sleep ?? wait;
  let delay = opts.delayMs ?? 150;
  let last: unknown;
  for (let i = 0; i < tries; i++) {
    try {
      return await f();
    } catch (e) {
      last = e;
      if (i < tries - 1) {
        await sleep(delay);
        delay *= 2;
      }
    }
  }
  throw last;
}
