import { beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ platform: "macos", appStore: false }));

vi.mock("@tauri-apps/plugin-os", () => ({ platform: () => mocks.platform }));
vi.mock("~/shared/app-store", () => ({
  isAppStoreBuild: () => mocks.appStore,
}));

import {
  getInitialStep,
  getOnboardingSteps,
  limitToMissingSelection,
  saveOnboardingStep,
} from "./config";

beforeEach(() => {
  localStorage.clear();
  mocks.appStore = false;
});

it.each([
  [
    "macos",
    false,
    [
      "permissions",
      "transcription",
      "intelligence",
      "dictation",
      "capture",
      "final",
    ],
  ],
  [
    "macos",
    true,
    ["permissions", "transcription", "intelligence", "dictation", "final"],
  ],
  ["windows", false, ["transcription", "intelligence", "dictation", "final"]],
  [
    "linux",
    false,
    ["transcription", "intelligence", "dictation", "capture", "final"],
  ],
])(
  "sets up transcription, intelligence and dictation practice (and offers Capture outside the App Store build) before finishing on %s, App Store %s",
  (platform, appStore, steps) => {
    mocks.platform = platform;
    mocks.appStore = appStore;
    expect(getOnboardingSteps()).toEqual(steps);
    expect(getInitialStep()).toBe(steps[0]);
  },
);

it("resumes at the saved step after a restart, or at the step whose choice is missing", () => {
  mocks.platform = "macos";
  saveOnboardingStep("transcription");
  expect(getInitialStep()).toBe("transcription");

  saveOnboardingStep("final");
  expect(getInitialStep()).toBe("final");
  expect(limitToMissingSelection("final", null)).toBe("final");
  expect(limitToMissingSelection("final", "transcription")).toBe(
    "transcription",
  );
  expect(limitToMissingSelection("permissions", "transcription")).toBe(
    "permissions",
  );

  mocks.platform = "windows";
  saveOnboardingStep("permissions");
  expect(getInitialStep()).toBe("transcription");

  saveOnboardingStep(null);
  expect(getInitialStep()).toBe("transcription");
});
