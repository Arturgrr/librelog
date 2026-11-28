export const CHANGELOG_SOURCE_LOCALE = "en";

// "pt-BR" -> ["pt-BR", "pt", "en"]. Invalid tags fall back to English so a
// locale can never point outside the content directory.
export function changelogLocaleChain(locale: string | null | undefined) {
  let tag: string;
  try {
    tag = new Intl.Locale(locale ?? "").toString();
  } catch {
    return [CHANGELOG_SOURCE_LOCALE];
  }

  const chain: string[] = [];
  const subtags = tag.split("-");
  for (let length = subtags.length; length > 0; length--) {
    const candidate = subtags.slice(0, length).join("-");
    if (candidate === CHANGELOG_SOURCE_LOCALE) break;
    chain.push(candidate);
  }
  chain.push(CHANGELOG_SOURCE_LOCALE);
  return chain;
}

// English lives at `<version>.md`; a translation at `<locale>/<version>.md`.
export function changelogPath(version: string, locale: string) {
  return locale === CHANGELOG_SOURCE_LOCALE
    ? `${version}.md`
    : `${locale}/${version}.md`;
}

export function findLocalizedChangelog<T>(
  files: Partial<Record<string, T>>,
  version: string,
  locale: string | null | undefined,
): { locale: string; file: T } | null {
  for (const candidate of changelogLocaleChain(locale)) {
    const file = files[changelogPath(version, candidate)];
    if (file !== undefined) return { locale: candidate, file };
  }
  return null;
}

// Translations whose English source is missing; each one should be removed or
// renamed to match a released version.
export function orphanChangelogTranslations(paths: Iterable<string>) {
  const all = new Set(paths);
  return [...all].filter((path) => {
    const match = path.match(/^[^/]+\/([^/]+\.md)$/);
    return match !== null && !all.has(match[1]);
  });
}
