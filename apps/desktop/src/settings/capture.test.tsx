import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  platform: "linux",
  screenRecording: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-os", () => ({ platform: () => mocks.platform }));
vi.mock("~/shared/app-store", () => ({ isAppStoreBuild: () => false }));
vi.mock("@tauri-apps/api/path", () => ({
  appDataDir: async () => "/data",
  downloadDir: async () => "/home/me/Downloads",
  homeDir: async () => "/home/me",
}));
vi.mock("@anlg/plugin-annotate", () => ({
  commands: { screenRecording: mocks.screenRecording },
}));
vi.mock("@anlg/plugin-capture", () => ({
  commands: {
    systemShortcuts: async () => ({
      status: "ok",
      data: { area: true, fullscreen: true, options: true, replaced: false },
    }),
  },
}));
vi.mock("./capture-recent", () => ({ RecentCaptures: () => null }));
vi.mock("./capture-shortcuts", async (original) => ({
  ...(await original<typeof import("./capture-shortcuts")>()),
  CaptureShortcutRows: () => null,
}));
vi.mock("~/settings/queries", () => ({
  setSettingValue: vi.fn(),
  setSettingValues: vi.fn(),
  getStoredSettingValues: vi.fn(),
  useSetSettingValue: () => vi.fn(),
}));
vi.mock("~/shared/config", () => ({
  useConfigValue: (key: string) =>
    ({
      capture_enabled: true,
      capture_save_to_folder: false,
      capture_replace_system_shortcuts: false,
      capture_image_format: "png",
      capture_record_fps: 30,
      capture_gif_fps: 15,
      capture_gif_max_width: 720,
    })[key] ?? false,
  resolveConfigValues: vi.fn(),
}));

import { SettingsCapture } from "./capture";

afterEach(cleanup);

function renderPage() {
  render(
    <QueryClientProvider client={new QueryClient()}>
      <SettingsCapture />
    </QueryClientProvider>,
  );
}

const MAC_ONLY = [
  "Replace the macOS screenshot shortcuts",
  "Screen Recording",
  "Include the pointer",
  "Countdown",
  "Record the microphone",
  "Record system audio",
];

it("hides what only macOS has on Linux and says how the desktop asks and how to stop", () => {
  mocks.platform = "linux";
  renderPage();
  for (const title of MAC_ONLY) {
    expect(screen.queryByText(title)).toBeNull();
  }
  expect(
    screen.getByText(/asks before Librelog takes a screenshot/),
  ).toBeTruthy();
  expect(screen.getByText(/To stop a recording/)).toBeTruthy();
  expect(screen.queryByText(/this Mac/)).toBeNull();
  expect(mocks.screenRecording).not.toHaveBeenCalled();
});

it("keeps them on macOS", async () => {
  mocks.platform = "macos";
  mocks.screenRecording.mockResolvedValue({ status: "ok", data: "granted" });
  renderPage();
  for (const title of MAC_ONLY) {
    expect(await screen.findByText(title)).toBeTruthy();
  }
});
