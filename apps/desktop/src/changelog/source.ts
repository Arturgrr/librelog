import { findLocalizedChangelog } from "@anlg/changelog";

const CHANGELOG_DIR = "../../../../packages/changelog/content/desktop/";

const globbed = import.meta.glob<string>(
  [
    "../../../../packages/changelog/content/desktop/*.md",
    "../../../../packages/changelog/content/desktop/*/*.md",
  ],
  { query: "?raw", import: "default" },
);

// Keyed by path relative to content/desktop: `<version>.md` for English,
// `<locale>/<version>.md` for a translation.
export const bundledChangelogs = Object.fromEntries(
  Object.entries(globbed).map(([path, load]) => [
    path.slice(CHANGELOG_DIR.length),
    load,
  ]),
);

export async function loadBundledChangelog(
  version: string,
  locale: string,
): Promise<{ locale: string; raw: string } | null> {
  const found = findLocalizedChangelog(bundledChangelogs, version, locale);
  return found ? { locale: found.locale, raw: await found.file() } : null;
}
