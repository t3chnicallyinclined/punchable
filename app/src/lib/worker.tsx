// The home worker's status, shared: one small request (GET /api/ask/chats carries `worker`), kept for 60 s, so
// every screen that queues a job (ask, riff, punch-up, bridge, imports, memos) can show the same honest line:
// "Your PC is off: this waits until it's back." Nothing here polls; a screen asks when it shows a queued state.
import React, { useEffect, useState } from 'react';
import { Text, View } from 'react-native';
import { get } from './api';
import type { Worker } from './ask';
import { C, F } from './theme';

let last: { at: number; w: Worker | null } = { at: 0, w: null };
let inflight: Promise<Worker | null> | null = null;
const listeners = new Set<(w: Worker | null) => void>();

export async function fetchWorker(maxAgeMs = 60_000): Promise<Worker | null> {
  if (Date.now() - last.at < maxAgeMs) return last.w;
  if (inflight) return inflight;
  inflight = get<{ worker?: Worker }>('/api/ask/chats')
    .then((r) => r?.worker || null)
    .catch(() => null)
    .then((w) => {
      last = { at: Date.now(), w };
      inflight = null;
      listeners.forEach((f) => f(w));
      return w;
    });
  return inflight;
}
/** A screen that just learned the status (the say / tonight responses carry `worker`) shares it. */
export function noteWorker(w: Worker | null | undefined) {
  if (!w) return;
  last = { at: Date.now(), w };
  listeners.forEach((f) => f(w));
}
export function useWorker(active = true): Worker | null {
  const [w, setW] = useState<Worker | null>(last.w);
  useEffect(() => {
    listeners.add(setW);
    if (active) fetchWorker();
    return () => {
      listeners.delete(setW);
    };
  }, [active]);
  return w;
}

/** The line under anything queued for the home PC. Renders nothing while the PC is online or unknown. */
export function WorkerLine({ what = 'This', testID, worker }: { what?: string; testID?: string; worker?: Worker | null }) {
  const live = useWorker(!worker);
  const w = worker ?? live;
  if (!w || w.online) return null;
  return (
    <View testID={testID || 'worker-offline'} accessibilityRole={'status' as any} style={{ padding: 10, borderRadius: 12, backgroundColor: C.warnBg, gap: 2 }}>
      <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 14, color: C.amber }}>Your PC is off</Text>
      <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, color: C.mut }}>{what} waits and gets done when it’s back on.{w.last_seen ? '' : ' It hasn’t checked in since Punchable started.'}</Text>
    </View>
  );
}
