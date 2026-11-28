import { t } from "@lingui/core/macro";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useState } from "react";

import {
  commands as annotate,
  events as annotateEvents,
  type Finalized,
  type TranscriptInput,
} from "@anlg/plugin-annotate";
import { commands as dictation } from "@anlg/plugin-dictation";
import { commands as opener } from "@anlg/plugin-opener2";
import { commands as permissions } from "@anlg/plugin-permissions";
import { commands as shortcuts, events } from "@anlg/plugin-shortcut";
import { commands as transcription } from "@anlg/plugin-transcription";
import { toast } from "@anlg/ui/components/ui/toast";
import { useMountEffect } from "@anlg/ui/hooks/use-mount-effect";

import { createAnnotateFlow, narrationMicrophone } from "./flow";
import { chainRegistration } from "./registration";
import { annotateRuns, useAnnotateStatus } from "./state";
import {
  annotateAvailable,
  annotateOnLinux,
  openScreenRecordingSettings,
} from "./system";

import { useDictationStatus } from "~/dictation/state";
import { localizeExternalMessage } from "~/i18n/external-messages";
import { useSettingsReady } from "~/settings/queries";
import { useConfigValue } from "~/shared/config";
import { holdLocalModel } from "~/stt/local-model-hold";
import { useRunBatch } from "~/stt/useRunBatch";
import { useSTTConnection } from "~/stt/useSTTConnection";

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

export async function revealAnnotation(path: string) {
  unwrap(await opener.revealItemInDir(path));
}

/** Applies the retention setting now and hourly, whether Annotate is on or not. */
export function AnnotateRetention() {
  const settingsReady = useSettingsReady();
  const retention = useConfigValue("annotate_retention");
  if (!settingsReady || !annotateAvailable()) return null;
  return <ApplyRetention key={retention} retention={retention} />;
}

function ApplyRetention({ retention }: { retention: string }) {
  useMountEffect(() => {
    const preset =
      retention === "month" || retention === "forever" ? retention : "week";
    const apply = async () => {
      unwrap(await annotate.setRetention(preset));
    };
    apply().catch((error) => console.error("[annotate] retention", error));
  });
  return null;
}

export function AnnotateLifecycle() {
  const settingsReady = useSettingsReady();
  const enabled = useConfigValue("annotate_enabled");
  const phase = useAnnotateStatus((state) => state.phase);
  if (!settingsReady || !annotateAvailable() || !annotateRuns(enabled, phase)) {
    return null;
  }
  return <ActiveAnnotate />;
}

type Flow = ReturnType<typeof createAnnotateFlow>;

const BATCH_ID = `annotate-${crypto.randomUUID()}`;

/**
 * What the mounted Annotate gives the flow: its transcriber, the speech-to-text
 * connection it uses, and the microphone.
 */
const live: {
  runBatch: ReturnType<typeof useRunBatch> | null;
  conn: ReturnType<typeof useSTTConnection>["conn"];
  microphone: string;
  avoidBluetooth: boolean;
} = { runBatch: null, conn: null, microphone: "", avoidBluetooth: true };
let shared: Flow | null = null;

/**
 * One flow for the window's lifetime, outside React: a stopped annotation is
 * finished by the flow that stopped it even when Annotate remounts, and a
 * remount's recovery never starts a second transcription of it.
 */
