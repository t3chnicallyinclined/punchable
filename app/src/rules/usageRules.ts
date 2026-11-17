// Settings › Usage (SPEC 2026-11-17 §13, phase 1): what GET /api/admin/usage says, turned into what the page shows.
// Pure, so tests/usageRules.test.mjs proves it (and its --red arms show the checks can fail). Counts and money only:
// the server sends nothing he wrote, and nothing here adds any. Copy reviewed against the HIG (writing, lists,
// progress indicators) by the iOS design expert, 2026-11-17.

export type UsageKind = { kind: string; usd: number; calls: number };
export type UsageDay = {
  day: string; ai_usd: number; ai_calls: number; requests: number; writes: number; bits: number; redlines: number;
  drafts: number; sessions: number; active: boolean;
};
export type UsageMedia = { kind: string; count: number; seconds: number; bytes: number };
export type UsageReport = {
  from: string; to: string;
  days: UsageDay[];
  totals: {
    ai_usd: number; ai_calls: number; by_kind: UsageKind[]; active_days: number; requests: number; writes: number;
    bits: number; redlines: number; drafts: number; sessions: number; media: UsageMedia[];
  };
  month: {
    month: string; ai_usd: number; days_elapsed: number; days_in_month: number; projected_usd: number;
    active_days: number; usd_per_active_day: number; price_to_cover: { margin: number; usd: number; assumption: string };
  };
  prices: { read: string; source: string };
};
/** GET /api/usage/access: may this account read the page, and has the server pinned an owner at all. */
export type UsageAccess = { owner: boolean; pinned: boolean };

const KIND_NAMES: Record<string, string> = {
  riff: 'Riffs', punchup: 'Punch-ups', ask: 'Ask your material', ask_chat: 'Conversations', bridge: 'Bridges',
  facts: 'Fact checks', rant: 'Rant cuts', hook: 'Hooks', rant_draft: 'Rant drafts', rant_line: 'Rant lines',
  redline: 'Redline', set_text: 'Sorting taped sets', embed: 'Keeping search up to date', memolink: 'Matching memos to bits',
};

/** A job kind as he reads it. */
export function kindName(kind: string): string {
  return KIND_NAMES[kind] ?? 'Other';
}

/** Money spent, as the page shows it: never "$0.00" for spend that happened (a cent's fraction reads "under 1¢"). */
export function money(usd: number): string {
  if (!Number.isFinite(usd) || usd <= 0) return '$0';
  if (usd < 0.005) return 'under 1¢';
  if (usd < 100) return `$${usd.toFixed(2)}`;
  return `$${Math.round(usd).toLocaleString('en-US')}`;
}

/** A forecast: whole dollars from $1 up (cents would claim a precision a guess does not have). */
export function whole(usd: number): string {
  return usd < 1 ? money(usd) : `$${Math.round(usd).toLocaleString('en-US')}`;
}

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

/** "2026-11" -> "November". */
export function monthName(ym: string): string {
  const m = Number(ym.slice(5, 7));
  return MONTHS[m - 1] ?? ym;
}

/** "2026-11-17" -> "Nov 17, 2026". */
export function dayLabel(d: string): string {
  const [y, m, day] = d.split('-').map(Number);
  if (!y || !m || !day) return d;
  return `${MONTHS[m - 1].slice(0, 3)} ${day}, ${y}`;
}

const count = (n: number) => n.toLocaleString('en-US');
const times = (n: number) => (n === 1 ? 'asked once' : `asked ${count(n)} times`);

/** The fewest days into a month before "at this pace" means anything. */
export const PACE_FROM_DAY = 3;

export type UsageView = {
  heading: string;          // "AI cost, November so far"
  spent: string;            // "$1.23"
  spentLabel: string;       // what VoiceOver reads for the big number
  pace: string;             // "About $3 by the end of November at this pace"
  active: string;           // "Used on 6 of 17 days"
  perDay: string | null;    // "$0.21 a day you used it"
  kinds: { key: string; name: string; usd: string; detail: string }[];
  cover: { line: string; note: string } | null;
  activityHeading: string;  // "What you did in November"
  activity: { key: string; label: string; sub?: string; value: string }[];
  footnote: string;
  empty: boolean;
};

/** The page from the report: this month first, then what cost what (most expensive first), what he did, and where the
 *  prices came from. */
