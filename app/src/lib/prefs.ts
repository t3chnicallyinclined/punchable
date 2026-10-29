// Per-device preferences (the engineering rule: his material lives in the hosted DB; what THIS device shows is a
// device preference). Appearance (System / Dark / Light), haptics on/off. Web: localStorage. Phone: a small JSON
// file in the app's document directory (expo-file-system legacy API), read once at start into a memory cache.
import { Platform } from 'react-native';

export type SchemePref = 'system' | 'dark' | 'light';
export type BitsSort = 'recent' | 'performed' | 'strongest';
export type MapMode = 'cloud' | 'links';
export type Prefs = {
  appearance: SchemePref; haptics: boolean; /** the galaxy map's legend strip open */ mapLegend: boolean;
  /** the Bits list's order (2026-10-29: Recent by default, like a Docs list) */ bitsSort: BitsSort;
  /** Bits › Map: the cloud (bits placed by meaning) or the links map */ mapMode: MapMode;
};
export const DEFAULT_PREFS: Prefs = { appearance: 'system', haptics: true, mapLegend: true, bitsSort: 'recent', mapMode: 'cloud' };

const KEY = 'pk.prefs.v1';
let cache: Prefs = { ...DEFAULT_PREFS };
let loaded = Platform.OS === 'web';
const listeners = new Set<(p: Prefs) => void>();

function clean(raw: any): Prefs {
  const p = { ...DEFAULT_PREFS };
  if (raw && typeof raw === 'object') {
    if (raw.appearance === 'system' || raw.appearance === 'dark' || raw.appearance === 'light') p.appearance = raw.appearance;
    if (typeof raw.haptics === 'boolean') p.haptics = raw.haptics;
    if (typeof raw.mapLegend === 'boolean') p.mapLegend = raw.mapLegend;
    if (raw.bitsSort === 'recent' || raw.bitsSort === 'performed' || raw.bitsSort === 'strongest') p.bitsSort = raw.bitsSort;
    if (raw.mapMode === 'cloud' || raw.mapMode === 'links') p.mapMode = raw.mapMode;
  }
  return p;
}

if (Platform.OS === 'web') {
  try {
    const raw = (globalThis as any).localStorage?.getItem(KEY);
    if (raw) cache = clean(JSON.parse(raw));
  } catch {
    /* no storage: defaults */
  }
}

async function nativeFile(): Promise<{ read: () => Promise<string | null>; write: (s: string) => Promise<void> } | null> {
  try {
    const FS: any = await import('expo-file-system/legacy');
    const uri = `${FS.documentDirectory}prefs.json`;
    return {
      read: async () => ((await FS.getInfoAsync(uri)).exists ? FS.readAsStringAsync(uri) : null),
      write: (s: string) => FS.writeAsStringAsync(uri, s),
    };
  } catch {
    return null;
  }
}

/** Phone: read the file once (the root layout awaits this before the first render). Web: already loaded. */
export async function loadPrefs(): Promise<Prefs> {
  if (loaded) return cache;
  const f = await nativeFile();
  try {
    const raw = f ? await f.read() : null;
    if (raw) cache = clean(JSON.parse(raw));
  } catch {
    /* unreadable: defaults */
  }
  loaded = true;
  listeners.forEach((l) => l(cache));
  return cache;
}
export const getPrefs = (): Prefs => cache;
export function setPrefs(patch: Partial<Prefs>) {
  cache = clean({ ...cache, ...patch });
  listeners.forEach((l) => l(cache));
  const s = JSON.stringify(cache);
  if (Platform.OS === 'web') {
    try {
      (globalThis as any).localStorage?.setItem(KEY, s);
    } catch {}
  } else {
    nativeFile().then((f) => f?.write(s)).catch(() => {});
  }
}
export function onPrefs(f: (p: Prefs) => void) {
  listeners.add(f);
  return () => {
    listeners.delete(f);
  };
}
