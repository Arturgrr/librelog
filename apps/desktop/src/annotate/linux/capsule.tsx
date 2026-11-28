import { motion } from "motion/react";
import { useState } from "react";

import {
  type AnnotateSurface,
  commands as annotate,
  events as annotateEvents,
} from "@anlg/plugin-annotate";
import {
  Check,
  MicrophoneSlash,
  PencilSimple,
  WarningCircle,
  X,
} from "@anlg/ui/components/icons";
import { useMountEffect } from "@anlg/ui/hooks/use-mount-effect";
import { cn } from "@anlg/utils";

type Target = "pen" | "stop" | "discard" | "confirm_discard" | "keep";

async function press(target: Target) {
  try {
    await annotate.capsulePressed(target);
  } catch (error) {
    console.error("[annotate] capsule", error);
  }
}

/** `m:ss`, hours past an hour, as on macOS. */
function clock(ms: number) {
  const seconds = Math.floor(Math.max(0, ms) / 1000);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor(seconds / 60) % 60;
  const rest = String(seconds % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}`
    : `${minutes}:${rest}`;
}

/**
 * The dictation HUD's plate (`apps/desktop/src/dictation/hud/scope.ts`'s
 * palette): white or #191919 with the system, a hairline rim and a soft
 * shadow 4 px below, 26 px tall. Recording it is as wide as its controls;
 * preparing, ready and failed it narrows to the HUD's processing width.
 */
const PLATE = cn([
  "flex h-[26px] items-center overflow-hidden rounded-full",
  "bg-white text-[#161616] shadow-[0_4px_7px_rgba(0,0,0,0.12)]",
  "ring-1 ring-black/12 ring-inset",
  "[@media(prefers-color-scheme:dark)]:bg-[#191919]",
  "[@media(prefers-color-scheme:dark)]:text-[#fafafa]",
  "[@media(prefers-color-scheme:dark)]:shadow-[0_4px_7px_rgba(0,0,0,0.45)]",
  "[@media(prefers-color-scheme:dark)]:ring-white/16",
  "text-[13px] tabular-nums select-none",
]);

const BUTTON = cn([
  "flex size-[22px] shrink-0 items-center justify-center rounded-full",
  "hover:bg-black/8 [@media(prefers-color-scheme:dark)]:hover:bg-white/12",
]);

const DANGER =
  "text-[#b82d35] [@media(prefers-color-scheme:dark)]:text-[#f17471]";

const WIDTH = { recording: 200, asking: 204, compact: 76 };
/** The HUD's springs: 0.3 s, a little bounce. */
const SPRING = { type: "spring", duration: 0.3, bounce: 0.15 } as const;

/**
 * The recording capsule on Linux: `● 0:07 ✎ │ ■ ✕`, or `Discard? [Discard]
 * [Keep]` while it asks, then the annotation being prepared, ready or
 * failed, as the dictation HUD shows its transcription. The plugin owns
 * what it shows and closes the window; this one only reports presses.
 */
export function AnnotateCapsuleScreen() {
  const [surface, setSurface] = useState<AnnotateSurface | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [level, setLevel] = useState(0);

  useMountEffect(() => {
    let disposed = false;
    const unlisten: (() => void)[] = [];
    const keep = (stop: () => void) =>
      disposed ? stop() : unlisten.push(stop);
    void annotateEvents.annotateSurface
      .listen(({ payload }) => setSurface(payload))
      .then(keep);
    void annotateEvents.annotateLevel
      .listen(({ payload }) => setLevel(payload.level))
      .then(keep);
    void (async () => {
      const result = await annotate.surface();
      if (!disposed && result.status === "ok") {
        setSurface((current) => current ?? result.data);
      }
    })();
    void (async () => {
      try {
        await annotate.surfaceReady();
      } catch (error) {
        console.error("[annotate] capsule", error);
      }
    })();
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => {
      disposed = true;
      for (const stop of unlisten) stop();
      clearInterval(timer);
    };
  });

  const stage = surface?.stage ?? "recording";
  const width =
    stage !== "recording"
      ? WIDTH.compact
      : surface?.asking
        ? WIDTH.asking
        : WIDTH.recording;

  return (
    <div className="flex h-screen w-screen items-center justify-center">
      <motion.div
        className={PLATE}
        initial={{ opacity: 0, scale: 0.9, width }}
        animate={{ opacity: 1, scale: 1, width }}
        transition={SPRING}
      >
        {stage === "recording" ? (
          <Recording surface={surface} now={now} level={level} />
        ) : (
          <Outcome stage={stage} />
        )}
      </motion.div>
    </div>
  );
}

function Recording({
  surface,
  now,
  level,
}: {
  surface: AnnotateSurface | null;
  now: number;
  level: number;
}) {
  const labels = surface?.labels;
  // The app's words come with the state; English until then.
  const discard = labels?.discard ?? "Discard";
  const draw = labels?.draw ?? "Draw";
  const stop = labels?.stop ?? "Stop";

  if (surface?.asking) {
    return (
      <div className="flex w-full items-center gap-1 pr-0.5 pl-3">
        <span className="min-w-0 flex-1 truncate">
          {labels?.discardQuestion ?? "Discard?"}
        </span>
        <button
          type="button"
          className="h-[22px] rounded-full bg-[#ff3b30] px-2.5 font-medium text-white"
          onClick={() => void press("confirm_discard")}
        >
          {discard}
        </button>
        <button
          type="button"
          className={cn([
            "h-[22px] rounded-full px-2.5 font-medium",
            "bg-black/6 [@media(prefers-color-scheme:dark)]:bg-white/12",
          ])}
          onClick={() => void press("keep")}
        >
          {labels?.keep ?? "Keep"}
        </button>
      </div>
    );
  }

  return (
    <div className="flex w-full items-center gap-0.5 pr-0.5 pl-2.5">
      <Dot
        unheard={surface?.unheard ?? false}
        label={labels?.unheard ?? "The narration isn't being recorded"}
        level={level}
      />
      <span className="min-w-0 flex-1 pl-1.5">
        {clock(surface ? now - surface.startedWallMs : 0)}
      </span>
      {surface?.drawable && (
        <button
          type="button"
          title={draw}
          aria-label={draw}
          aria-pressed={surface.penOn}
          className={cn([BUTTON, surface.penOn && "bg-[#ff3b30] text-white"])}
          onClick={() => void press("pen")}
        >
          <PencilSimple className="size-3.5" />
        </button>
      )}
      <span className="mx-1 h-3.5 w-px bg-current opacity-20" />
      <button
        type="button"
        title={stop}
        aria-label={stop}
        className={BUTTON}
        onClick={() => void press("stop")}
      >
        <span className="size-2 rounded-[2px] bg-current" />
      </button>
      <button
        type="button"
        title={discard}
        aria-label={discard}
        className={BUTTON}
        onClick={() => void press("discard")}
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}

/**
 * The recording dot, ringed by the narration's level so the person sees
 * they are heard; a crossed-out microphone while nothing records it.
 */
function Dot({
  unheard,
  label,
  level,
}: {
  unheard: boolean;
  label: string;
  level: number;
}) {
  if (unheard) {
    return (
      <span title={label} aria-label={label} role="img" className="shrink-0">
        <MicrophoneSlash className="size-3.5" weight="bold" />
      </span>
    );
  }
  return (
    <span className="relative flex size-3 shrink-0 items-center justify-center">
      <span
        className="absolute inset-0 rounded-full bg-[#ff3b30]/30 transition-transform duration-75"
        style={{ transform: `scale(${(2 + 4 * level) / 3})` }}
      />
      <span className="relative size-2 rounded-full bg-[#ff3b30]" />
    </span>
  );
}

/** Preparing: light passing along a faint trace; then a check or a warning. */
function Outcome({ stage }: { stage: "processing" | "ready" | "failed" }) {
  if (stage === "processing") {
    return (
      <div className="relative mx-auto h-0.5 w-11 overflow-hidden rounded-full bg-current/25">
        <motion.span
          className="absolute inset-y-0 w-4 rounded-full bg-current"
          initial={{ x: -16 }}
          animate={{ x: 44 }}
          transition={{ duration: 1.1, ease: "easeInOut", repeat: Infinity }}
        />
      </div>
    );
  }
  const Glyph = stage === "ready" ? Check : WarningCircle;
  return (
    <motion.span
      className={cn(["mx-auto", stage === "failed" && DANGER])}
      initial={{ scale: 0.05 }}
      animate={{ scale: 1 }}
      transition={{ type: "spring", duration: 0.16, bounce: 0.3 }}
    >
      <Glyph className="size-4" weight="bold" />
    </motion.span>
  );
}
