# Website

The public landing page: a static site in English (`/`) and Portuguese (`/pt/`), built by `build.mjs` with no dependencies and published to GitHub Pages by `.github/workflows/site.yaml`. `README.md` has the commands.

## Rules

- The page loads nothing from other servers: no analytics, CDNs, web fonts or embeds. Fonts are self-hosted in `public/fonts`, and any new third-party file needs its license next to it and an entry in `THIRD_PARTY_NOTICES.md`. The copy says this, so keep it true.
- Every claim about the app must match `docs/` and `README.md` for the version in `apps/desktop/package.json`. Mark experimental or off-by-default features as such, and never show a recording of a feature nobody has run on a Mac.
- Copy lives in `src/i18n/en.json` and `src/i18n/pt-BR.json`, which must have the same keys; the build fails otherwise. `{{key}}` in `src/index.html` inserts a value as HTML, `{{@key}}` escapes it for an attribute. The `js` object is what `src/site.js` reads.
- The hero show is driven by `data-*` flags on `.stage` that `src/site.js` sets on a timeline; `src/site.css` decides what each flag shows. The page must read correctly without JavaScript and with reduced motion, where it shows the finished state.
- Keep sounds synthesized and silent until the visitor clicks.
