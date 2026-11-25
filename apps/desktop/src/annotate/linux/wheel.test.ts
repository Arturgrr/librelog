import { describe, expect, it } from "vitest";

import { PIXELS_PER_CLICK, wheelClicks } from "./wheel";

describe("wheelClicks", () => {
  it("adds a touchpad's small deltas up to whole clicks instead of one click each", () => {
    let pending = { x: 0, y: 0 };
    let total = 0;
    for (let i = 0; i < 20; i++) {
      const { clicks, rest } = wheelClicks(pending, {
        deltaX: 0,
        deltaY: 8,
        deltaMode: 0,
      });
      total += clicks.y;
      pending = rest;
    }
    expect(total).toBe(Math.trunc((20 * 8) / PIXELS_PER_CLICK));
    // Three lines are one click.
    const lines = wheelClicks(
      { x: 0, y: 0 },
      { deltaX: 0, deltaY: -3, deltaMode: 1 },
    );
    expect(lines.clicks).toEqual({ x: 0, y: -1 });
  });
});
