/** A wheel click's worth of a scroll in pixels, as WebKitGTK reports one. */
export const PIXELS_PER_CLICK = 53;
/** X scrolls three lines per wheel click. */
export const LINES_PER_CLICK = 3;

/**
 * Gathers a scroll into whole wheel clicks: a touchpad's or a smooth
 * scroll's many small deltas add up to the clicks they are worth, instead of
 * one click each. Returns the clicks along each axis and what is left over
 * for the next event.
 */
export function wheelClicks(
  pending: { x: number; y: number },
  event: { deltaX: number; deltaY: number; deltaMode: number },
) {
  // Pages are rare from a wheel; take one as a click.
  const unit =
    event.deltaMode === 1
      ? LINES_PER_CLICK
      : event.deltaMode === 2
        ? 1
        : PIXELS_PER_CLICK;
  const x = pending.x + event.deltaX / unit;
  const y = pending.y + event.deltaY / unit;
  const clicks = { x: Math.trunc(x), y: Math.trunc(y) };
  return { clicks, rest: { x: x - clicks.x, y: y - clicks.y } };
}
