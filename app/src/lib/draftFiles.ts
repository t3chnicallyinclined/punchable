// Small local drafts on the web (nothing typed is lost if the tab closes): localStorage, one key per draft,
// `pk.<space>.v1.<name>`. The phone keeps each as a file in Documents (draftFiles.native.ts). Raw strings in and
// out: each caller parses its own format.
const key = (space: string) => `pk.${space}.v1.`;
const store = () => (globalThis as any).localStorage as Storage | undefined;

export async function readDraftRaw(space: string, name: string): Promise<string | null> {
  try {
    return store()?.getItem(key(space) + name) ?? null;
  } catch {
    return null;
  }
}

/** Resolves false when the browser refused the write (no storage, or it is full). */
export async function writeDraftRaw(space: string, name: string, raw: string): Promise<boolean> {
  try {
    const s = store();
    if (!s) return false;
    s.setItem(key(space) + name, raw);
    return true;
  } catch {
    return false;
  }
}

export async function dropDraftRaw(space: string, name: string): Promise<void> {
  try {
    store()?.removeItem(key(space) + name);
  } catch {}
}

/** Every draft in a space, as raw strings. */
export async function allDraftsRaw(space: string): Promise<string[]> {
  const s = store();
  if (!s) return [];
  const out: string[] = [];
  try {
    for (let i = 0; i < s.length; i++) {
      const k = s.key(i);
      if (!k || !k.startsWith(key(space))) continue;
      const raw = s.getItem(k);
      if (raw) out.push(raw);
    }
  } catch {}
  return out;
}
