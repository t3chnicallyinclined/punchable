// The URL rule expo-router applies to app/src/app (docs/design/HIG.md §2.1), kept pure so tests/routes.test.mjs can
// prove, from the files on disk, that every URL the app and the web link to still resolves after the move into
// route groups: a group "(x)" never appears in the URL, "index" is its directory, "[id]" is a parameter, "_layout"
// and "+not-found" are not pages.

/** The URLs that must never 404 (the deep links, the universal links, the tabs, the sign-in pages). */
export const REQUIRED_URLS = [
  '/', '/bits', '/sets', '/list/x', '/joke/x', '/set/x', '/board/x', '/map', '/inbox', '/say', '/stage/x', '/merge',
  '/settings', '/write', '/builder/x', '/notebook', '/boards', '/review', '/ask', '/ask/x', '/login', '/auth/callback',
  '/auth/logout', '/profile', '/record', '/today', '/import', '/learned', '/notes', '/desk', '/shows', '/calendar',
  '/bit/x', '/map/x', '/venue/x', '/myset/x', '/workingset/x', '/setlist/x', '/create', '/add/x',
];
/** Served as static files by the web image (app/public), outside the router. */
export const STATIC_URLS: Record<string, string> = { '/privacy': 'privacy.html', '/terms': 'terms.html' };

/** "(tabs)/(bits)/joke/[id].tsx" -> "/joke/:id"; a layout or a special file -> null. */
export function patternOf(file: string): string | null {
  const f = file.replace(/\\/g, '/');
  if (!/\.(tsx|ts|jsx|js)$/.test(f)) return null;
  const parts = f.replace(/\.(tsx|ts|jsx|js)$/, '').split('/').filter(Boolean);
  const last = parts[parts.length - 1];
  if (!last || last.startsWith('_') || parts.some((p) => p.startsWith('+'))) return null;
  const segs = parts.filter((p) => !(p.startsWith('(') && p.endsWith(')')));
  if (segs[segs.length - 1] === 'index') segs.pop();
  return '/' + segs.map((s) => s.replace(/^\[\.\.\.(.+)\]$/, '*$1').replace(/^\[(.+)\]$/, ':$1')).join('/');
}

/** Does a concrete URL (no query) match a pattern? ":id" takes one segment, "*rest" the remainder. */
export function matches(url: string, pattern: string): boolean {
  const u = url.split('?')[0].split('#')[0].split('/').filter(Boolean);
  const p = pattern.split('/').filter(Boolean);
  const catchAll = p.length && p[p.length - 1].startsWith('*');
  const fixed = catchAll ? p.slice(0, -1) : p;
  if (catchAll ? u.length < fixed.length : u.length !== fixed.length) return false;
  return fixed.every((s, i) => s.startsWith(':') || s === u[i]);
}

/** Every pattern a URL resolves to (exactly one is the healthy answer). */
export function resolve(url: string, patterns: string[]): string[] {
  return patterns.filter((p) => matches(url, p));
}

/** The route files that map to the same URL (an ambiguity expo-router would warn about). */
export function duplicates(files: string[]): Record<string, string[]> {
  const by: Record<string, string[]> = {};
  for (const f of files) {
    const p = patternOf(f);
    if (p) (by[p] ||= []).push(f);
  }
  return Object.fromEntries(Object.entries(by).filter(([, v]) => v.length > 1));
}
