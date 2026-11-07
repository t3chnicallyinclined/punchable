// The Tonight screen's one request (GET /api/tonight/screen), shared now that Tonight's parts live on other tabs
// (SPEC 2026-11-06 R2): the set list bar on Sets, "N to sort" on Bits (and its badge on the Bits tab), the worker's
// state for the AI rows. Every tab that mounts it keeps the badge and the worker line current; the Rant tab mounts
// it first, so the badge is right from launch.
import { useEffect } from 'react';
import { useApi } from './api';
import type { TonightScreen } from './say';
import { noteWorker } from './worker';
import { useUI } from './ui';

export function useTonight() {
  const ui = useUI();
  const s = useApi<TonightScreen>('/api/tonight/screen', { poll: 30000 });
  const d = s.data;
  useEffect(() => { noteWorker(d?.worker); }, [d?.worker?.online, d?.worker?.last_seen]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (d) ui.setToSort(d.to_sort || 0); }, [d?.to_sort]); // eslint-disable-line react-hooks/exhaustive-deps
  return s;
}
