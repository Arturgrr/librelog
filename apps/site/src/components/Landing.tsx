import { Footer, Install, Letter, NotList, Privacy } from "./Sections";
import { Stage } from "./stage/Stage";
import { Story } from "./Story";
import { Zeros } from "./Zeros";

import { LANGS, type Lang, otherLang, withVersion } from "~/lib/site";

export function Landing({ lang }: { lang: Lang }) {
  const t = LANGS[lang].dict;
  const other = LANGS[otherLang(lang)];
  return (
    <>
      <a
        href="#historia"
        className="bg-btn text-btn-fg absolute -top-16 left-4 rounded-lg px-3.5 py-2.5 no-underline focus:top-4"
      >
        {t.a11y.skip}
      </a>
      <main>
        <Stage
          t={{
            a11y: t.a11y,
            nav: t.nav,
            notch: t.notch,
            hero: t.hero,
            ui: t.ui,
            show: t.show,
            console: t.console,
          }}
          eyebrowText={withVersion(t.hero.eyebrow)}
          other={{ href: other.path, lang: other.code }}
        />
        <Story t={t.story} />
        <Zeros t={t.zeros} />
        <Privacy t={t.net} />
        <Letter t={t.letter} />
        <NotList t={t.not} />
        <Install
          t={t.install}
          notMac={t.hero.notMac}
          ui={t.ui}
          cookie={t.cookie}
        />
      </main>
      <Footer t={t.footer} />
    </>
  );
}
