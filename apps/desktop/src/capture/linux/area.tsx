import { useLingui } from "@lingui/react/macro";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useRef, useState } from "react";

import {
  type AreaChoice,
  type AreaFrame,
  type AreaRect,
  commands as capture,
} from "@anlg/plugin-capture";
import { useMountEffect } from "@anlg/ui/hooks/use-mount-effect";
import { cn } from "@anlg/utils";

import {
  dragRect,
  isClick,
  pictureLayout,
  type Point,
  toPicture,
} from "./area-geometry";

async function choose(choice: AreaChoice) {
  try {
    await capture.areaChosen(choice);
  } catch (error) {
    console.error("[capture] area", error);
  }
}

function viewSize() {
  return { width: window.innerWidth, height: window.innerHeight };
}

/**
 * Recording an area on Linux: the frozen screen, full screen, to drag an
 * area on. Return (or a click) takes the whole screen, Space a window
 * picked in the desktop's dialog, Escape cancels.
 */
export function CaptureAreaScreen() {
  const { t } = useLingui();
  const [frame, setFrame] = useState<AreaFrame | null>(null);
  const [from, setFrom] = useState<Point | null>(null);
  const [rect, setRect] = useState<AreaRect | null>(null);
  const answered = useRef(false);

  const answer = (choice: AreaChoice) => {
    if (answered.current) return;
    answered.current = true;
    void choose(choice);
  };

  useMountEffect(() => {
    void (async () => {
      const result = await capture.areaFrame();
      if (result.status === "ok" && result.data) setFrame(result.data);
      else answer({ type: "cancel" });
    })();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") answer({ type: "cancel" });
      else if (event.key === "Enter") answer({ type: "whole" });
      else if (event.key === " ") answer({ type: "window" });
      else return;
      event.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (!frame) return <div className="h-screen w-screen bg-black" />;

  const view = viewSize();
  const layout = pictureLayout(
    { width: frame.imageWidth, height: frame.imageHeight },
    frame.region,
    view,
  );
  const picked = rect ? toPicture(rect, frame.region, view) : null;
  const point = (event: React.PointerEvent): Point => ({
    x: event.clientX,
    y: event.clientY,
  });

  return (
    <div
      className="relative h-screen w-screen cursor-crosshair overflow-hidden bg-black select-none"
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture(event.pointerId);
        setFrom(point(event));
        setRect(null);
      }}
      onPointerMove={(event) => {
        if (from) setRect(dragRect(from, point(event)));
      }}
      onPointerUp={(event) => {
        if (!from) return;
        const drawn = dragRect(from, point(event));
        setFrom(null);
        if (isClick(drawn)) return answer({ type: "whole" });
        answer({ type: "area", ...toPicture(drawn, frame.region, view) });
      }}
    >
      <img
        src={convertFileSrc(frame.imagePath)}
        alt=""
        draggable={false}
        className="pointer-events-none absolute max-w-none"
        style={layout}
      />
      {rect ? (
        <div
          className="pointer-events-none absolute outline outline-1 outline-white"
          style={{
            left: rect.x,
            top: rect.y,
            width: rect.width,
            height: rect.height,
            boxShadow: "0 0 0 100vmax rgb(0 0 0 / 0.45)",
          }}
        >
          {picked && (
            <span className="absolute -bottom-7 left-0 rounded bg-black/75 px-1.5 py-0.5 text-xs whitespace-nowrap text-white tabular-nums">
              {picked.width} × {picked.height}
            </span>
          )}
        </div>
      ) : (
        <div className="pointer-events-none absolute inset-0 bg-black/45" />
      )}
      <div
        className="absolute top-6 left-1/2 flex -translate-x-1/2 flex-col items-center gap-2"
        // Its buttons work without starting a drag.
        onPointerDown={(event) => event.stopPropagation()}
        onPointerUp={(event) => event.stopPropagation()}
      >
        <p className="pointer-events-none rounded-full bg-black/75 px-4 py-2 text-sm text-white">
          {frame.hint}
        </p>
        <div className="flex gap-2">
          {(
            [
              ["whole", t`Record whole screen`],
              ["window", t`Pick a window`],
              ["cancel", t`Cancel`],
            ] as const
          ).map(([type, label]) => (
            <button
              key={type}
              type="button"
              className={cn([
                "cursor-pointer rounded-full px-4 py-1.5 text-sm text-white",
                "bg-black/75 hover:bg-black/90",
              ])}
              onClick={() => answer({ type })}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
