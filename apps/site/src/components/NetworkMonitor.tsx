"use client";

import { useState } from "react";

import { cn } from "@anlg/utils";

import { chip, eyebrow } from "./styles";

import type { Dictionary } from "~/i18n/en";

// The monitor only ever shows the three addresses the app knows, and only when the visitor asks.
export function NetworkMonitor({ t }: { t: Dictionary["net"] }) {
  const [shown, setShown] = useState<number[]>([]);
  const add = (i: number) =>
    setShown((rows) => (rows.includes(i) ? rows : [...rows, i]));

  return (
    <div className="border-line bg-card overflow-hidden rounded-[10px] border">
      <div className="border-line bg-surface flex flex-wrap items-center justify-between gap-3 border-b px-[18px] py-3.5">
        <span className={eyebrow}>{t.label}</span>
        <span className="text-mic flex items-center gap-2 font-mono text-[13px]">
          <span className="animate-blink bg-mic size-2.5 rounded-full" />
          {t.recording}
        </span>
      </div>
      <div className="border-line grid grid-cols-3 border-b">
        {[
          [String(shown.length), t.requests, ""],
          [`${shown.length}/3`, t.addresses, ""],
          ["0", t.ours, "text-ok"],
        ].map(([value, label, tone], i) => (
          <div
            key={label}
            className={cn([
              "text-muted flex flex-col gap-1 px-[18px] py-4 text-[13px]",
              i > 0 && "border-line border-l",
            ])}
          >
            <b
              className={cn([
                "font-mono text-[34px] leading-none tabular-nums",
                tone || "text-fg",
              ])}
            >
              {value}
            </b>
            <span>{label}</span>
          </div>
        ))}
      </div>
      <div className="overflow-x-auto">
        <div className="border-line text-muted grid min-w-[420px] grid-cols-[minmax(140px,1fr)_minmax(200px,1.6fr)] gap-3 border-b px-[18px] py-2.5 font-mono text-[11px] tracking-[0.08em] uppercase">
          <span>{t.colDest}</span>
          <span>{t.colWhy}</span>
        </div>
        {shown.length === 0 && (
          <p className="text-muted flex flex-col gap-1 px-[18px] py-[34px] text-center text-sm">
            <span className="text-ok font-mono">{t.empty}</span>
            <span>{t.emptyHint}</span>
          </p>
        )}
        {shown.map((i) => (
          <div
            key={i}
            className="animate-slide-in border-line grid min-w-[420px] grid-cols-[minmax(140px,1fr)_minmax(200px,1.6fr)] gap-3 border-b px-[18px] py-3 text-sm [animation-duration:0.4s]"
          >
            <span className="text-ink font-mono font-semibold">
              {t.rows[i].host}
            </span>
            <span>{t.rows[i].why}</span>
          </div>
        ))}
      </div>
      <div className="bg-surface flex flex-wrap gap-2 px-[18px] py-3.5">
        {t.rows.map((row, i) => (
          <button
            key={row.host}
            type="button"
            onClick={() => add(i)}
            className={chip}
          >
            {row.button}
          </button>
        ))}
        <button
          type="button"
          onClick={() => setShown([])}
          className={cn([chip, "border-line2 text-muted"])}
        >
          {t.reset}
        </button>
      </div>
      {shown.length === t.rows.length && (
        <p className="bg-btn text-btn-fg px-[18px] py-3.5 text-[15px]">
          {t.full}
        </p>
      )}
    </div>
  );
}
