import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, extname, join, normalize, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { build } from "./build.mjs";

const here = dirname(fileURLToPath(import.meta.url));

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".png": "image/png",
  ".woff2": "font/woff2",
  ".txt": "text/plain; charset=utf-8",
  ".xml": "application/xml",
};

// Serves `out` on 127.0.0.1. With `rebuild`, every page request rebuilds the site first, so edits show on reload.
export function serve({ out, port, rebuild }) {
  const server = createServer(async (req, res) => {
    let path = decodeURIComponent(
      new URL(req.url, "http://localhost").pathname,
    );
    if (path.endsWith("/")) path += "index.html";
    try {
      if (rebuild && path.endsWith(".html")) await rebuild();
      const file = normalize(join(out, path));
      if (file !== out && !file.startsWith(out + sep))
        throw new Error("outside");
      const body = await readFile(file);
      res.writeHead(200, {
        "content-type": TYPES[extname(file)] ?? "application/octet-stream",
        "cache-control": "no-store",
      });
      res.end(body);
    } catch {
      const body = await readFile(join(out, "404.html")).catch(
        () => "Not found",
      );
      res.writeHead(404, { "content-type": TYPES[".html"] });
      res.end(body);
    }
  });
  return new Promise((resolve) =>
    server.listen(port, "127.0.0.1", () => resolve(server)),
  );
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const port = Number(process.env.PORT || 4321);
  const out = join(here, "dist");
  const siteUrl = `http://127.0.0.1:${port}/`;
  await build({ out, siteUrl });
  await serve({ out, port, rebuild: () => build({ out, siteUrl }) });
  console.log(`Librelog site on ${siteUrl} (Portuguese at ${siteUrl}pt/)`);
}
