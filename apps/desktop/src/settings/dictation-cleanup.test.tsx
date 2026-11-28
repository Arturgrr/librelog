import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MockLanguageModelV3 } from "ai/test";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  settings: {} as Record<string, unknown>,
  model: null as unknown,
  prepare: vi.fn(),
  resolve: vi.fn(),
}));
vi.mock("@anlg/plugin-dictation", () => ({
  commands: {
    prepareDictationRefinement: mocks.prepare,
    resolveDictationRefinement: mocks.resolve,
  },
}));
vi.mock("~/ai/hooks/useLLMConnection", () => ({
  useDictationCleanupModel: () => mocks.model,
}));
vi.mock("~/shared/config", () => ({
  useConfigValue: (key: string) => mocks.settings[key],
}));
vi.mock("~/settings/queries", () => ({
  useSetSettingValue: () => vi.fn(),
  useSetSettingValues: () => vi.fn(),
}));
vi.mock("~/dictation/learning", () => ({
  rankStoredDictationKeywords: async () => ["Wednesday"],
}));
vi.mock("~/settings/ai/llm/status", () => ({
  useLlmProviderStatuses: () => ({ providers: {}, isReady: true }),
}));
vi.mock("~/settings/ai/llm/model-lists", () => ({
  useLlmModelLists: () => ({}),
}));
vi.mock("~/settings/ai/shared/list-builtin", async (importOriginal) => ({
  ...(await importOriginal<
    typeof import("~/settings/ai/shared/list-builtin")
  >()),
  useBuiltinProviderListed: () => false,
}));

import { DictationCleanupSettings } from "./dictation-cleanup";

function replying(text: string | null) {
  return new MockLanguageModelV3({
    doGenerate: () =>
      text === null
        ? new Promise(() => {})
        : Promise.resolve({
            content: [{ type: "text" as const, text }],
            finishReason: { unified: "stop" as const, raw: "stop" },
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
}

async function tryIt(text: string) {
  render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { mutations: { retry: false } } })
      }
    >
      <DictationCleanupSettings />
    </QueryClientProvider>,
  );
  fireEvent.change(screen.getByLabelText("Text to clean up"), {
    target: { value: text },
  });
  fireEvent.click(screen.getByRole("button", { name: "Clean up" }));
}

beforeEach(() => {
  mocks.settings = {
    dictation_cleanup_instructions: "Write in lowercase.",
    dictation_cleanup_deadline_seconds: 3,
    dictation_cleanup_provider: "",
    dictation_cleanup_model: "",
    personalization_dictionary_replacements: [{ from: "wed", to: "Wed" }],
    ai_language: "en",
    spoken_languages: [],
    dictation_remove_fillers: true,
    dictation_spoken_formatting: false,
    dictation_spoken_symbols: false,
  };
  mocks.prepare.mockImplementation(
    async (request: { text: string; deadlineMs: number }) => ({
      type: "ask",
      call: { system: "rules", prompt: request.text, deadlineMs: 20 },
      pending: {
        kind: "cleanup",
        raw: request.text,
        selection: null,
        before: null,
        instructed: true,
        replacements: [],
      },
    }),
  );
  mocks.resolve.mockImplementation(
    async (pending: { raw: string }, reply: { type: string; text?: string }) =>
      reply.type === "text"
        ? { type: "cleaned", text: reply.text }
        : { type: "raw", text: pending.raw, reason: reply.type },
  );
});
afterEach(cleanup);

it("tries a sentence through dictation's own cleanup, with the person's commands, replacements, instructions and deadline", async () => {
  mocks.model = replying("let's meet on wednesday.");
  await tryIt("um let's meet on tuesday, no, wednesday");
  expect(await screen.findByText("let's meet on wednesday.")).toBeTruthy();
  expect(mocks.prepare).toHaveBeenCalledWith(
    expect.objectContaining({
      text: "um let's meet on tuesday, no, wednesday",
      policy: { cleanup: true, voiceEdit: false },
      modelReady: true,
      selection: null,
      terms: ["Wednesday"],
      replacements: [{ from: "wed", to: "Wed" }],
      instructions: "Write in lowercase.",
      deadlineMs: 3_000,
      spoken: {
        languages: ["en"],
        fillers: true,
        formatting: false,
        symbols: false,
      },
    }),
  );
});

it("says when the model ran out of time and shows the words as they would be typed", async () => {
  mocks.model = replying(null);
  await tryIt("um send it");
  expect(
    await screen.findByText(
      "Your model didn't answer within 3 seconds, so dictation would type your words as you said them.",
    ),
  ).toBeTruthy();
  expect(screen.getByText("um send it", { selector: "output" })).toBeTruthy();
});
