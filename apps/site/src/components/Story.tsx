"use client";

import { useEffect, useRef, useState } from "react";

import { cn } from "@anlg/utils";

import { CaptureScene, MeetScene, SayScene, ShowScene } from "./StoryScenes";
import { eyebrow, h2, h3, hand, wrap } from "./styles";

import type { Dictionary } from "~/i18n/en";

// The window follows whichever chapter crosses the middle of the viewport.
export function Story({ t }: { t: Dictionary["story"] }) {
  const [active, setActive] = useState(0);
  const chapters = useRef<(HTMLElement | null)[]>([]);

  useEffect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting)
            setActive(chapters.current.indexOf(entry.target as HTMLElement));
        }
      },
      { rootMargin: "-45% 0px -45% 0px" },
    );
    chapters.current.forEach((el) => el && observer.observe(el));
    return () => observer.disconnect();
  }, []);

  const pick = (i: number) => {
    setActive(i);
    if (matchMedia("(min-width: 61.25rem)").matches) {
      chapters.current[i]?.scrollIntoView({
        behavior: matchMedia("(prefers-reduced-motion: reduce)").matches
          ? "auto"
          : "smooth",
        block: "center",
      });
    }
  };

  return (
    <section id="historia" aria-labelledby="story-title" className="py-[110px]">
      <div className={wrap}>
        <div className="flex max-w-[62ch] flex-col gap-3">
          <p className={eyebrow}>{t.eyebrow}</p>
          <h2 id="story-title" className={h2}>
            {t.title}
          </h2>
          <p className="text-muted">{t.intro}</p>
        </div>
        <div className="mt-12 flex flex-wrap items-start gap-12">
          <div
            aria-hidden="true"
            className="border-fg bg-card sticky top-6 flex min-w-0 flex-[1_1_520px] flex-col gap-3 rounded-2xl border-[1.5px] p-3 shadow-[0_24px_60px_rgba(22,24,29,0.1)] max-[61.24rem]:relative max-[61.24rem]:top-0"
          >
            <div className="border-line2 bg-surface relative h-[440px] overflow-hidden rounded-xl border">
              {active === 0 && <SayScene t={t.s1} />}
              {active === 1 && <MeetScene t={t.s2} />}
              {active === 2 && <ShowScene t={t.s3} />}
              {active === 3 && <CaptureScene t={t.s4} />}
            </div>
            <div
              role="tablist"
              aria-label={t.tabsLabel}
              className="flex flex-wrap gap-1.5"
            >
              {t.tabs.map((label, i) => (
                <button
                  key={label}
                  type="button"
                  role="tab"
                  aria-selected={active === i}
                  onClick={() => pick(i)}
                  className={cn([
                    "min-h-11 min-w-24 flex-1 cursor-pointer rounded-lg border-[1.5px] text-sm font-semibold",
                    active === i
                      ? "border-fg bg-fg text-bg"
                      : "border-line bg-card text-fg",
                  ])}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>
          <div className="flex min-w-0 flex-[1_1_380px] flex-col">
            {t.chapters.map((chapter, i) => (
              <article
                key={chapter.num}
                ref={(el) => {
                  chapters.current[i] = el;
                }}
                className={cn([
                  "flex min-h-[560px] flex-col justify-center gap-3.5 transition-opacity duration-500",
                  "max-[61.24rem]:min-h-0 max-[61.24rem]:py-7 max-[61.24rem]:opacity-100",
                  active === i ? "opacity-100" : "nojs:opacity-100 opacity-35",
                ])}
              >
                <p className={cn([hand, "text-[34px]"])}>{chapter.num}</p>
                <h3 className={h3}>{chapter.title}</h3>
                <p>{chapter.body}</p>
                <p className="text-muted text-sm">{chapter.limit}</p>
              </article>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
