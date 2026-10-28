// "Backstage Amber": the design tokens from the approved prototype (App.dc.html <helmet> CSS).
// Rules for using them (contrast, the draft rule, the laugh ramp, the light variant): docs/DESIGN.md.
// 2026-10 (docs/UX_2026-10.md): the ground, the cards and the accent follow the approved Mix* artboards
// (#0B0B0D ground, #1C1C1F cards, amber #FFB340, record red #FF453A). Every screen gets them through these tokens.
//
// Appearance (HIG): dark is the default; the app follows the system (useColorScheme) with a per-device override in
// Settings (System / Dark / Light; lib/prefs). `C` is ONE mutable object: setScheme() swaps its values in place and
// the root remounts, so every `C.x` read at render time picks the new palette (only ui.tsx keeps static styles, and
// it rebuilds them on the change).
export const DARK = {
  bg: '#0B0B0D',
  bgDeep: '#060607',
  surface: '#161618',
  surface2: '#1C1C1F',
  card: '#1C1C1F',
  card2: '#161618',
  mut2: '#A1A1A8', // the artboards' secondary text (6.6:1 on #0B0B0D, 5.4:1 on #1C1C1F)
  tabOff: '#8E8E93',
  rec: '#FF453A', // the hold-to-talk button
  gap: '#FF6B5A', // a gap segment in the set bar
  gapText: '#FF8A7D',
  border: '#2d3139',
  borderStrong: '#4b5059',
  hairline: '#1F1F22',
  text: '#f3f4f6',
  text2: '#d1d5db',
  mut: '#9ca3af',
  dim: '#878d97', // was #6b7280 (3.5:1 on surface, fails AA for placeholder text); now >= 4.6:1 on bg/surface/surface2
  amber: '#FFB340',
  amberDeep: '#7f5700',
  amberSoft: 'rgba(245,181,68,.15)',
  amberPill: 'rgba(245,181,68,.22)',
  amberMid: '#c98f2c', // the middle step where a ramp has only 3 (map pins, heat dot)
  red: '#FF453A',
  redSoft: '#fca5a5',
  dangerBorder: '#7f1d1d', // error cards, the danger button
  warnBg: '#3b2a0a', // offline banner
  green: '#22c55e',
  greenDeep: '#052e14',
  greenSoft: 'rgba(34,197,94,.15)',
  greenBorder: 'rgba(34,197,94,.45)',
  blue: '#38bdf8',
  blueSoft: 'rgba(56,189,248,.15)',
  // a bit's status dot (Bits, the all-bits map): kills · working · fresh · resting; systemGreen / amber / systemCyan / the off tab
  statusKills: '#30D158',
  statusWorking: '#FFB340',
  statusFresh: '#64D2FF',
  statusResting: '#8E8E93',
  scrim: 'rgba(9,10,11,.8)',
  // prototype .ai chip, .add button, drafts
  aiBorder: '#1e3a52',
  aiBg: 'rgba(56,189,248,.08)',
  addBorder: '#5c4a1f',
  addBg: '#241e12',
  draftBorder: '#a7781f',
  draftBg: 'rgba(245,181,68,.05)',
  // dev-only fixture banner: violet on purpose, a colour nothing real uses
  fixture: '#7c3aed',
  fixtureBg: 'rgba(124,58,237,.12)',
  fixtureText: '#c4b5fd',
  panel: '#16171a',
  // the snackbar: inverted (text on bg)
  snackBg: '#f3f4f6',
  snackText: '#0B0B0D',
  snackAction: '#7f5700',
  // laugh density: one amber ramp everywhere (UX_SPEC 5.3)
  heat: ['rgba(245,181,68,.12)', 'rgba(245,181,68,.35)', 'rgba(245,181,68,.6)', 'rgba(245,181,68,.85)', '#f5b544'],
  // the status bar and keyboard follow the scheme
  scheme: 'dark' as 'dark' | 'light',
};
export type Tokens = typeof DARK;

/** The light variant (docs/DESIGN.md §3.2, extended to the full token set). Amber never carries text on light:
 *  text and links use amberInk; marks use amberMark. Buttons stay amber with ink text. The owner can swap the hues
 *  here; the structure (which role each token plays) is the same as DARK. */
