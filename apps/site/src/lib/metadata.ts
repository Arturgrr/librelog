import type { Metadata, Viewport } from "next";

import { LANGS, type Lang, SITE_URL } from "./site";

export function metadataFor(lang: Lang): Metadata {
  const { dict, path } = LANGS[lang];
  const url = new URL(path.slice(1), SITE_URL).href;
  return {
    metadataBase: new URL(SITE_URL),
    title: dict.meta.title,
    description: dict.meta.description,
    alternates: {
      canonical: url,
      languages: {
        en: SITE_URL,
        "pt-BR": `${SITE_URL}pt/`,
        "x-default": SITE_URL,
      },
    },
    openGraph: {
      type: "website",
      siteName: "Librelog",
      title: dict.meta.ogTitle,
      description: dict.meta.description,
      url,
      locale: dict.meta.ogLocale,
      images: [
        {
          url: `${SITE_URL}og.png`,
          width: 1200,
          height: 630,
          alt: dict.meta.ogAlt,
        },
      ],
    },
    twitter: { card: "summary_large_image" },
  };
}

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  viewportFit: "cover",
  colorScheme: "light dark",
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#eef0f3" },
    { media: "(prefers-color-scheme: dark)", color: "#121418" },
  ],
};
