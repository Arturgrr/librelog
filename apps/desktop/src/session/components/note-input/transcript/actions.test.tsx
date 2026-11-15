import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  audioPath: vi.fn(),
  handleBatchFailed: vi.fn(),
  queueAutoEnhanceIfSummaryEmpty: vi.fn(),
  runBatch: vi.fn(),
  toastError: vi.fn(),
  toastInfo: vi.fn(),
  getTranscriptRecord: vi.fn(),
}));

vi.mock("@anlg/plugin-fs-sync", () => ({
  commands: { audioPath: mocks.audioPath },
}));

vi.mock("@anlg/ui/components/ui/toast", () => ({
  toast: { error: mocks.toastError, info: mocks.toastInfo },
}));

vi.mock("~/services/enhancer", () => ({
  getEnhancerService: () => ({
    queueAutoEnhanceIfSummaryEmpty: mocks.queueAutoEnhanceIfSummaryEmpty,
  }),
}));

vi.mock("~/stt/contexts", () => ({
  useListener: (selector: (state: unknown) => unknown) =>
    selector({ handleBatchFailed: mocks.handleBatchFailed }),
}));

vi.mock("~/stt/useRunBatch", () => ({
  isStoppedTranscriptionError: (error: unknown) =>
    error instanceof Error && error.message === "Transcription stopped.",
  useRunBatch: () => mocks.runBatch,
}));

vi.mock("~/stt/queries", () => ({
  getTranscriptRecord: mocks.getTranscriptRecord,
}));

import { useRegenerateTranscript, useSeparateSpeakers } from "./actions";

describe("useRegenerateTranscript", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.audioPath.mockResolvedValue({
      status: "ok",
      data: "/tmp/session.wav",
    });
  });

  it("shows batch transcription failures even when an old transcript exists", async () => {
    mocks.runBatch.mockRejectedValue(new Error("Authentication failed"));
    const { result } = renderHook(() => useRegenerateTranscript("session-1"));

    await act(async () => {
      await result.current();
    });

    expect(mocks.runBatch).toHaveBeenCalledWith("/tmp/session.wav", {
      promotion: { scope: "whole_session" },
    });
    expect(mocks.handleBatchFailed).toHaveBeenCalledWith(
      "session-1",
      "Authentication failed",
    );
    expect(mocks.toastError).toHaveBeenCalledWith("Re-transcription failed", {
      id: "transcript-regenerate-failed-session-1",
      description: "Authentication failed",
    });
  });
});

describe("useSeparateSpeakers", () => {
  const words = [
    { id: "w1", text: " Hello", start_ms: 0, end_ms: 400, channel: 0 },
  ];

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.audioPath.mockResolvedValue({
      status: "ok",
      data: "/tmp/session.wav",
    });
    mocks.getTranscriptRecord.mockResolvedValue({
      id: "transcript-1",
      startedAt: 1000,
      words,
      speakerHints: [],
    });
  });

  it.each([
    [{ separated: false }, true],
    [{ separated: true }, false],
  ])(
    "labels the existing words in place and says when only one voice was heard (%o)",
    async (outcome, oneVoice) => {
      mocks.runBatch.mockResolvedValue(outcome);
      const { result } = renderHook(() => useSeparateSpeakers("session-1"));

      await act(async () => {
        await result.current("transcript-1");
      });

      expect(mocks.runBatch).toHaveBeenCalledWith("/tmp/session.wav", {
        provider: "local_diarization",
        model: "local-diarization",
        baseUrl: "",
        apiKey: "",
        diarizeWords: words,
        phase: "refining_speakers",
        promotion: {
          scope: "current_capture",
          audioOffsetMs: 0,
          replaceTranscriptId: "transcript-1",
          startedAt: 1000,
        },
      });
      expect(mocks.toastInfo).toHaveBeenCalledTimes(oneVoice ? 1 : 0);
    },
  );

  it("surfaces a failure to separate", async () => {
    mocks.runBatch.mockRejectedValue(new Error("diarizer failed"));
    const { result } = renderHook(() => useSeparateSpeakers("session-1"));

    await act(async () => {
      await result.current("transcript-1");
    });

    expect(mocks.handleBatchFailed).toHaveBeenCalledWith(
      "session-1",
      "diarizer failed",
    );
    expect(mocks.toastError).toHaveBeenCalledWith(
      "Speakers could not be separated",
      {
        id: "transcript-separate-failed-session-1",
        description: "diarizer failed",
      },
    );
  });
});