export const LIGHT_TOKENS: Tokens = {
  ...DARK,
  bg: '#f6f6f4',
  bgDeep: '#ebebe8',
  surface: '#ffffff',
  surface2: '#eeeeeb',
  card: '#ffffff',
  card2: '#f0f0ec',
  mut2: '#5d6169',
  tabOff: '#6b6f78',
  border: '#d9d9d4',
  borderStrong: '#8f8b82',
  hairline: '#e4e4df',
  text: '#111214',
  text2: '#3d4048',
  mut: '#5d6169',
  dim: '#6b7079',
  amber: '#b47a0e',      // amberMark: carries marks and small text on light (4.6:1 on white)
  amberDeep: '#8a5a00',  // amberInk
  amberSoft: 'rgba(180,122,14,.12)',
  amberPill: 'rgba(180,122,14,.2)',
  amberMid: '#c98f2c',
  red: '#b91c1c',
  redSoft: '#b91c1c',
  gap: '#d9482f',
  gapText: '#b63a22',
  rec: '#d9382c',
  dangerBorder: '#f1b5b5',
  warnBg: '#fff2d6',
  green: '#15803d',
  greenDeep: '#052e14',
  greenSoft: 'rgba(21,128,61,.12)',
  greenBorder: 'rgba(21,128,61,.45)',
  blue: '#0369a1',
  blueSoft: 'rgba(3,105,161,.12)',
  statusKills: '#248a3d',
  statusWorking: '#b47a0e',
  statusFresh: '#0a6e9e',
  statusResting: '#6b6f78',
  scrim: 'rgba(17,18,20,.45)',
  aiBorder: '#b9dcf0',
  aiBg: 'rgba(3,105,161,.06)',
  addBorder: '#e2cfa3',
  addBg: '#fbf3df',
  draftBorder: '#b47a0e',
  draftBg: 'rgba(180,122,14,.05)',
  fixtureText: '#5b21b6',
  panel: '#ffffff',
  snackBg: '#1b1b1f',
  snackText: '#f3f4f6',
  snackAction: '#ffb340',
  heat: ['rgba(180,122,14,.12)', 'rgba(180,122,14,.3)', 'rgba(180,122,14,.5)', 'rgba(180,122,14,.75)', '#b47a0e'],
  scheme: 'light',
};
/** Kept for surfaces that must sit on white regardless of the scheme (printed set lists, email). */
export const LIGHT = {
  bg: LIGHT_TOKENS.bg, surface: LIGHT_TOKENS.surface, surface2: LIGHT_TOKENS.surface2, border: LIGHT_TOKENS.border,
  borderStrong: LIGHT_TOKENS.borderStrong, text: LIGHT_TOKENS.text, text2: LIGHT_TOKENS.text2, mut: LIGHT_TOKENS.mut,
  amber: '#f5b544', amberInk: '#8a5a00', amberMark: '#b47a0e', red: '#b91c1c', green: '#15803d', blue: '#0369a1',
};

/** The live tokens. Read `C.x` at render time (not at module load) so a scheme change shows. */
export const C: Tokens = { ...DARK };

export type SchemeName = 'dark' | 'light';
export type SchemePref = 'system' | SchemeName;
const schemeListeners = new Set<(s: SchemeName) => void>();
export function currentScheme(): SchemeName {
  return C.scheme;
}
/** Swap the palette in place and tell the listeners (ui.tsx rebuilds its static styles; the root remounts). */
export function setScheme(name: SchemeName) {
  if (C.scheme === name) return;
  Object.assign(C, name === 'light' ? LIGHT_TOKENS : DARK);
  schemeListeners.forEach((f) => f(name));
}
export function onSchemeChange(f: (s: SchemeName) => void) {
  schemeListeners.add(f);
  return () => {
    schemeListeners.delete(f);
  };
}
/** The scheme to show for a preference and what the OS says. */
export function resolveScheme(pref: SchemePref, system: string | null | undefined): SchemeName {
  if (pref === 'light' || pref === 'dark') return pref;
  return system === 'light' ? 'light' : 'dark';
}

export const F = {
  body: 'Inter_400Regular',
  medium: 'Inter_500Medium',
  semi: 'Inter_600SemiBold',
  bold: 'Inter_700Bold',
  black: 'Inter_800ExtraBold',
  g5: 'SpaceGrotesk_500Medium',
  g6: 'SpaceGrotesk_600SemiBold',
  g7: 'SpaceGrotesk_700Bold',
};

export type Stage = 'idea' | 'to_try' | 'testing' | 'working' | 'retired';
export const STAGES: Stage[] = ['idea', 'to_try', 'testing', 'working', 'retired'];
export const STAGE: Record<string, { label: string; fg: string; bg: string; help: string }> = {
  idea: { label: 'Ideas', fg: '#38bdf8', bg: 'rgba(56,189,248,.15)', help: 'Raw premises and saved lines.' },
  to_try: { label: 'To Try', fg: '#e5e7eb', bg: '#22252a', help: 'Ready for your next mic.' },
  testing: { label: 'Testing', fg: '#f5b544', bg: 'rgba(245,181,68,.15)', help: 'Being rated show by show. Working needs 3 rooms.' },
  working: { label: 'Working', fg: '#22c55e', bg: 'rgba(34,197,94,.15)', help: 'Tried and true. Only you put jokes here.' },
  retired: { label: 'Retired', fg: '#9ca3af', bg: '#22252a', help: 'Cut or resting. Bring back anytime.' },
};
export const stageInfo = (s?: string | null) => STAGE[s || 'idea'] || STAGE.idea;
/** The stage as one word in rows ("Working", "Idea"). */
export const stageWord = (s?: string | null) => (s === 'idea' ? 'Idea' : stageInfo(s).label);

export const RATING: Record<string, { label: string; fg: string }> = {
  killed: { label: 'Killed', fg: C.amber },
  worked: { label: 'Worked', fg: C.text },
  meh: { label: 'Meh', fg: C.mut },
  died: { label: 'Died', fg: C.redSoft },
};

export const WIDE = 900;

export const fmtMs = (ms: number | null | undefined) => {
  const s = Math.max(0, Math.floor((ms || 0) / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
};

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
export const MONTHS_LONG = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
export const fmtDate = (iso?: string | null) => {
  if (!iso) return '';
  const [y, m, d] = iso.slice(0, 10).split('-').map(Number);
  return `${MONTHS[(m || 1) - 1]} ${d}, ${y}`;
};
