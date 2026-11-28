import { readdirSync, readFileSync, watch } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { Plugin, ViteDevServer } from "vite";

const __dirname = fileURLToPath(new URL(".", import.meta.url));
const changelogDir = resolve(
  __dirname,
  "../../../packages/changelog/content/desktop",
);
const packageJsonPath = resolve(__dirname, "../package.json");

const VIRTUAL_ID = "virtual:changelog";
const RESOLVED_ID = "\0" + VIRTUAL_ID;

// Tauri reads the app version from the same package.json.
function getAppVersion(): string | null {
  try {
    return JSON.parse(readFileSync(packageJsonPath, "utf-8")).version ?? null;
  } catch {
    return null;
  }
}

function readLatestFiles(version: string): Record<string, string> {
  const paths = [`${version}.md`];
  try {
    for (const entry of readdirSync(changelogDir, { withFileTypes: true })) {
      if (entry.isDirectory()) paths.push(`${entry.name}/${version}.md`);
    }
  } catch {}

  const files: Record<string, string> = {};
  for (const path of paths) {
    try {
      files[path] = readFileSync(resolve(changelogDir, path), "utf-8");
    } catch {}
  }
  return files;
}

// The current version's notes are inlined in every language, keyed like
// `findLocalizedChangelog` expects.
function buildModule(): string {
  const latest = getAppVersion();

  return [
    `export const latestVersion = ${JSON.stringify(latest)};`,
    `export const latestFiles = ${JSON.stringify(latest ? readLatestFiles(latest) : {})};`,
  ].join("\n");
}

export function changelog(): Plugin {
  return {
    name: "changelog",
    resolveId(id) {
      if (id === VIRTUAL_ID) return RESOLVED_ID;
    },
    load(id) {
      if (id === RESOLVED_ID) return buildModule();
    },
    configureServer(server: ViteDevServer) {
      if (process.env.NODE_ENV === "test" || process.env.VITEST) {
        return;
      }

      try {
        watch(resolve(changelogDir, "../.."), { recursive: true }, () => {
          const mod = server.moduleGraph.getModuleById(RESOLVED_ID);
          if (mod) {
            server.moduleGraph.invalidateModule(mod);
            server.ws.send({ type: "full-reload" });
          }
        });
      } catch {}
    },
  };
}
