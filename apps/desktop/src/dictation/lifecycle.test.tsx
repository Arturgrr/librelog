import { act, cleanup, render, waitFor } from "@testing-library/react";
import { MockLanguageModelV3 } from "ai/test";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  connection: null as {
    provider: string;
    model: string;
    apiKey: string;
    baseUrl: string;
  } | null,
  platform: "macos",
  settings: {
    dictation_enabled: true,
    dictation_shortcuts: ["Control+Alt+Space"],
    dictation_hands_free: false,
    microphone_device: "",
    dictation_avoid_bluetooth_microphone: true,
  } as Record<string, unknown>,
  meeting: { status: "inactive", loading: false },
  configure: vi.fn(),
  setActive: vi.fn(),
  hud: vi.fn(),
  checkInsertion: vi.fn(),
  frontmostApp: vi.fn(),
  startSystemRecording: vi.fn(),
  setWarmMicrophone: vi.fn(async () => ({ status: "ok", data: null })),
  stopRecording: vi.fn(),
  cancelRecording: vi.fn(),
  discardRecording: vi.fn(),
  insertText: vi.fn(),
  insertDictation: vi.fn(),
  readDictationField: vi.fn(),
  prepareDictationRefinement: vi.fn(),
  resolveDictationRefinement: vi.fn(),
  model: null as unknown,
  rankDictationTerms: vi.fn(),
  recordUsedTerms: vi.fn(),
  checkPermission: vi.fn(),
  getCaptureState: vi.fn(),
  runBatch: vi.fn(),
  play: vi.fn(),
  recordDictation: vi.fn(),
  holdLocalModel: vi.fn(),
  releaseModel: vi.fn(),
  keepOnClipboard: vi.fn(),
  stopTranscription: vi.fn(),
  unlisten: vi.fn(),
  listener: null as ((event: { payload: { type: string } }) => void) | null,
}));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    constructor(public onmessage: (event: unknown) => void) {}
  },
}));
vi.mock("~/stt/useSTTConnection", () => ({
  useSTTConnection: () => ({
    conn: mocks.connection,
  }),
}));
vi.mock("~/ai/hooks/useLLMConnection", () => ({
  useLanguageModel: () => mocks.model,
}));
vi.mock("@tauri-apps/plugin-os", () => ({ platform: () => mocks.platform }));
vi.mock("~/settings/queries", () => ({ useSettingsReady: () => true }));
vi.mock("~/shared/config", () => ({
  useConfigValue: (key: string) => mocks.settings[key],
}));
vi.mock("~/stt/contexts", () => ({
  useListener: (select: (state: unknown) => unknown) =>
    select({ live: mocks.meeting, stopTranscription: mocks.stopTranscription }),
}));
vi.mock("~/stt/useRunBatch", () => ({ useRunBatch: () => mocks.runBatch }));
vi.mock("~/stt/local-model-hold", () => ({
  holdLocalModelUntilReleased: mocks.holdLocalModel,
}));
vi.mock("@anlg/plugin-shortcut", () => ({
  commands: { configure: mocks.configure, setActive: mocks.setActive },
  events: {
    shortcutEvent: {
      listen: vi.fn(async (listener) => {
        mocks.listener = listener;
        return mocks.unlisten;
      }),
    },
  },
}));
vi.mock("./history/store", () => ({
  recordDictation: mocks.recordDictation,
}));
vi.mock("./clipboard", () => ({ keepOnClipboard: mocks.keepOnClipboard }));
vi.mock("./learning", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./learning")>()),
  recordUsedTerms: mocks.recordUsedTerms,
}));
vi.mock("@anlg/plugin-dictation", () => ({
  commands: {
    hud: mocks.hud,
    checkInsertion: mocks.checkInsertion,
    frontmostApp: mocks.frontmostApp,
    startSystemRecording: mocks.startSystemRecording,
    setWarmMicrophone: mocks.setWarmMicrophone,
    stopRecording: mocks.stopRecording,
    cancelRecording: mocks.cancelRecording,
    discardRecording: mocks.discardRecording,
    insertText: mocks.insertText,
    // What a fitted insertion typed, as heard; `insertText` decides how it went.
    insertDictation: async (
      text: string,
      mode: unknown,
      exclusions: unknown,
    ) => {
      mocks.insertDictation(text, mode, exclusions);
      const result = await mocks.insertText(text);
      return result.status === "ok" ? { status: "ok", data: text } : result;
    },
    readDictationField: mocks.readDictationField,
    prepareDictationRefinement: mocks.prepareDictationRefinement,
    resolveDictationRefinement: mocks.resolveDictationRefinement,
    rankDictationTerms: mocks.rankDictationTerms,
  },
}));
vi.mock("@anlg/plugin-permissions", () => ({
  commands: { checkPermission: mocks.checkPermission },
}));
vi.mock("@anlg/plugin-transcription", () => ({
  commands: { getCaptureState: mocks.getCaptureState },
}));
vi.mock("@anlg/plugin-sfx", () => ({ commands: { play: mocks.play } }));
vi.mock("@anlg/ui/components/ui/toast", () => ({
  toast: { error: vi.fn() },
}));

import {
  DictationLifecycle,
  useDictationStatus,
  waitForDictationCleanup,
} from "./lifecycle";

const USED = { provider: "deepgram", model: "nova-3", languages: ["en"] };

