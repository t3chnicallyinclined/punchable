// The script's local draft on the web (SPEC 2026-11-06 §12.4 Saving): localStorage, one key per topic. The phone keeps
// it as a file in Documents (scriptDraft.native.ts), so nothing typed is lost offline or if the app dies.
import { parseLocal, type LocalDraft } from '@/rules/scriptRules';

const KEY = 'pk.script.v1.';
const store = () => (globalThis as any).localStorage as Storage | undefined;

export async function readDraft(topic: string): Promise<LocalDraft | null> {
  try {
    return parseLocal(store()?.getItem(KEY + topic));
  } catch {
    return null;
  }
}

export async function writeDraft(d: LocalDraft): Promise<void> {
  try {
    store()?.setItem(KEY + d.topic, JSON.stringify(d));
  } catch {}
}

export async function dropDraft(topic: string): Promise<void> {
  try {
    store()?.removeItem(KEY + topic);
  } catch {}
}

/** Every draft still waiting to be saved (the tab's focus sends them). */
export async function pendingDrafts(): Promise<LocalDraft[]> {
  const s = store();
  if (!s) return [];
  const out: LocalDraft[] = [];
  for (let i = 0; i < s.length; i++) {
    const k = s.key(i);
    if (!k || !k.startsWith(KEY)) continue;
    const d = parseLocal(s.getItem(k));
    if (d?.pending) out.push(d);
  }
  return out;
}
