import assert from "node:assert/strict";
import { test } from "node:test";

import {
  changelogLocaleChain,
  findLocalizedChangelog,
  orphanChangelogTranslations,
} from "./locale.ts";

test("a locale falls back through its parents to English", () => {
  assert.deepEqual(changelogLocaleChain("pt-BR"), ["pt-BR", "pt", "en"]);
  assert.deepEqual(changelogLocaleChain("zh-Hant-TW"), [
    "zh-Hant-TW",
    "zh-Hant",
    "zh",
    "en",
  ]);
  assert.deepEqual(changelogLocaleChain("en-GB"), ["en-GB", "en"]);
  for (const locale of ["", null, undefined, "../main", "pt_BR"]) {
    assert.deepEqual(changelogLocaleChain(locale), ["en"]);
  }
});

test("a translation is picked over English, and English when it is missing", () => {
  const files = {
    "0.10.0-beta.1.md": "english",
    "pt/0.10.0-beta.1.md": "portuguese",
    "0.9.0-beta.1.md": "older english",
  };

  assert.deepEqual(findLocalizedChangelog(files, "0.10.0-beta.1", "pt-BR"), {
    locale: "pt",
    file: "portuguese",
  });
  assert.deepEqual(findLocalizedChangelog(files, "0.9.0-beta.1", "pt"), {
    locale: "en",
    file: "older english",
  });
  assert.deepEqual(findLocalizedChangelog(files, "0.10.0-beta.1", "ja"), {
    locale: "en",
    file: "english",
  });
  assert.equal(findLocalizedChangelog(files, "0.11.0", "pt"), null);
});

test("translations without an English source are reported", () => {
  assert.deepEqual(
    orphanChangelogTranslations([
      "0.10.0-beta.1.md",
      "pt/0.10.0-beta.1.md",
      "pt/0.10.1.md",
    ]),
    ["pt/0.10.1.md"],
  );
});
