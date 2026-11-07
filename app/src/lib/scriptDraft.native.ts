// The script's local draft on the phone (SPEC 2026-11-06 §12.4 Saving): Documents/script/<topic>.json, the recordings'
// file pattern (lib/recordings.native.ts: the directory rebuilt from documentDirectory every time, file names from the
// topic id's safe characters only). Written on every settled edit, so nothing typed is lost offline or if the app dies.
// The web twin is scriptDraft.ts.
import * as FS from 'expo-file-system/legacy';
import { draftFile, parseLocal, type LocalDraft } from '@/rules/scriptRules';

const dir = () => `${FS.documentDirectory}script/`;
let made = false;
async function ensureDir() {
  if (made) return;
  await FS.makeDirectoryAsync(dir(), { intermediates: true }).catch(() => {});
  made = true;
}

export async function readDraft(topic: string): Promise<LocalDraft | null> {
  try {
    const p = dir() + draftFile(topic);
    const info = await FS.getInfoAsync(p);
    if (!info.exists) return null;
    return parseLocal(await FS.readAsStringAsync(p));
  } catch {
    return null;
  }
}

export async function writeDraft(d: LocalDraft): Promise<void> {
  try {
    await ensureDir();
    // one whole-file write (a half-written file reads as no draft: parseLocal refuses it, the server's version shows)
    await FS.writeAsStringAsync(dir() + draftFile(d.topic), JSON.stringify(d));
  } catch {}
}

export async function dropDraft(topic: string): Promise<void> {
  try {
    await FS.deleteAsync(dir() + draftFile(topic), { idempotent: true });
  } catch {}
}

/** Every draft still waiting to be saved (the tab's focus sends them). */
export async function pendingDrafts(): Promise<LocalDraft[]> {
  try {
    await ensureDir();
    const names = (await FS.readDirectoryAsync(dir())).filter((n) => n.endsWith('.json'));
    const out: LocalDraft[] = [];
    for (const n of names) {
      const d = parseLocal(await FS.readAsStringAsync(dir() + n).catch(() => null));
      if (d?.pending) out.push(d);
    }
    return out;
  } catch {
    return [];
  }
}
