// Small local drafts on the phone (nothing typed is lost offline or if the app dies): one JSON file per draft in
// Documents/<space>/, the recordings' file pattern (lib/recordings.native.ts: the directory rebuilt from
// documentDirectory every time, file names from the id's safe characters only). `space` is `script` (a rant's script,
// lib/scriptDraft) or `bit` (a bit's document, lib/bitDraft). Raw strings in and out: each caller parses its own
// format and treats a half-written file as no draft. The web twin is draftFiles.ts.
import * as FS from 'expo-file-system/legacy';
import { draftFile } from '@/rules/scriptRules';

const dir = (space: string) => `${FS.documentDirectory}${space}/`;
const made = new Set<string>();
async function ensureDir(space: string) {
  if (made.has(space)) return;
  await FS.makeDirectoryAsync(dir(space), { intermediates: true }).catch(() => {});
  made.add(space);
}

export async function readDraftRaw(space: string, name: string): Promise<string | null> {
  try {
    const p = dir(space) + draftFile(name);
    const info = await FS.getInfoAsync(p);
    if (!info.exists) return null;
    return await FS.readAsStringAsync(p);
  } catch {
    return null;
  }
}

/** One whole-file write. Resolves false when the phone refused it (the caller may say so). */
export async function writeDraftRaw(space: string, name: string, raw: string): Promise<boolean> {
  try {
    await ensureDir(space);
    await FS.writeAsStringAsync(dir(space) + draftFile(name), raw);
    return true;
  } catch {
    return false;
  }
}

export async function dropDraftRaw(space: string, name: string): Promise<void> {
  try {
    await FS.deleteAsync(dir(space) + draftFile(name), { idempotent: true });
  } catch {}
}

/** Every draft in a space, as raw strings. */
export async function allDraftsRaw(space: string): Promise<string[]> {
  try {
    await ensureDir(space);
    const names = (await FS.readDirectoryAsync(dir(space))).filter((n) => n.endsWith('.json'));
    const out: string[] = [];
    for (const n of names) {
      const raw = await FS.readAsStringAsync(dir(space) + n).catch(() => null);
      if (raw) out.push(raw);
    }
    return out;
  } catch {
    return [];
  }
}
