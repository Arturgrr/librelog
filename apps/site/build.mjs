import { createHash } from "node:crypto";
import { cp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..", "..");

const LANGS = [
  { code: "en", file: "en.json", dir: "", base: "", ogLocale: "en_US" },
  {
    code: "pt-BR",
    file: "pt-BR.json",
    dir: "pt/",
    base: "../",
    ogLocale: "pt_BR",
  },
];

// Sends a Portuguese-speaking first visit to /pt/, once; a choice made with the language link wins.
const LANG_REDIRECT =
  'try{if(!localStorage.getItem("librelog-lang")&&/^pt\\b/i.test(navigator.language))location.replace("pt/")}catch(e){}';

const flatten = (obj, prefix = "", out = {}) => {
  for (const [key, value] of Object.entries(obj)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (value && typeof value === "object" && !Array.isArray(value))
      flatten(value, path, out);
    else out[path] = value;
  }
  return out;
};

const shape = (value) =>
  Array.isArray(value) ? `array(${value.length})` : typeof value;

const escapeAttr = (value) =>
  value
    .replaceAll("&", "&amp;")
    .replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");

const hash = (text) =>
  createHash("sha256").update(text).digest("hex").slice(0, 10);

function render(template, context, name) {
  const missing = new Set();
  const html = template.replace(/\{\{(@?)([\w.]+)\}\}/g, (_, attr, key) => {
    if (!(key in context)) {
      missing.add(key);
      return "";
    }
    const value = String(context[key]);
    return attr ? escapeAttr(value) : value;
  });
  if (missing.size)
    throw new Error(`${name}: no text for ${[...missing].join(", ")}`);
  return html;
}

function compareDictionaries(dicts) {
  const [first, ...rest] = dicts;
  const reference = flatten(first.strings);
  const problems = [];
  for (const other of rest) {
    const keys = flatten(other.strings);
    for (const key of Object.keys(reference)) {
      if (!(key in keys)) problems.push(`${other.file} lacks ${key}`);
      else if (shape(keys[key]) !== shape(reference[key]))
        problems.push(`${other.file} has a different shape for ${key}`);
    }
    for (const key of Object.keys(keys))
      if (!(key in reference)) problems.push(`${first.file} lacks ${key}`);
  }
  if (problems.length)
    throw new Error(`Translations differ:\n${problems.join("\n")}`);
}

export async function build({
  out = join(here, "dist"),
  siteUrl = process.env.SITE_URL || "https://arturgrr.github.io/librelog/",
} = {}) {
  if (!siteUrl.endsWith("/")) siteUrl += "/";
  const desktop = JSON.parse(
    await readFile(join(repo, "apps/desktop/package.json"), "utf8"),
  );
  const version = desktop.version.replace(/^(\d+\.\d+).*$/, "$1");

  const dicts = await Promise.all(
    LANGS.map(async (lang) => {
      const raw = await readFile(join(here, "src/i18n", lang.file), "utf8");
      return {
        ...lang,
        strings: JSON.parse(raw.replaceAll("{version}", version)),
      };
    }),
  );
  compareDictionaries(dicts);

  const [template, notFound, css, js] = await Promise.all(
    ["src/index.html", "src/404.html", "src/site.css", "src/site.js"].map(
      (file) => readFile(join(here, file), "utf8"),
    ),
  );
  const cssVersion = hash(css);
  const jsVersion = hash(js);

  await rm(out, { recursive: true, force: true });
  await mkdir(join(out, "assets"), { recursive: true });
  await cp(join(here, "public"), out, { recursive: true });
  await writeFile(join(out, "assets/site.css"), css);
  await writeFile(join(out, "assets/site.js"), js);

  for (const lang of dicts) {
    const other = dicts.find((d) => d !== lang);
    const { js: strings, ...text } = lang.strings;
    const context = {
      ...flatten(text),
      lang: lang.code,
      ogLocale: lang.ogLocale,
      base: lang.base,
      langDir: lang.dir,
      siteUrl,
      pageUrl: siteUrl + lang.dir,
      otherLang: other.code,
      otherLangHref: lang.base + other.dir,
      cssHref: `${lang.base}assets/site.css?v=${cssVersion}`,
      jsHref: `${lang.base}assets/site.js?v=${jsVersion}`,
      stringsJson: JSON.stringify(strings).replaceAll("<", "\\u003c"),
      langRedirect: lang.dir ? "" : LANG_REDIRECT,
    };
    await mkdir(join(out, lang.dir), { recursive: true });
    await writeFile(
      join(out, lang.dir, "index.html"),
      render(template, context, lang.file),
    );
  }

  const sitePath = new URL(siteUrl).pathname;
  await writeFile(
    join(out, "404.html"),
    render(
      notFound,
      { sitePath, cssHref: `${sitePath}assets/site.css?v=${cssVersion}` },
      "404.html",
    ),
  );
  await writeFile(
    join(out, "robots.txt"),
    `User-agent: *\nAllow: /\nSitemap: ${siteUrl}sitemap.xml\n`,
  );
  await writeFile(
    join(out, "sitemap.xml"),
    `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${LANGS.map(
      (l) => `  <url><loc>${siteUrl}${l.dir}</loc></url>`,
    ).join("\n")}\n</urlset>\n`,
  );
  return { out, version };
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { out, version } = await build();
  console.log(`Built the site for beta ${version} in ${out}`);
}
