import "../../globals.css";

import type { ReactNode } from "react";

import { cn } from "@anlg/utils";

import { fontVariables } from "~/lib/fonts";
import { metadataFor } from "~/lib/metadata";

export { viewport } from "~/lib/metadata";
export const metadata = metadataFor("pt");

export default function PortugueseLayout({
  children,
}: {
  children: ReactNode;
}) {
  return (
    <html
      lang="pt-BR"
      className={cn(["no-js", fontVariables])}
      suppressHydrationWarning
    >
      <head>
        <script
          dangerouslySetInnerHTML={{
            __html: 'document.documentElement.classList.remove("no-js")',
          }}
        />
      </head>
      <body>{children}</body>
    </html>
  );
}
