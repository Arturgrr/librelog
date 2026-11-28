import { Trans, useLingui } from "@lingui/react/macro";
import { useForm } from "@tanstack/react-form";
import { useMutation } from "@tanstack/react-query";

import type { RefineOutcome } from "@anlg/plugin-dictation";
import { Button } from "@anlg/ui/components/ui/button";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@anlg/ui/components/ui/select";
import { Textarea } from "@anlg/ui/components/ui/textarea";

import { useDictationCleanupModel } from "~/ai/hooks/useLLMConnection";
import { rankStoredDictationKeywords } from "~/dictation/learning";
import {
  CLEANUP_DEADLINES,
  cleanupDeadlineSeconds,
  MAX_CLEANUP_INSTRUCTIONS,
  refineOutcome,
  useCleanupPreferences,
  useSpokenRules,
} from "~/dictation/refine";
import { useLlmModelLists } from "~/settings/ai/llm/model-lists";
import { PROVIDERS } from "~/settings/ai/llm/shared";
import { useLlmProviderStatuses } from "~/settings/ai/llm/status";
import { useBuiltinProviderListed } from "~/settings/ai/shared/list-builtin";
import { displayLlmModelId } from "~/settings/ai/shared/model-display";
import { providerDisplayName } from "~/settings/ai/shared/provider-labels";
import { useSetSettingValue, useSetSettingValues } from "~/settings/queries";
import { SETTING_CONTROL_CLASS, SettingRow } from "~/settings/setting-row";
import { useConfigValue } from "~/shared/config";

// Radix Select can't hold an empty value, so "the selected AI model" has its own.
const SAME_AS_SELECTED = "selected";

/** What "Clean up with AI" uses, shown while it is on. */
export function DictationCleanupSettings() {
  return (
    <div className="border-border flex flex-col gap-6 border-l-2 pl-4">
      <CleanupModel />
      <CleanupDeadline />
      <CleanupInstructions />
      <TryCleanup />
    </div>
  );
}

function CleanupModel() {
  const { t, i18n } = useLingui();
  const provider = useConfigValue("dictation_cleanup_provider");
  const model = useConfigValue("dictation_cleanup_model");
  const setValues = useSetSettingValues();
  const statuses = useLlmProviderStatuses();
  const lists = useLlmModelLists(statuses.providers);
  const builtinListed = useBuiltinProviderListed();
  const chosen = provider && model ? JSON.stringify([provider, model]) : null;

  const groups = PROVIDERS.filter(
    ({ id }) =>
      statuses.providers[id]?.configured && (builtinListed || id !== "builtin"),
  ).flatMap((entry) => {
    const models = lists[entry.id]?.result?.models ?? [];
    // The chosen model stays listed while its provider's list loads.
    const shown =
      entry.id === provider && model && !models.includes(model)
        ? [model, ...models]
        : models;
    return shown.length > 0
      ? [{ provider: entry, label: providerDisplayName(entry, i18n), shown }]
      : [];
  });

  return (
    <SettingRow
      title={<Trans>Cleanup model</Trans>}
      description={
        <Trans>
          A fast model just for cleaning up dictation, from the providers you
          set up in Intelligence. Edits by voice still use your selected AI
          model.
        </Trans>
      }
    >
      {(labelProps) => (
        <Select
          value={chosen ?? SAME_AS_SELECTED}
          onValueChange={(value) => {
            const [nextProvider, nextModel] =
              value === SAME_AS_SELECTED
                ? ["", ""]
                : (JSON.parse(value) as [string, string]);
            setValues({
              dictation_cleanup_provider: nextProvider,
              dictation_cleanup_model: nextModel,
            });
          }}
        >
          <SelectTrigger {...labelProps} className={SETTING_CONTROL_CLASS}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={SAME_AS_SELECTED}>
              {t`Same as your AI model`}
            </SelectItem>
            {groups.map((group) => (
              <SelectGroup key={group.provider.id}>
                <SelectLabel>{group.label}</SelectLabel>
                {group.shown.map((id) => (
                  <SelectItem
                    key={id}
                    value={JSON.stringify([group.provider.id, id])}
                  >
                    {displayLlmModelId(id)}
                  </SelectItem>
                ))}
              </SelectGroup>
            ))}
          </SelectContent>
        </Select>
      )}
    </SettingRow>
  );
}

