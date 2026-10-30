// Layout rules that the HIG spec (docs/design/HIG.md §0, §6) states as numbers, kept pure so they can be unit-tested
// without a device. Rule of thumb from the Tonight header overflow: no row holds a large-title Text and more than one
// fixed-width control; titles get numberOfLines={1}; trailing items flexShrink: 0; rows re-stack at font scale >= 1.3
// (HIG/layout: "horizontally adjacent views may need to stack vertically to provide more space for text").

/** The OS text-size multiplier at which a title row's trailing pill stacks under the title. */
export const STACK_AT = 1.3;
/** The cap on the large title's own scaling (HIG/typography: Large Title is 34 pt; we match it). */
export const LARGE_TITLE_MAX_SCALE = 1.3;
export const LARGE_TITLE_PT = 34;

export type TitleRow = {
  /** `column` = the pill sits under the title; `row` = beside it. */
  direction: 'row' | 'column';
  /** The title's own font scale after its cap. */
  titleScale: number;
};

/** The Tonight title row ("Tonight" + the "N to sort" pill) at a given OS font scale. */
export function tonightTitleRow(fontScale: number): TitleRow {
  const fs = Number.isFinite(fontScale) && fontScale > 0 ? fontScale : 1;
  return { direction: fs >= STACK_AT ? 'column' : 'row', titleScale: Math.min(LARGE_TITLE_MAX_SCALE, fs) };
}

/**
 * Does one row fit? `width` = the row's inner width in pt; `title` = the title's natural width at scale 1; `fixed` = the
 * widths of the trailing controls; `gap` between items. The test uses it to show the two-row header never over-commits
 * on a 402 pt phone where the old one did (title 125 + pill 100 + 44 + 70 + 44 + gaps > 362).
 */
export function rowFits(width: number, title: number, fixed: number[], gap = 8): boolean {
  const items = fixed.length + 1;
  return title + fixed.reduce((a, b) => a + b, 0) + gap * (items - 1) <= width;
}

/** The header's top padding: the system inset, never a fixed number (HIG/layout: respect the safe area). */
export function headerTopPadding(safeAreaTop: number | undefined): number {
  return Math.max(0, safeAreaTop || 0);
}
