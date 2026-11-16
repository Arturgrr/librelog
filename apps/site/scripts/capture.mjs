// Renders the share image and records the hero show with Playwright's Chromium (not a dependency of this
// repository); MP4 output needs ffmpeg. Build first: pnpm -F @anlg/site build.
//
//   node scripts/capture.mjs --og                     writes public/og.png
//   node scripts/capture.mjs --video show.mp4 [--pt]  records the show, without sound
import { execFileSync } from "node:child_process";
import { mkdtemp, readdir, rename, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { serve } from "./serve.mjs";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const { chromium } = createRequire(import.meta.url)("playwright");
const MAC_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

const args = process.argv.slice(2);
const wantsOg = args.includes("--og");
const videoIndex = args.indexOf("--video");
const videoPath =
  videoIndex >= 0 ? resolve(args[videoIndex + 1] ?? "show.mp4") : null;
const portuguese = args.includes("--pt");
if (!wantsOg && !videoPath) {
  console.error("Pass --og, --video <file>, or both.");
  process.exit(1);
}

const port = Number(process.env.PORT || 4399);
const basePath = new URL(
  process.env.SITE_URL || "https://arturgrr.github.io/librelog/",
).pathname.replace(/\/$/, "");
const root = `http://127.0.0.1:${port}${basePath}/`;
const server = await serve({ port, basePath });
const browser = await chromium.launch();
const work = await mkdtemp(join(tmpdir(), "librelog-site-"));

try {
  if (wantsOg) {
    // 1440 CSS px wide, scaled to the 1200 × 630 share size.
    const page = await browser.newPage({
      viewport: { width: 1440, height: 756 },
      deviceScaleFactor: 1200 / 1440,
      userAgent: MAC_UA,
      locale: "en-US",
      colorScheme: "light",
    });
    await page.goto(root);
    await page.locator("[data-skip]").click();
    await page.waitForTimeout(2000);
    await page.evaluate(() => scrollTo(0, 0));
    await page.addStyleTag({
      content: "[data-share-hide], [data-share-hide] * { visibility: hidden; }",
    });
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(site, "public/og.png") });
    console.log("Wrote public/og.png");
  }

  if (videoPath) {
    const dir = join(work, "video");
    const context = await browser.newContext({
      viewport: { width: 1440, height: 810 },
      userAgent: MAC_UA,
      locale: portuguese ? "pt-BR" : "en-US",
      recordVideo: { dir, size: { width: 1440, height: 810 } },
    });
    const page = await context.newPage();
    await page.goto(portuguese ? `${root}pt/` : root);
    await page.waitForTimeout(1500);
    await page.locator("[data-start]").click();
    await page.waitForTimeout(39000);
    await context.close();
    const [webm] = await readdir(dir);
    if (extname(videoPath) === ".webm")
      await rename(join(dir, webm), videoPath);
    else {
      execFileSync("ffmpeg", [
        "-y",
        "-loglevel",
        "error",
        "-i",
        join(dir, webm),
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-crf",
        "20",
        "-movflags",
        "+faststart",
        videoPath,
      ]);
    }
    console.log(`Wrote ${videoPath}`);
  }
} finally {
  await browser.close();
  server.close();
  await rm(work, { recursive: true, force: true });
}
