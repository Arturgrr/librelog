import { expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ platform: "macos", appStore: false }));

vi.mock("@tauri-apps/plugin-os", () => ({ platform: () => mocks.platform }));
vi.mock("~/shared/app-store", () => ({
  isAppStoreBuild: () => mocks.appStore,
}));
vi.mock("~/shared/config", () => ({}));
vi.mock("@anlg/plugin-annotate", () => ({ commands: {} }));

import { annotateAvailable } from "./system";

it.each([
  ["macos", false, true],
  ["macos", true, false],
  ["linux", false, true],
  ["windows", false, false],
])(
  "offers Annotate on macOS and Linux outside the App Store build (%s, App Store %s)",
  (platform, appStore, available) => {
    mocks.platform = platform;
    mocks.appStore = appStore;
    expect(annotateAvailable()).toBe(available);
  },
);
