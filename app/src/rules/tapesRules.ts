// Rants become sets; the tabs become Bits · Sets · Tapes (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §3.6,
// Slice 6): the rules, pure and tested (tests/tapesRules.test.mjs).
//   - A topic of the old Rant tab is a SET once the migration (`cargo xtask rants-to-sets`) has made a list for it: a
//     list whose `seed.topic` names it. The app works before and after the migration runs: a topic not yet made into a
//     set is listed on Sets under "From the Rant tab" and opens its old page; one made into a set opens that set
//     (/topic/<id> redirects), and never shows twice.
//   - Tapes is ONE list of everything he recorded, newest first: the recordings kept on this phone (memos, filmed
//     takes, sets taped here) and the sets the server holds. A set taped on this phone whose words reached the server
//     is shown once: the server's set, when one of the same day carries its title.

/** A set list's seed as GET /api/boards sends it: the migration's `{topic}` or `{rant}`, or the app's own drafts'. */
export type Seed = { topic?: string | null; rant?: string | null; [k: string]: unknown } | null | undefined;
export type ListLike = { id: string; title: string; seed?: Seed };
export type TopicLike = { id: string; text: string; touched?: string | null; created?: string | null };

/** The list made from a topic, if the migration made one (live or archived: an archived one still means "made"). */
export function boardOfTopic<T extends ListLike>(lists: readonly T[], topicId: string): T | null {
  return lists.find((b) => b.seed && typeof b.seed === 'object' && b.seed.topic === topicId) || null;
}

/** The topics no list was made from yet, last touched first: Sets lists them until the migration runs. */
export function topicsNotYetSets<T extends TopicLike>(topics: readonly T[], lists: readonly ListLike[]): T[] {
  const made = new Set(lists.map((b) => (b.seed && typeof b.seed === 'object' ? b.seed.topic : null)).filter(Boolean));
  return topics
    .filter((t) => !made.has(t.id))
    .slice()
    .sort((a, b) => (b.touched || b.created || '').localeCompare(a.touched || a.created || ''));
}

/** A recording kept on this phone, as Tapes reads it (rules/memoRules LocalRec). */
export type PhoneTape = { id: string; kind: 'memo' | 'set' | 'film'; recorded_at: string; title?: string | null; board?: { id: string; title: string } | null; state: string; mode?: 'sound' | 'video' };
/** A set the server holds (GET /api/sets, lib/sets SetCard). */
export type ServerTape = { id: string; title: string; date: string | null };
/** One row of Tapes. */
export type TapeRow =
  | { key: string; at: string; from: 'phone'; rec: PhoneTape }
  | { key: string; at: string; from: 'server'; set: ServerTape };

const day = (s: string | null | undefined) => (s && /^\d{4}-\d{2}-\d{2}/.test(s) ? s.slice(0, 10) : '');
const norm = (s: string | null | undefined) => (s || '').trim().toLowerCase().replace(/\s+/g, ' ');

/** Tapes, newest first. A phone set already on the server (`sent`) is dropped when the server holds a set of the same
 *  day whose title is the phone row's title (the server names a phone set with the title the phone sent): one tape,
 *  shown once. Everything else is kept: a tape is never hidden because it merely looks like another. */
export function tapeRows(phone: readonly PhoneTape[], server: readonly ServerTape[]): TapeRow[] {
  const onServer = new Set(server.map((s) => `${day(s.date)}|${norm(s.title)}`));
  const rows: TapeRow[] = [];
  for (const r of phone) {
    const title = r.title || r.board?.title || '';
    if (r.kind === 'set' && r.state === 'sent' && title && onServer.has(`${day(r.recorded_at)}|${norm(title)}`)) continue;
    rows.push({ key: `phone:${r.id}`, at: r.recorded_at || '', from: 'phone', rec: r });
  }
  for (const s of server) rows.push({ key: `server:${s.id}`, at: s.date || '', from: 'server', set: s });
  return rows.sort((a, b) => b.at.localeCompare(a.at) || a.key.localeCompare(b.key));
}

/** The row's kind in one word, the way the list says it (never colour alone). */
export function tapeKind(r: TapeRow): 'Set' | 'Filmed set' | 'Video' | 'Memo' {
  if (r.from === 'server') return 'Set';
  if (r.rec.kind === 'film') return 'Video';
  if (r.rec.kind === 'memo') return 'Memo';
  return r.rec.mode === 'video' ? 'Filmed set' : 'Set';
}
