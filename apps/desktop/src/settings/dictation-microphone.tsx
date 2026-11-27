import { Trans } from "@lingui/react/macro";
import { useQuery } from "@tanstack/react-query";
import { Channel } from "@tauri-apps/api/core";
import { platform } from "@tauri-apps/plugin-os";
import { useState } from "react";

import {
  commands as dictation,
  type LevelSource,
  type LevelUpdate,
} from "@anlg/plugin-dictation";
import { commands as transcription } from "@anlg/plugin-transcription";
import { Button } from "@anlg/ui/components/ui/button";
import { useMountEffect } from "@anlg/ui/hooks/use-mount-effect";
import { cn } from "@anlg/utils";

import { AudioDeviceRow } from "~/settings/general/audio-settings";
import { useSetSettingValue } from "~/settings/queries";
import { SettingSwitchRow } from "~/settings/setting-row";
import { useConfigValue } from "~/shared/config";
import { usePermission } from "~/shared/hooks/usePermissions";
import { useListener } from "~/stt/contexts";

/**
 * The dictation microphone, the one it actually records from when a
 * Bluetooth default is passed over, and its live level.
 */
export function DictationMicrophoneSettings() {
  const os = platform();
  // Linux opens microphones through ALSA, whose names never match the
  // system's device names, so a Bluetooth default cannot be replaced there.
  const swaps = os === "macos" || os === "windows";
  const microphone = useConfigValue("microphone_device");
  const setMicrophone = useSetSettingValue("microphone_device");
  const avoidBluetooth = useConfigValue("dictation_avoid_bluetooth_microphone");
  const setAvoidBluetooth = useSetSettingValue(
    "dictation_avoid_bluetooth_microphone",
  );
  const microphones = useQuery({
    queryKey: ["microphone-devices"],
    queryFn: async () => {
      const result = await transcription.listMicrophoneDevices();
      if (result.status === "error") throw new Error(result.error);
      return result.data;
    },
    refetchInterval: 3_000,
  });
  const choice = useQuery({
    queryKey: ["dictation-microphone", microphone, avoidBluetooth],
    queryFn: async () => {
      const result = await dictation.dictationMicrophone(
        microphone || null,
        avoidBluetooth,
      );
      if (result.status === "error") throw new Error(result.error);
      return result.data;
    },
    enabled: swaps,
    refetchInterval: 3_000,
  });
  const used = choice.data?.device;
  const passedOver = choice.data?.passedOver;

  return (
    <div className="flex flex-col gap-2">
      <AudioDeviceRow
        title={<Trans>Microphone</Trans>}
        description={
          <Trans>Choose the microphone that captures your voice.</Trans>
        }
        value={microphone}
        devices={microphones.data ?? []}
        onChange={setMicrophone}
      />
      {used && passedOver ? (
        <p className="text-muted-foreground text-xs">
          <Trans>
            Using {used} instead of {passedOver} so your music keeps its
            quality.
          </Trans>
        </p>
      ) : null}
      {swaps && choice.isPending ? null : (
        // Restarts on the microphone a newly connected headset gives way to.
        <MicrophoneLevel
          key={`${microphone}:${avoidBluetooth}:${used ?? ""}`}
          device={microphone || null}
          avoidBluetooth={avoidBluetooth}
        />
      )}
      {swaps ? (
        <SettingSwitchRow
          title={<Trans>Avoid Bluetooth headset microphone</Trans>}
          description={
            <Trans>
              When your default microphone is a Bluetooth headset, dictate with
              your computer's built-in or wired microphone instead, so what
              plays in the headset doesn't drop to call quality. A microphone
              you choose above is always used.
            </Trans>
          }
          checked={avoidBluetooth}
          onChange={setAvoidBluetooth}
        />
      ) : null}
    </div>
  );
}

/**
 * Shows the live level only where opening the microphone is safe: with
 * access already granted, so it never prompts, and with no meeting
 * recording it.
 */
