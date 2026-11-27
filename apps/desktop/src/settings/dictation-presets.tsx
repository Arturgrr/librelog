import { Trans, useLingui } from "@lingui/react/macro";
import { useForm } from "@tanstack/react-form";
import { useMutation } from "@tanstack/react-query";
import { useId } from "react";

import { commands as shortcutCommands } from "@anlg/plugin-shortcut";
import { Plus, X } from "@anlg/ui/components/icons";
import { Button } from "@anlg/ui/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@anlg/ui/components/ui/select";

import { DesktopShortcutsNote } from "./desktop-shortcuts";
import {
  ShortcutCapture,
  ShortcutCaptureHint,
  ShortcutKeys,
  shortcutKeys,
  useKeyLabel,
  useShortcutRecorder,
} from "./dictation-shortcut";

import { useAnnotateShortcuts } from "~/annotate/system";
import { useCaptureShortcuts } from "~/capture/system";
import { useDictationPresetStatus } from "~/dictation/preset-switch";
import {
  type DictationPreset,
  parseDictationPresets,
} from "~/dictation/presets";
import { localizeExternalMessage } from "~/i18n/external-messages";
import { providerDisplayName } from "~/settings/ai/shared/provider-labels";
import { useSttCatalog } from "~/settings/ai/stt/catalog";
import { displayModelLabel, PROVIDERS } from "~/settings/ai/stt/shared";
import {
  CORE_TRANSCRIPTION_LANGUAGE_CODES,
  getBaseLanguageCode,
  getBaseLanguageDisplayName,
} from "~/settings/general/language";
import { setSettingValue } from "~/settings/queries";
import { SettingRow } from "~/settings/setting-row";
import { useConfigValue } from "~/shared/config";

const MAX_PRESETS = 6;

type ModelOption = {
  value: string;
  provider: string;
  model: string;
  label: string;
  modelLabel: string;
};

function optionValue(provider: string, model: string) {
  return `${provider}|${model}`;
}

/** Every model dictation can use now: set-up providers, downloaded local ones. */
function useModelOptions(): ModelOption[] {
  const { i18n } = useLingui();
  const { providers } = useSttCatalog();
  return PROVIDERS.flatMap((provider) => {
    const status = providers[provider.id];
    if (!status?.configured || status.unsupportedPlatform) return [];
    const name = providerDisplayName(provider, i18n);
    return status.models
      .filter((model) => model.isDownloaded && !model.isDeprecated)
      .map((model) => ({
        value: optionValue(provider.id, model.id),
        provider: provider.id,
        model: model.id,
        label: `${displayModelLabel(model.id, model.displayName)} (${name})`,
        modelLabel: displayModelLabel(model.id, model.displayName),
      }));
  });
}

export function DictationPresetSettings() {
  const presets = parseDictationPresets(useConfigValue("dictation_presets"));
  return (
    <div className="flex flex-col gap-3">
      <div>
        <h3 className="mb-1 text-sm font-medium">
          <Trans>Dictation presets</Trans>
        </h3>
        <p className="text-muted-foreground text-xs">
          <Trans>
            Switch the dictation language and speech model with one shortcut, or
            from the tray menu, without opening Settings. A preset's speech
            model is also the one meetings use.
          </Trans>
        </p>
      </div>
      <PresetList presets={presets} />
      {presets.length >= 2 && <PresetShortcutSetting />}
    </div>
  );
}

