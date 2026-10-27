// "Backstage Amber": the design tokens from the approved prototype (App.dc.html <helmet> CSS).
// Rules for using them (contrast, the draft rule, the laugh ramp, the light variant): docs/DESIGN.md.
// 2026-10 (docs/UX_2026-10.md): the ground, the cards and the accent follow the approved Mix* artboards
// (#0B0B0D ground, #1C1C1F cards, amber #FFB340, record red #FF453A). Every screen gets them through these tokens.
export const C = {
  bg: '#0B0B0D',
  bgDeep: '#060607',
  surface: '#161618',
  surface2: '#1C1C1F',
  card: '#1C1C1F',
  card2: '#161618',
  mut2: '#A1A1A8', // the artboards' secondary text
  tabOff: '#8E8E93',
  rec: '#FF453A', // the hold-to-talk button
  gap: '#FF6B5A', // a gap segment in the set bar
  gapText: '#FF8A7D',
  border: '#2d3139',
  borderStrong: '#4b5059',
  text: '#f3f4f6',
  text2: '#d1d5db',
  mut: '#9ca3af',
  dim: '#878d97', // was #6b7280 (3.5:1 on surface, fails AA for placeholder text); now ≥ 4.6:1 on bg/surface/surface2
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
  // laugh density: one amber ramp everywhere (UX_SPEC 5.3)
  heat: ['rgba(245,181,68,.12)', 'rgba(245,181,68,.35)', 'rgba(245,181,68,.6)', 'rgba(245,181,68,.85)', '#f5b544'],
};

/** The light variant (docs/DESIGN.md §3.2). The app ships dark only (app.json userInterfaceStyle); these are for
 *  surfaces that must sit on white: printed set lists, email, light-mode web pages. Amber never carries text on light:
 *  amberInk for text and links, amberMark for marks. Buttons stay amber with ink text. */
export const LIGHT = {
  bg: '#f6f6f4',
  surface: '#ffffff',
  surface2: '#eeeeeb',
  border: '#d9d9d4',
  borderStrong: '#8f8b82',
  text: '#111214',
  text2: '#3d4048',
  mut: '#5d6169',
  amber: '#f5b544',
  amberInk: '#8a5a00',
  amberMark: '#b47a0e',
  red: '#b91c1c',
  green: '#15803d',
  blue: '#0369a1',
};

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