/** A batch transcription that hears `text`. */
const hears =
  (text: string) =>
  async (
    _path: string,
    options: { handlePersist: (words: object[]) => void },
  ) => {
    options.handlePersist([{ text, start_ms: 0 }]);
    return USED;
  };

/** The Rust recorder reports each capture's end over its channel. */
const endCapture = () => {
  const channel = mocks.startSystemRecording.mock.lastCall?.[3];
  channel?.onmessage({ type: "ended" });
};

describe("dictation access and lifecycle", () => {
  afterEach(async () => {
    cleanup();
    await waitForDictationCleanup();
  });
  it("retains the last transcript through reconfiguration and clears it when disabled", async () => {
    const { rerender } = render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    useDictationStatus.setState({ lastTranscript: "Recover this text" });
    mocks.settings.dictation_shortcuts = ["Control+Alt+Space", "Fn"];
    rerender(<DictationLifecycle />);
    await waitFor(() =>
      expect(mocks.configure).toHaveBeenLastCalledWith([
        "Control+Alt+Space",
        "Fn",
      ]),
    );
    expect(useDictationStatus.getState().lastTranscript).toBe(
      "Recover this text",
    );
    mocks.settings.dictation_enabled = false;
    rerender(<DictationLifecycle />);
    expect(useDictationStatus.getState().lastTranscript).toBe("");
  });

  it("unregisters dictation while capturing a shortcut and restores it afterwards", async () => {
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    act(() => useDictationStatus.setState({ capturingShortcut: true }));
    await waitFor(() => expect(mocks.configure).toHaveBeenLastCalledWith([]));
    expect(useDictationStatus.getState().ready).toBe(false);
    act(() => useDictationStatus.setState({ capturingShortcut: false }));
    await waitFor(() =>
      expect(mocks.configure).toHaveBeenLastCalledWith(["Control+Alt+Space"]),
    );
    expect(mocks.startSystemRecording).not.toHaveBeenCalled();
  });

  beforeEach(() => {
    vi.clearAllMocks();
    useDictationStatus.setState({ capturingShortcut: false });
    mocks.platform = "macos";
    mocks.connection = null;
    mocks.settings.dictation_live_preview = false;
    mocks.settings.dictation_sounds = true;
    mocks.settings.dictation_enabled = true;
    mocks.settings.dictation_shortcuts = ["Control+Alt+Space"];
    mocks.settings.microphone_device = "";
    mocks.settings.dictation_audio_behavior = "nothing";
    mocks.settings.dictation_hud_position = "top";
    mocks.settings.dictation_presets = "[]";
    mocks.settings.dictation_learning = false;
    mocks.settings.dictation_warm_microphone = false;
    mocks.meeting = { status: "inactive", loading: false };
    mocks.listener = null;
    for (const fn of [
      mocks.runBatch,
      mocks.stopRecording,
      mocks.cancelRecording,
    ])
      fn.mockReset();
    for (const fn of [
      mocks.configure,
      mocks.setActive,
      mocks.hud,
      mocks.checkInsertion,
      mocks.startSystemRecording,
      mocks.discardRecording,
      mocks.insertText,
    ])
      fn.mockResolvedValue({ status: "ok", data: null });
    mocks.frontmostApp.mockResolvedValue({
      status: "ok",
      data: { name: "Notes", id: "com.apple.Notes" },
    });
    mocks.recordDictation.mockResolvedValue({ audioKept: false });
    mocks.holdLocalModel.mockReturnValue(mocks.releaseModel);
    mocks.rankDictationTerms.mockResolvedValue([]);
    mocks.recordUsedTerms.mockResolvedValue(undefined);
    mocks.keepOnClipboard.mockResolvedValue(undefined);
    mocks.model = null;
    mocks.settings.dictation_voice_edit = false;
    mocks.readDictationField.mockResolvedValue({
      text: null,
      selection: null,
      fingerprint: null,
      context: null,
    });
    mocks.prepareDictationRefinement.mockImplementation(
      async (request: { text: string }) => ({
        type: "done",
        outcome: { type: "raw", text: request.text, reason: "cleanupOff" },
      }),
    );
    mocks.cancelRecording.mockImplementation(async () => {
      endCapture();
      return { status: "ok", data: null };
    });
    mocks.checkPermission.mockResolvedValue({
      status: "ok",
      data: "authorized",
    });
    mocks.getCaptureState.mockResolvedValue({ status: "ok", data: "inactive" });
    mocks.stopRecording.mockImplementation(async () => {
      endCapture();
      return {
        status: "ok",
        data: { filePath: "/tmp/dictation.wav", durationMs: 1_000 },
      };
    });
    mocks.stopTranscription.mockResolvedValue(undefined);
    mocks.runBatch.mockImplementation(hears("Hello"));
    useDictationStatus.setState({
      ready: false,
      phase: "idle",
      lastTranscript: "",
      error: null,
      retry: 0,
      cancel: null,
    });
  });

  it("does not register shortcuts when disabled", async () => {
    mocks.settings.dictation_enabled = false;
    render(<DictationLifecycle />);
    await act(async () => {});
    expect(mocks.configure).not.toHaveBeenCalled();
    expect(mocks.startSystemRecording).not.toHaveBeenCalled();
  });

  it("dictates into other apps while a meeting records, without playing cues into it", async () => {
    mocks.meeting.status = "active";
    mocks.getCaptureState.mockResolvedValue({ status: "ok", data: "active" });
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    await act(async () => {
      mocks.listener?.({ payload: { type: "released" } });
    });
    await waitFor(() => expect(mocks.insertText).toHaveBeenCalledWith("Hello"));
    expect(mocks.play).not.toHaveBeenCalled();
  });

  it("keeps a dictation going when a meeting starts during it", async () => {
    const view = render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    await waitFor(() =>
      expect(useDictationStatus.getState().phase).toBe("recording"),
    );
    mocks.meeting = { status: "active", loading: false };
    view.rerender(<DictationLifecycle />);
    await act(async () => {
      mocks.listener?.({ payload: { type: "released" } });
    });
    await waitFor(() => expect(mocks.insertText).toHaveBeenCalledWith("Hello"));
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
  });

  it.each([
    ["a self-hosted server", "http://127.0.0.1:50060", false],
    ["a cloud provider", "https://api.deepgram.com", true],
  ])(
    "during a meeting, opens a live connection to %s only if it can take a second one",
    async (_target, baseUrl, streams) => {
      mocks.meeting.status = "active";
      mocks.connection = {
        provider: "deepgram",
        model: "nova-3",
        apiKey: "test",
        baseUrl,
      };
      render(<DictationLifecycle />);
      await waitFor(() =>
        expect(useDictationStatus.getState().ready).toBe(true),
      );
      await act(async () => {
        mocks.listener?.({ payload: { type: "pressed" } });
      });
      expect(mocks.startSystemRecording.mock.calls[0]![2]).toEqual(
        streams ? expect.objectContaining({ baseUrl }) : null,
      );
    },
  );

  it("registers without an account or plan and inserts into the captured field", async () => {
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    expect(mocks.configure).toHaveBeenCalledWith(["Control+Alt+Space"]);
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    await act(async () => {
      mocks.listener?.({ payload: { type: "released" } });
    });
    await waitFor(() => expect(mocks.insertText).toHaveBeenCalledWith("Hello"));
    expect(mocks.startSystemRecording).toHaveBeenCalledWith(
      null,
      expect.stringMatching(/^system-dictation-/u),
      null,
      expect.anything(),
      expect.anything(),
      expect.any(Number),
      true,
    );
    expect(mocks.discardRecording).toHaveBeenCalledWith("/tmp/dictation.wav");
  });

  it("keeps the microphone open only while Keep microphone ready is on", async () => {
    mocks.settings.dictation_warm_microphone = true;
    mocks.settings.microphone_device = "USB microphone";
    mocks.checkPermission.mockResolvedValue({
      status: "ok",
      data: "authorized",
    });
    const view = render(<DictationLifecycle />);
    await waitFor(() =>
      expect(mocks.setWarmMicrophone).toHaveBeenCalledWith(
        true,
        "USB microphone",
        true,
      ),
    );
    view.unmount();
    await waitFor(() =>
      expect(mocks.setWarmMicrophone).toHaveBeenLastCalledWith(
        false,
        null,
        true,
      ),
    );
  });

  it("shows the HUD before the microphone opens and tells it how the dictation ends", async () => {
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    expect(mocks.hud.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.startSystemRecording.mock.invocationCallOrder[0]!,
    );
    await act(async () => {
      mocks.listener?.({ payload: { type: "released" } });
    });
    await waitFor(() =>
      expect(mocks.hud.mock.calls.map(([event]) => event.type)).toEqual([
        "started",
        "processing",
        "completed",
      ]),
    );

    mocks.hud.mockClear();
    mocks.runBatch.mockImplementation(async () => USED);
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    await act(async () => {
      mocks.listener?.({ payload: { type: "released" } });
    });
    await waitFor(() =>
      expect(mocks.hud).toHaveBeenLastCalledWith({ type: "failed" }),
    );

    mocks.hud.mockClear();
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    await act(async () => {
      mocks.listener?.({ payload: { type: "cancelled" } });
    });
    await waitFor(() =>
      expect(mocks.hud).toHaveBeenLastCalledWith({ type: "cancelled" }),
    );
  });

  it("silences the next dictation's cues as soon as sounds are turned off, without restarting dictation", async () => {
    const view = render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    const dictate = async (count: number) => {
      await act(async () => {
        mocks.listener?.({ payload: { type: "pressed" } });
      });
      await act(async () => {
        mocks.listener?.({ payload: { type: "released" } });
      });
      await waitFor(() =>
        expect(mocks.insertText).toHaveBeenCalledTimes(count),
      );
      await waitFor(() =>
        expect(useDictationStatus.getState().phase).toBe("idle"),
      );
    };
    await dictate(1);
    expect(mocks.play.mock.calls).toEqual([
      ["StartRecording"],
      ["StopRecording"],
    ]);

    mocks.play.mockClear();
    mocks.settings.dictation_sounds = false;
    view.rerender(<DictationLifecycle />);
    await dictate(2);
    expect(mocks.play).not.toHaveBeenCalled();
  });

  it("still dictates when the HUD cannot show, since the indicator is only a cue", async () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    mocks.hud.mockResolvedValue({ status: "error", error: "no panel" });
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    expect(mocks.startSystemRecording).toHaveBeenCalled();
    expect(useDictationStatus.getState().error).toBeNull();
    expect(logged).toHaveBeenCalledWith(
      expect.stringMatching(/indicator/u),
      "no panel",
    );
    await act(async () => {
      mocks.listener?.({ payload: { type: "released" } });
    });
    await waitFor(() => expect(mocks.insertText).toHaveBeenCalledWith("Hello"));
    logged.mockRestore();
  });

  it("starts the HUD where the settings put it, read at each press", async () => {
    mocks.settings.dictation_hud_position = "bottom";
    const view = render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    expect(mocks.hud).toHaveBeenCalledWith({
      type: "started",
      position: "bottom",
    });
    await act(async () => {
      mocks.listener?.({ payload: { type: "cancelled" } });
    });

    mocks.hud.mockClear();
    mocks.platform = "windows";
    mocks.settings.dictation_hud_position = "notch";
    view.rerender(<DictationLifecycle />);
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    expect(mocks.hud).toHaveBeenCalledWith({
      type: "started",
      position: "top",
    });
  });

  it("reuses the finalized live transcript even when preview display is disabled", async () => {
    mocks.settings.dictation_live_preview = false;
    mocks.connection = {
      provider: "wisprflow",
      model: "flow",
      apiKey: "test",
      baseUrl: "https://platform-api.wisprflow.ai",
    };
    mocks.stopRecording.mockResolvedValue({
      status: "ok",
      data: { filePath: "/tmp/dictation.wav", transcript: "Finished live." },
    });
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    expect(mocks.startSystemRecording.mock.calls[0]![2]).toEqual(
      expect.objectContaining({ provider: "wisprflow" }),
    );
    const channel = mocks.startSystemRecording.mock.calls[0]![3];
    await act(async () => {
      channel.onmessage({
        type: "transcript",
        text: "Hidden",
        partial: "words",
      });
      channel.onmessage({ type: "previewUnavailable" });
    });
    expect(mocks.hud).not.toHaveBeenCalledWith(
      expect.objectContaining({ type: "preview" }),
    );
    await act(async () => {
      mocks.listener?.({ payload: { type: "released" } });
    });
    await waitFor(() =>
      expect(mocks.insertText).toHaveBeenCalledWith("Finished live."),
    );
    expect(mocks.runBatch).not.toHaveBeenCalled();
    expect(mocks.discardRecording).toHaveBeenCalledWith("/tmp/dictation.wav");
  });

  it("aborts and removes shortcuts when dictation is disabled during recording", async () => {
    const view = render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    await waitFor(() =>
      expect(useDictationStatus.getState().phase).toBe("recording"),
    );
    mocks.settings.dictation_enabled = false;
    view.rerender(<DictationLifecycle />);
    await waitFor(() => expect(mocks.configure).toHaveBeenLastCalledWith([]));
    expect(mocks.cancelRecording).toHaveBeenCalledWith(
      expect.stringMatching(/^system-dictation-/u),
    );
    expect(mocks.insertText).not.toHaveBeenCalled();
    expect(useDictationStatus.getState().lastTranscript).toBe("");
  });

  it("reports missing microphone permission without capturing a target or audio", async () => {
    mocks.checkPermission.mockResolvedValue({ status: "ok", data: "denied" });
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    expect(mocks.hud).not.toHaveBeenCalled();
    expect(mocks.checkInsertion).not.toHaveBeenCalled();
    expect(mocks.startSystemRecording).not.toHaveBeenCalled();
    expect(useDictationStatus.getState().error).toMatch(
      /microphone permission/u,
    );
  });

  it("holds the local model only once the start can no longer fail before the microphone, and releases it if the microphone fails", async () => {
    mocks.connection = {
      provider: "soniqo",
      model: "soniqo-parakeet-batch",
      apiKey: "",
      baseUrl: "http://127.0.0.1:50060",
    };
    mocks.checkInsertion.mockResolvedValueOnce({
      status: "error",
      error: "No text field is focused.",
    });
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    await waitFor(() =>
      expect(useDictationStatus.getState().phase).toBe("idle"),
    );
    expect(mocks.holdLocalModel).not.toHaveBeenCalled();

    mocks.startSystemRecording.mockResolvedValueOnce({
      status: "error",
      error: "microphone unavailable",
    });
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    await waitFor(() =>
      expect(useDictationStatus.getState().phase).toBe("idle"),
    );
    expect(mocks.holdLocalModel).toHaveBeenCalledTimes(1);
    expect(mocks.holdLocalModel.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.startSystemRecording.mock.invocationCallOrder[0]!,
    );
    expect(mocks.releaseModel).toHaveBeenCalled();
  });

  it.each(["windows", "linux"])(
    "opens the selected microphone on %s without probing a potentially missing default device",
    async (platform) => {
      mocks.platform = platform;
      mocks.settings.microphone_device = "USB microphone";
      mocks.checkPermission.mockResolvedValue({ status: "ok", data: "denied" });
      render(<DictationLifecycle />);
      await waitFor(() =>
        expect(useDictationStatus.getState().ready).toBe(true),
      );
      await act(async () => {
        mocks.listener?.({ payload: { type: "pressed" } });
      });
      expect(mocks.checkPermission).not.toHaveBeenCalled();
      expect(mocks.startSystemRecording).toHaveBeenCalledWith(
        "USB microphone",
        expect.stringMatching(/^system-dictation-/u),
        null,
        expect.anything(),
        expect.anything(),
        expect.any(Number),
        true,
      );
    },
  );
  it("shows live words without inserting until finish, then ignores stale preview updates", async () => {
    mocks.settings.dictation_live_preview = true;
    mocks.connection = {
      provider: "deepgram",
      model: "nova-3",
      apiKey: "test",
      baseUrl: "https://api.deepgram.com",
    };
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    const channel = mocks.startSystemRecording.mock.calls[0]![3];
    expect(mocks.startSystemRecording.mock.calls[0]![2]).toEqual(
      expect.objectContaining({ provider: "deepgram", apiKey: "test" }),
    );
    await act(async () => {
      channel.onmessage({
        type: "transcript",
        text: "Hello",
        partial: "there",
      });
    });
    expect(mocks.hud).toHaveBeenLastCalledWith({
      type: "preview",
      stable: "Hello",
      tentative: " there",
    });
    expect(mocks.insertText).not.toHaveBeenCalled();
    await act(async () => {
      mocks.listener?.({ payload: { type: "released" } });
    });
    await waitFor(() => expect(mocks.insertText).toHaveBeenCalledWith("Hello"));
    await act(async () => {
      channel.onmessage({ type: "transcript", text: "Stale", partial: "" });
    });
    expect(mocks.hud).toHaveBeenLastCalledWith({ type: "completed" });
    expect(useDictationStatus.getState().lastTranscript).toBe("Hello");
  });

  it("drops preview updates after cancellation and does not transcribe or insert", async () => {
    mocks.settings.dictation_live_preview = true;
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    const channel = mocks.startSystemRecording.mock.calls[0]![3];
    await act(async () => {
      useDictationStatus.getState().cancel?.();
    });
    await waitFor(() =>
      expect(useDictationStatus.getState().phase).toBe("idle"),
    );
    await act(async () => {
      channel.onmessage({
        type: "transcript",
        text: "Cancelled speech",
        partial: "",
      });
    });
    expect(mocks.hud).toHaveBeenLastCalledWith({ type: "cancelled" });
    expect(mocks.runBatch).not.toHaveBeenCalled();
    expect(mocks.insertText).not.toHaveBeenCalled();
  });
  const shortcut = async (type: string) => {
    await act(async () => {
      mocks.listener?.({ payload: { type } });
    });
  };
  const holdFirstTranscription = () => {
    let finish!: () => void;
    mocks.runBatch.mockImplementationOnce(async (_path, options) => {
      await new Promise<void>((resolve) => {
        finish = resolve;
        options.signal.addEventListener("abort", resolve);
      });
      options.handlePersist([{ text: "First.", start_ms: 0 }]);
      return USED;
    });
    return () => act(async () => finish());
  };

  it("dictates again while the last dictation transcribes, taking Escape only during each capture and inserting in order", async () => {
    const finishFirst = holdFirstTranscription();
    mocks.runBatch.mockImplementationOnce(hears("Second."));
    for (const filePath of ["/tmp/1.wav", "/tmp/2.wav"])
      mocks.stopRecording.mockImplementationOnce(async () => {
        endCapture();
        return { status: "ok", data: { filePath, durationMs: 1_000 } };
      });
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    await shortcut("released");
    await waitFor(() =>
      expect(mocks.setActive).toHaveBeenLastCalledWith(false),
    );
    expect(useDictationStatus.getState()).toMatchObject({
      phase: "transcribing",
      capturing: false,
      pending: 1,
    });
    await shortcut("pressed");
    await waitFor(() =>
      expect(useDictationStatus.getState()).toMatchObject({
        phase: "recording",
        capturing: true,
        pending: 1,
      }),
    );
    await waitFor(() => expect(mocks.setActive).toHaveBeenLastCalledWith(true));
    await shortcut("released");
    await finishFirst();
    await waitFor(() =>
      expect(useDictationStatus.getState().phase).toBe("idle"),
    );
    expect(mocks.insertText.mock.calls).toEqual([["First."], ["Second."]]);
    expect(mocks.hud.mock.calls.map(([event]) => event.type)).toEqual([
      "started",
      "processing",
      "started",
      "processing",
      "completed",
    ]);
    expect(mocks.discardRecording.mock.calls).toEqual([
      ["/tmp/1.wav"],
      ["/tmp/2.wav"],
    ]);
  });

  it("cancels only the new capture when Escape is pressed while an earlier dictation transcribes", async () => {
    const finishFirst = holdFirstTranscription();
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    await shortcut("released");
    await shortcut("pressed");
    await waitFor(() =>
      expect(useDictationStatus.getState().phase).toBe("recording"),
    );
    await shortcut("cancelled");
    await waitFor(() =>
      expect(useDictationStatus.getState().phase).toBe("transcribing"),
    );
    expect(mocks.cancelRecording).toHaveBeenCalledOnce();
    await finishFirst();
    await waitFor(() =>
      expect(mocks.insertText).toHaveBeenCalledWith("First."),
    );
    expect(mocks.hud.mock.calls.map(([event]) => event.type)).toEqual([
      "started",
      "processing",
      "started",
      "processing",
      "completed",
    ]);
  });

  it("still inserts and records a dictation waiting to be transcribed when the shortcuts change", async () => {
    const finishFirst = holdFirstTranscription();
    const view = render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    await shortcut("released");
    await waitFor(() => expect(mocks.runBatch).toHaveBeenCalled());
    mocks.settings.dictation_shortcuts = ["Control+Alt+Space", "Fn"];
    view.rerender(<DictationLifecycle />);
    await waitFor(() =>
      expect(mocks.configure).toHaveBeenLastCalledWith([
        "Control+Alt+Space",
        "Fn",
      ]),
    );
    await finishFirst();
    await waitFor(() =>
      expect(mocks.recordDictation).toHaveBeenCalledWith(
        expect.objectContaining({ finalText: "First.", inserted: true }),
      ),
    );
    expect(mocks.insertText).toHaveBeenCalledWith("First.");
  });

  it("holds a ready dictation while a shortcut is recorded, so its paste never lands in Settings", async () => {
    const finishFirst = holdFirstTranscription();
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    await shortcut("released");
    await waitFor(() => expect(mocks.runBatch).toHaveBeenCalled());
    act(() => useDictationStatus.setState({ capturingShortcut: true }));
    await finishFirst();
    await waitFor(() => expect(mocks.configure).toHaveBeenLastCalledWith([]));
    expect(mocks.insertText).not.toHaveBeenCalled();
    act(() => useDictationStatus.setState({ capturingShortcut: false }));
    await waitFor(() =>
      expect(mocks.insertText).toHaveBeenCalledWith("First."),
    );
  });

  it("records without a live connection to a self-hosted server while an earlier dictation is still transcribing", async () => {
    mocks.settings.dictation_live_preview = true;
    mocks.connection = {
      provider: "soniqo",
      model: "soniqo-parakeet-streaming",
      apiKey: "",
      baseUrl: "http://127.0.0.1:1234",
    };
    const finishFirst = holdFirstTranscription();
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    await shortcut("released");
    await shortcut("pressed");
    expect(mocks.startSystemRecording.mock.calls[0]![2]).toEqual(
      expect.objectContaining({ baseUrl: "http://127.0.0.1:1234" }),
    );
    expect(mocks.startSystemRecording.mock.calls[1]![2]).toBeNull();
    await finishFirst();
  });

  it("streams the live preview to the selected local server", async () => {
    mocks.settings.dictation_live_preview = true;
    mocks.connection = {
      provider: "soniqo",
      model: "soniqo-parakeet-streaming",
      apiKey: "",
      baseUrl: "http://127.0.0.1:1234",
    };
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await act(async () => {
      mocks.listener?.({ payload: { type: "pressed" } });
    });
    expect(mocks.startSystemRecording.mock.calls[0]![2]).toEqual(
      expect.objectContaining({ baseUrl: "http://127.0.0.1:1234", apiKey: "" }),
    );
  });
  it("dictates in the active preset's language instead of the spoken languages", async () => {
    mocks.settings.dictation_live_preview = true;
    mocks.settings.spoken_languages = ["en"];
    mocks.settings.current_stt_provider = "soniqo";
    mocks.settings.current_stt_model = "soniqo-parakeet-streaming";
    mocks.settings.dictation_preset_active = "pt";
    mocks.settings.dictation_presets = JSON.stringify([
      {
        id: "pt",
        language: "pt",
        provider: "soniqo",
        model: "soniqo-parakeet-streaming",
        modelLabel: "Parakeet",
      },
    ]);
    mocks.connection = {
      provider: "soniqo",
      model: "soniqo-parakeet-streaming",
      apiKey: "",
      baseUrl: "https://stt.example.com",
    };
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    expect(mocks.startSystemRecording.mock.calls[0]![2]).toEqual(
      expect.objectContaining({
        params: expect.objectContaining({ languages: ["pt"] }),
      }),
    );
    endCapture();
    await shortcut("released");
    await waitFor(() =>
      expect(mocks.runBatch).toHaveBeenCalledWith(
        "/tmp/dictation.wav",
        expect.objectContaining({ languages: ["pt"] }),
      ),
    );
  });

  it("sends the ranked dictionary to batch transcription and weighs the terms once inserted", async () => {
    mocks.settings.personalization_dictionary_terms = ["Librelog"];
    mocks.settings.personalization_dictionary_replacements = [
      { from: "work mail", to: "me@example.com" },
    ];
    mocks.settings.dictation_learning = true;
    mocks.settings.dictation_term_weights = "weights";
    mocks.rankDictationTerms.mockResolvedValue(["Librelog", "me@example.com"]);
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    await shortcut("released");
    await waitFor(() => expect(mocks.insertText).toHaveBeenCalledWith("Hello"));
    expect(mocks.rankDictationTerms).toHaveBeenCalledWith(
      expect.objectContaining({
        dictionary: ["Librelog"],
        replacements: [{ from: "work mail", to: "me@example.com" }],
        weights: "weights",
      }),
    );
    expect(mocks.runBatch).toHaveBeenCalledWith(
      "/tmp/dictation.wav",
      expect.objectContaining({ keywords: ["Librelog", "me@example.com"] }),
    );
    await waitFor(() =>
      expect(mocks.recordUsedTerms).toHaveBeenCalledWith("Hello"),
    );
  });

  it("records each dictation with the app it went to and what transcribed it, leaving audio the history kept", async () => {
    mocks.recordDictation.mockResolvedValue({ audioKept: true });
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    await shortcut("released");
    await waitFor(() => expect(mocks.recordDictation).toHaveBeenCalled());
    expect(mocks.frontmostApp.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.hud.mock.invocationCallOrder[0]!,
    );
    expect(mocks.recordDictation).toHaveBeenCalledWith(
      expect.objectContaining({
        targetApp: { name: "Notes", id: "com.apple.Notes" },
        provider: "deepgram",
        model: "nova-3",
        language: "en",
        finalText: "Hello",
        inserted: true,
        audioPath: "/tmp/dictation.wav",
      }),
    );
    await waitFor(() =>
      expect(useDictationStatus.getState().phase).toBe("idle"),
    );
    expect(mocks.discardRecording).not.toHaveBeenCalled();
  });

  it("rewrites the selection by voice only over the selection it read, keeping the spoken instruction as raw text", async () => {
    mocks.settings.dictation_voice_edit = true;
    mocks.settings.dictation_context_exclusions = { apps: [], websites: [] };
    mocks.model = new MockLanguageModelV3({
      doGenerate: async () => ({
        content: [{ type: "text", text: "Ship it Friday." }],
        finishReason: { unified: "stop", raw: "stop" },
        usage: {
          inputTokens: {
            total: 4,
            noCache: 4,
            cacheRead: 0,
            cacheWrite: 0,
          },
          outputTokens: { total: 2, text: 2, reasoning: 0 },
        },
        warnings: [],
      }),
    });
    mocks.readDictationField.mockResolvedValue({
      text: { before: "", selected: "Ship it Monday.", after: "" },
      selection: "Ship it Monday.",
      fingerprint: "field-1",
      context: null,
    });
    mocks.prepareDictationRefinement.mockImplementation(
      async (request: { text: string; selection: string | null }) => ({
        type: "ask",
        call: { system: "rules", prompt: request.text, deadlineMs: 5_000 },
        pending: {
          kind: "edit",
          raw: request.text,
          selection: request.selection,
          before: null,
          instructed: false,
          replacements: [],
        },
      }),
    );
    mocks.resolveDictationRefinement.mockImplementation(
      async (_pending: unknown, reply: { type: string; text?: string }) =>
        reply.type === "text"
          ? { type: "edited", text: reply.text }
          : { type: "editFailed", reason: reply.type },
    );
    mocks.runBatch.mockImplementation(hears("Make it Friday"));
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    await shortcut("released");
    await waitFor(() => expect(mocks.recordDictation).toHaveBeenCalled());
    expect(mocks.prepareDictationRefinement).toHaveBeenCalledWith(
      expect.objectContaining({ selection: "Ship it Monday." }),
    );
    expect(mocks.insertDictation).toHaveBeenCalledWith(
      "Ship it Friday.",
      { type: "replaceSelection", fingerprint: "field-1" },
      { apps: [], websites: [] },
    );
    expect(mocks.recordDictation).toHaveBeenCalledWith(
      expect.objectContaining({
        rawText: "Make it Friday",
        finalText: "Ship it Friday.",
        inserted: true,
      }),
    );
  });

  it("types over the selection as said without a model, and keeps a failed voice edit's instruction from being the text", async () => {
    mocks.settings.dictation_voice_edit = true;
    mocks.readDictationField.mockResolvedValue({
      text: { before: "", selected: "Ship it Monday.", after: "" },
      selection: "Ship it Monday.",
      fingerprint: "field-1",
      context: null,
    });
    mocks.runBatch.mockImplementation(hears("Make it Friday"));
    const view = render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    await shortcut("released");
    await waitFor(() => expect(mocks.recordDictation).toHaveBeenCalled());
    expect(mocks.prepareDictationRefinement).toHaveBeenCalledWith(
      expect.objectContaining({ selection: null }),
    );
    expect(mocks.insertDictation.mock.calls[0]?.slice(0, 2)).toEqual([
      "Make it Friday",
      { type: "fit" },
    ]);

    mocks.recordDictation.mockClear();
    mocks.insertDictation.mockClear();
    mocks.model = new MockLanguageModelV3();
    view.rerender(<DictationLifecycle />);
    mocks.prepareDictationRefinement.mockResolvedValue({
      type: "done",
      outcome: { type: "editFailed", reason: "tooLong" },
    });
    await waitFor(() =>
      expect(useDictationStatus.getState().phase).toBe("idle"),
    );
    await shortcut("pressed");
    await shortcut("released");
    await waitFor(() => expect(mocks.recordDictation).toHaveBeenCalled());
    expect(mocks.insertDictation).not.toHaveBeenCalled();
    expect(mocks.recordDictation).toHaveBeenCalledWith(
      expect.objectContaining({
        rawText: "Make it Friday",
        finalText: "",
        inserted: false,
      }),
    );
  });

  it("applies the spoken commands turned on, in the dictated languages, with no model call", async () => {
    const doGenerate = vi.fn();
    mocks.model = new MockLanguageModelV3({ doGenerate });
    Object.assign(mocks.settings, {
      ai_language: "en",
      spoken_languages: ["pt-BR"],
      dictation_cleanup: false,
      dictation_remove_fillers: true,
      dictation_spoken_formatting: true,
      dictation_spoken_symbols: false,
    });
    mocks.prepareDictationRefinement.mockResolvedValue({
      type: "done",
      outcome: { type: "raw", text: "Eu acho.\nTchau", reason: "cleanupOff" },
    });
    mocks.runBatch.mockImplementation(hears("É... eu acho. Nova linha. Tchau"));
    try {
      render(<DictationLifecycle />);
      await waitFor(() =>
        expect(useDictationStatus.getState().ready).toBe(true),
      );
      await shortcut("pressed");
      await shortcut("released");
      await waitFor(() => expect(mocks.recordDictation).toHaveBeenCalled());
      expect(mocks.prepareDictationRefinement).toHaveBeenCalledWith(
        expect.objectContaining({
          text: "É... eu acho. Nova linha. Tchau",
          policy: { cleanup: false, voiceEdit: false },
          spoken: {
            languages: ["en", "pt-BR"],
            fillers: true,
            formatting: true,
            symbols: false,
          },
        }),
      );
      expect(mocks.recordDictation).toHaveBeenCalledWith(
        expect.objectContaining({
          finalText: "Eu acho.\nTchau",
          inserted: true,
        }),
      );
      expect(doGenerate).not.toHaveBeenCalled();
    } finally {
      for (const key of [
        "ai_language",
        "spoken_languages",
        "dictation_cleanup",
        "dictation_remove_fillers",
        "dictation_spoken_formatting",
        "dictation_spoken_symbols",
      ]) {
        delete mocks.settings[key];
      }
    }
  });

  it("applies the spoken commands in the active preset's language", async () => {
    Object.assign(mocks.settings, {
      ai_language: "en",
      spoken_languages: ["en"],
      dictation_cleanup: false,
      dictation_remove_fillers: true,
      dictation_preset_active: "pt",
      current_stt_provider: "deepgram",
      current_stt_model: "nova-3",
      dictation_presets: JSON.stringify([
        {
          id: "pt",
          language: "pt",
          provider: "deepgram",
          model: "nova-3",
          modelLabel: "Nova 3",
        },
      ]),
    });
    mocks.prepareDictationRefinement.mockResolvedValue({
      type: "done",
      outcome: { type: "raw", text: "Eu quero um café", reason: "cleanupOff" },
    });
    mocks.runBatch.mockImplementation(hears("Eu quero um café"));
    try {
      render(<DictationLifecycle />);
      await waitFor(() =>
        expect(useDictationStatus.getState().ready).toBe(true),
      );
      await shortcut("pressed");
      await shortcut("released");
      await waitFor(() => expect(mocks.recordDictation).toHaveBeenCalled());
      expect(mocks.prepareDictationRefinement).toHaveBeenCalledWith(
        expect.objectContaining({
          text: "Eu quero um café",
          spoken: expect.objectContaining({ languages: ["pt"], fillers: true }),
        }),
      );
    } finally {
      for (const key of [
        "ai_language",
        "spoken_languages",
        "dictation_cleanup",
        "dictation_remove_fillers",
        "dictation_preset_active",
        "current_stt_provider",
        "current_stt_model",
      ]) {
        delete mocks.settings[key];
      }
    }
  });

  it("keeps a cancelled dictation out of the history even when its text was ready", async () => {
    const finishFirst = holdFirstTranscription();
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    await shortcut("released");
    await shortcut("pressed");
    await finishFirst();
    await act(async () => {
      useDictationStatus.getState().cancel?.();
    });
    await waitFor(() =>
      expect(useDictationStatus.getState().phase).toBe("idle"),
    );
    expect(mocks.insertText).not.toHaveBeenCalled();
    expect(mocks.recordDictation).not.toHaveBeenCalled();
    expect(mocks.discardRecording).toHaveBeenCalledWith("/tmp/dictation.wav");
  });

  it("leaves the text on the clipboard only after its insertion settles, failed ones included", async () => {
    let fail!: () => void;
    mocks.insertText.mockReturnValueOnce(
      new Promise((resolve) => {
        fail = () => resolve({ status: "error", error: "Focus changed" });
      }),
    );
    render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    await shortcut("released");
    await waitFor(() => expect(mocks.insertText).toHaveBeenCalled());
    expect(mocks.keepOnClipboard).not.toHaveBeenCalled();
    await act(async () => fail());
    await waitFor(() =>
      expect(mocks.keepOnClipboard).toHaveBeenCalledWith("Hello"),
    );
  });

  it("quiets other audio after the start cue and plays the stop cue once it is back, but never while a meeting records", async () => {
    mocks.settings.dictation_audio_behavior = "mute";
    const view = render(<DictationLifecycle />);
    await waitFor(() => expect(useDictationStatus.getState().ready).toBe(true));
    await shortcut("pressed");
    const environment = mocks.startSystemRecording.mock.lastCall?.[4];
    expect(environment.config).toEqual({
      behavior: { type: "mute" },
      preventSleep: true,
    });
    expect(environment.delayMs).toBeGreaterThan(0);
    mocks.stopRecording.mockResolvedValueOnce({
      status: "ok",
      data: { filePath: "/tmp/dictation.wav", durationMs: 1_000 },
    });
    await shortcut("released");
    await waitFor(() => expect(mocks.insertText).toHaveBeenCalled());
    expect(mocks.play.mock.calls).toEqual([["StartRecording"]]);
    await act(async () => endCapture());
    await waitFor(() =>
      expect(mocks.play.mock.calls).toEqual([
        ["StartRecording"],
        ["StopRecording"],
      ]),
    );

    mocks.meeting = { status: "active", loading: false };
    view.rerender(<DictationLifecycle />);
    await shortcut("pressed");
    expect(mocks.startSystemRecording.mock.lastCall?.[4]).toBeNull();
    await shortcut("released");
  });
});
