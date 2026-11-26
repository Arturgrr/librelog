import {
  commands as dictation,
  type FieldText,
  type Replacement,
} from "@anlg/plugin-dictation";

import { getStoredSettingValues, updateSettingValue } from "~/settings/queries";
import { resolveConfigValue } from "~/shared/config";

// One read of the field this long after an insert tells what the person kept of it.
export const KEPT_READ_MS = 20_000;
const MAX_SUGGESTIONS = 20;
const MAX_SETTLED = 200;

export type DictionarySuggestions = { terms: string[]; dismissed: string[] };

/** Reads the field being dictated into, around its caret; null when it can't be read. */
export type ReadField = () => Promise<FieldText | null>;

/**
 * Integration point for the native field read (`read_dictation_field`). Until
 * it exists, nothing is read and correction suggestions stay off.
 */
export const readDictationField: ReadField = async () => null;

/**
 * The terms speech-to-text should hear for one dictation, highest first:
 * the Dictionary, replacement targets and, with learning on, their weights.
 */
export async function rankDictationKeywords({
  dictionary,
  replacements,
  weights,
}: {
  dictionary: string[];
  replacements: Replacement[];
  weights: string | null;
}): Promise<string[]> {
  try {
    return await dictation.rankDictationTerms({
      nowMs: Date.now(),
      dictionary,
      replacements,
      contextTerms: [],
      weights,
      limit: null,
    });
  } catch (error) {
    console.error("[dictation] keywords were not ranked", error);
    return dictionary;
  }
}

/** {@link rankDictationKeywords} from the settings as stored now. */
export async function rankStoredDictationKeywords(): Promise<string[]> {
  const stored = await getStoredSettingValues();
  return rankDictationKeywords({
    dictionary: resolveConfigValue("personalization_dictionary_terms", stored),
    replacements: resolveConfigValue(
      "personalization_dictionary_replacements",
      stored,
    ),
    weights: resolveConfigValue("dictation_learning", stored)
      ? resolveConfigValue("dictation_term_weights", stored) || null
      : null,
  });
}

let learningWrites: Promise<void> = Promise.resolve();

// One at a time, each from the weights the last one stored, so none is lost.
function serial(task: () => Promise<void>) {
  const next = learningWrites.then(task);
  learningWrites = next.catch((error: unknown) => {
    console.error("[dictation] learning", error);
  });
  return learningWrites;
}

async function learningSettings() {
  const stored = await getStoredSettingValues();
  return {
    learning: resolveConfigValue("dictation_learning", stored),
    dictionary: resolveConfigValue("personalization_dictionary_terms", stored),
    weights: resolveConfigValue("dictation_term_weights", stored) || null,
    suggestions: parseSuggestions(
      resolveConfigValue("dictation_dictionary_suggestions", stored),
    ),
  };
}

/** Weighs the Dictionary terms a delivered dictation used; only with learning on. */
export function recordUsedTerms(text: string): Promise<void> {
  return serial(async () => {
    const { learning, dictionary, weights } = await learningSettings();
    if (!learning || !text.trim()) return;
    const next = await dictation.recordDictationTerms({
      nowMs: Date.now(),
      dictionary,
      weights,
      event: { type: "used", text },
    });
    await writeWhileLearning("dictation_term_weights", () => next);
  });
}

/**
 * Suggests Dictionary terms from how the person corrected a dictation: the
 * field read when it started and again a while after `inserted` was typed.
 */
export function learnFromCorrection({
  start,
  now,
  inserted,
}: {
  start: FieldText;
  now: FieldText;
  inserted: string;
}): Promise<void> {
  return serial(async () => {
    const { learning, dictionary, weights, suggestions } =
      await learningSettings();
    if (!learning) return;
    const kept = await dictation.keptDictationText(start, now);
    if (!kept || kept === inserted.trim()) return;
    const terms = await dictation.suggestDictationTerms({
      inserted,
      kept,
      dictionary,
      seen: [...suggestions.terms, ...suggestions.dismissed],
    });
    if (terms.length === 0) return;
    const next = await dictation.recordDictationTerms({
      nowMs: Date.now(),
      dictionary,
      weights,
      event: { type: "corrected", terms },
    });
    // Read-modify-write, so a suggestion settled meanwhile stays settled.
    await writeWhileLearning("dictation_dictionary_suggestions", (current) =>
      JSON.stringify(addSuggestions(parseSuggestions(current), terms)),
    );
    await writeWhileLearning("dictation_term_weights", () => next);
  });
}

/**
 * Waits `delayMs` after a dictation was typed, reads the field again and
 * learns from what the person kept. Does nothing once `signal` aborts.
 */
export async function watchCorrection({
  start,
  inserted,
  readField,
  signal,
  delayMs = KEPT_READ_MS,
}: {
  start: Promise<FieldText | null>;
  inserted: string;
  readField: ReadField;
  signal: AbortSignal;
  delayMs?: number;
}): Promise<void> {
  const before = await start;
  if (!before || signal.aborted) return;
  await new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, delayMs);
    signal.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );
  });
  if (signal.aborted) return;
  const now = await readField().catch(() => null);
  if (!now || signal.aborted) return;
  await learnFromCorrection({ start: before, now, inserted });
}

// The setting is read again inside the write, which waits behind other
// settings writes: learning turned off meanwhile stores nothing.
async function writeWhileLearning<
  K extends "dictation_term_weights" | "dictation_dictionary_suggestions",
>(key: K, next: (current: string | undefined) => string) {
  const { learning } = await learningSettings();
  if (!learning) return;
  await updateSettingValue(key, (current) => next(current as string));
}

export function parseSuggestions(
  value: string | undefined | null,
): DictionarySuggestions {
  if (!value) return { terms: [], dismissed: [] };
  try {
    const parsed = JSON.parse(value) as Partial<Record<string, unknown>>;
    return {
      terms: strings(parsed.terms).slice(0, MAX_SUGGESTIONS),
      dismissed: strings(parsed.dismissed).slice(-MAX_SETTLED),
    };
  } catch {
    return { terms: [], dismissed: [] };
  }
}

function addSuggestions(
  current: DictionarySuggestions,
  terms: readonly string[],
): DictionarySuggestions {
  const known = new Set(
    [...current.terms, ...current.dismissed].map((term) => term.toLowerCase()),
  );
  const added = terms.filter((term) => !known.has(term.toLowerCase()));
  return {
    ...current,
    terms: [...current.terms, ...added].slice(-MAX_SUGGESTIONS),
  };
}

/** Takes `term` off the pending suggestions, so it isn't offered again. */
export function settleSuggestion(term: string): Promise<unknown> {
  return updateSettingValue("dictation_dictionary_suggestions", (current) => {
    const suggestions = parseSuggestions(current);
    return JSON.stringify({
      terms: suggestions.terms.filter((value) => value !== term),
      dismissed: [...suggestions.dismissed, term].slice(-MAX_SETTLED),
    });
  });
}

function strings(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((entry): entry is string => typeof entry === "string")
    : [];
}