function MicrophoneLevel({
  device,
  avoidBluetooth,
}: {
  device: string | null;
  avoidBluetooth: boolean;
}) {
  const mac = platform() === "macos";
  const permission = usePermission("microphone");
  const meetingActive = useListener(
    (state) => state.live.status !== "inactive" || state.live.loading,
  );
  const [attempt, setAttempt] = useState(0);

  if (mac && permission.status !== "authorized") {
    if (permission.status === undefined) return null;
    return (
      <div className="flex items-center justify-between gap-4">
        <p className="text-muted-foreground text-xs">
          <Trans>Allow microphone access to check its level here.</Trans>
        </p>
        <Button
          size="sm"
          variant="outline"
          disabled={permission.isPending}
          onClick={() =>
            permission.status === "denied"
              ? void permission.open()
              : permission.request()
          }
        >
          <Trans>Allow access</Trans>
        </Button>
      </div>
    );
  }
  if (meetingActive) {
    return (
      <p className="text-muted-foreground text-xs">
        <Trans>The microphone level shows once your meeting ends.</Trans>
      </p>
    );
  }
  return (
    <LiveLevel
      key={attempt}
      device={device}
      avoidBluetooth={avoidBluetooth}
      onRetry={() => setAttempt((attempt) => attempt + 1)}
    />
  );
}

// Starts and stops the meter one change at a time, so a stop never
// overtakes the start it ends.
let meter: Promise<unknown> = Promise.resolve();

function queueMeter(step: () => Promise<unknown>) {
  meter = meter
    .then(step)
    .catch((error: unknown) =>
      console.warn("[dictation] the microphone level failed", error),
    );
}

function LiveLevel({
  device,
  avoidBluetooth,
  onRetry,
}: {
  device: string | null;
  avoidBluetooth: boolean;
  onRetry: () => void;
}) {
  const [peak, setPeak] = useState(0);
  const [state, setState] = useState<
    "starting" | LevelSource | "ended" | "failed"
  >("starting");

  useMountEffect(() => {
    let mounted = true;
    queueMeter(async () => {
      if (!mounted) return;
      const levels = new Channel<LevelUpdate>((update) => {
        if (!mounted) return;
        if (update.type === "level") {
          setPeak(update.peak);
        } else {
          setPeak(0);
          setState((state) => (state === "inUse" ? state : "ended"));
        }
      });
      const result = await dictation.startInputLevel(
        device,
        avoidBluetooth,
        levels,
      );
      if (!mounted) return;
      if (result.status === "error") {
        setState("failed");
        return;
      }
      // A dictation recording ends the meter at once, before this answer.
      const source = result.data;
      setState((state) =>
        state === "ended" && source !== "inUse" ? state : source,
      );
    });
    return () => {
      mounted = false;
      queueMeter(() => dictation.stopInputLevel());
    };
  });

  if (state === "ended" || state === "inUse" || state === "failed") {
    return (
      <div className="flex items-center justify-between gap-4">
        <p className="text-muted-foreground text-xs">
          {state === "inUse" ? (
            <Trans>Dictation is using the microphone.</Trans>
          ) : state === "failed" ? (
            <Trans>The microphone could not be opened.</Trans>
          ) : (
            <Trans>The microphone level stopped.</Trans>
          )}
        </p>
        <Button size="sm" variant="outline" onClick={onRetry}>
          <Trans>Check again</Trans>
        </Button>
      </div>
    );
  }
  return (
    <div className="flex items-center gap-3">
      <span className="text-muted-foreground shrink-0 text-xs">
        <Trans>Input level</Trans>
      </span>
      <div
        role="meter"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(levelFraction(peak) * 100)}
        className={cn(["bg-muted h-1.5 flex-1 overflow-hidden rounded-full"])}
      >
        <div
          className={cn([
            "bg-primary h-full rounded-full",
            "transition-[width] duration-75",
          ])}
          style={{ width: `${levelFraction(peak) * 100}%` }}
        />
      </div>
    </div>
  );
}

/** A peak from 0 to 1 as a meter's fill, over the 60 dB below full scale. */
function levelFraction(peak: number) {
  if (peak <= 0) return 0;
  return Math.min(1, Math.max(0, (20 * Math.log10(peak) + 60) / 60));
}
