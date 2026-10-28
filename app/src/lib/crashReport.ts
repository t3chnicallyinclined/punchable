// Keeps the last crash where the next launch can show it (lib/crashlog.ts has the record; this file is the storage
// and the hook into React Native's error path).
//   Phone: `last-crash.json` in the app's document directory (expo-file-system legacy API, the same one prefs.ts
//          and the upload queue already use). Web: localStorage.
//   installCrashReporter(): wraps ErrorUtils' global handler. A FATAL error is written first, then handed to the
//   previous handler (React Native's, which in a release build ends the process: RCTExceptionsManager.reportFatal ->
//   RCTFatal). The hand-off waits for the write or WRITE_CAP_MS, whichever is first, so a crash is never swallowed and
//   the record is on disk before the process goes. Non-fatal errors pass straight through.
//   The root ErrorBoundary (lib/CrashScreen.tsx) records with source 'boundary' itself: React catches those before the
//   global handler ever sees them.
import { Platform } from 'react-native';
import { CrashRecord, formatCrash, parseCrash, serializeCrash } from './crashlog';

const KEY = 'punchable.last-crash';
const FILE = 'last-crash.json';
/** How long the fatal hand-off waits for the write (ms). */
export const WRITE_CAP_MS = 1500;

async function fs(): Promise<any> {
  return import('expo-file-system/legacy');
}
async function fileUri(): Promise<string> {
  const FS = await fs();
  return `${FS.documentDirectory}${FILE}`;
}

async function writeRecord(r: CrashRecord): Promise<void> {
  const s = serializeCrash(r);
  if (Platform.OS === 'web') {
    (globalThis as any).localStorage?.setItem(KEY, s);
    return;
  }
  const FS = await fs();
  await FS.writeAsStringAsync(await fileUri(), s);
}

/** The record from the previous run, or null. Never throws. */
export async function readLastCrash(): Promise<CrashRecord | null> {
  try {
    if (Platform.OS === 'web') return parseCrash((globalThis as any).localStorage?.getItem(KEY) ?? null);
    const FS = await fs();
    const uri = await fileUri();
    if (!(await FS.getInfoAsync(uri)).exists) return null;
    return parseCrash(await FS.readAsStringAsync(uri));
  } catch {
    return null;
  }
}

export async function clearLastCrash(): Promise<void> {
  try {
    if (Platform.OS === 'web') {
      (globalThis as any).localStorage?.removeItem(KEY);
      return;
    }
    const FS = await fs();
    await FS.deleteAsync(await fileUri(), { idempotent: true });
  } catch {
    /* nothing to clear */
  }
}

let contextFn: () => Record<string, string> = () => ({});
/** What every record carries besides the error (the root layout sets the current path). */
export function setCrashContext(f: () => Record<string, string>) {
  contextFn = f;
}
function context(extra?: Record<string, string>): Record<string, string> {
  let c: Record<string, string> = {};
  try {
    c = contextFn();
  } catch {
    c = {};
  }
  return { platform: `${Platform.OS} ${String(Platform.Version)}`, ...c, ...(extra || {}) };
}

/** Write a record for `err`. Resolves when it is on disk (or failed to be); never rejects. */
export function recordCrash(err: unknown, source: CrashRecord['source'], extra?: Record<string, string>): Promise<void> {
  try {
    return writeRecord(formatCrash(err, source, context(extra))).catch(() => {});
  } catch {
    return Promise.resolve();
  }
}

type Handler = (error: unknown, isFatal?: boolean) => void;
let installed = false;
/** Once, at startup (the root layout's module scope). Safe where ErrorUtils is missing (tests, the web). */
export function installCrashReporter() {
  const EU = (globalThis as any).ErrorUtils as { getGlobalHandler?: () => Handler; setGlobalHandler?: (h: Handler) => void } | undefined;
  if (installed || !EU?.setGlobalHandler) return;
  installed = true;
  const prev: Handler | undefined = EU.getGlobalHandler?.();
  EU.setGlobalHandler((error, isFatal) => {
    const pass = () => {
      try {
        prev?.(error, isFatal);
      } catch {
        /* the previous handler is React Native's; it does not throw */
      }
    };
    if (!isFatal) return pass();
    let done = false;
    const once = () => {
      if (done) return;
      done = true;
      pass();
    };
    setTimeout(once, WRITE_CAP_MS);
    recordCrash(error, 'fatal').then(once, once);
  });
}
