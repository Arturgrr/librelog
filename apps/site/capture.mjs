// Renders the share image and records the hero show with a headless Chromium.
// Needs Playwright with Chromium (not a dependency of this repo) and, for MP4, ffmpeg.
//
//   node apps/site/capture.mjs --og                 writes apps/site/public/og.png
//   node apps/site/capture.mjs --video show.mp4     records the show (no sound)
import { execFileSync } from "node:child_process";
import { mkdtemp, readdir, rename, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "./build.mjs";
import { serve } from "./dev.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const { chromium } = createRequire(import.meta.url)("playwright");
const MAC_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

const args = process.argv.slice(2);
const wantsOg = args.includes("--og");
const videoIndex = args.indexOf("--video");
const videoPath =
  videoIndex >= 0 ? resolve(args[videoIndex + 1] ?? "show.mp4") : null;
if (!wantsOg && !videoPath) {
  console.error("Pass --og, --video <file>, or both.");
  process.exit(1);
}

const port = Number(process.env.PORT || 4399);
const work = await mkdtemp(join(tmpdir(), "librelog-site-"));
const out = join(work, "dist");
await build({ out, siteUrl: `http://127.0.0.1:${port}/` });
const server = await serve({ out, port });
const browser = await chromium.launch();

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
    await page.goto(`http://127.0.0.1:${port}/`);
    await page.evaluate(() =>
      document.querySelector('[data-action="skip"]').click(),
    );
    await page.waitForTimeout(2000);
    await page.evaluate(() => scrollTo(0, 0));
    await page.addStyleTag({
      content: ".sound-toggle, .yourturn, .controls { visibility: hidden; }",
    });
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(here, "public/og.png") });
    console.log("Wrote apps/site/public/og.png");
  }

  if (videoPath) {
    const dir = join(work, "video");
    const context = await browser.newContext({
      viewport: { width: 1440, height: 810 },
      userAgent: MAC_UA,
      locale: args.includes("--pt") ? "pt-BR" : "en-US",
      recordVideo: { dir, size: { width: 1440, height: 810 } },
    });
    const page = await context.newPage();
    await page.goto(
      `http://127.0.0.1:${port}/${args.includes("--pt") ? "pt/" : ""}`,
    );
    await page.waitForTimeout(1500);
    await page.evaluate(() => document.querySelector(".marker").click());
    await page.waitForTimeout(39000);
    await context.close();
    const [webm] = await readdir(dir);
    if (extname(videoPath) === ".webm") {
      await rename(join(dir, webm), videoPath);
    } else {
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
