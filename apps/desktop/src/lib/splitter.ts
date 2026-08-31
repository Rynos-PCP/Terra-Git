/** Clamps a panel width to [min,max] and rounds to whole pixels. */
export function clampWidth(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, Math.round(value)));
}
