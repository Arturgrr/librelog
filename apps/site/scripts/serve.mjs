// Serves the static export in out/ the way GitHub Pages does: under the site's base path, with 404.html
// for anything missing. Run `pnpm -F @anlg/site build` first.
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, extname, join, normalize, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const out = join(dirname(fileURLToPath(import.meta.url)), "..", "out");

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".txt": "text/plain; charset=utf-8",
  ".xml": "application/xml",
  ".png": "image/png",
  ".woff2": "font/woff2",
};

export function serve({ port, basePath }) {
  const server = createServer(async (req, res) => {
    let path = decodeURIComponent(
      new URL(req.url, "http://localhost").pathname,
    );
    try {
      if (basePath && !path.startsWith(`${basePath}/`))
        throw new Error("outside the base path");
      path = path.slice(basePath.length);
      if (path.endsWith("/")) path += "index.html";
      const file = normalize(join(out, path));
      if (!file.startsWith(out + sep)) throw new Error("outside the export");
      const body = await readFile(file);
      res.writeHead(200, {
        "content-type": TYPES[extname(file)] ?? "application/octet-stream",
      });
      res.end(body);
    } catch {
      res.writeHead(404, { "content-type": TYPES[".html"] });
      res.end(await readFile(join(out, "404.html")).catch(() => "Not found"));
    }
  });
  return new Promise((resolve) =>
    server.listen(port, "127.0.0.1", () => resolve(server)),
  );
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const port = Number(process.env.PORT || 4322);
  const basePath = new URL(
    process.env.SITE_URL || "https://arturgrr.github.io/librelog/",
  ).pathname.replace(/\/$/, "");
  await serve({ port, basePath });
  console.log(`Serving out/ on http://127.0.0.1:${port}${basePath}/`);
}
