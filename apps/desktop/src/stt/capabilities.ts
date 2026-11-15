import type { LocalModel } from "@anlg/plugin-local-stt";
import {
  commands as listenerCommands,
  type TranscriptionMode,
} from "@anlg/plugin-transcription";

type LiveTranscriptionConfig = {
  languages: string[];
  omittedLanguages?: string[];
  transcriptionMode?: TranscriptionMode;
};

const SONIQO_PARAKEET_BATCH_LANGUAGE_CODES = new Set([
  "bg",
  "cs",
  "da",
  "de",
  "el",
  "en",
  "es",
  "et",
  "fi",
  "fr",
  "hr",
  "hu",
  "it",
  "lt",
  "lv",
  "mt",
  "nl",
  "pl",
  "pt",
  "ro",
  "ru",
  "sk",
  "sl",
  "sv",
  "uk",
]);
const SONIQO_STREAMING_LANGUAGE_CODES = SONIQO_PARAKEET_BATCH_LANGUAGE_CODES;

// Base codes of SpeechTranscriber.supportedLocales on macOS 26 (30 regional
// variants across these 10 languages).
const APPLE_SPEECH_LANGUAGE_CODES = new Set([
  "de",
  "en",
  "es",
  "fr",
  "it",
  "ja",
  "ko",
  "pt",
  "yue",
  "zh",
]);

/// Whether Apple Speech can transcribe the language at all. A language it supports but
/// the user has not added in System Settings is a different problem with a different fix.
export function canAppleSpeechTranscribe(language: string) {
  return APPLE_SPEECH_LANGUAGE_CODES.has(baseLanguageCode(language));
}

function liveLanguageCodesForModel(model: string | null | undefined) {
  return model === "apple-speech"
    ? APPLE_SPEECH_LANGUAGE_CODES
    : SONIQO_STREAMING_LANGUAGE_CODES;
}

export function isSupportedLocalSttModel(
  model?: string | null,
): model is LocalModel {
  return (
    typeof model === "string" &&
    (model.startsWith("soniqo-") ||
      model.startsWith("gguf-") ||
      model === "apple-speech" ||
      model.startsWith("Quantized"))
  );
}

// Batch models that accept a whole meeting in one request. Their recordings are
// transcribed once after capture stops, instead of in minute-long recovery
// chunks whose speaker labels reset every chunk.
export function requiresRetainedBatchAudio(
  provider?: string | null,
  model?: string | null,
) {
  return (
    (provider === "elevenlabs" && model === "scribe_v2") ||
    (provider === "assemblyai" &&
      (model === "universal-3-5-pro" || model === "universal-3-pro"))
  );
}

export function isOnDeviceSttModel(
  provider?: string | null,
  model?: string | null,
): model is LocalModel {
  if (!isSupportedLocalSttModel(model)) {
    return false;
  }

  if (provider === "soniqo") {
    return model.startsWith("soniqo-");
  }

  if (provider === "apple_speech" || provider === "apple-speech") {
    return model === "apple-speech";
  }

  if (provider === "gguf") {
    return model.startsWith("gguf-");
  }

  return false;
}

export function isLocalFileSttModel(
  provider?: string | null,
  model?: string | null,
) {
  return provider === "local_file" && model === "local-file";
}

const APPLE_SILICON_TARGETS = ["macos/aarch64"];
const GGUF_ENGINE_TARGETS = [
  "macos/aarch64",
  "windows/x86_64",
  "linux/x86_64",
  "linux/aarch64",
];
const LOCAL_STT_PROVIDER_TARGETS = new Map<string, readonly string[]>([
  ["soniqo", APPLE_SILICON_TARGETS],
  ["apple_speech", APPLE_SILICON_TARGETS],
  ["gguf", GGUF_ENGINE_TARGETS],
  ["local_file", GGUF_ENGINE_TARGETS],
]);

function localSttProviderId(provider: string) {
  return provider === "apple-speech" ? "apple_speech" : provider;
}

export function isLocalSttProvider(provider?: string | null) {
  return (
    !!provider && LOCAL_STT_PROVIDER_TARGETS.has(localSttProviderId(provider))
  );
}

export function isLocalSttProviderAvailable(
  provider: string | null | undefined,
  currentPlatform: string,
  currentArch: string,
) {
  const targets = provider
    ? LOCAL_STT_PROVIDER_TARGETS.get(localSttProviderId(provider))
    : undefined;
  return targets?.includes(`${currentPlatform}/${currentArch}`) ?? false;
}

export function getUnsupportedDesktopLocalSttRepair(
  currentPlatform: string,
  currentArch: string,
  provider: string | undefined,
  model: string | undefined,
) {
  if (
    (!isOnDeviceSttModel(provider, model) &&
      !isLocalFileSttModel(provider, model)) ||
    isLocalSttProviderAvailable(provider, currentPlatform, currentArch)
  ) {
    return null;
  }

  return { provider: "", model: "" };
}

