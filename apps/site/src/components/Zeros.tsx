"use client";

import { useRef } from "react";

import { cn } from "@anlg/utils";

import { hand, wrap } from "./styles";
import { useInView } from "./useInView";

import type { Dictionary } from "~/i18n/en";

const COLUMNS = ["987654321", "418279365", "739162845", "264983715"];

// A parody of a startup metrics board: the counters roll like a slot machine and land on zero.
export function Zeros({ t }: { t: Dictionary["zeros"] }) {
  const ref = useRef<HTMLDivElement>(null);
  const state = useInView(ref, 0.3);
  return (
    <section aria-labelledby="zeros-title" className="py-[60px]">
      <div className={wrap}>
        <h2 id="zeros-title" className="sr-only">
          {t.title}
        </h2>
        <div
          ref={ref}
          className="border-fg grid grid-cols-[repeat(auto-fit,minmax(min(100%,240px),1fr))] border-y-[1.5px]"
        >
          {t.items.map((label, i) => (
            <div key={label} className="flex flex-col gap-1.5 px-5 py-[30px]">
              <span
                aria-hidden="true"
                className="font-display max-tab:text-7xl block h-[1em] overflow-hidden text-8xl leading-none font-extrabold"
              >
                <span
                  className={cn([
                    "flex flex-col",
                    state === "waiting" && "translate-y-0",
                    state === "in" && "animate-roll",
                    state === "static" && "-translate-y-[90%]",
                  ])}
                  style={{ animationDelay: `${i * 0.15}s` }}
                >
                  {[...COLUMNS[i], "0"].map((digit, j) => (
                    <span key={j}>{digit}</span>
                  ))}
                </span>
              </span>
              <span className="sr-only">0</span>
              <span>{label}</span>
            </div>
          ))}
        </div>
        <p className={cn([hand, "mt-4 text-[34px]"])}>{t.note}</p>
      </div>
    </section>
  );
}
