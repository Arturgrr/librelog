import { Trans, useLingui } from "@lingui/react/macro";
import { useForm } from "@tanstack/react-form";
import { platform } from "@tauri-apps/plugin-os";

import { Button } from "@anlg/ui/components/ui/button";
import { Textarea } from "@anlg/ui/components/ui/textarea";

import { useLanguageModel } from "~/ai/hooks/useLLMConnection";
import { useSetSettingValue } from "~/settings/queries";
import { SettingSwitchRow } from "~/settings/setting-row";
import { useConfigValue } from "~/shared/config";
import { useTabs } from "~/store/zustand/tabs";

const MAX_EXCLUSIONS = 200;

/** Dictation features that use the person's selected AI model, each off until turned on. */
export function DictationAiSettings() {
  const cleanup = useConfigValue("dictation_cleanup");
  const setCleanup = useSetSettingValue("dictation_cleanup");
  const voiceEdit = useConfigValue("dictation_voice_edit");
  const setVoiceEdit = useSetSettingValue("dictation_voice_edit");
  const context = useConfigValue("dictation_context");
  const setContext = useSetSettingValue("dictation_context");
  const model = useLanguageModel();
  const openNew = useTabs((state) => state.openNew);

  return (
    <div className="flex flex-col gap-6">
      <SettingSwitchRow
        title={<Trans>Clean up with AI</Trans>}
        description={
          <Trans>
            Removes filler words, fixes punctuation and applies the corrections
            you say ("no, I mean Friday") using your selected AI model. If the
            model is slow or fails, your words are typed as you said them.
          </Trans>
        }
        checked={cleanup}
        onChange={setCleanup}
      />
      <SettingSwitchRow
        title={<Trans>Edit selected text by voice</Trans>}
        description={
          platform() === "windows" ? (
            <Trans>
              Select text, then dictate an instruction such as "make this
              shorter" to rewrite it with your selected AI model. Not available
              on Windows yet.
            </Trans>
          ) : (
            <Trans>
              Select text, then dictate an instruction such as "make this
              shorter" to rewrite it with your selected AI model. If the edit
              fails, the selection is left unchanged.
            </Trans>
          )
        }
        checked={voiceEdit}
        onChange={setVoiceEdit}
      />
      <SettingSwitchRow
        title={<Trans>Use what's on screen</Trans>}
        description={
          <Trans>
            Reads text from the window you dictate into so AI cleanup and edits
            spell its names and terms correctly. Password fields, password
            managers, private windows and the apps and websites below are never
            read, and nothing read is kept.
          </Trans>
        }
        checked={context}
        onChange={setContext}
      />
      {context && <ContextExclusions />}
      {(cleanup || voiceEdit) && !model && (
        <div className="flex flex-col items-start gap-2" role="status">
          <p className="text-muted-foreground text-xs">
            <Trans>
              No AI model is selected, so dictation is typed as you said it.
            </Trans>
          </p>
          <Button
            variant="outline"
            size="sm"
            onClick={() =>
              openNew({ type: "settings", state: { tab: "intelligence" } })
            }
          >
            <Trans>Choose an AI model</Trans>
          </Button>
        </div>
      )}
    </div>
  );
}

function ContextExclusions() {
  const { t } = useLingui();
  const stored = useConfigValue("dictation_context_exclusions");
  const setStored = useSetSettingValue("dictation_context_exclusions");
  const form = useForm({
    defaultValues: {
      apps: (stored?.apps ?? []).join("\n"),
      websites: (stored?.websites ?? []).join("\n"),
    },
    onSubmit: ({ value }) => {
      const next = JSON.stringify({
        apps: lines(value.apps),
        websites: lines(value.websites),
      });
      setStored(next);
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
      {stored === null && (
        <p role="alert" className="text-destructive text-xs">
          <Trans>
            Your exclusions could not be read, so no window is read until you
            save them again.
          </Trans>
        </p>
      )}
      <form.Field name="apps">
        {(field) => (
          <label className="flex flex-col gap-2 text-sm">
            <span>
              <Trans>Never read these apps</Trans>
            </span>
            <Textarea
              value={field.state.value}
              placeholder={t`One app name or bundle ID per line`}
              onChange={(event) => field.handleChange(event.target.value)}
              onBlur={() => {
                field.handleBlur();
                save();
              }}
            />
          </label>
        )}
      </form.Field>
      <form.Field name="websites">
        {(field) => (
          <label className="flex flex-col gap-2 text-sm">
            <span>
              <Trans>Never read these websites</Trans>
            </span>
            <Textarea
              value={field.state.value}
              placeholder={t`One domain per line, like bank.example`}
              onChange={(event) => field.handleChange(event.target.value)}
              onBlur={() => {
                field.handleBlur();
                save();
              }}
            />
          </label>
        )}
      </form.Field>
    </form>
  );
}

function lines(value: string) {
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .slice(0, MAX_EXCLUSIONS);
}
