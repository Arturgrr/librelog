import { describe, expect, it } from "vitest";

import { orphanChangelogTranslations, parseFrontmatter } from "@anlg/changelog";

import { bundledChangelogs, loadBundledChangelog } from "./source";

describe("bundled changelog", () => {
  it("reads release notes from the bundled changelog package", async () => {
    const found = await loadBundledChangelog("0.1.0-beta.1", "en");
    expect(found?.locale).toBe("en");
    expect(found?.raw).toContain("date:");
  });

  it("shows the translation for the UI language and English without one", async () => {
    expect((await loadBundledChangelog("0.10.0-beta.1", "pt"))?.locale).toBe(
      "pt",
    );
    expect((await loadBundledChangelog("0.1.0-beta.1", "pt"))?.locale).toBe(
      "en",
    );
  });

  it("returns nothing for unknown versions and path injection", async () => {
    for (const version of [
      "../../main",
      "0.1.0-beta.999",
      "1.4.24?query=1",
      "",
    ]) {
      await expect(loadBundledChangelog(version, "pt")).resolves.toBeNull();
    }
    await expect(
      loadBundledChangelog("0.1.0-beta.1", "../.."),
    ).resolves.toMatchObject({ locale: "en" });
  });

  it("only has translations of released notes, with the same date and a summary", async () => {
    const paths = Object.keys(bundledChangelogs).filter(
      (path) => !path.endsWith("AGENTS.md"),
    );
    expect(orphanChangelogTranslations(paths)).toEqual([]);

    for (const path of paths.filter((path) => path.includes("/"))) {
      const translated = parseFrontmatter(await bundledChangelogs[path]!());
      const english = parseFrontmatter(
        await bundledChangelogs[path.split("/")[1]!]!(),
      );
      expect(translated.date, path).toBe(english.date);
      expect(translated.summary, path).toBeTruthy();
      expect(translated.body, path).not.toBe("");
    }
  });
});
