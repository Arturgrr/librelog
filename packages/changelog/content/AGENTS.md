# Instruction

- Keep only user-facing changes. No internal changes or infra updates.
- Maintain desktop notes in `desktop/<version>.md`, where `<version>` matches `apps/desktop/package.json` exactly, beta suffix included (`0.1.0-beta.2.md`). The app shows that file as "What's new", and the release workflow refuses to publish a version without one and copies it into the GitHub release.
- Each changelog must include `date` and `summary` frontmatter. Keep `summary` to one concise, plain-text, user-facing sentence with no markdown or custom tags.

```md
---
date: "YYYY-MM-DD"
summary: "One concise, user-facing sentence for the changelog index preview."
---
```

# Translations

- The English file is the source of truth. A translation lives at `desktop/<locale>/<version>.md`, where `<locale>` is the desktop app's Lingui locale id from `apps/desktop/lingui.config.ts` (Portuguese is `pt`, written in Brazilian Portuguese). Add one by copying the English file there and translating the body and `summary`; keep `date`, the headings' structure, links and custom tags.
- Use the product terms of that locale's catalog (`apps/desktop/src/i18n/locales/<locale>/messages.po`), such as "Ditado", "Anotar", "Configurações" and "Dicionário" in Portuguese.
- The app shows the translation for the UI language and falls back through the locale's parents to English (`pt-BR` → `pt` → `en`), silently. A translation without an English file for the same version fails the desktop changelog tests.
- The GitHub release keeps English first and adds each translation below it in a collapsed section (`pt` is labelled "Português (Brasil)").

# Custom Tags

The desktop changelog renderer (Streamdown-based) supports custom HTML tags beyond standard markdown.

## `<banner>`

Use for announcements, important notices, or highlights.

Attributes:

- `title` (optional): Bold heading text at the top of the banner.
- `variant` (optional): `"warning"` for amber/yellow style, `"info"` for blue style. Defaults to amber/info style.

```mdx
<banner title="Welcome to the beta">
Expect rough edges, and report anything that breaks from Help → Report Bug.
</banner>

<banner title="Breaking Change" variant="warning">
The old plugin format is no longer supported. Please update your plugins.
</banner>
```
