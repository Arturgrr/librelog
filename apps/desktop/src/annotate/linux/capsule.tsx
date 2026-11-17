import { useState } from "react";

import {
  type AnnotateSurface,
  commands as annotate,
  events as annotateEvents,
} from "@anlg/plugin-annotate";
import { PencilSimple, X } from "@anlg/ui/components/icons";
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

const BUTTON = cn([
  "flex size-7 items-center justify-center rounded-[7px]",
  "text-white hover:bg-white/15",
]);

/**
 * The recording capsule on Linux: `● 0:07 ✎ │ ■ ✕`, or `Discard? [Discard]
 * [Keep]` while it asks. The plugin owns what it shows; this window only
 * reports presses.
 */
export function AnnotateCapsuleScreen() {
  const [surface, setSurface] = useState<AnnotateSurface | null>(null);
  const [now, setNow] = useState(() => Date.now());

  useMountEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void annotateEvents.annotateSurface
      .listen(({ payload }) => setSurface(payload))
      .then((stop) => (disposed ? stop() : (unlisten = stop)));
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
      unlisten?.();
      clearInterval(timer);
    };
  });

  const labels = surface?.labels;
  // The app's words come with the state; English until then.
  const discard = labels?.discard ?? "Discard";
  const draw = labels?.draw ?? "Draw";
  const stop = labels?.stop ?? "Stop";

  return (
    <div
      className={cn([
        "flex h-9 w-[216px] items-center gap-1 rounded-full pr-2 pl-3",
        "bg-neutral-950/90 text-[13px] text-white tabular-nums",
        "ring-1 ring-white/15 select-none ring-inset",
      ])}
    >
      {surface?.asking ? (
        <>
          <span className="min-w-0 flex-1 truncate font-medium">
            {labels?.discardQuestion ?? "Discard?"}
          </span>
          <button
            type="button"
            className="h-7 rounded-[7px] bg-[#ff3b30] px-2 font-semibold"
            onClick={() => void press("confirm_discard")}
          >
            {discard}
          </button>
          <button
            type="button"
            className="h-7 rounded-[7px] bg-white/15 px-2 font-semibold"
            onClick={() => void press("keep")}
          >
            {labels?.keep ?? "Keep"}
          </button>
        </>
      ) : (
        <>
          <span className="ml-0.5 size-2 shrink-0 rounded-full bg-[#ff3b30]" />
          <span className="min-w-0 flex-1 pl-1.5 font-medium">
            {clock(surface ? now - surface.startedWallMs : 0)}
          </span>
          {surface?.drawable && (
            <button
              type="button"
              title={draw}
              aria-label={draw}
              aria-pressed={surface?.penOn ?? false}
              className={cn([BUTTON, surface?.penOn && "bg-[#ff3b30]"])}
              onClick={() => void press("pen")}
            >
              <PencilSimple className="size-4" />
            </button>
          )}
          <span className="mx-1 h-4 w-px bg-white/20" />
          <button
            type="button"
            title={stop}
            aria-label={stop}
            className={BUTTON}
            onClick={() => void press("stop")}
          >
            <span className="size-2.5 rounded-[2px] bg-white" />
          </button>
          <button
            type="button"
            title={discard}
            aria-label={discard}
            className={BUTTON}
            onClick={() => void press("discard")}
          >
            <X className="size-4" />
          </button>
        </>
      )}
    </div>
  );
}
