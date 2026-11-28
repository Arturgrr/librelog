import { t } from "@lingui/core/macro";
import type { LanguageModel } from "ai";

import type {
  DictationField,
  RefineOutcome,
  Replacement,
  SpokenRules,
} from "@anlg/plugin-dictation";

import { refineDictation } from "~/ai/dictation-refine";
import { useConfigValue } from "~/shared/config";

/** The selected text could not be edited by voice; nothing was typed. */
export class VoiceEditFailed extends Error {}

/** Within what dictation-core keeps of the person's instructions. */
export const MAX_CLEANUP_INSTRUCTIONS = 4_000;

/** The deadlines offered for cleanup, in seconds; 6 is the one it always had. */
export const CLEANUP_DEADLINES = [3, 6, 10, 20] as const;

/**
 * The spoken commands the person turned on, with the languages they dictate
 * in, which choose the words: only English and Portuguese have them. A
 * dictation preset's language replaces them, as it does for transcription.
 */
export function useSpokenRules(presetLanguage?: string): SpokenRules {
  const main = useConfigValue("ai_language");
  const spoken = useConfigValue("spoken_languages");
  return {
    languages: presetLanguage
      ? [presetLanguage]
      : [main, ...(spoken || [])].filter(Boolean),
    fillers: useConfigValue("dictation_remove_fillers"),
    formatting: useConfigValue("dictation_spoken_formatting"),
    symbols: useConfigValue("dictation_spoken_symbols"),
  };
}

/** The person's own instructions and how long cleanup waits for the model. */
export function useCleanupPreferences(): {
  instructions: string;
  deadlineMs: number;
} {
  const instructions = useConfigValue("dictation_cleanup_instructions");
  const seconds = useConfigValue("dictation_cleanup_deadline_seconds");
  return {
    instructions: instructions || "",
    deadlineMs: cleanupDeadlineSeconds(seconds) * 1_000,
  };
}

/** One of {@link CLEANUP_DEADLINES}: anything else stored reads as the default. */
export function cleanupDeadlineSeconds(stored: unknown) {
  return CLEANUP_DEADLINES.find((seconds) => seconds === stored) ?? 6;
}

/** The person's dictation choices, read when a dictation starts. */
export type RefineOptions = {
  cleanup: boolean;
  voiceEdit: boolean;
  context: boolean;
  replacements: Replacement[];
  spoken: SpokenRules;
  /** Empty for none: only the built-in rules apply. */
  instructions: string;
  deadlineMs: number;
};

/**
 * The models dictation uses: `cleanup` for plain dictation, which may be one
 * chosen just for it, and `edit`, the selected AI model, for voice edits.
 */
export type DictationModels = {
  cleanup: LanguageModel | null;
  edit: LanguageModel | null;
};

type RefineArgs = {
  operationId: string;
  text: string;
  terms: string[];
  options: RefineOptions;
  field: DictationField | null;
  models: DictationModels;
  signal: AbortSignal;
};

/**
 * How dictation-core and the model refine a transcript: spoken commands, the
 * person's replacements, and cleanup or a voice edit with their instructions,
 * model and deadline. Dictation and Settings' "Try it" both run this.
 */
export function refineOutcome({
  operationId,
  text,
  terms,
  options,
  field,
  models,
  signal,
  onModelError,
}: RefineArgs & {
  onModelError?: (error: unknown) => void;
}): Promise<RefineOutcome> {
  // Only a selection whose field can be checked again before the edit is
  // written, and only with a model to edit it: without one, the words are
  // typed over the selection as said.
  const selection =
    options.voiceEdit && models.edit && field?.fingerprint
      ? field.selection
      : null;
  const model = selection ? models.edit : models.cleanup;
  return refineDictation({
    model,
    signal,
    onModelError,
    request: {
      operationId,
      nowMs: Date.now(),
      text,
      selection,
      policy: { cleanup: options.cleanup, voiceEdit: options.voiceEdit },
      modelReady: model !== null,
      terms,
      replacements: options.replacements,
      instructions: options.instructions.trim() || null,
      before: null,
      context: options.context ? (field?.context ?? null) : null,
      spoken: options.spoken,
      deadlineMs: options.deadlineMs,
    },
  });
}

/**
 * What to type for a transcript: the words with the spoken commands the person
 * turned on and their replacements, cleaned up by their model when they asked
 * for it, or a voice edit of the selected text. Throws when nothing should be
 * typed.
 */
export async function refineTranscript(
  args: RefineArgs,
): Promise<{ text: string; edit: boolean }> {
  const outcome = await refineOutcome(args);
  switch (outcome.type) {
    case "cleaned":
    case "raw":
      return { text: outcome.text, edit: false };
    case "edited":
      return { text: outcome.text, edit: true };
    case "editFailed":
      throw new VoiceEditFailed(
        t`Could not edit the selected text with your AI model. It was left unchanged.`,
      );
    case "empty":
      throw new Error(t`No speech detected. Please try again.`);
    case "cancelled":
      args.signal.throwIfAborted();
      throw new Error(t`Dictation cancelled.`);
  }
}
