# Website

The public landing page (`@anlg/site`): Next.js with the App Router, React and Tailwind CSS 4, exported as a static site in English (`/`) and Portuguese (`/pt/`) and published to GitHub Pages by `.github/workflows/site.yaml`. `README.md` has the commands.

## Rules

- The page loads nothing from other servers: no analytics, CDNs, remote fonts or embeds. Fonts are self-hosted with `next/font/local` from `src/fonts`; any new third-party file needs its license next to it and an entry in `THIRD_PARTY_NOTICES.md`. The copy says this, so keep it true. Builds run with `NEXT_TELEMETRY_DISABLED=1`.
- Every claim about the app must match `docs/` and `README.md` for the version in `apps/desktop/package.json`. Mark experimental or off-by-default features as such, and never show a recording of a feature nobody has run on a Mac.
- Copy lives in `src/i18n/en.ts` and `src/i18n/pt-BR.ts`; the Portuguese dictionary is typed as the English one, so `typecheck` fails on a missing or extra key. Components get the slice of the dictionary they need as props.
- Keep `output: "export"`: no server features (route handlers that read the request, cookies, rewrites, image optimization). Each language has its own root layout in a route group; `src/app/global-not-found.tsx` is the 404.
- The hero show is a timeline in `src/components/stage/useShow.ts` that sets state; `Stage.tsx` renders from that state. The page must read correctly before hydration, without JavaScript (the `nojs:` variant) and with reduced motion, where it shows the finished state.
- Animations are CSS keyframes in `src/app/globals.css`, exposed as `animate-*` utilities. Sounds are synthesized in `src/lib/sound.ts` and stay silent until the visitor clicks.