function annotateFlow(): Flow {
  shared ??= createAnnotateFlow({
    plugin: {
      status: async () => unwrap(await annotate.status()),
      screenRecording: async () => unwrap(await annotate.screenRecording()),
      requestScreenRecording: async () =>
        unwrap(await annotate.requestScreenRecording()),
      start: async (narrate) => {
        const microphone = await narrationMicrophone(
          async () => unwrap(await transcription.getCurrentMicrophoneDevice()),
          live.microphone || null,
        );
        return unwrap(
          await annotate.start(microphone, live.avoidBluetooth, narrate, {
            discardQuestion: t`Discard?`,
            discard: t`Discard`,
            keep: t`Keep`,
            draw: t`Draw`,
            stop: t`Stop`,
            unheard: t`The narration isn't being recorded`,
          }),
        );
      },
      stop: async () => unwrap(await annotate.stop()),
      finalize: async (annotation, transcript) =>
        unwrap(await annotate.finalize(annotation, transcript)),
      cancel: async () => {
        unwrap(await annotate.cancel());
      },
      setInkMode: async (mode) => {
        unwrap(await annotate.setInkMode(mode));
      },
      togglePen: async () => {
        unwrap(await annotate.togglePen());
      },
      latest: async () => unwrap(await annotate.latest()),
    },
    microphoneAuthorized: async () =>
      unwrap(await permissions.checkPermission("microphone")) === "authorized",
    transcribe: async (audioPath) => {
      let words: TranscriptInput["words"] = [];
      const runBatch = live.runBatch;
      if (!runBatch) throw new Error("Annotate is not running.");
      const used = await runBatch(audioPath, {
        deferAudioFinalization: true,
        notifyOnCompletion: false,
        numSpeakers: 1,
        handlePersist: (heard) => {
          words = heard
            .slice()
            .sort((a, b) => a.start_ms - b.start_ms)
            .map((word) => ({
              startMs: Math.max(0, Math.round(word.start_ms)),
              endMs: Math.max(0, Math.round(word.end_ms)),
              text: word.text,
            }));
        },
      });
      const text = (words ?? [])
        .map((word) => word.text)
        .join("")
        .replace(/\s+/gu, " ")
        .trim();
      if (!text) return null;
      return { provider: used.provider, model: used.model, text, words };
    },
    // The 20-minute cap, and preparing the annotation.
    holdModel: () => holdLocalModel(live.conn, 25 * 60_000),
    copy: (text) => writeText(text),
    insert: async (text) => {
      unwrap(await dictation.insertText(text));
    },
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    setRecording: async (recording) => {
      unwrap(await shortcuts.setAnnotateRecording(recording));
    },
    notify: {
      ready: (finalized: Finalized) =>
        toast.success(t`Annotation ready`, {
          description: t`Its summary is on the clipboard. Use your Paste last annotation shortcut in your coding agent to paste it with its image.`,
          duration: 8_000,
          action: {
            label: annotateOnLinux() ? t`Show in folder` : t`Show in Finder`,
            onClick: () =>
              void revealAnnotation(finalized.contactSheet).catch(
                (error: unknown) => toast.error(describe(error)),
              ),
          },
        }),
      screenRecordingDenied: () =>
        toast.error(t`Librelog can't record the screen.`, {
          description: t`Allow Librelog in System Settings › Privacy & Security › Screen Recording, then try again.`,
          duration: Infinity,
          action: {
            label: t`Open System Settings`,
            onClick: () =>
              void openScreenRecordingSettings().catch((error: unknown) =>
                toast.error(describe(error)),
              ),
          },
        }),
      needsRestart: () =>
        toast.warning(t`Quit and reopen Librelog to record the screen.`, {
          description: t`macOS applies the Screen Recording permission after a relaunch.`,
        }),
      microphoneDenied: () =>
        toast.warning(
          t`This annotation records the screen without narration.`,
          {
            description: t`Enable microphone permission in Settings > Permissions to narrate.`,
          },
        ),
      narration: (heard) =>
        heard
          ? toast(t`The microphone is recording the narration again.`)
          : toast.warning(t`The narration isn't being recorded.`, {
              description: t`The microphone stopped or could not be opened. The screen keeps recording, and the narration resumes if the microphone comes back.`,
            }),
      transcriptionFailed: (error) => {
        console.error("[annotate] transcription", error);
        toast.warning(
          t`The narration could not be transcribed. The annotation is saved without it.`,
        );
      },
      transcriptionWaiting: () =>
        toast(t`Waiting to transcribe the narration`, {
          description: annotateOnLinux()
            ? t`Another recording is being transcribed on this computer. The annotation will be ready when it finishes.`
            : t`Another recording is being transcribed on this Mac. The annotation will be ready when it finishes.`,
        }),
      nothingToPaste: () => toast(t`There's no annotation to paste yet.`),
      stillPreparing: () =>
        toast(t`The annotation is still being prepared.`, {
          description: t`Paste it again once the capsule shows it is ready.`,
        }),
      error: (error) => toast.error(describe(error)),
    },
    onPhase: (phase) => useAnnotateStatus.setState({ phase }),
  });
  return shared;
}

function ActiveAnnotate() {
  const shortcut = useConfigValue("annotate_shortcut");
  const paste = useConfigValue("annotate_paste_shortcut");
  const pen = useConfigValue("annotate_pen_shortcut");
  const capturingShortcut = useDictationStatus(
    (state) => state.capturingShortcut,
  );
  const [flow, setFlow] = useState<Flow | null>(null);
  const runBatch = useRunBatch(BATCH_ID);
  const microphone = useConfigValue("microphone_device");
  // The setting dictation's microphone shares: a Bluetooth headset's
  // microphone drops what it plays to call quality while narrating too.
  const avoidBluetooth = useConfigValue("dictation_avoid_bluetooth_microphone");
  live.runBatch = runBatch;
  const { conn } = useSTTConnection();
  live.conn = conn;
  live.microphone = microphone;
  live.avoidBluetooth = avoidBluetooth;

  useMountEffect(() => {
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    const flow = annotateFlow();
    const listening = Promise.allSettled([
      annotateEvents.annotateEvent.listen(
        ({ payload }) => void flow.onEvent(payload),
      ),
      events.annotateShortcutEvent.listen(({ payload }) => {
        if (useDictationStatus.getState().capturingShortcut) return;
        void flow.onShortcut(payload.type);
      }),
      events.pasteLastAnnotationEvent.listen(() => void flow.pasteLast()),
    ]).then((results) => {
      for (const result of results) {
        if (result.status === "rejected") {
          console.error("[annotate] listen", result.reason);
        } else if (disposed) result.value();
        else unlisteners.push(result.value);
      }
    });
    // A recording that outlived a reload, or stopped while nobody listened:
    // once listening, so a stop in between is not missed.
    void listening.then(() => {
      if (!disposed) void flow.recover();
    });
    setFlow(flow);
    return () => {
      disposed = true;
      for (const stop of unlisteners) stop();
    };
  });

  if (!flow || capturingShortcut) return null;
  return (
    <AnnotateShortcuts
      key={`${shortcut}:${paste}:${pen}`}
      shortcut={shortcut}
      paste={paste}
      pen={pen}
    />
  );
}

// Registered again when any shortcut changes, and released while a
// shortcut recorder listens for keys. The pen's is held only while a
// recording runs.
function AnnotateShortcuts({
  shortcut,
  paste,
  pen,
}: {
  shortcut: string;
  paste: string;
  pen: string;
}) {
  useMountEffect(() => {
    let disposed = false;
    chainRegistration(async () => {
      if (disposed) return;
      unwrap(await shortcuts.configureAnnotate(shortcut, paste, pen || null));
      useAnnotateStatus.setState({ error: null });
    }).catch((error) => {
      if (disposed) return;
      const message = describe(error);
      useAnnotateStatus.setState({ error: message });
      toast.error(message);
    });
    return () => {
      disposed = true;
      useAnnotateStatus.setState({ error: null });
      chainRegistration(async () => {
        unwrap(await shortcuts.configureAnnotate(null, null, null));
      }).catch((error) => console.error("[annotate] unregister", error));
    };
  });
  return null;
}
