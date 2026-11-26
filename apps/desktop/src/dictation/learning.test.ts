import { beforeEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  values: {} as Record<string, unknown>,
  suggestDictationTerms: vi.fn(),
  recordDictationTerms: vi.fn(),
}));

vi.mock("@anlg/plugin-dictation", () => ({
  commands: {
    keptDictationText: async (
      start: { before: string },
      now: { before: string },
    ) =>
      now.before.startsWith(start.before)
        ? now.before.slice(start.before.length).trim() || null
        : null,
    suggestDictationTerms: mocks.suggestDictationTerms,
    recordDictationTerms: mocks.recordDictationTerms,
  },
}));

vi.mock("~/settings/queries", () => ({
  getStoredSettingValues: async () => ({
    values: mocks.values,
    hasValues: new Set(Object.keys(mocks.values)),
  }),
  updateSettingValue: async (
    key: string,
    update: (current: unknown) => unknown,
  ) => {
    mocks.values[key] = update(mocks.values[key]);
  },
  useStoredSettingValues: vi.fn(),
}));

import { parseSuggestions, watchCorrection } from "./learning";

const field = (before: string) => ({ before, selected: "", after: "" });

beforeEach(() => {
  mocks.values = {
    dictation_learning: true,
    personalization_dictionary_terms: "[]",
    dictation_dictionary_suggestions: JSON.stringify({
      terms: [],
      dismissed: ["Charr"],
    }),
  };
  mocks.suggestDictationTerms.mockImplementation(
    async ({ kept }: { kept: string }) =>
      kept.includes("Fastrepl") ? ["Fastrepl", "Charr"] : [],
  );
  mocks.recordDictationTerms.mockResolvedValue("learned-weights");
});

test("suggests the words the person corrected after dictating, except those already dismissed", async () => {
  await watchCorrection({
    start: Promise.resolve(field("Notes: ")),
    inserted: "Ask fast ripple about it",
    readField: async () => field("Notes: Ask Fastrepl about it"),
    signal: new AbortController().signal,
    delayMs: 0,
  });

  expect(mocks.suggestDictationTerms).toHaveBeenCalledWith(
    expect.objectContaining({
      inserted: "Ask fast ripple about it",
      kept: "Ask Fastrepl about it",
    }),
  );
  expect(mocks.recordDictationTerms).toHaveBeenCalledWith(
    expect.objectContaining({
      event: { type: "corrected", terms: ["Fastrepl", "Charr"] },
    }),
  );
  expect(
    parseSuggestions(mocks.values.dictation_dictionary_suggestions as string)
      .terms,
  ).toEqual(["Fastrepl"]);
  expect(mocks.values.dictation_term_weights).toBe("learned-weights");
});
