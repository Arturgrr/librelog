# Librelog website

The landing page, built with Next.js, React and Tailwind CSS and exported as a static site: English at `/`, Portuguese at `/pt/`.

```bash
pnpm dev:site                  # http://127.0.0.1:4321
pnpm build:site                # static export in apps/site/out
pnpm -F @anlg/site start       # serves out/ under the base path, like GitHub Pages
pnpm -F @anlg/site typecheck
```

The build reads the version from `apps/desktop/package.json` and the address from `SITE_URL` (default `https://arturgrr.github.io/librelog/`). Its path becomes the Next.js `basePath`, and the canonical links, share image and sitemap use the full address.

## Publishing

`.github/workflows/site.yaml` typechecks and builds every pull request that touches the site and publishes `main` to GitHub Pages. Before the first deploy, choose **GitHub Actions** as the source in the repository's **Settings → Pages**. For a custom domain, set it there and add a repository variable `SITE_URL` with the full address, ending in `/`.

## Share image and video

`scripts/capture.mjs` uses Playwright's Chromium (not a dependency of this repository) against the static export to render `public/og.png` and record the hero show. MP4 output needs ffmpeg.

```bash
pnpm build:site
node apps/site/scripts/capture.mjs --og
node apps/site/scripts/capture.mjs --video show.mp4 --pt
```

The four scenes under "Say it. Meet. Show it. Capture it." are drawn with CSS in `src/components/StoryScenes.tsx`. When real recordings of the app exist, replace a scene with a muted, looping `<video>` that uses the scene as its poster.
