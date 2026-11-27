import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  settings: {} as Record<string, unknown>,
  configureDictationPreset: vi.fn(),
  setTrayDictationPresets: vi.fn(),
  holdLocalModel: vi.fn(),
  toast: Object.assign(vi.fn(), { error: vi.fn() }),
  listener: null as
    | ((event: { payload: { preset: number | null } }) => void)
    | null,
}));
vi.mock("~/settings/queries", () => ({
  useSettingsReady: () => true,
  getStoredSettingValues: async () => ({
    values: { ...mocks.settings },
    hasValues: new Set(Object.keys(mocks.settings)),
  }),
  setSettingValues: async (values: Record<string, unknown>) => {
    Object.assign(mocks.settings, values);
  },
}));
vi.mock("~/shared/config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("~/shared/config")>()),
  useConfigValue: (key: string) => mocks.settings[key],
}));
vi.mock("~/stt/local-model-hold", () => ({
  holdLocalModel: mocks.holdLocalModel,
}));
vi.mock("./lifecycle", () => ({
  waitForDictationCleanup: () => Promise.resolve(),
}));
vi.mock("./paste-last", () => ({
  waitForPasteLastCleanup: () => Promise.resolve(),
}));
vi.mock("@anlg/plugin-shortcut", () => ({
  commands: { configureDictationPreset: mocks.configureDictationPreset },
  events: {
    dictationPresetEvent: {
      listen: async (
        listener: (event: { payload: { preset: number | null } }) => void,
      ) => {
        mocks.listener = listener;
        return () => {
          if (mocks.listener === listener) mocks.listener = null;
        };
      },
    },
  },
}));
vi.mock("@anlg/plugin-tray", () => ({
  commands: { setTrayDictationPresets: mocks.setTrayDictationPresets },
}));
vi.mock("@anlg/ui/components/ui/toast", () => ({ toast: mocks.toast }));

import {
  DictationPresetLifecycle,
  waitForDictationPresetCleanup,
} from "./preset-switch";
import { useDictationStatus } from "./state";

import { listenerStore } from "~/store/zustand/listener/instance";

const PRESETS = [
  {
    id: "pt",
    language: "pt",
    provider: "gguf",
    model: "whisper-large-v3-turbo",
    modelLabel: "Whisper Turbo",
  },
  {
    id: "en",
    language: "en",
    provider: "soniqo",
    model: "soniqo-parakeet-batch",
    modelLabel: "Parakeet",
  },
];

const current = () => ({
  provider: mocks.settings.current_stt_provider,
  model: mocks.settings.current_stt_model,
  active: mocks.settings.dictation_preset_active,
});

async function press() {
  await waitFor(() => expect(mocks.listener).not.toBeNull());
  const answered = mocks.toast.mock.calls.length;
  await act(async () => {
    mocks.listener?.({ payload: { preset: null } });
  });
  await waitFor(() =>
    expect(mocks.toast.mock.calls.length).toBeGreaterThan(answered),
  );
}

describe("dictation presets", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.settings = {
      dictation_enabled: true,
      dictation_presets: JSON.stringify(PRESETS),
      dictation_preset_shortcut: "Alt+Shift+KeyL",
      dictation_preset_active: "",
      current_stt_provider: "deepgram",
      current_stt_model: "nova-3",
    };
    mocks.configureDictationPreset.mockResolvedValue({
      status: "ok",
      data: null,
    });
    mocks.setTrayDictationPresets.mockResolvedValue({
      status: "ok",
      data: null,
    });
    mocks.holdLocalModel.mockReturnValue(() => {});
    useDictationStatus.setState({
      capturingShortcut: false,
      capturing: false,
      pending: 0,
    });
  });
  afterEach(async () => {
    cleanup();
    listenerStore.setState(listenerStore.getInitialState(), true);
    await waitForDictationPresetCleanup();
  });

  it("cycles through the presets, applying each one's model and starting to load it", async () => {
    render(<DictationPresetLifecycle />);
    await waitFor(() =>
      expect(mocks.configureDictationPreset).toHaveBeenCalledWith(
        "Alt+Shift+KeyL",
      ),
    );

    await press();
    expect(current()).toEqual({
      provider: "gguf",
      model: "whisper-large-v3-turbo",
      active: "pt",
    });
    expect(mocks.holdLocalModel).toHaveBeenLastCalledWith(
      PRESETS[0],
      expect.any(Number),
    );
    expect(mocks.toast).toHaveBeenLastCalledWith("Português · Whisper Turbo");

    await press();
    expect(current()).toEqual({
      provider: "soniqo",
      model: "soniqo-parakeet-batch",
      active: "en",
    });
    await press();
    expect(current().active).toBe("pt");
  });

  it("waits for the dictation in progress and those still transcribing before switching", async () => {
    render(<DictationPresetLifecycle />);
    useDictationStatus.setState({ capturing: true });
    await press();
    expect(current().active).toBe("");

    act(() => useDictationStatus.setState({ capturing: false, pending: 1 }));
    expect(current().active).toBe("");

    act(() => useDictationStatus.setState({ pending: 0 }));
    await waitFor(() => expect(current().active).toBe("pt"));
  });

  it("keeps a live meeting's model until the meeting ends", async () => {
    const live = (status: "active" | "finalizing" | "inactive") =>
      listenerStore.setState((state) => ({
        live: { ...state.live, status },
      }));
    render(<DictationPresetLifecycle />);
    act(() => live("active"));
    await press();
    expect(current()).toEqual({
      provider: "deepgram",
      model: "nova-3",
      active: "",
    });
    expect(mocks.toast).toHaveBeenLastCalledWith(
      "Switching to Português · Whisper Turbo after this meeting.",
    );

    act(() => live("finalizing"));
    expect(current().active).toBe("");

    act(() => live("inactive"));
    await waitFor(() => expect(current().active).toBe("pt"));
  });
});
