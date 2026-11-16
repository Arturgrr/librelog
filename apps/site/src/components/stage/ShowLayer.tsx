import type { CSSProperties } from "react";

import { cn } from "@anlg/utils";

import { type CardKind, SLOTS, type ShowState } from "./useShow";

import type { Dictionary } from "~/i18n/en";

const TAGS: Record<CardKind, string> = {
  meet: "bg-[#e6ebf8] text-[#2147a8]",
  dict: "bg-[#fbeedd] text-[#8a4705]",
  ann: "bg-[#fbe3e3] text-[#a11d1d]",
  cap: "bg-[#e2f2e8] text-[#155f34]",
  net: "bg-[#e2f2e8] text-[#155f34]",
  auto: "bg-[#e8eaee] text-[#2b3038]",
  model: "bg-[#ece6f8] text-[#4b2a99]",
};

// Notifications that pile up beside the copy, and words flying into the notch.
export function Cards({
  state,
  tags,
}: {
  state: ShowState;
  tags: Dictionary["show"]["tags"];
}) {
  return (
    <div aria-hidden="true" className="pointer-events-none absolute inset-0">
      {state.cards.map((card) => {
        const slot = SLOTS[card.slot];
        return (
          <div
            key={card.id}
            className={cn([
              "bg-card shadow-card wide:block absolute hidden w-[clamp(190px,15vw,230px)] rounded-xl border px-[13px] py-[11px] text-[13px] leading-[1.38]",
              "transition-[border-color,box-shadow] duration-500",
              card.leaving ? "animate-card-out" : "animate-card-in",
              card.hot ? "border-mic ring-mic/20 ring-[3px]" : "border-line",
            ])}
            style={
              {
                left: `${slot.x}%`,
                top: slot.y,
                "--r": `${card.tilt}deg`,
                "--dx": `${slot.side * 220}px`,
              } as CSSProperties
            }
          >
            <span
              className={cn([
                "mb-1.5 inline-block rounded px-1.5 py-1 font-mono text-[10px] leading-none font-semibold tracking-[0.08em] uppercase",
                TAGS[card.kind],
              ])}
            >
              {tags[card.kind]}
            </span>
            <b className="block">{card.head}</b>
            <p className="text-fg2">{card.body}</p>
          </div>
        );
      })}
      {state.flies.map((fly) => (
        <span
          key={fly.id}
          className="animate-fly font-display text-mic absolute text-[22px] leading-none font-extrabold whitespace-nowrap"
          style={
            {
              "--sx": `${fly.x}%`,
              "--sy": `${fly.y}px`,
              animationDelay: `${fly.delay}s`,
            } as CSSProperties
          }
        >
          {fly.word}
        </span>
      ))}
    </div>
  );
}

export function ContactSheet({
  count,
  label,
}: {
  count: number;
  label: string;
}) {
  if (!count) return null;
  return (
    <div
      aria-hidden="true"
      className="animate-slide-in border-line bg-card shadow-card max-tab:hidden wide:w-[min(400px,30%)] absolute bottom-7 left-[2.5%] flex w-[min(360px,40%)] flex-col gap-2 rounded-xl border px-3.5 py-3"
    >
      <span className="text-muted font-mono text-xs font-semibold tracking-[0.12em] uppercase">
        {label}
      </span>
      <div className="flex gap-2">
        {[1, 2, 3].slice(0, count).map((n) => (
          <div
            key={n}
            className="animate-pop border-ink bg-bg relative flex h-[70px] flex-1 items-end rounded-md border-[1.5px] px-[7px] py-1"
          >
            <i className="bg-line absolute inset-x-2 top-2.5 h-1.5 rounded-sm" />
            <i className="bg-line absolute top-[22px] right-[40%] left-2 h-1.5 rounded-sm" />
            <span className="font-hand text-ink text-[22px] font-bold">
              {n}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

const TONES = {
  cmd: "text-amber",
  dim: "text-term-dim",
  ok: "text-[#6fdd8b]",
  plain: "",
};

export function AgentTerminal({ state }: { state: ShowState }) {
  if (!state.term) return null;
  return (
    <div
      aria-hidden="true"
      className={cn([
        "animate-slide-in bg-term text-term-fg absolute right-[2.5%] bottom-7 w-[min(440px,92%)] overflow-hidden rounded-xl shadow-[0_20px_40px_rgba(0,0,0,0.3)]",
        "max-tab:relative max-tab:bottom-auto max-tab:right-auto max-tab:mx-auto max-tab:mt-5",
      ])}
    >
      <div className="text-term-dim flex justify-between gap-2.5 border-b border-[#2a2e35] px-3.5 py-2 font-mono text-xs">
        <span>claude code · ~/librelog-site</span>
        <span className="text-[#6fdd8b]">{state.badge}</span>
      </div>
      <div className="min-h-[130px] px-3.5 py-3 font-mono text-[13px] leading-[1.6] whitespace-pre-wrap">
        {state.termLines.map((line, i) => (
          <div key={i} className={TONES[line.tone]}>
            {line.text}
          </div>
        ))}
        <div>{state.agent}</div>
      </div>
    </div>
  );
}
