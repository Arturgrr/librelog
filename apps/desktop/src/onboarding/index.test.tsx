import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  config: {} as Record<string, unknown>,
}));

vi.mock("@tauri-apps/plugin-os", () => ({ platform: () => "windows" }));
vi.mock("@anlg/plugin-sfx", () => ({
  commands: {
    play: () => Promise.resolve(),
    stop: () => Promise.resolve(),
    setVolume: () => Promise.resolve(),
  },
}));
vi.mock("~/capture/system", () => ({
  captureAvailable: () => false,
  captureOnLinux: () => false,
}));
vi.mock("./capture", () => ({
  CaptureStep: () => null,
  useSkipCapture: (onNext: () => void) => onNext,
}));
vi.mock("./final", () => ({ FinalStep: () => <p>Final step</p> }));
vi.mock("./language-picker", () => ({ WelcomeLanguagePicker: () => null }));
vi.mock("./permissions", () => ({
  PermissionsSection: () => null,
  DictationPermissions: () => null,
}));
vi.mock("~/dictation/lifecycle", () => ({
  DictationLifecycle: () => null,
  waitForDictationCleanup: () => Promise.resolve(),
}));
vi.mock("~/settings/desktop-shortcuts", () => ({
  DesktopShortcutsNote: () => null,
}));
vi.mock("~/shared/window-shell", () => ({
  StandaloneWindowShell: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("~/contexts/notifications", () => ({
  NotificationProvider: ({ children }: { children: ReactNode }) => children,
  useNotifications: () => ({ activeDownloads: [] }),
}));
vi.mock("~/settings/queries", () => ({
  useSettingsReady: () => true,
  setSettingValue: vi.fn(),
  setSettingValues: vi.fn(),
}));
vi.mock("~/shared/config", () => ({
  useConfigValue: (key: string) => mocks.config[key],
  useConfigValues: (keys: readonly string[]) =>
    Object.fromEntries(keys.map((key) => [key, mocks.config[key]])),
}));
vi.mock("~/settings/ai/stt", () => ({ SttModelPicker: () => null }));
vi.mock("~/settings/ai/llm", () => ({ LlmModelPicker: () => null }));
vi.mock("~/settings/hydration-boundary", () => ({
  SettingsHydrationBoundary: ({ children }: { children: ReactNode }) =>
    children,
}));
vi.mock("~/settings/providers", () => ({
  useAiProviders: (type: string) =>
    type === "llm"
      ? {
          "llm:openai": {
            type: "llm",
            base_url: "https://api.openai.com/v1",
            api_key: "sk-key",
          },
        }
      : {},
}));
vi.mock("~/settings/ai/shared", async (importOriginal) => ({
  ...(await importOriginal<typeof import("~/settings/ai/shared")>()),
  useProviderAvailability: (type: string) =>
    type === "llm" ? { openai: true } : {},
}));
vi.mock("~/settings/ai/stt/health", () => ({
  useConnectionHealth: () => ({ status: null }),
}));
vi.mock("~/settings/ai/llm/health", () => ({
  useConnectionHealth: () => ({ status: "success" }),
}));

import { StandaloneOnboardingScreen } from "./index";

beforeEach(() => {
  localStorage.clear();
  Element.prototype.scrollIntoView = vi.fn();
  mocks.config = {
    current_llm_provider: "openai",
    current_llm_model: "gpt-5.5",
    dictation_enabled: false,
    dictation_shortcuts: ["Control+Alt+Space"],
    dictation_hands_free: true,
  };
});

afterEach(cleanup);

function renderOnboarding() {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <StandaloneOnboardingScreen onFinish={vi.fn()} />
    </QueryClientProvider>,
  );
}

test("going on without a transcription model and skipping dictation reaches the last step, and a restart stays there", async () => {
  renderOnboarding();

  fireEvent.click(
    screen.getByRole("button", { name: "Continue without a model" }),
  );
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "Continue to Intelligence" }),
    ).toBeNull(),
  );
  const toDictation = screen.getByRole("button", { name: "Continue" });
  expect((toDictation as HTMLButtonElement).disabled).toBe(false);
  fireEvent.click(toDictation);

  expect(await screen.findByText("Try dictation")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Skip" }));
  expect(await screen.findByText("Final step")).toBeTruthy();

  cleanup();
  renderOnboarding();
  expect(screen.getByText("Final step")).toBeTruthy();
});
