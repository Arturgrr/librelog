import "../globals.css";

import type { ReactNode } from "react";

import { cn } from "@anlg/utils";

import { fontVariables } from "~/lib/fonts";
import { metadataFor } from "~/lib/metadata";
import { LANG_REDIRECT } from "~/lib/site";

export { viewport } from "~/lib/metadata";
export const metadata = metadataFor("en");

export default function EnglishLayout({ children }: { children: ReactNode }) {
  return (
    <html
      lang="en"
      className={cn(["no-js", fontVariables])}
      suppressHydrationWarning
    >
      <head>
        <script
          dangerouslySetInnerHTML={{
            __html: `document.documentElement.classList.remove("no-js");${LANG_REDIRECT}`,
          }}
        />
      </head>
      <body>{children}</body>
    </html>
  );
}
