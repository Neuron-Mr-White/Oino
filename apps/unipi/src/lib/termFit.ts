/**
 * Our own terminal-fit calculation (replaces @xterm/addon-fit's
 * `proposeDimensions`): that addon always reserves
 * `overviewRuler?.width || 14` px for a scrollbar/overview-ruler
 * gutter whenever `scrollback !== 0`, even though our terminal hides
 * the scrollbar entirely (DESIGN §3.3) — that reservation plus the
 * cell-width remainder both land on the right edge, producing a wide
 * unused band there while the left gutter stays at its intended 6px.
 *
 * `computeFit` takes the *content box* (css px, scrollbar already
 * excluded by the caller) and the measured cell size, and returns the
 * integer cols/rows plus the leftover pixels split evenly as extra
 * horizontal/vertical padding so both edges match.
 */

export type FitInput = {
  /** Container content-box width/height in CSS px (no xterm padding). */
  containerWidth: number;
  containerHeight: number;
  /** Measured cell size (css px) from the renderer. */
  cellWidth: number;
  cellHeight: number;
  /** Base horizontal/vertical padding to apply before centring. */
  basePaddingX: number;
  basePaddingY: number;
};

export type FitResult = {
  cols: number;
  rows: number;
  /** Horizontal padding (each side) including the centred remainder. */
  paddingX: number;
  /** Vertical padding (top) including the centred remainder. */
  paddingY: number;
};

export function computeFit(input: FitInput): FitResult | undefined {
  const { containerWidth, containerHeight, cellWidth, cellHeight, basePaddingX, basePaddingY } =
    input;
  if (cellWidth <= 0 || cellHeight <= 0) return undefined;
  const usableWidth = Math.max(0, containerWidth - basePaddingX * 2);
  const usableHeight = Math.max(0, containerHeight - basePaddingY * 2);
  const cols = Math.max(2, Math.floor(usableWidth / cellWidth));
  const rows = Math.max(1, Math.floor(usableHeight / cellHeight));
  const usedWidth = cols * cellWidth;
  const usedHeight = rows * cellHeight;
  const leftoverX = Math.max(0, containerWidth - usedWidth);
  const leftoverY = Math.max(0, containerHeight - usedHeight);
  return {
    cols,
    rows,
    paddingX: leftoverX / 2,
    paddingY: leftoverY / 2,
  };
}
