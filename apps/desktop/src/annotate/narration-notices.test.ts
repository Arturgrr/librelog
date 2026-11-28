import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { narrationNotices } from "./narration-notices";

describe("narration notices", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("tells a flapping microphone once, and that it is back once it stays back", () => {
    const shown: string[] = [];
    const notices = narrationNotices({
      lost: () => shown.push("lost"),
      back: () => shown.push("back"),
    });

    for (let flap = 0; flap < 5; flap++) {
      notices.heard(false);
      vi.advanceTimersByTime(500);
      notices.heard(true);
      vi.advanceTimersByTime(500);
    }
    expect(shown).toEqual(["lost"]);

    vi.advanceTimersByTime(2_000);
    expect(shown).toEqual(["lost", "back"]);

    notices.heard(false);
    notices.reset();
    notices.heard(false);
    expect(shown).toEqual(["lost", "back", "lost", "lost"]);
  });
});
