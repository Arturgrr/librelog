import { t } from "@lingui/core/macro";
import type { LanguageModel } from "ai";

import type { DictationField, Replacement } from "@anlg/plugin-dictation";

import { refineDictation } from "~/ai/dictation-refine";

/** The selected text could not be edited by voice; nothing was typed. */
export class VoiceEditFailed extends Error {}

/** The person's dictation choices, read when a dictation starts. */
export type RefineOptions = {
  cleanup: boolean;
  voiceEdit: boolean;
  context: boolean;
  replacements: Replacement[];
};

/**
 * What to type for a transcript: the words with the person's replacements,
 * cleaned up by their selected model when they asked for it, or a voice edit
 * of the selected text. Throws when nothing should be typed.
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
