import { cn } from "@anlg/utils";

import type { ShowState } from "./useShow";

import type { Dictionary } from "~/i18n/en";

export function Bars() {
  return (
    <span className="flex h-4 items-center gap-[3px]">
      {[0, 0.12, 0.3, 0.18, 0.42].map((delay) => (
        <b
          key={delay}
          className="animate-level bg-mic-bright block h-[30%] w-[3px] rounded-sm"
          style={{ animationDelay: `${delay}s` }}
        />
      ))}
    </span>
  );
}

export function Spinner({ className }: { className?: string }) {
  return (
    <span
      className={cn([
        "border-notch-fg size-[13px] animate-spin rounded-full border-[2.5px] border-t-transparent",
        className,
      ])}
    />
  );
}

export function Notch({
  state,
  t,
}: {
  state: ShowState["notch"];
  t: Dictionary["notch"];
}) {
  return (
    <div
      aria-hidden="true"
      className={cn([
        "absolute top-0 left-1/2 flex h-[34px] -translate-x-1/2 items-center justify-center gap-2.5 overflow-hidden whitespace-nowrap",
        "bg-notch text-notch-fg rounded-b-2xl font-mono text-xs font-semibold dark:ring-1 dark:ring-[#2f343c]",
        "transition-[width] duration-500 ease-[cubic-bezier(0.2,1.3,0.3,1)]",
        state === "idle" && "max-tab:h-[30px] max-tab:w-24 w-[120px]",
        state === "rec" && "w-[250px]",
        state !== "idle" && state !== "rec" && "w-[300px]",
      ])}
    >
      {state === "idle" && (
        <span className="size-[7px] rounded-full bg-[#3d434e]" />
      )}
      {state === "listen" && (
        <>
          <span className="bg-mic-bright size-2.5 rounded-full" />
          <Bars />
          <span className="text-[#ffb366]">{t.listen}</span>
        </>
      )}
      {state === "work" && (
        <>
          <Spinner />
          <span>{t.work}</span>
        </>
      )}
      {state === "done" && <span className="text-[#4fcf85]">✓ {t.done}</span>}
      {state === "rec" && (
        <>
          <span className="animate-blink size-2.5 rounded-full bg-[#ff5d52]" />
          <span className="text-[#ff8a80]">{t.rec}</span>
        </>
      )}
    </div>
  );
}
