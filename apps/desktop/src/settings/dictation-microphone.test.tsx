import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  permission: "authorized" as string,
  startInputLevel: vi.fn(),
  stopInputLevel: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-os", () => ({ platform: () => "macos" }));
vi.mock("@tauri-apps/api/core", () => ({ Channel: class {} }));
vi.mock("@anlg/plugin-dictation", () => ({
  commands: {
    dictationMicrophone: async () => ({
      status: "ok",
      data: { device: null, passedOver: null },
    }),
    startInputLevel: mocks.startInputLevel,
    stopInputLevel: mocks.stopInputLevel,
  },
}));
vi.mock("@anlg/plugin-transcription", () => ({
  commands: { listMicrophoneDevices: async () => ({ status: "ok", data: [] }) },
}));
vi.mock("~/settings/general/audio-settings", () => ({
  AudioDeviceRow: () => null,
}));
vi.mock("~/settings/queries", () => ({ useSetSettingValue: () => vi.fn() }));
vi.mock("~/shared/config", () => ({
  useConfigValue: (key: string) =>
    ({ microphone_device: "", dictation_avoid_bluetooth_microphone: true })[
      key
    ],
}));
vi.mock("~/shared/hooks/usePermissions", () => ({
  usePermission: () => ({
    status: mocks.permission,
    isPending: false,
    request: vi.fn(),
    open: vi.fn(),
  }),
}));
vi.mock("~/stt/contexts", () => ({
  useListener: (select: (state: unknown) => unknown) =>
    select({ live: { status: "inactive", loading: false } }),
}));

import { DictationMicrophoneSettings } from "./dictation-microphone";

function setup() {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <DictationMicrophoneSettings />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.permission = "authorized";
  mocks.startInputLevel.mockResolvedValue({
    status: "ok",
    data: "microphone",
  });
  mocks.stopInputLevel.mockResolvedValue(undefined);
});
afterEach(cleanup);

describe("dictation microphone level", () => {
  it("never opens the microphone before access is allowed, so macOS never prompts from Settings", async () => {
    mocks.permission = "notDetermined";
    setup();
    await screen.findByRole("button", { name: "Allow access" });
    expect(mocks.startInputLevel).not.toHaveBeenCalled();
  });

  it("lets go of the microphone when Settings closes", async () => {
    const view = setup();
    await waitFor(() => expect(mocks.startInputLevel).toHaveBeenCalled());
    view.unmount();
    await waitFor(() => expect(mocks.stopInputLevel).toHaveBeenCalled());
  });
});
