"use client";

import { useEffect, useRef } from "react";

import { cn } from "@anlg/utils";

// Free drawing in red, like Annotate. Strokes are kept as fractions of the canvas so they survive a resize.
export function InkCanvas({ active }: { active: boolean }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const strokes = useRef<[number, number][][]>([]);
  const current = useRef<[number, number][] | null>(null);

  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const ctx = el.getContext("2d");
    if (!ctx) return;

    const redraw = () => {
      const { width, height } = el.getBoundingClientRect();
      ctx.clearRect(0, 0, width, height);
      ctx.lineCap = "round";
      ctx.lineJoin = "round";
      ctx.lineWidth = 4;
      ctx.strokeStyle =
        getComputedStyle(document.documentElement)
          .getPropertyValue("--ink")
          .trim() || "#c42323";
      const all = current.current
        ? [...strokes.current, current.current]
        : strokes.current;
      for (const stroke of all) {
        ctx.beginPath();
        stroke.forEach(([x, y], i) =>
          i
            ? ctx.lineTo(x * width, y * height)
            : ctx.moveTo(x * width, y * height),
        );
        ctx.stroke();
      }
    };
    const fit = () => {
      const { width, height } = el.getBoundingClientRect();
      const dpr = Math.min(devicePixelRatio || 1, 2);
      el.width = Math.round(width * dpr);
      el.height = Math.round(height * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      redraw();
    };
    const point = (e: PointerEvent): [number, number] => {
      const r = el.getBoundingClientRect();
      return [(e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height];
    };
    const down = (e: PointerEvent) => {
      el.setPointerCapture(e.pointerId);
      current.current = [point(e)];
    };
    const move = (e: PointerEvent) => {
      if (!current.current) return;
      current.current.push(point(e));
      redraw();
    };
    const up = () => {
      if (current.current && current.current.length > 1)
        strokes.current.push(current.current);
      current.current = null;
      redraw();
    };

    const observer = new ResizeObserver(fit);
    observer.observe(el);
    el.addEventListener("pointerdown", down);
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
    return () => {
      observer.disconnect();
      el.removeEventListener("pointerdown", down);
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
    };
  }, []);

  return (
    <canvas
      ref={canvas}
      aria-hidden="true"
      className={cn([
        "absolute inset-0 size-full",
        active
          ? "pointer-events-auto cursor-crosshair touch-none"
          : "pointer-events-none",
      ])}
    />
  );
}
