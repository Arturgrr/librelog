import { t } from "@lingui/core/macro";
import type { LanguageModel } from "ai";

import type {
  DictationField,
  Replacement,
  SpokenRules,
} from "@anlg/plugin-dictation";

import { refineDictation } from "~/ai/dictation-refine";
import { useConfigValue } from "~/shared/config";

/** The selected text could not be edited by voice; nothing was typed. */
export class VoiceEditFailed extends Error {}

/**
 * The spoken commands the person turned on, with the languages they dictate
 * in, which choose the words: only English and Portuguese have them.
 */
export function useSpokenRules(): SpokenRules {
  const main = useConfigValue("ai_language");
  const spoken = useConfigValue("spoken_languages");
  return {
    languages: [main, ...(spoken || [])].filter(Boolean),
    fillers: useConfigValue("dictation_remove_fillers"),
    formatting: useConfigValue("dictation_spoken_formatting"),
    symbols: useConfigValue("dictation_spoken_symbols"),
  };
}

/** The person's dictation choices, read when a dictation starts. */
export type RefineOptions = {
  cleanup: boolean;
  voiceEdit: boolean;
  context: boolean;
  replacements: Replacement[];
  spoken: SpokenRules;
};

/**
 * What to type for a transcript: the words with the spoken commands the person
 * turned on and their replacements, cleaned up by their selected model when
 * they asked for it, or a voice edit of the selected text. Throws when nothing
 * should be typed.
 */
export async function refineTranscript({
  operationId,
  text,
  terms,
  options,
  field,
  model,
  signal,
}: {
  operationId: string;
  text: string;
  terms: string[];
  options: RefineOptions;
  field: DictationField | null;
  model: LanguageModel | null;
  signal: AbortSignal;
}): Promise<{ text: string; edit: boolean }> {
  const outcome = await refineDictation({
    model,
    signal,
    request: {
      operationId,
      nowMs: Date.now(),
      text,
      // Only a selection whose field can be checked again before the edit is
      // written, and only with a model to edit it: without one, the words are
      // typed over the selection as said.
      selection:
        options.voiceEdit && model && field?.fingerprint
          ? field.selection
          : null,
      policy: { cleanup: options.cleanup, voiceEdit: options.voiceEdit },
      modelReady: model !== null,
      terms,
      replacements: options.replacements,
      instructions: null,
      before: null,
      context: options.context ? (field?.context ?? null) : null,
      spoken: options.spoken,
    },
  });
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
      signal.throwIfAborted();
      throw new Error(t`Dictation cancelled.`);
  }
}
