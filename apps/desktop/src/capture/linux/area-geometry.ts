import type { AreaRect } from "@anlg/plugin-capture";

export type Point = { x: number; y: number };
export type Size = { width: number; height: number };

/** A drag shorter than this, either way, is a click. */
const CLICK = 4;

/** The rectangle between two points, in the window's pixels. */
export function dragRect(from: Point, to: Point): AreaRect {
  return {
    x: Math.min(from.x, to.x),
    y: Math.min(from.y, to.y),
    width: Math.abs(to.x - from.x),
    height: Math.abs(to.y - from.y),
  };
}

export function isClick(rect: AreaRect) {
  return rect.width < CLICK && rect.height < CLICK;
}

/**
 * Where the frozen picture goes so that `region` (the selector's monitor
 * in it) fills a `view`-sized window.
 */
export function pictureLayout(
  picture: Size,
  region: AreaRect,
  view: Size,
): { left: number; top: number; width: number; height: number } {
  const sx = view.width / Math.max(region.width, 1);
  const sy = view.height / Math.max(region.height, 1);
  return {
    left: 0 - region.x * sx,
    top: 0 - region.y * sy,
    width: picture.width * sx,
    height: picture.height * sy,
  };
}

/**
 * A rectangle drawn in the window, in the picture's pixels: inside
 * `region`, rounded outwards to whole pixels.
 */
export function toPicture(
  rect: AreaRect,
  region: AreaRect,
  view: Size,
): AreaRect {
  const sx = region.width / Math.max(view.width, 1);
  const sy = region.height / Math.max(view.height, 1);
  const clamp = (value: number, max: number) =>
    Math.min(Math.max(value, 0), max);
  const left = Math.floor(clamp(rect.x, view.width) * sx);
  const top = Math.floor(clamp(rect.y, view.height) * sy);
  const right = Math.ceil(clamp(rect.x + rect.width, view.width) * sx);
  const bottom = Math.ceil(clamp(rect.y + rect.height, view.height) * sy);
  return {
    x: region.x + left,
    y: region.y + top,
    width: Math.min(right, region.width) - left,
    height: Math.min(bottom, region.height) - top,
  };
}
