import { expect, it } from "vitest";

import { dragRect, isClick, pictureLayout, toPicture } from "./area-geometry";

it("maps an area drawn on a scaled monitor into the frozen picture's pixels", () => {
  // The right of two monitors, 2560 px wide, shown in a 1280 px window.
  const region = { x: 1920, y: 0, width: 2560, height: 1440 };
  const view = { width: 1280, height: 720 };
  const drawn = dragRect({ x: 300.4, y: 200.6 }, { x: 100.2, y: 50.5 });
  expect(drawn).toEqual({
    x: 100.2,
    y: 50.5,
    width: 300.4 - 100.2,
    height: 200.6 - 50.5,
  });
  expect(toPicture(drawn, region, view)).toEqual({
    x: 1920 + 200,
    y: 101,
    width: 601 - 200,
    height: 402 - 101,
  });
  // Dragged past the window's edge: kept on the monitor.
  expect(
    toPicture({ x: 1200, y: 700, width: 500, height: 500 }, region, view),
  ).toEqual({ x: 1920 + 2400, y: 1400, width: 160, height: 40 });
  // The whole picture is laid out so that the monitor fills the window.
  expect(pictureLayout({ width: 4480, height: 1440 }, region, view)).toEqual({
    left: -960,
    top: 0,
    width: 2240,
    height: 720,
  });
});

it("tells a click from a drag", () => {
  expect(isClick(dragRect({ x: 10, y: 10 }, { x: 12, y: 13 }))).toBe(true);
  expect(isClick(dragRect({ x: 10, y: 10 }, { x: 10, y: 30 }))).toBe(false);
});
