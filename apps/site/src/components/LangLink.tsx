"use client";

import Link from "next/link";

import { storage } from "~/lib/sound";

export function LangLink({
  href,
  lang,
  label,
  className,
}: {
  href: string;
  lang: string;
  label: string;
  className?: string;
}) {
  return (
    <Link
      href={href}
      hrefLang={lang}
      lang={lang}
      className={className}
      onClick={() => storage.set("librelog-lang", lang)}
    >
      {label}
    </Link>
  );
}
