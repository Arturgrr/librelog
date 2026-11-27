import { t } from "@lingui/core/macro";
import { create } from "zustand";

import { commands as shortcuts, events } from "@anlg/plugin-shortcut";
import { commands as tray } from "@anlg/plugin-tray";
import { toast } from "@anlg/ui/components/ui/toast";
import { useMountEffect } from "@anlg/ui/hooks/use-mount-effect";

import { waitForDictationCleanup } from "./lifecycle";
import { waitForPasteLastCleanup } from "./paste-last";
import {
  activePresetIndex,
  type DictationPreset,
  parseDictationPresets,
  presetLabel,
} from "./presets";
import { useDictationStatus } from "./state";

import { localizeExternalMessage } from "~/i18n/external-messages";
import {
  getStoredSettingValues,
  setSettingValues,
  useSettingsReady,
} from "~/settings/queries";
import { resolveConfigValues, useConfigValue } from "~/shared/config";
import { listenerStore } from "~/store/zustand/listener/instance";
import { holdLocalModel } from "~/stt/local-model-hold";

// Long enough for the next dictation to find the model loaded.
const PRESET_HOLD_MS = 5 * 60_000;

export const useDictationPresetStatus = create<{ error: string | null }>(
  () => ({ error: null }),
);

let registration: Promise<void> = Promise.resolve();
let trayUpdates: Promise<void> = Promise.resolve();
let switching: Promise<void> = Promise.resolve();
// The preset to switch to once the dictation or meeting in progress is done.
let deferred: number | null = null;
let releaseHold = () => {};

export function waitForDictationPresetCleanup() {
  return registration;
}

function unwrap<T>(
  result: { status: "ok"; data: T } | { status: "error"; error: string },
): T {
  if (result.status === "error") throw new Error(result.error);
  return result.data;
}

function describe(error: unknown) {
  return localizeExternalMessage(
    error instanceof Error ? error.message : String(error),
  );
}

// A new model mid-dictation would transcribe the recording, or the ones
// waiting, with another model than the one it started with.
function dictating() {
  const { capturing, pending } = useDictationStatus.getState();
  return capturing || pending > 0;
}

// A meeting transcribes with the same provider and model until it ends.
function meetingRecording() {
  const { status } = listenerStore.getState().live;
  return status === "active" || status === "finalizing";
}

async function apply(preset: DictationPreset) {
  await setSettingValues({
    current_stt_provider: preset.provider,
    current_stt_model: preset.model,
    dictation_preset_active: preset.id,
  });
  releaseHold();
  releaseHold = holdLocalModel(preset, PRESET_HOLD_MS);
  toast(presetLabel(preset));
}

/**
 * Switches dictation to the preset at `request`, or to the one after the
 * preset in use when `request` is null. During a dictation the switch waits
 * for it, and every dictation waiting to be transcribed, to finish; during a
 * meeting, for the meeting to end.
 */
export function switchDictationPreset(request: number | null) {
  switching = switching
    .then(async () => {
      const values = resolveConfigValues(
        [
          "dictation_presets",
          "dictation_preset_active",
          "current_stt_provider",
          "current_stt_model",
        ] as const,
        await getStoredSettingValues(),
      );
      const presets = parseDictationPresets(values.dictation_presets);
      if (presets.length < 2) return;
      const from =
        deferred ??
        activePresetIndex(presets, values.dictation_preset_active, {
          provider: values.current_stt_provider,
          model: values.current_stt_model,
        });
      const index = request ?? (from + 1) % presets.length;
      const preset = presets[index];
      if (!preset) return;
      if (dictating() || meetingRecording()) {
        deferred = index;
        const label = presetLabel(preset);
        toast(
          dictating()
            ? t`Switching to ${label} after this dictation.`
            : t`Switching to ${label} after this meeting.`,
        );
        return;
      }
      deferred = null;
      await apply(preset);
    })
    .catch((error) => {
      toast.error(describe(error));
    });
  return switching;
}

export function DictationPresetLifecycle() {
  const settingsReady = useSettingsReady();
  const dictationEnabled = useConfigValue("dictation_enabled");
  const presets = parseDictationPresets(useConfigValue("dictation_presets"));
  const shortcut = useConfigValue("dictation_preset_shortcut");
  const capturingShortcut = useDictationStatus(
    (state) => state.capturingShortcut,
  );

  if (!settingsReady || !dictationEnabled || presets.length < 2) return null;
  return (
    <>
      <PresetRequests />
      <PresetTray />
      {shortcut && !capturingShortcut && (
        <PresetShortcut key={shortcut} shortcut={shortcut} />
      )}
    </>
  );
}

function PresetRequests() {
  useMountEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void events.dictationPresetEvent
      .listen(({ payload }) => void switchDictationPreset(payload.preset))
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      });
    const applyDeferred = () => {
      if (deferred === null || dictating() || meetingRecording()) return;
      const index = deferred;
      deferred = null;
      void switchDictationPreset(index);
    };
    const unsubscribeDictation = useDictationStatus.subscribe(applyDeferred);
    const unsubscribeMeeting = listenerStore.subscribe(applyDeferred);
    return () => {
      disposed = true;
      unlisten?.();
      unsubscribeDictation();
      unsubscribeMeeting();
      deferred = null;
      releaseHold();
      releaseHold = () => {};
    };
  });
  return null;
}

function PresetTray() {
  const presets = parseDictationPresets(useConfigValue("dictation_presets"));
  const activeId = useConfigValue("dictation_preset_active");
  const provider = useConfigValue("current_stt_provider");
  const model = useConfigValue("current_stt_model");
  const index = activePresetIndex(presets, activeId, { provider, model });
  const menu = {
    title: t`Dictation preset`,
    items: presets.map(presetLabel),
    active: index === -1 ? null : index,
  };
  return <PresetTrayMenu key={JSON.stringify(menu)} menu={menu} />;
}

function showTrayPresets(
  menu: { title: string; items: string[]; active: number | null } | null,
) {
  trayUpdates = trayUpdates
    .then(async () => {
      unwrap(await tray.setTrayDictationPresets(menu));
    })
    .catch((error) => console.error("[dictation-preset] tray menu", error));
}

function PresetTrayMenu({
  menu,
}: {
  menu: { title: string; items: string[]; active: number | null };
}) {
  useMountEffect(() => {
    showTrayPresets(menu);
    return () => showTrayPresets(null);
  });
  return null;
}

// Registered after the dictation and paste-last shortcuts, which it gives
// way to, and released while a shortcut recorder listens for keys.
function PresetShortcut({ shortcut }: { shortcut: string }) {
  useMountEffect(() => {
    let disposed = false;
    registration = registration
      .then(() => waitForDictationCleanup())
      .then(() => waitForPasteLastCleanup())
      .then(async () => {
        if (disposed) return;
        unwrap(await shortcuts.configureDictationPreset(shortcut));
        useDictationPresetStatus.setState({ error: null });
      })
      .catch((error) => {
        if (disposed) return;
        const message = describe(error);
        useDictationPresetStatus.setState({ error: message });
        toast.error(message);
      });
    return () => {
      disposed = true;
      useDictationPresetStatus.setState({ error: null });
      registration = registration
        .then(async () => {
          unwrap(await shortcuts.configureDictationPreset(null));
        })
        .catch((error) =>
          console.error("[dictation-preset] unregister", error),
        );
    };
  });
  return null;
}
