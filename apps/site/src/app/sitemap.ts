import type { MetadataRoute } from "next";

import { SITE_URL } from "~/lib/site";

export const dynamic = "force-static";

export default function sitemap(): MetadataRoute.Sitemap {
  return [
    { url: SITE_URL, alternates: { languages: { "pt-BR": `${SITE_URL}pt/` } } },
    { url: `${SITE_URL}pt/`, alternates: { languages: { en: SITE_URL } } },
  ];
}