export function isConfiguredSttModel(
  provider?: string | null,
  model?: string | null,
) {
  if (!provider || !model) {
    return false;
  }

  if (provider === "soniqo") {
    return model.startsWith("soniqo-");
  }

  if (provider === "apple_speech") {
    return model === "apple-speech";
  }

  if (provider === "local_file") {
    return model === "local-file";
  }

  if (provider === "gguf") {
    return model.startsWith("gguf-");
  }

  return true;
}

export function isRealtimeLocalModel(model?: string | null) {
  return model === "soniqo-parakeet-streaming" || model === "apple-speech";
}

export const LOCAL_DIARIZATION_TARGET = {
  provider: "local_diarization",
  model: "local-diarization",
  baseUrl: "",
  apiKey: "",
} as const;

// The on-device engine that re-reads a live on-device recording to separate
// the voices sharing one channel. Parakeet Batch only counts when it is
// already downloaded and speaks the meeting's language: running it otherwise
// would start a multi-gigabyte download or replace the transcript with the
// wrong language. Apple Speech can re-read its own recordings. Without either,
// the bundled diarizer labels the live words without transcribing again.
export function getSpeakerRefinementTarget(
  liveModel: string | null | undefined,
  parakeetBatchDownloaded: boolean,
  languages: readonly string[],
) {
  if (!isRealtimeLocalModel(liveModel)) {
    return null;
  }

  const [liveLanguage] = getOnDeviceTranscriptionConfig(
    liveModel,
    languages,
  ).languages;
  if (
    parakeetBatchDownloaded &&
    (!liveLanguage ||
      SONIQO_PARAKEET_BATCH_LANGUAGE_CODES.has(baseLanguageCode(liveLanguage)))
  ) {
    return {
      provider: "soniqo",
      model: "soniqo-parakeet-batch",
      baseUrl: "soniqo://local",
      apiKey: "",
    };
  }

  if (liveModel === "apple-speech") {
    return {
      provider: "apple_speech",
      model: "apple-speech",
      baseUrl: "apple-speech://local",
      apiKey: "",
    };
  }

  return LOCAL_DIARIZATION_TARGET;
}

// Mirrors REMOTE_SILENCE_MAX_SECONDS in listener2-core: a system-audio
// channel with no more speech than this carried no call.
const REMOTE_SILENCE_MAX_MS = 3_000;
// An untimed system-audio word still shows someone spoke on the far end.
const UNTIMED_WORD_MS = 300;

// Whether the voices of a live on-device transcript are worth separating once
// the recording stops. A mic that heard speech while system audio stayed
// silent was an in-person meeting (or a solo memo, which the diarizer keeps as
// one voice), whoever was invited. On a call, one known remote participant
// already names the far end, and splitting a single voice would only add noise.
export function shouldRefineSpeakers(
  words: readonly { channel?: number; start_ms?: number; end_ms?: number }[],
  remoteParticipants: number,
) {
  const remoteSpeechMs = words
    .filter((word) => word.channel === 1)
    .reduce(
      (total, word) =>
        total +
        (word.start_ms === undefined || word.end_ms === undefined
          ? UNTIMED_WORD_MS
          : Math.max(0, word.end_ms - word.start_ms)),
      0,
    );
  if (
    remoteSpeechMs <= REMOTE_SILENCE_MAX_MS &&
    words.some((word) => word.channel === 0)
  ) {
    return true;
  }
  if (remoteParticipants === 1) return false;
  return remoteParticipants > 1
    ? words.length > 0
    : words.some((word) => word.channel === 1);
}

export function getSttModelTranscriptionMode(
  provider?: string | null,
  model?: string | null,
): TranscriptionMode | undefined {
  if (isLocalFileSttModel(provider, model)) {
    return "batch";
  }

  if (
    ["nari", "inworld", "gradium", "modulate", "alebex", "nvidia"].includes(
      provider ?? "",
    )
  )
    return "live";

  if (provider === "smallestai" && model === "pulse-pro") {
    return "batch";
  }

  if (
    provider === "groq" ||
    provider === "openrouter" ||
    provider === "siliconflow" ||
    provider === "together" ||
    provider === "zai" ||
    provider === "speechmatics" ||
    provider === "azure_speech" ||
    provider === "google_cloud" ||
    provider === "aws_transcribe" ||
    provider === "amazon_bedrock" ||
    provider === "revai" ||
    provider === "pyannote" ||
    provider === "aquavoice" ||
    provider === "cohere"
  ) {
    return "batch";
  }

  if (provider === "google_generative_ai") {
    if (model?.includes("transcribe-live")) {
      return "live";
    }
    if (model) {
      return "batch";
    }
  }

  if (provider === "openai") {
    if (model === "gpt-live-transcribe") return "live";
    if (
      model === "gpt-transcribe" ||
      model === "gpt-4o-transcribe-diarize" ||
      model === "gpt-4o-transcribe" ||
      model === "gpt-4o-mini-transcribe" ||
      model === "whisper-1"
    ) {
      return "batch";
    }
  }

  if (provider === "assemblyai") {
    if (model === "universal-3-pro" || model === "universal-3-5-pro") {
      return "batch";
    }
    if (model === "u3-rt-pro" || model === "universal-3-5-pro-realtime") {
      return "live";
    }
  }

  if (provider === "elevenlabs") {
    if (model === "scribe_v2") return "batch";
    if (model === "scribe_v2_realtime") return "live";
  }

  if (provider === "mistral") {
    if (model === "voxtral-mini-2602" || model === "voxtral-mini-latest") {
      return "batch";
    }
    if (model === "voxtral-mini-transcribe-realtime-2602") return "live";
  }

  if (provider === "soniox") {
    if (model === "stt-async-v5" || model === "stt-async-v4") return "batch";
    if (
      model === "stt-rt-v5" ||
      model === "stt-rt-v4" ||
      model === "stt-v5" ||
      model === "stt-v4"
    ) {
      return "live";
    }
  }

  if (provider === "deepgram" && model?.startsWith("flux-")) {
    return "live";
  }

  if (provider === "gladia" && model === "solaria-3") {
    return "batch";
  }

  return undefined;
}

