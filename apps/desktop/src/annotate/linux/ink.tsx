import { useRef } from "react";

import {
  type AnnotateSurface,
  commands as annotate,
  events as annotateEvents,
} from "@anlg/plugin-annotate";
import { useMountEffect } from "@anlg/ui/hooks/use-mount-effect";

// The macOS overlay's look: red #ff3b30, 4.5 pt strokes, a lifted stroke
// held 650 ms, then faded over 650 ms on a squared curve.
const RED = "255, 59, 48";
const LINE_WIDTH = 4.5;
const HOLD_MS = 650;
const FADE_MS = 650;

type Point = { x: number; y: number };
type Stroke = { points: Point[]; liftedAt: number | null };
type PointerKind = "move" | "down" | "up" | "leave";

function opacity(stroke: Stroke, now: number) {
  if (stroke.liftedAt === null) return 1;
  const age = now - stroke.liftedAt;
  if (age <= HOLD_MS) return 1;
  const t = Math.min((age - HOLD_MS) / FADE_MS, 1);
  return (1 - t) * (1 - t);
}

function paint(
  context: CanvasRenderingContext2D,
  strokes: Stroke[],
  now: number,
) {
  const scale = window.devicePixelRatio || 1;
  context.setTransform(1, 0, 0, 1, 0, 0);
  context.clearRect(0, 0, context.canvas.width, context.canvas.height);
  context.setTransform(scale, 0, 0, scale, 0, 0);
  context.lineCap = "round";
  context.lineJoin = "round";
  context.lineWidth = LINE_WIDTH;
  for (const stroke of strokes) {
    const [first] = stroke.points;
    const alpha = opacity(stroke, now);
    if (!first || alpha <= 0) continue;
    const color = `rgba(${RED}, ${alpha})`;
    context.beginPath();
    if (stroke.points.length === 1) {
      context.fillStyle = color;
      context.arc(first.x, first.y, LINE_WIDTH / 2, 0, Math.PI * 2);
      context.fill();
      continue;
    }
    // Through the midpoints, each point a control point: smooth, and on the
    // pointer's path.
    context.moveTo(first.x, first.y);
    for (let i = 1; i < stroke.points.length - 1; i++) {
      const point = stroke.points[i];
      const next = stroke.points[i + 1];
      context.quadraticCurveTo(
        point.x,
        point.y,
        (point.x + next.x) / 2,
        (point.y + next.y) / 2,
      );
    }
    const last = stroke.points[stroke.points.length - 1];
    context.lineTo(last.x, last.y);
    context.strokeStyle = color;
    context.stroke();
  }
}

async function report(kind: PointerKind, point: Point) {
  try {
    await annotate.inkPointer(kind, point.x, point.y);
  } catch (error) {
    console.error("[annotate] ink", error);
  }
}

async function stopDrawing() {
  try {
    await annotate.inkEscape();
  } catch (error) {
    console.error("[annotate] ink", error);
  }
}

async function ready() {
  try {
    await annotate.surfaceReady();
  } catch (error) {
    console.error("[annotate] ink", error);
  }
}

/**
 * The ink overlay over the recorded screen on Linux. The plugin owns the
 * ink mode, the strokes it records and where the window takes the pointer;
 * this window draws what the pointer does while ink is on. A right click
 * turns ink off, and so does a click while the held shortcut inks.
 */
export function AnnotateInkScreen() {
  const canvas = useRef<HTMLCanvasElement>(null);

  useMountEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let ink: AnnotateSurface["ink"] = "off";
    let strokes: Stroke[] = [];
    let live: Stroke | null = null;
    let frame = 0;

    const render = () => {
      frame = 0;
      const context = canvas.current?.getContext("2d");
      if (!context) return;
      const now = performance.now();
      strokes = strokes.filter((stroke) => opacity(stroke, now) > 0);
      paint(context, strokes, now);
      if (strokes.length > 0) frame = requestAnimationFrame(render);
    };
    const redraw = () => {
      if (!frame) frame = requestAnimationFrame(render);
    };
    const lift = () => {
      if (!live) return;
      live.liftedAt = performance.now();
      live = null;
      redraw();
    };
    const begin = (point: Point) => {
      lift();
      live = { points: [point], liftedAt: null };
      strokes.push(live);
    };
    const follow = (surface: AnnotateSurface | null) => {
      const next = surface?.ink ?? "off";
      if (next === ink) return;
      lift();
      ink = next;
    };
    const resize = () => {
      const element = canvas.current;
      if (!element) return;
      const scale = window.devicePixelRatio || 1;
      element.width = Math.round(window.innerWidth * scale);
      element.height = Math.round(window.innerHeight * scale);
      redraw();
    };
    const pointer = (kind: PointerKind) => (event: PointerEvent) => {
      if (kind !== "move" && kind !== "leave" && event.button !== 0) return;
      const point = { x: event.clientX, y: event.clientY };
      void report(kind, point);
      if (ink === "hover") {
        if (kind === "down" || kind === "up") return;
        if (kind === "leave") lift();
        else if (live) live.points.push(point);
        else begin(point);
      } else if (ink === "drag") {
        if (kind === "down") begin(point);
        else if (kind === "move" && live) live.points.push(point);
        else if (kind === "up" && live) {
          live.points.push(point);
          lift();
        }
      }
      redraw();
    };
    const listeners: [EventTarget, string, EventListener][] = [
      [window, "resize", resize],
      [window, "pointermove", pointer("move") as EventListener],
      [window, "pointerdown", pointer("down") as EventListener],
      [window, "pointerup", pointer("up") as EventListener],
      [
        document.documentElement,
        "pointerleave",
        pointer("leave") as EventListener,
      ],
      [
        window,
        "contextmenu",
        (event) => {
          event.preventDefault();
          void stopDrawing();
        },
      ],
    ];
    for (const [target, type, listener] of listeners) {
      target.addEventListener(type, listener);
    }
    resize();
    void ready();
    void annotateEvents.annotateSurface
      .listen(({ payload }) => follow(payload))
      .then((stop) => (disposed ? stop() : (unlisten = stop)));
    void (async () => {
      const result = await annotate.surface();
      if (!disposed && result.status === "ok") follow(result.data);
    })();
    return () => {
      disposed = true;
      unlisten?.();
      cancelAnimationFrame(frame);
      for (const [target, type, listener] of listeners) {
        target.removeEventListener(type, listener);
      }
    };
  });

  return (
    <canvas
      ref={canvas}
      className="fixed inset-0 h-screen w-screen touch-none bg-transparent"
    />
  );
}
