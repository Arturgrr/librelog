"use client";

import { useEffect, useState } from "react";

import type { Dictionary } from "~/i18n/en";

// Downloads are for Apple-silicon Macs only; say so instead of offering a file that won't open.
export function PlatformNote({ t }: { t: Dictionary["hero"]["notMac"] }) {
  const [other, setOther] = useState(false);

  useEffect(() => {
    const ua = navigator.userAgent;
    setOther(
      !/Macintosh/.test(ua) && /Windows|Android|CrOS|iPhone|Linux/i.test(ua),
    );
  }, []);

  if (!other) return null;
  return (
    <p className="bg-ink-wash text-fg2 max-w-[52ch] rounded-[10px] px-3.5 py-2.5 text-[15px]">
      {t.before}
      <a
        href="https://github.com/Arturgrr/librelog#readme"
        className="underline"
      >
        {t.link}
      </a>
      {t.after}
    </p>
  );
}
