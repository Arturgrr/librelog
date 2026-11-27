import { getBaseLanguageDisplayName } from "~/settings/general/language";
import { useConfigValue } from "~/shared/config";

/** A spoken language and the speech model dictation transcribes it with. */
export type DictationPreset = {
  id: string;
  language: string;
  provider: string;
  model: string;
  /** The model's name as Settings showed it when it was picked. */
  modelLabel: string;
};

export function parseDictationPresets(value: unknown): DictationPreset[] {
  let parsed: unknown;
  try {
    parsed = typeof value === "string" ? JSON.parse(value) : value;
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  return parsed.flatMap((entry) => {
    if (!entry || typeof entry !== "object") return [];
    const { id, language, provider, model, modelLabel } = entry as Record<
      string,
      unknown
    >;
    if (
      typeof id !== "string" ||
      typeof language !== "string" ||
      typeof provider !== "string" ||
      typeof model !== "string" ||
      !id ||
      !language ||
      !provider ||
      !model
    )
      return [];
    return [
      {
        id,
        language,
        provider,
        model,
        modelLabel: typeof modelLabel === "string" ? modelLabel : "",
      },
    ];
  });
}

/**
 * The preset dictation uses: the one last switched to, while its model is
 * still the one selected. Choosing another model in Settings leaves it.
 */
export function activePresetIndex(
  presets: readonly DictationPreset[],
  activeId: string,
  current: { provider?: string; model?: string },
) {
  return presets.findIndex(
    (preset) =>
      preset.id === activeId &&
      preset.provider === current.provider &&
      preset.model === current.model,
  );
}

export function presetLabel(preset: DictationPreset) {
  // The language in its own words, so "Português" reads the same in any UI.
  const name = getBaseLanguageDisplayName(preset.language, preset.language);
  const language =
    name.charAt(0).toLocaleUpperCase(preset.language) + name.slice(1);
  return `${language} · ${preset.modelLabel || preset.model}`;
}

/** The preset shortcut while it is registered, so others can't take it. */
export function useDictationPresetShortcut(): string | null {
  const enabled = useConfigValue("dictation_enabled");
  const presets = parseDictationPresets(useConfigValue("dictation_presets"));
  const shortcut = useConfigValue("dictation_preset_shortcut");
  return enabled && presets.length >= 2 && shortcut ? shortcut : null;
}

export function useActiveDictationPreset(): DictationPreset | null {
  const presets = parseDictationPresets(useConfigValue("dictation_presets"));
  const index = activePresetIndex(
    presets,
    useConfigValue("dictation_preset_active"),
    {
      provider: useConfigValue("current_stt_provider"),
      model: useConfigValue("current_stt_model"),
    },
  );
  return presets[index] ?? null;
}