function PresetList({ presets }: { presets: DictationPreset[] }) {
  const { t, i18n } = useLingui();
  const options = useModelOptions();
  const mainLanguage = useConfigValue("ai_language");
  const provider = useConfigValue("current_stt_provider");
  const model = useConfigValue("current_stt_model");
  const form = useForm({
    defaultValues: { presets },
    listeners: {
      onChange: ({ formApi }) => void formApi.handleSubmit(),
    },
    onSubmit: ({ value }) =>
      setSettingValue("dictation_presets", JSON.stringify(value.presets)),
  });
  const languages = CORE_TRANSCRIPTION_LANGUAGE_CODES.map((code) => ({
    code,
    name: getBaseLanguageDisplayName(code, i18n.locale),
  })).sort((a, b) => a.name.localeCompare(b.name, i18n.locale));

  return (
    <form.Field name="presets">
      {(field) => {
        const list = field.state.value;
        const update = (index: number, patch: Partial<DictationPreset>) =>
          field.handleChange(
            list.map((preset, position) =>
              position === index ? { ...preset, ...patch } : preset,
            ),
          );
        const add = () => {
          const current = options.find(
            (option) => option.provider === provider && option.model === model,
          );
          field.handleChange([
            ...list,
            {
              id: crypto.randomUUID(),
              language: getBaseLanguageCode(mainLanguage) || "en",
              provider: current?.provider ?? provider ?? "",
              model: current?.model ?? model ?? "",
              modelLabel: current?.modelLabel ?? "",
            },
          ]);
        };
        return (
          <div className="flex flex-col gap-2">
            {list.map((preset, index) => {
              const known = options.some(
                (option) =>
                  option.value === optionValue(preset.provider, preset.model),
              );
              const shown = known
                ? options
                : [
                    {
                      value: optionValue(preset.provider, preset.model),
                      provider: preset.provider,
                      model: preset.model,
                      label: preset.modelLabel || preset.model,
                      modelLabel: preset.modelLabel,
                    },
                    ...options,
                  ];
              const number = index + 1;
              return (
                <div key={preset.id} className="flex items-center gap-2">
                  <Select
                    value={getBaseLanguageCode(preset.language)}
                    onValueChange={(language) => update(index, { language })}
                  >
                    <SelectTrigger
                      className="h-8 w-40"
                      aria-label={t`Preset ${number} language`}
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {languages.map(({ code, name }) => (
                        <SelectItem key={code} value={code}>
                          {name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <Select
                    value={optionValue(preset.provider, preset.model)}
                    onValueChange={(value) => {
                      const option = shown.find(
                        (entry) => entry.value === value,
                      );
                      if (option)
                        update(index, {
                          provider: option.provider,
                          model: option.model,
                          modelLabel: option.modelLabel,
                        });
                    }}
                  >
                    <SelectTrigger
                      className="h-8 min-w-0 flex-1"
                      aria-label={t`Preset ${number} speech model`}
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {shown.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                          {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="text-muted-foreground hover:text-foreground"
                    aria-label={t`Remove preset ${number}`}
                    onClick={() =>
                      field.handleChange(
                        list.filter((_, position) => position !== index),
                      )
                    }
                  >
                    <X className="size-4" />
                  </Button>
                </div>
              );
            })}
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="self-start"
              disabled={list.length >= MAX_PRESETS || !provider || !model}
              onClick={add}
            >
              <Plus className="size-3.5" />
              <Trans>Add preset</Trans>
            </Button>
            {list.length === 1 && (
              <p className="text-muted-foreground text-xs">
                <Trans>Add another preset to switch between them.</Trans>
              </p>
            )}
          </div>
        );
      }}
    </form.Field>
  );
}

function PresetShortcutSetting() {
  const { t } = useLingui();
  const label = useKeyLabel();
  const hintId = useId();
  const shortcut = useConfigValue("dictation_preset_shortcut");
  const dictation = useConfigValue("dictation_shortcuts");
  const pasteLastEnabled = useConfigValue("dictation_paste_last_enabled");
  const pasteLast = useConfigValue("dictation_paste_last_shortcut");
  const annotate = useAnnotateShortcuts();
  const capture = useCaptureShortcuts();
  const registrationError = useDictationPresetStatus((state) => state.error);
  const recorder = useShortcutRecorder<"preset">({
    validate: async (value) => {
      const taken = [
        ...dictation,
        ...(pasteLastEnabled ? [pasteLast] : []),
        ...annotate,
        ...capture,
      ];
      const result = await shortcutCommands.validateDictationPreset(
        value,
        taken,
      );
      return result.status === "error"
        ? localizeExternalMessage(result.error)
        : null;
    },
    persist: (value) => setSettingValue("dictation_preset_shortcut", value),
  });
  const clear = useMutation({
    mutationFn: () => setSettingValue("dictation_preset_shortcut", ""),
  });
  const parts = shortcut ? shortcutKeys(shortcut) : [];
  const keys = parts.map(label).join(" ");
  const busy = recorder.saving || clear.isPending;
  const error =
    recorder.error?.message ?? clear.error?.message ?? registrationError;

  return (
    <div className="flex flex-col gap-3">
      <SettingRow
        title={<Trans>Switch preset shortcut</Trans>}
        description={
          <Trans>
            Switches dictation to the next preset. During a dictation, the
            switch waits until it finishes.
          </Trans>
        }
        controlWidth="content"
      >
        {() =>
          recorder.editing !== null ? (
            <ShortcutCapture
              label={t`New switch preset shortcut`}
              doubleTap={false}
              describedBy={hintId}
              busy={recorder.saving}
              onInput={recorder.input}
              onConfirm={(value) => recorder.save(value, "preset")}
              onCancel={recorder.finish}
            />
          ) : (
            <div className="flex items-center gap-1">
              <Button
                type="button"
                variant="outline"
                aria-label={
                  shortcut ? t`Change shortcut ${keys}` : t`Set a shortcut`
                }
                disabled={busy}
                onClick={() => {
                  clear.reset();
                  recorder.start("preset");
                }}
              >
                {shortcut ? (
                  <ShortcutKeys keys={parts} />
                ) : (
                  <span className="text-muted-foreground text-xs">
                    <Trans>Not set</Trans>
                  </span>
                )}
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="text-muted-foreground hover:text-foreground"
                aria-label={t`Clear shortcut`}
                disabled={busy || !shortcut}
                onClick={() => clear.mutate()}
              >
                <X className="size-4" />
              </Button>
            </div>
          )
        }
      </SettingRow>
      {shortcut && <DesktopShortcutsNote shortcuts={[shortcut]} />}
      {recorder.editing !== null && (
        <ShortcutCaptureHint id={hintId} doubleTap={false} />
      )}
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}
    </div>
  );
}
