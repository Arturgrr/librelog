import { Trans, useLingui } from "@lingui/react/macro";
import { platform } from "@tauri-apps/plugin-os";
import { motion, useReducedMotion } from "motion/react";
import { Fragment, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";

import { CheckCircle } from "@anlg/ui/components/icons";
import { Button } from "@anlg/ui/components/ui/button";
import { Switch } from "@anlg/ui/components/ui/switch";
import { Textarea } from "@anlg/ui/components/ui/textarea";
import { useMountEffect } from "@anlg/ui/hooks/use-mount-effect";

import { useTranscriptionSetup } from "./model-status";
import { DictationPermissions } from "./permissions";
import { OnboardingButton } from "./shared";

import { useDictationStatus } from "~/dictation/state";
import { DownloadProgressBar } from "~/settings/ai/stt/model-rows";
import { formatDownloadProgress } from "~/settings/ai/stt/shared";
import { DesktopShortcutsNote } from "~/settings/desktop-shortcuts";
import {
  isDoubleTapShortcut,
  ShortcutKeys,
  shortcutKeys,
} from "~/settings/dictation-shortcut";
import { setSettingValue } from "~/settings/queries";
import { SettingRow } from "~/settings/setting-row";
import { useConfigValue } from "~/shared/config";

// A paste can reach the field a moment after its dictation reports done.
const LATE_INSERT_MS = 600;

export function DictationStep({
  onContinue,
  onChooseModel,
}: {
  onContinue: () => void;
  onChooseModel: () => void;
}) {
  const enabled = useConfigValue("dictation_enabled");

  return (
    <div className="flex max-w-xl flex-col gap-5">
      <ShortcutExplanation />
      <SettingRow
        title={<Trans>Turn on dictation</Trans>}
        description={
          <Trans>You can change this later in Settings › Dictation.</Trans>
        }
        controlWidth="content"
      >
        {(labelProps) => (
          <Switch
            {...labelProps}
            checked={enabled}
            onCheckedChange={(on) =>
              void setSettingValue("dictation_enabled", on)
            }
          />
        )}
      </SettingRow>
      {enabled ? (
        <>
          <DictationPermissions />
          <Practice onChooseModel={onChooseModel} />
        </>
      ) : null}
      <OnboardingButton onClick={onContinue}>
        <Trans>Continue</Trans>
      </OnboardingButton>
    </div>
  );
}

function ShortcutExplanation() {
  const shortcuts = useConfigValue("dictation_shortcuts");
  const handsFree = useConfigValue("dictation_hands_free");
  const doubleTap =
    shortcuts.length > 0 && shortcuts.every(isDoubleTapShortcut);
  const os = platform();

  return (
    <div className="flex flex-col gap-2 text-sm">
      <p className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span>
          <Trans>Your dictation shortcut:</Trans>
        </span>
        {shortcuts.map((shortcut, index) => (
          <Fragment key={shortcut}>
            {index > 0 ? (
              <span className="text-muted-foreground">
                <Trans>or</Trans>
              </span>
            ) : null}
            <ShortcutKeys keys={shortcutKeys(shortcut)} />
          </Fragment>
        ))}
      </p>
      <p className="text-muted-foreground">
        {doubleTap ? (
          handsFree ? (
            <Trans>
              Press the key twice quickly to start, speak, then press it once to
              finish. Your words are typed where your cursor is.
            </Trans>
          ) : (
            <Trans>
              Press the key twice quickly and keep holding it while you speak,
              then release it. Your words are typed where your cursor is.
            </Trans>
          )
        ) : handsFree ? (
          <Trans>
            Press the shortcut to start, speak, then press it again to finish.
            Your words are typed where your cursor is.
          </Trans>
        ) : (
          <Trans>
            Hold the shortcut while you speak, then release it. Your words are
            typed where your cursor is.
          </Trans>
        )}
      </p>
      <p className="text-muted-foreground text-xs">
        {os === "linux" ? (
          <Trans>
            On Wayland, your desktop asks you to approve Librelog's shortcut and
            may choose other keys. You can change the shortcut in Settings ›
            Dictation.
          </Trans>
        ) : (
          <Trans>You can change the shortcut in Settings › Dictation.</Trans>
        )}
      </p>
      <DesktopShortcutsNote shortcuts={shortcuts} failure />
    </div>
  );
}

function Practice({ onChooseModel }: { onChooseModel: () => void }) {
  const model = useTranscriptionSetup();

  if (model.kind === "ready") return <PracticeField />;

  return (
    <div
      role="status"
      className="bg-raised border-border-weak flex flex-col gap-2 rounded-lg border p-3 text-sm"
    >
      {model.kind === "none" ? (
        <>
          <p>
            <Trans>
              Dictation needs a transcription model, and none is set up yet. You
              can choose one now, or continue and set it up later.
            </Trans>
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="self-start"
            onClick={onChooseModel}
          >
            <Trans>Choose a model</Trans>
          </Button>
        </>
      ) : model.kind === "installing" ? (
        <InstallingModel name={model.model} progress={model.progress} />
      ) : (
        <p className={model.kind === "error" ? "text-danger" : undefined}>
          {model.message}
        </p>
      )}
    </div>
  );
}

function InstallingModel({
  name,
  progress,
}: {
  name: string;
  progress: number | null;
}) {
  const { t } = useLingui();
  const percent = formatDownloadProgress(progress ?? 0) ?? "0%";

  return (
    <>
      <p>
        <Trans>
          {name} is still downloading. You can try dictation once it finishes,
          or continue now; it keeps downloading in the background.
        </Trans>
      </p>
      <div className="flex items-center gap-2">
        <DownloadProgressBar
          label={t`Installing ${name}`}
          progress={progress ?? 0}
          className="flex-1"
        />
        <span className="text-faint font-mono text-xs">{percent}</span>
      </div>
    </>
  );
}

type Attempt = "waiting" | "listening" | "succeeded" | "missed";

function PracticeField() {
  const { t } = useLingui();
  const fieldRef = useRef<HTMLTextAreaElement>(null);
  // The field's text when the dictation stopped recording: anything added
  // after that came from the dictation.
  const before = useRef("");
  const [attempt, setAttempt] = useState<Attempt>("waiting");
  const status = useDictationStatus(
    useShallow((state) => ({
      phase: state.phase,
      error: state.error,
      lastTranscript: state.lastTranscript,
    })),
  );

  const received = () => {
    const text = fieldRef.current?.value ?? "";
    return text.trim() !== "" && text !== before.current;
  };

  useMountEffect(() => {
    let late: ReturnType<typeof setTimeout> | undefined;
    const unsubscribe = useDictationStatus.subscribe((state, previous) => {
      if (state.capturing && !previous.capturing) {
        clearTimeout(late);
        before.current = fieldRef.current?.value ?? "";
        setAttempt((current) =>
          current === "succeeded" ? current : "listening",
        );
      }
      if (state.phase === "transcribing" && previous.phase !== "transcribing") {
        before.current = fieldRef.current?.value ?? "";
      }
      if (state.phase === "idle" && previous.phase !== "idle") {
        clearTimeout(late);
        late = setTimeout(() => {
          setAttempt((current) =>
            current !== "listening"
              ? current
              : received()
                ? "succeeded"
                : "missed",
          );
        }, LATE_INSERT_MS);
      }
    });
    return () => {
      clearTimeout(late);
      unsubscribe();
    };
  });

  const tryAgain = () => {
    if (fieldRef.current) {
      fieldRef.current.value = "";
      fieldRef.current.focus();
    }
    before.current = "";
    setAttempt("waiting");
  };

  return (
    <div className="flex flex-col gap-2">
      <label className="flex flex-col gap-2 text-sm">
        <span className="font-medium">
          <Trans>Try it here</Trans>
        </span>
        <Textarea
          ref={fieldRef}
          rows={3}
          placeholder={t`Click here, then use your dictation shortcut and say something.`}
          onInput={() => {
            // Typed by hand while recording doesn't count; text that arrives
            // while the dictation is being transcribed is its insertion.
            if (
              attempt === "listening" &&
              useDictationStatus.getState().phase === "transcribing" &&
              received()
            ) {
              setAttempt("succeeded");
            }
          }}
        />
      </label>
      <PracticeStatus
        attempt={attempt}
        phase={status.phase}
        error={status.error}
        lastTranscript={status.lastTranscript}
        onTryAgain={tryAgain}
      />
    </div>
  );
}

function PracticeStatus({
  attempt,
  phase,
  error,
  lastTranscript,
  onTryAgain,
}: {
  attempt: Attempt;
  phase: string;
  error: string | null;
  lastTranscript: string;
  onTryAgain: () => void;
}) {
  const reduceMotion = useReducedMotion();

  if (attempt === "succeeded") {
    return (
      <motion.div
        role="status"
        initial={reduceMotion ? false : { opacity: 0, y: 4 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.2, ease: "easeOut" }}
        className="flex flex-wrap items-center gap-2 text-sm"
      >
        <CheckCircle className="text-foreground size-4" aria-hidden />
        <span className="flex-1">
          <Trans>It works. Your words went straight into the field.</Trans>
        </span>
        <Button type="button" variant="outline" size="sm" onClick={onTryAgain}>
          <Trans>Try again</Trans>
        </Button>
      </motion.div>
    );
  }

  if (attempt === "missed") {
    return (
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span
          role={error ? "alert" : "status"}
          className={
            error ? "text-danger flex-1" : "text-muted-foreground flex-1"
          }
        >
          {error ? (
            error
          ) : lastTranscript ? (
            <Trans>
              Librelog heard “{lastTranscript}”, but it didn't reach the field.
              Click in the field first, then try again.
            </Trans>
          ) : (
            <Trans>
              Nothing reached the field. Click in it, then try again.
            </Trans>
          )}
        </span>
        <Button type="button" variant="outline" size="sm" onClick={onTryAgain}>
          <Trans>Try again</Trans>
        </Button>
      </div>
    );
  }

  return (
    <p role="status" className="text-muted-foreground text-xs">
      {attempt === "listening" && phase === "recording" ? (
        <Trans>Listening…</Trans>
      ) : attempt === "listening" && phase === "starting" ? (
        <Trans>Starting microphone…</Trans>
      ) : attempt === "listening" ? (
        <Trans>Transcribing…</Trans>
      ) : (
        <Trans>Your words appear in the field when you finish.</Trans>
      )}
    </p>
  );
}
