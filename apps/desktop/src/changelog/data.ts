import { useLingui } from "@lingui/react";
import { useQuery } from "@tanstack/react-query";
// @ts-ignore virtual module provided by the Vite changelog plugin
import { latestFiles, latestVersion } from "virtual:changelog";

import { findLocalizedChangelog, processContent } from "@anlg/changelog";

import { loadBundledChangelog } from "./source";

export function useChangelogContent(version: string) {
  const { i18n } = useLingui();
  const locale = i18n.locale;
  const query = useQuery({
    queryKey: ["changelog", version, locale],
    queryFn: async () => {
      if (version === latestVersion) {
        const latest = findLocalizedChangelog<string>(
          latestFiles,
          version,
          locale,
        );
        if (latest) return latest.file;
      }
      return (await loadBundledChangelog(version, locale))?.raw ?? null;
    },
    staleTime: Infinity,
  });
  const parsed = query.data ? processContent(query.data) : null;
  return {
    content: parsed?.content ?? null,
    date: parsed?.date ?? null,
    loading: query.isPending,
  };
}
