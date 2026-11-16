"use client";

import { useEffect, useState } from "react";

import { cn } from "@anlg/utils";

import { chip } from "./styles";

import type { Dictionary } from "~/i18n/en";
import { storage } from "~/lib/sound";

export function CopyButton({
  text,
  label,
  done,
}: {
  text: string;
  label: string;
  done: string;
}) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className={chip}
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(text);
          setCopied(true);
          setTimeout(() => setCopied(false), 2000);
        } catch {}
      }}
    >
      {copied ? done : label}
    </button>
  );
}

// A joke banner: the site has no cookies to ask about.
export function CookieNotice({ t }: { t: Dictionary["cookie"] }) {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    setOpen(storage.get("librelog-cookie") !== "dismissed");
  }, []);

  if (!open) return null;
  return (
    <div className="bg-btn text-btn-fg flex max-w-[560px] flex-wrap items-center gap-4 self-start rounded-xl px-[18px] py-3.5">
      <span className="min-w-[220px] flex-1 text-[15px]">{t.text}</span>
      <button
        type="button"
        onClick={() => {
          storage.set("librelog-cookie", "dismissed");
          setOpen(false);
        }}
        className={cn([chip, "border-btn-fg text-btn-fg bg-transparent"])}
      >
        {t.button}
      </button>
    </div>
  );
}

export function Elapsed({
  template,
  fallback,
}: {
  template: string;
  fallback: string;
}) {
  const [seconds, setSeconds] = useState<number | null>(null);

  useEffect(() => {
    const began = Date.now();
    const tick = () => setSeconds(Math.floor((Date.now() - began) / 1000));
    tick();
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  }, []);

  if (seconds === null) return <>{fallback}</>;
  return (
    <>
      {template.replace(
        "{t}",
        `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`,
      )}
    </>
  );
}
