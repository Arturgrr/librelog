import "./globals.css";

import type { Metadata } from "next";
import Link from "next/link";

import { cn } from "@anlg/utils";

import { button, eyebrow, h2, hand } from "~/components/styles";
import { fontVariables } from "~/lib/fonts";

export const metadata: Metadata = {
  title: "Librelog · 404",
  robots: { index: false },
};

export default function GlobalNotFound() {
  return (
    <html lang="en" className={fontVariables}>
      <body>
        <main className="dots bg-bg min-h-svh">
          <div className="mx-auto flex max-w-[980px] flex-col items-center gap-5 px-6 pt-40 text-center">
            <p className={eyebrow}>404</p>
            <p className={h2}>This page was never synced. Nothing here is.</p>
            <p lang="pt-BR" className={cn([hand, "text-3xl"])}>
              Esta página não foi sincronizada. Aliás, nada aqui é.
            </p>
            <div className="flex flex-wrap justify-center gap-3.5">
              <Link
                href="/"
                className={cn([button, "bg-btn text-btn-fg hover:text-btn-fg"])}
              >
                Librelog
              </Link>
              <Link
                href="/pt/"
                lang="pt-BR"
                className={cn([button, "border-fg text-fg border-[1.5px]"])}
              >
                Em português
              </Link>
            </div>
          </div>
        </main>
      </body>
    </html>
  );
}
