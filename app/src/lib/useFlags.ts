// The remote switches on the phone (lib/flags.ts has the pure part). Read once per launch from /api/health (no auth
// needed: the API answers it signed out too), kept in memory, pushed to every mounted useFlags(). A build-time
// EXPO_PUBLIC_GLASS_ACCESSORY=0 is final: the server cannot turn a feature back on that the build turned off.
import { useEffect, useState } from 'react';
import { get } from './api';
import { Flags, flagsFromEnv, readFlags } from './flags';

const ENV = flagsFromEnv(process.env as Record<string, string | undefined>);
let flags: Flags = { ...ENV };
let fetched: Promise<Flags> | null = null;
const listeners = new Set<(f: Flags) => void>();

/** Fetches once; later calls return the same promise. Unreachable = the current flags, no error. */
export function fetchFlags(): Promise<Flags> {
  if (!fetched) {
    fetched = get<unknown>('/api/health')
      .then((body) => {
        const remote = readFlags(body, ENV);
        flags = { glassAccessory: ENV.glassAccessory && remote.glassAccessory };
        listeners.forEach((f) => f(flags));
        return flags;
      })
      .catch(() => flags);
  }
  return fetched;
}

export function getFlags(): Flags {
  return flags;
}

export function useFlags(): Flags {
  const [f, setF] = useState<Flags>(flags);
  useEffect(() => {
    listeners.add(setF);
    fetchFlags();
    return () => {
      listeners.delete(setF);
    };
  }, []);
  return f;
}
