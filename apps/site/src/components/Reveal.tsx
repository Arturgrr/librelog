"use client";

import { type ReactNode, useRef } from "react";

import { cn } from "@anlg/utils";

import { useInView } from "./useInView";

// Sets data-wait until first seen and data-in after, so children can start an entrance with
// `group-data-in/reveal:` and hide until then with `group-data-wait/reveal:`.
export function Reveal({
  className,
  rise = false,
  children,
}: {
  className?: string;
  rise?: boolean;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const state = useInView(ref);
  return (
    <div
      ref={ref}
      data-wait={state === "waiting" || undefined}
      data-in={state === "in" || undefined}
      className={cn([
        "group/reveal",
        rise &&
          "transition-[opacity,translate] duration-700 ease-[cubic-bezier(0.2,1,0.3,1)] data-wait:translate-y-6 data-wait:opacity-0",
        className,
      ])}
    >
      {children}
    </div>
  );
}
