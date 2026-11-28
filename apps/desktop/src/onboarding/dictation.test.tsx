import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  config: {} as Record<string, unknown>,
}));

vi.mock("@tauri-apps/plugin-os", () => ({ platform: () => "macos" }));
vi.mock("~/shared/config", () => ({
  useConfigValue: (key: string) => mocks.config[key],
}));
vi.mock("~/settings/queries", () => ({
  setSettingValue: vi.fn(),
  setSettingValues: vi.fn(),
}));
vi.mock("~/dictation/lifecycle", () => ({
  waitForDictationCleanup: () => Promise.resolve(),
}));
vi.mock("~/settings/desktop-shortcuts", () => ({
  DesktopShortcutsNote: () => null,
}));
vi.mock("./permissions", () => ({ DictationPermissions: () => null }));
vi.mock("./model-status", () => ({
  useTranscriptionSetup: () => ({
    kind: "ready",
    model: "Parakeet Batch",
    provider: "Soniqo",
  }),
}));

import { DictationStep } from "./dictation";

import { useDictationStatus } from "~/dictation/state";

beforeEach(() => {
  mocks.config = {
    dictation_enabled: true,
    dictation_shortcuts: ["DoubleTap+AltLeft"],
    dictation_hands_free: true,
  };
  useDictationStatus.setState({
    phase: "idle",
    capturing: false,
    pending: 0,
    error: null,
    lastTranscript: "",
  });
});

afterEach(cleanup);

function renderStep() {
  return render(<DictationStep onContinue={vi.fn()} onChooseModel={vi.fn()} />);
}

test("the practice step explains the dictation shortcut the person has configured", () => {
  mocks.config.dictation_shortcuts = ["DoubleTap+AltLeft", "Fn"];
  renderStep();

  const keys = screen.getAllByText(/./, { selector: "kbd" });
  expect(keys.map((key) => key.textContent)).toEqual([
    "Left ⌥",
    "×2",
    "Fn / Globe",
  ]);
  expect(
    screen.getByText(/Press the shortcut to start, speak, then press it again/),
  ).toBeTruthy();
});

test("text a dictation types into the practice field marks the practice done, and Try again starts over", () => {
  renderStep();
  const field = screen.getByRole("textbox") as HTMLTextAreaElement;

  // Typing by hand is not a dictation.
  fireEvent.input(field, { target: { value: "typed" } });
  expect(screen.queryByText(/It works/)).toBeNull();

  act(() => {
    useDictationStatus.setState({ capturing: true, phase: "recording" });
  });
  expect(screen.getByText("Listening…")).toBeTruthy();
  act(() => {
    useDictationStatus.setState({
      capturing: false,
      phase: "transcribing",
      pending: 1,
      lastTranscript: "Hello there",
    });
  });
  fireEvent.input(field, { target: { value: "typed Hello there" } });
  expect(screen.getByText(/It works/)).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(field.value).toBe("");
  expect(screen.queryByText(/It works/)).toBeNull();
});
