import type { ContextExclusions, Replacement } from "@anlg/plugin-dictation";

import {
  useStoredSettingValues,
  type StoredSettingValues,
} from "~/settings/queries";
import {
  SETTING_DEFINITIONS,
  type SettingKey,
  type SettingValue,
} from "~/settings/schema";

type JsonParsedKeys =
  | "dictation_shortcuts"
  | "spoken_languages"
  | "personalization_dictionary_terms"
  | "ignored_platforms"
  | "included_platforms"
  | "dictation_send_after_paste_apps";

type ConfigValueType<K extends SettingKey> = K extends JsonParsedKeys
  ? string[]
  : K extends "personalization_dictionary_replacements"
    ? Replacement[]
    : K extends "dictation_context_exclusions"
      ? ContextExclusions | null
      : K extends keyof typeof SETTING_DEFINITIONS
        ? "default" extends keyof (typeof SETTING_DEFINITIONS)[K]
          ? SettingValue<K>
          : SettingValue<K> | undefined
        : never;

const JSON_PARSED_KEYS = new Set<SettingKey>([
  "dictation_shortcuts",
  "spoken_languages",
  "personalization_dictionary_terms",
  "ignored_platforms",
  "included_platforms",
  "dictation_send_after_paste_apps",
]);

export function useConfigValue<K extends SettingKey>(
  key: K,
): ConfigValueType<K> {
  return resolveConfigValue(key, useStoredSettingValues());
}

export function useConfigValues<K extends SettingKey>(
  keys: readonly K[],
): { [P in K]: ConfigValueType<P> } {
  return resolveConfigValues(keys, useStoredSettingValues());
}

export function resolveConfigValues<K extends SettingKey>(
  keys: readonly K[],
  stored: StoredSettingValues,
): { [P in K]: ConfigValueType<P> } {
  const result = {} as { [P in K]: ConfigValueType<P> };
  for (const key of keys) result[key] = resolveConfigValue(key, stored);
  return result;
}

export function resolveConfigValue<K extends SettingKey>(
  key: K,
  { values, hasValues }: StoredSettingValues,
): ConfigValueType<K> {
  const definition = SETTING_DEFINITIONS[key];
  const defaultValue = "default" in definition ? definition.default : undefined;

  if (
    key === "audio_retention" &&
    values.save_recordings === false &&
    !hasValues.has("audio_retention")
  ) {
    return "none" as ConfigValueType<K>;
  }

  if (key === "notification_bounce" && !hasValues.has(key)) {
    const legacyKeys = [
      "notification_bounce_summary",
      "notification_bounce_transcript",
    ] as const;
    const legacyValues = legacyKeys
      .filter((legacyKey) => hasValues.has(legacyKey))
      .map((legacyKey) => values[legacyKey]);
    if (legacyValues.length > 0) {
      return legacyValues.every(Boolean) as ConfigValueType<K>;
    }
  }

  if (key === "dictation_shortcuts" && !hasValues.has(key)) {
    const legacy = values.dictation_shortcut;
    if (hasValues.has("dictation_shortcut") && legacy) {
      return [legacy] as ConfigValueType<K>;
    }
  }

  const value = hasValues.has(key) ? values[key] : defaultValue;
  if (key === "personalization_dictionary_replacements") {
    return parseReplacements(value) as ConfigValueType<K>;
  }
  if (key === "dictation_context_exclusions") {
    return parseContextExclusions(value) as ConfigValueType<K>;
  }
  if (JSON_PARSED_KEYS.has(key)) {
    return parseStringArray(
      value,
      parseStringArray(defaultValue, []),
    ) as ConfigValueType<K>;
  }

  return value as ConfigValueType<K>;
}

function parseStringArray(value: unknown, fallback: string[]): string[] {
  if (Array.isArray(value)) {
    return value.filter((entry): entry is string => typeof entry === "string");
  }
  if (typeof value !== "string") return fallback;
  try {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed)
      ? parsed.filter((entry): entry is string => typeof entry === "string")
      : fallback;
  } catch {
    return fallback;
  }
}

function parseJson(value: unknown): unknown {
  if (typeof value !== "string") return value;
  try {
    return JSON.parse(value);
  } catch {
    return undefined;
  }
}

function parseReplacements(value: unknown): Replacement[] {
  const parsed = parseJson(value);
  if (!Array.isArray(parsed)) return [];
  return parsed.flatMap((entry) =>
    entry &&
    typeof entry.from === "string" &&
    typeof entry.to === "string" &&
    entry.from.trim() !== ""
      ? [{ from: entry.from, to: entry.to }]
      : [],
  );
}

// Null when the stored value can't be read, which denies window context.
function parseContextExclusions(value: unknown): ContextExclusions | null {
  if (value === "" || value === undefined) return { apps: [], websites: [] };
  const parsed = parseJson(value);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return null;
  }
  const { apps = [], websites = [] } = parsed as Record<string, unknown>;
  if (!isStringArray(apps) || !isStringArray(websites)) return null;
  return { apps, websites };
}

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((entry) => typeof entry === "string")
  );
}
