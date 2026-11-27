import { afterEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  holdModel: vi.fn(),
  releaseModelHold: vi.fn(),
}));
vi.mock("@anlg/plugin-local-stt", () => ({
  commands: {
    holdModel: mocks.holdModel,
    releaseModelHold: mocks.releaseModelHold,
  },
}));

import { holdLocalModelUntilReleased } from "./local-model-hold";

describe("holdLocalModelUntilReleased", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it("lets the last hold go only once the renewed one is taken", async () => {
    vi.useFakeTimers();
    let takeSecond = (_: unknown) => {};
    mocks.holdModel
      .mockResolvedValueOnce({ status: "ok", data: "first" })
      .mockReturnValueOnce(
        new Promise((resolve) => {
          takeSecond = resolve;
        }),
      );
    mocks.releaseModelHold.mockResolvedValue({ status: "ok", data: null });

    const release = holdLocalModelUntilReleased({
      provider: "gguf",
      model: "gguf-whisper-large-v3-turbo",
    });
    await vi.advanceTimersByTimeAsync(5 * 60_000);
    expect(mocks.holdModel).toHaveBeenCalledTimes(2);
    expect(mocks.releaseModelHold).not.toHaveBeenCalled();

    takeSecond({ status: "ok", data: "second" });
    await vi.waitFor(() =>
      expect(mocks.releaseModelHold).toHaveBeenCalledWith("first"),
    );
    expect(mocks.releaseModelHold).not.toHaveBeenCalledWith("second");

    release();
    await vi.waitFor(() =>
      expect(mocks.releaseModelHold).toHaveBeenCalledWith("second"),
    );
  });
});