export function usageView(r: UsageReport): UsageView {
  const m = r.month;
  const month = monthName(m.month);
  const kinds = [...(r.totals.by_kind || [])]
    .filter((k) => k.calls > 0 || k.usd > 0)
    .sort((a, b) => b.usd - a.usd || b.calls - a.calls)
    .map((k) => ({ key: k.kind, name: kindName(k.kind), usd: money(k.usd), detail: times(k.calls) }));
  const recordings = (r.totals.media || []).reduce((n, x) => n + (x.count || 0), 0);
  const minutes = Math.round((r.totals.media || []).reduce((n, x) => n + (x.seconds || 0), 0) / 60);
  const t = r.totals;
  const spent = money(m.ai_usd);
  let pace: string;
  if (m.ai_usd <= 0) pace = 'Nothing spent on AI this month yet';
  else if (m.days_elapsed < PACE_FROM_DAY) pace = `Too early in ${month} to guess the month`;
  else pace = `About ${whole(m.projected_usd)} by the end of ${month} at this pace`;
  const keep = Math.round(m.price_to_cover.margin * 100);
  return {
    heading: `AI cost, ${month} so far`,
    spent,
    spentLabel: `${spent} spent on AI in ${month} so far`,
    pace,
    active: `Used on ${m.active_days} of ${m.days_elapsed} ${m.days_elapsed === 1 ? 'day' : 'days'}`,
    perDay: m.active_days > 0 && m.ai_usd > 0 ? `${money(m.usd_per_active_day)} a day you used it` : null,
    kinds,
    cover: m.ai_usd > 0
      ? {
        line: `To keep ${keep}¢ of every dollar, charge about ${whole(m.price_to_cover.usd)} a month`,
        note: 'An assumption, not a price: it counts the AI only. Hosting, Apple’s cut and taxes would push it higher.',
      }
      : null,
    activityHeading: `What you did in ${month}`,
    activity: [
      { key: 'requests', label: 'Times the app talked to the server', sub: 'Every load and save, including the ones it does on its own', value: count(t.requests) },
      { key: 'sessions', label: 'Times the app connected', sub: 'Each time it comes back to the front or reconnects', value: count(t.sessions) },
      { key: 'bits', label: 'Bits and versions saved', value: count(t.bits) },
      { key: 'drafts', label: 'Times you asked the AI', value: count(t.drafts) },
      { key: 'redlines', label: 'Redline rounds', value: count(t.redlines) },
      { key: 'recordings', label: 'Sets and recordings received', value: minutes > 0 ? `${recordings} · ${minutes} min` : String(recordings) },
    ],
    footnote: `AI prices from Anthropic’s price list, read ${dayLabel(r.prices.read)}. Counts and money only: nothing you wrote is in these numbers.`,
    empty: kinds.length === 0 && t.requests === 0,
  };
}

/** The line shown when the page cannot load: the server's reason when it is one he can act on. */
export function usageError(msg: string | null | undefined): string {
  const m = msg || '';
  if (m.includes('PK_OWNER_SUB') || m.includes('not pinned')) return 'Usage isn’t switched on yet. It needs a one-time setting on the server.';
  if (m.includes('owner’s only') || m.includes("owner's only")) return 'Only the owner can see usage.';
  if (/not found|HTTP 404/i.test(m)) return 'Usage comes with the next server update.';
  return 'Can’t reach Punchable right now.';
}

/** The line over numbers already showing when a refresh failed. */
export const STALE = 'Couldn’t update just now.';

/** Whether Settings shows the Usage row: to the owner, or to everyone while the server has no owner pinned (the page
 *  then says it is not switched on); never to a tester, and never when the server cannot say (an older server). */
export function showUsageRow(a: UsageAccess | null | undefined): boolean {
  return !!a && (a.owner === true || a.pinned === false);
}

/** The export: one line per day, counts and money only. */
export function usageCsv(r: UsageReport): string {
  const head = 'day,ai_usd,ai_calls,requests,writes,bits,redlines,drafts,sessions,active';
  const rows = (r.days || []).map((d) =>
    [d.day, d.ai_usd.toFixed(6), d.ai_calls, d.requests, d.writes, d.bits, d.redlines, d.drafts, d.sessions, d.active].join(','));
  return [head, ...rows].join('\n') + '\n';
}

/** The export's file name. */
export function csvName(r: UsageReport): string {
  return `punchable-usage-${r.from}-to-${r.to}.csv`;
}