function baseLanguageCode(language: string) {
  return language.split(/[-_]/)[0]?.toLowerCase() ?? "";
}

function languageSupportProvider(provider: string) {
  if (provider === "custom" || provider === "cloudflare_workers_ai") {
    return "deepgram";
  }

  return provider === "apple_speech" ? "apple-speech" : provider;
}

export async function isSupportedLanguagesLive(
  provider: string,
  model: string | null | undefined,
  languages: readonly string[],
) {
  const result = await listenerCommands.isSupportedLanguagesLive(
    languageSupportProvider(provider),
    model ?? null,
    [...languages],
  );

  return result.status === "ok" ? result.data : true;
}

export async function isSupportedLanguagesBatch(
  provider: string,
  model: string | null | undefined,
  languages: readonly string[],
) {
  const result = await listenerCommands.isSupportedLanguagesBatch(
    languageSupportProvider(provider),
    model ?? null,
    [...languages],
  );

  return result.status === "ok" ? result.data : true;
}

export function getTranscriptionLanguages(
  mainLanguage: string | null | undefined,
  spokenLanguages: readonly string[] | null | undefined,
) {
  const seen = new Set<string>();
  const languages: string[] = [];

  for (const language of [mainLanguage, ...(spokenLanguages ?? [])]) {
    if (!language) {
      continue;
    }

    const baseCode = baseLanguageCode(language);
    if (!baseCode || seen.has(baseCode)) {
      continue;
    }

    seen.add(baseCode);
    languages.push(language);
  }

  return languages;
}

export function getOnDeviceTranscriptionConfig(
  model: string | null | undefined,
  languages: readonly string[],
): LiveTranscriptionConfig {
  if (!isRealtimeLocalModel(model)) {
    return {
      languages: [...languages],
      transcriptionMode: "batch",
    };
  }

  const liveLanguageCodes = liveLanguageCodesForModel(model);
  const supportedLiveLanguages = languages.filter((language) =>
    liveLanguageCodes.has(baseLanguageCode(language)),
  );

  if (languages.length > 0 && supportedLiveLanguages.length === 0) {
    return {
      languages: [],
      transcriptionMode: "live",
    };
  }

  return {
    languages:
      supportedLiveLanguages.length > 0
        ? [supportedLiveLanguages[0]]
        : [...languages],
    transcriptionMode: "live",
  };
}

export function getOnDeviceTranscriptionMode(
  model: string | null | undefined,
  languages: readonly string[] = [],
) {
  return getOnDeviceTranscriptionConfig(model, languages).transcriptionMode;
}

export async function getLiveTranscriptionConfig({
  provider,
  model,
  languages,
}: {
  provider?: string | null;
  model?: string | null;
  languages: readonly string[];
}): Promise<LiveTranscriptionConfig> {
  if (isLocalFileSttModel(provider, model)) {
    return {
      languages: [...languages],
      transcriptionMode: "batch",
    };
  }

  if (isOnDeviceSttModel(provider, model)) {
    return getOnDeviceTranscriptionConfig(model, languages);
  }

  const config = {
    languages: [...languages],
    transcriptionMode: getSttModelTranscriptionMode(provider, model),
  } satisfies LiveTranscriptionConfig;

  if (
    !provider ||
    config.transcriptionMode === "batch" ||
    languages.length <= 1
  ) {
    return config;
  }

  if (await isSupportedLanguagesLive(provider, model, languages)) {
    return config;
  }

  const primaryLanguage = languages[0];
  if (
    primaryLanguage &&
    (await isSupportedLanguagesLive(provider, model, [primaryLanguage]))
  ) {
    return {
      ...config,
      languages: [primaryLanguage],
      omittedLanguages: languages.slice(1),
    };
  }

  return config;
}

export async function isLiveTranscriptionSupported(
  provider?: string | null,
  model?: string | null,
) {
  if (!provider || !model) {
    return false;
  }

  if (isLocalFileSttModel(provider, model)) {
    return false;
  }

  return isSupportedLanguagesLive(provider, model, []);
}
