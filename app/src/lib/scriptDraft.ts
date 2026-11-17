// The script's local draft (SPEC 2026-11-06 §12.4 Saving): written on every settled edit, so nothing typed is lost
// offline or if the app dies. Kept where it always was: Documents/script/<topic>.json on the phone, localStorage
// `pk.script.v1.<topic>` on the web. The storage itself is lib/draftFiles (shared with a bit's document, lib/bitDraft).
import { parseLocal, type LocalDraft } from '@/rules/scriptRules';
import { allDraftsRaw, dropDraftRaw, readDraftRaw, writeDraftRaw } from './draftFiles';

const SPACE = 'script';

export async function readDraft(topic: string): Promise<LocalDraft | null> {
  // a half-written file reads as no draft: parseLocal refuses it, the server's version shows
  return parseLocal(await readDraftRaw(SPACE, topic));
}

export async function writeDraft(d: LocalDraft): Promise<void> {
  await writeDraftRaw(SPACE, d.topic, JSON.stringify(d));
}

export async function dropDraft(topic: string): Promise<void> {
  await dropDraftRaw(SPACE, topic);
}

/** Every draft still waiting to be saved (the tab's focus sends them). */
export async function pendingDrafts(): Promise<LocalDraft[]> {
  const out: LocalDraft[] = [];
  for (const raw of await allDraftsRaw(SPACE)) {
    const d = parseLocal(raw);
    if (d?.pending) out.push(d);
  }
  return out;
}