function CleanupDeadline() {
  const { t } = useLingui();
  const seconds = cleanupDeadlineSeconds(
    useConfigValue("dictation_cleanup_deadline_seconds"),
  );
  const setSeconds = useSetSettingValue("dictation_cleanup_deadline_seconds");

  return (
    <SettingRow
      title={<Trans>Wait for cleanup</Trans>}
      description={
        <Trans>
          How long to wait for the model before typing your words as you said
          them. Edits by voice wait at least 17 seconds.
        </Trans>
      }
    >
      {(labelProps) => (
        <Select
          value={String(seconds)}
          onValueChange={(value) => setSeconds(Number(value))}
        >
          <SelectTrigger {...labelProps} className={SETTING_CONTROL_CLASS}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {CLEANUP_DEADLINES.map((option) => (
              <SelectItem key={option} value={String(option)}>
                {t`${option} seconds`}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
    </SettingRow>
  );
}

function CleanupInstructions() {
  const { t } = useLingui();
  const stored = useConfigValue("dictation_cleanup_instructions");
  const setStored = useSetSettingValue("dictation_cleanup_instructions");
  const form = useForm({
    defaultValues: { instructions: stored },
    onSubmit: ({ value }) => {
      const next = value.instructions.trim();
      if (next !== stored) setStored(next);
    },
  });
  const save = () => void form.handleSubmit();

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(event) => {
        event.preventDefault();
        event.stopPropagation();
        save();
      }}
    >
      <div>
        <h3 className="mb-1 text-sm font-medium">
          <Trans>Your instructions</Trans>
        </h3>
        <p className="text-muted-foreground text-xs">
          <Trans>
            By default, cleanup removes filler words and stutters, fixes
            grammar, punctuation and capitalization, writes numbers, dates,
            money, emails and links the way they're typed, spells your
            Dictionary words, and applies the corrections you say. It never
            answers, adds to or translates what you said. Your instructions come
            after these rules and win where they conflict; they also apply to
            edits by voice.
          </Trans>
        </p>
      </div>
      <form.Field name="instructions">
        {(field) => (
          <div className="flex flex-col gap-2">
            <Textarea
              aria-label={t`Your instructions`}
              value={field.state.value}
              maxLength={MAX_CLEANUP_INSTRUCTIONS}
              placeholder={t`For example: Write in British English. Never use exclamation marks.`}
              onChange={(event) => field.handleChange(event.target.value)}
              onBlur={() => {
                field.handleBlur();
                save();
              }}
            />
            <div className="flex items-center justify-between gap-2">
              <span className="text-muted-foreground text-xs">
                {field.state.value.length} / {MAX_CLEANUP_INSTRUCTIONS}
              </span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={!field.state.value && !stored}
                onClick={() => {
                  field.handleChange("");
                  save();
                }}
              >
                <Trans>Reset to default</Trans>
              </Button>
            </div>
          </div>
        )}
      </form.Field>
    </form>
  );
}

function TryCleanup() {
  const { t } = useLingui();
  const model = useDictationCleanupModel();
  const spoken = useSpokenRules();
  const preferences = useCleanupPreferences();
  const replacements = useConfigValue(
    "personalization_dictionary_replacements",
  );
  const attempt = useMutation({
    mutationFn: async (text: string) => {
      let modelError: unknown = null;
      const outcome = await refineOutcome({
        operationId: crypto.randomUUID(),
        text,
        terms: await rankStoredDictationKeywords(),
        options: {
          cleanup: true,
          voiceEdit: false,
          context: false,
          replacements: replacements || [],
          spoken,
          ...preferences,
        },
        field: null,
        models: { cleanup: model, edit: null },
        signal: new AbortController().signal,
        onModelError: (error) => {
          modelError = error;
        },
      });
      return { outcome, modelError, deadlineMs: preferences.deadlineMs };
    },
  });
  const form = useForm({
    defaultValues: { text: "" },
    onSubmit: ({ value }) => attempt.mutate(value.text),
  });

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(event) => {
        event.preventDefault();
        event.stopPropagation();
        void form.handleSubmit();
      }}
    >
      <div>
        <h3 className="mb-1 text-sm font-medium">
          <Trans>Try it</Trans>
        </h3>
        <p className="text-muted-foreground text-xs">
          <Trans>
            Type or paste what you might say to see what dictation would type,
            with your spoken commands, replacements, model and instructions.
          </Trans>
        </p>
      </div>
      <form.Field name="text">
        {(field) => (
          <Textarea
            aria-label={t`Text to clean up`}
            value={field.state.value}
            placeholder={t`um so let's meet on Tuesday, no, actually Wednesday at three thirty`}
            onChange={(event) => field.handleChange(event.target.value)}
          />
        )}
      </form.Field>
      <form.Subscribe selector={(state) => state.values.text.trim() === ""}>
        {(empty) => (
          <Button
            type="submit"
            variant="outline"
            size="sm"
            className="self-start"
            disabled={empty || attempt.isPending}
          >
            {attempt.isPending ? (
              <Trans>Cleaning up…</Trans>
            ) : (
              <Trans>Clean up</Trans>
            )}
          </Button>
        )}
      </form.Subscribe>
      {attempt.isError && (
        <p role="alert" className="text-destructive text-xs">
          {attempt.error.message}
        </p>
      )}
      {attempt.isSuccess && <TryResult {...attempt.data} />}
    </form>
  );
}

function TryResult({
  outcome,
  modelError,
  deadlineMs,
}: {
  outcome: RefineOutcome;
  modelError: unknown;
  deadlineMs: number;
}) {
  const { t } = useLingui();
  const seconds = deadlineMs / 1_000;
  if (outcome.type === "empty") {
    return (
      <p role="status" className="text-muted-foreground text-xs">
        <Trans>There's nothing to clean up.</Trans>
      </p>
    );
  }
  if (outcome.type !== "cleaned" && outcome.type !== "raw") return null;
  const error = modelError instanceof Error ? modelError.message : "";
  const note =
    outcome.type === "cleaned"
      ? null
      : outcome.reason === "timedOut"
        ? t`Your model didn't answer within ${seconds} seconds, so dictation would type your words as you said them.`
        : outcome.reason === "noModel"
          ? t`No AI model is available, so dictation would type your words as you said them.`
          : outcome.reason === "failed"
            ? error
              ? t`Your model failed (${error}), so dictation would type your words as you said them.`
              : t`Your model failed, so dictation would type your words as you said them.`
            : t`The model's reply changed what you said, so dictation would type your words as you said them.`;

  return (
    <div className="flex flex-col gap-2">
      {note && (
        <p role="alert" className="text-destructive text-xs">
          {note}
        </p>
      )}
      <output className="bg-muted rounded-md px-3 py-2 text-sm whitespace-pre-wrap">
        {outcome.text}
      </output>
    </div>
  );
}
