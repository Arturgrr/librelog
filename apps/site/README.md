# Librelog website

A static page, in English at `/` and Portuguese at `/pt/`. It needs only Node.

```bash
pnpm dev:site      # http://127.0.0.1:4321, rebuilt on every reload
pnpm build:site    # writes apps/site/dist
```

`build.mjs` reads the version from `apps/desktop/package.json` and the page address from `SITE_URL` (default `https://arturgrr.github.io/librelog/`), which the canonical link, share image and sitemap use.

## Publishing

`.github/workflows/site.yaml` builds every pull request that touches the site and publishes `main` to GitHub Pages. Before the first deploy, choose **GitHub Actions** as the source in the repository's **Settings → Pages**. For a custom domain, set it there and add a repository variable `SITE_URL` with the full address, ending in `/`.

## Share image and video

`capture.mjs` uses Playwright's Chromium (not a dependency of this repository) to render `public/og.png` and to record the hero show. MP4 output needs ffmpeg.

```bash
node apps/site/capture.mjs --og
node apps/site/capture.mjs --video show.mp4 --pt
```

The four scenes under "Say it. Meet. Show it. Capture it." are drawn in CSS. When real recordings of the app exist, replace each `.scene` with a muted, looping `<video>` that has the scene as its poster.
