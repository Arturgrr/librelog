"use client";

import Image from "next/image";
import Link from "next/link";
import { useEffect, useState } from "react";

import { cn } from "@anlg/utils";

import icon from "../../assets/icon-64.png";
import { InkCanvas } from "./InkCanvas";
import { Notch } from "./Notch";
import { AgentTerminal, Cards, ContactSheet } from "./ShowLayer";
import { useShow } from "./useShow";

import { LangLink } from "~/components/LangLink";
import { PlatformNote } from "~/components/PlatformNote";
import { button, chip, eyebrow, hand } from "~/components/styles";
import type { Dictionary } from "~/i18n/en";
import { LATEST_RELEASE, REPO } from "~/lib/links";
import { storage } from "~/lib/sound";

const sentences = (text: string) =>
  text.match(/[^.!?]+[.!?]*/g)?.map((s) => s.trim()) ?? [text];

function Pen({ className, duration }: { className: string; duration: string }) {
  return (
    <svg
      viewBox="0 0 48 48"
      aria-hidden="true"
      className={cn([
        "animate-pen text-fg pointer-events-none absolute size-[46px]",
        className,
      ])}
      style={{ "--dur": duration } as React.CSSProperties}
    >
      <g transform="rotate(35 24 24)">
        <rect x="19" y="2" width="10" height="34" rx="3" fill="currentColor" />
        <rect x="19" y="2" width="10" height="10" rx="3" fill="#c42323" />
        <path d="M19 36h10l-5 10z" fill="#c42323" />
      </g>
    </svg>
  );
}

export function Stage({
  t,
  eyebrowText,
  other,
}: {
  t: Pick<
    Dictionary,
    "a11y" | "nav" | "notch" | "hero" | "ui" | "show" | "console"
  >;
  eyebrowText: string;
  other: { href: string; lang: string };
}) {
  const [soundOn, setSoundOn] = useState(true);
  const [inking, setInking] = useState(false);
  const [canvasKey, setCanvasKey] = useState(0);
  const { state, show } = useShow(t.show, soundOn);
  const idle = state.phase === "idle";
  const done = state.phase === "done";

  useEffect(() => {
    setSoundOn(storage.get("librelog-sound") !== "off");
    console.info(t.console);
  }, [t.console]);

  // ⌥⇧S is Annotate's shortcut in the app; here it starts the show, then picks up the pen.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.altKey && e.shiftKey && e.code === "KeyS") {
        e.preventDefault();
        if (show.phase() === "idle") show.start();
        else if (show.phase() === "done") setInking((on) => !on);
      } else if (e.key === "Escape") {
        if (show.phase() === "run") show.finish();
        setInking(false);
      }
    };
    addEventListener("keydown", onKey);
    return () => removeEventListener("keydown", onKey);
  }, [show]);

  const scrollToStage = () => {
    if (scrollY > 8)
      scrollTo({
        top: 0,
        behavior: matchMedia("(prefers-reduced-motion: reduce)").matches
          ? "auto"
          : "smooth",
      });
  };
  const start = () => {
    setInking(false);
    scrollToStage();
    show.start();
  };
  const toggleSound = () => {
    storage.set("librelog-sound", soundOn ? "off" : "on");
    setSoundOn(!soundOn);
  };

  let word = 0;

  return (
    <section
      id="topo"
      aria-labelledby="hero-title"
      className="dots bg-bg max-tab:min-h-0 max-tab:pb-7 relative min-h-[max(880px,100svh)] overflow-hidden pb-16"
    >
      <header className="max-tab:pt-[46px] relative flex items-center justify-between gap-4 px-6 py-[18px]">
        <Link
          href="/"
          aria-label={t.a11y.home}
          className="font-display flex items-center gap-2.5 text-[21px] font-extrabold tracking-[-0.02em] no-underline"
        >
          <Image
            src={icon}
            width={32}
            height={32}
            alt=""
            className="rounded-lg"
          />
          librelog
        </Link>
        <nav
          aria-label={t.a11y.nav}
          className="max-tab:gap-3.5 flex items-center gap-[22px] text-[15px] font-semibold"
        >
          {!idle && (
            <button
              type="button"
              aria-pressed={soundOn}
              onClick={toggleSound}
              data-share-hide
              className={cn([chip, "min-h-9 px-3 text-[13px]"])}
            >
              {soundOn ? t.ui.soundOn : t.ui.soundOff}
            </button>
          )}
          <a href="#historia" className="max-tab:hidden no-underline">
            {t.nav.what}
          </a>
          <a href="#instalar" className="max-tab:hidden no-underline">
            {t.nav.install}
          </a>
          <a
            href={REPO}
            className={cn(["no-underline", !idle && "max-tab:hidden"])}
          >
            GitHub
          </a>
          <LangLink
            href={other.href}
            lang={other.lang}
            label={t.nav.otherLang}
            className="no-underline"
          />
          <span className="max-tab:hidden relative inline-flex">
            <span className="border-line2 text-muted rounded-md border-[1.5px] px-3.5 py-2">
              {t.nav.signIn}
            </span>
            {state.entrar && (
              <>
                <svg
                  viewBox="0 0 100 40"
                  preserveAspectRatio="none"
                  aria-hidden="true"
                  className="pointer-events-none absolute -inset-x-1.5 -inset-y-1 h-[calc(100%+8px)] w-[calc(100%+12px)] overflow-visible"
                >
                  <path
                    className="ink-path"
                    strokeWidth={5}
                    pathLength={1000}
                    d="M4 24C30 18 70 16 96 14"
                    vectorEffect="non-scaling-stroke"
                  />
                </svg>
                <span
                  className={cn([
                    hand,
                    "animate-pop absolute top-full -right-1.5 mt-0.5 -rotate-[5deg] text-[22px] whitespace-nowrap",
                  ])}
                >
                  {t.nav.noAccount}
                </span>
              </>
            )}
          </span>
        </nav>
      </header>

      <Notch state={state.notch} t={t.notch} />
      {state.caption && (
        <p
          aria-hidden="true"
          className={cn([
            "border-line bg-surface/90 absolute top-12 left-1/2 w-max max-w-[min(92%,720px)] -translate-x-1/2 rounded-full border px-4 py-2 text-center font-mono text-[15px] leading-snug",
            "max-tab:top-[100px] max-tab:rounded-2xl max-tab:text-[13px]",
          ])}
        >
          {state.caption}
        </p>
      )}

      <div className="max-tab:pt-16 relative mx-auto flex max-w-[980px] flex-col items-center gap-5 px-6 pt-[100px] text-center">
        <p className={eyebrow}>{eyebrowText}</p>

        <div aria-hidden="true" className="relative inline-block">
          <span
            className={cn([
              "font-display text-[clamp(28px,4vw,56px)] leading-[1.1] font-extrabold tracking-[-0.025em] transition-colors duration-500",
              state.strike ? "text-faint" : "text-fg nojs:text-faint",
              state.strike && "max-tab:ink-line max-tab:animate-ink",
            ])}
          >
            {t.hero.old}
          </span>
          <svg
            viewBox="0 0 1000 100"
            preserveAspectRatio="none"
            className={cn([
              "max-tab:hidden pointer-events-none absolute top-0 -left-[2%] h-full w-[104%] overflow-visible",
              state.strike ? "block" : "nojs:block hidden",
            ])}
          >
            <path
              className="ink-path nojs:animate-none"
              strokeWidth={6}
              pathLength={1000}
              d="M6 62C220 50 520 44 994 54"
              vectorEffect="non-scaling-stroke"
            />
          </svg>
          {state.pen === "strike" && <Pen className="" duration="1s" />}
          {idle && (
            <span
              className={cn([
                hand,
                "nojs:hidden max-tab:-top-[30px] max-tab:right-0 max-tab:text-[23px] absolute -top-[34px] -right-[34px] rotate-6 text-[27px] whitespace-nowrap",
              ])}
            >
              {t.hero.hmm}
            </span>
          )}
        </div>

        <div className="max-tab:min-h-[130px] grid min-h-[150px] w-full place-items-center">
          <p
            aria-hidden="true"
            className={cn([
              "text-faint nojs:hidden max-w-[30ch] text-2xl leading-[1.4] font-medium transition-opacity duration-500 [grid-area:1/1]",
              (state.hand || state.applied) && "opacity-0",
            ])}
          >
            {t.hero.oldSub}
          </p>
          {state.hand && !state.applied && (
            <p
              aria-hidden="true"
              className={cn([
                hand,
                "relative -rotate-[1.5deg] text-[clamp(34px,4.6vw,62px)] [grid-area:1/1]",
              ])}
            >
              {sentences(t.hero.title).map((line, i) => (
                <span
                  key={line}
                  className="animate-reveal block"
                  style={{ animationDelay: `${i * 0.8}s` }}
                >
                  {line}
                </span>
              ))}
              {state.pen === "hand" && <Pen className="" duration="2.4s" />}
            </p>
          )}
          <h1
            id="hero-title"
            className={cn([
              "font-display text-[clamp(38px,5.2vw,76px)] leading-[1.02] font-extrabold tracking-[-0.03em] [grid-area:1/1]",
              state.applied ? "" : "nojs:not-sr-only sr-only",
            ])}
          >
            {sentences(t.hero.title).map((line) => (
              <span key={line} className="block">
                {line.split(" ").map((w, i, all) => (
                  <span key={i}>
                    <span
                      className={cn([
                        "inline-block",
                        state.applied && "animate-drop",
                      ])}
                      style={{ animationDelay: `${word++ * 70}ms` }}
                    >
                      {w}
                    </span>
                    {i < all.length - 1 && " "}
                  </span>
                ))}
              </span>
            ))}
          </h1>
        </div>

        <p className="text-fg2 max-w-[52ch] text-xl">{t.hero.lede}</p>

        <div className="relative flex flex-wrap items-center justify-center gap-3.5">
          <a
            href={LATEST_RELEASE}
            className={cn([
              button,
              "relative",
              state.applied
                ? "bg-ink text-on-ink hover:text-on-ink"
                : "bg-btn text-btn-fg hover:text-btn-fg",
            ])}
          >
            {t.hero.download}
            <svg
              viewBox="0 0 400 120"
              preserveAspectRatio="none"
              aria-hidden="true"
              className={cn([
                "pointer-events-none absolute -top-5 -left-[22px] h-[calc(100%+40px)] w-[calc(100%+44px)] overflow-visible",
                state.circled ? "block" : "nojs:block hidden",
              ])}
            >
              <path
                className="ink-path nojs:animate-none [animation-duration:1.1s]"
                strokeWidth={4}
                pathLength={1000}
                d="M60 14C10 30 6 96 120 108c120 12 270-4 272-50C394 12 260 2 150 6 100 8 66 14 44 30"
                vectorEffect="non-scaling-stroke"
              />
            </svg>
          </a>
          <a
            href={REPO}
            className={cn([button, "border-fg text-fg border-[1.5px]"])}
          >
            {t.hero.code}
          </a>
          {state.select && (
            <div
              aria-hidden="true"
              className="animate-select pointer-events-none absolute -inset-4 rounded-md border-2 border-white shadow-[0_0_0_4000px_rgba(11,12,14,0.42)] outline outline-1 outline-[#16181d]"
            >
              <span className="absolute top-full left-0 mt-2 rounded bg-[#16181d] px-1.5 py-1 font-mono text-xs leading-none text-[#f2f3f5]">
                344 × 84
              </span>
            </div>
          )}
          {state.arrow && (
            <span
              aria-hidden="true"
              className="animate-pop max-tab:hidden absolute -top-1.5 right-full mr-2 flex items-center gap-1"
            >
              <span
                className={cn([
                  hand,
                  "text-[26px] whitespace-nowrap text-white",
                ])}
              >
                {t.hero.arrow}
              </span>
              <svg viewBox="0 0 60 30" width="60" height="30">
                <path
                  d="M2 20C20 6 36 6 56 16m0 0-9-7m9 7-10 4"
                  fill="none"
                  stroke="#fff"
                  strokeWidth={3}
                  strokeLinecap="round"
                />
              </svg>
            </span>
          )}
        </div>
        <p className="text-muted font-mono text-[13px]">{t.hero.dlNote}</p>
        <PlatformNote t={t.hero.notMac} />

        {idle && (
          <div className="nojs:hidden mt-2 flex flex-col items-center gap-1">
            <button
              type="button"
              aria-label={t.hero.startLabel}
              onClick={start}
              data-start
              className="group max-tab:scale-[0.8] flex -rotate-[8deg] cursor-pointer items-center border-0 bg-transparent p-3 transition-transform duration-300 hover:-translate-y-[3px] hover:-rotate-[4deg]"
            >
              <span
                className={cn([
                  "bg-ink block h-[52px] w-[84px] rounded-[14px_6px_6px_14px] shadow-[inset_-6px_0_0_rgba(0,0,0,0.18)]",
                  "transition-transform duration-300 ease-[cubic-bezier(0.2,1.6,0.3,1)] group-hover:-translate-x-2 group-hover:-rotate-[4deg]",
                  state.uncapped && "animate-cap-off",
                ])}
              />
              <span className="animate-glow flex h-[46px] w-[230px] items-center justify-center gap-2 rounded-[6px_12px_12px_6px] border border-[#2f343c] bg-[#1a1c20] font-mono text-[13px] font-semibold tracking-[0.06em] text-[#f2f3f5]">
                <span className="bg-ink size-2 rounded-full" />
                LIBRELOG · ⌥⇧S
              </span>
              <span className="border-l-ink size-0 border-y-[14px] border-l-[26px] border-y-transparent" />
            </button>
            <span className={cn([hand, "text-[27px]"])}>{t.hero.uncap}</span>
            <button
              type="button"
              data-skip
              onClick={() => {
                show.finish();
                scrollToStage();
              }}
              className="text-muted min-h-11 cursor-pointer border-0 bg-transparent px-3 font-mono text-[13px] underline"
            >
              {t.hero.skip}
            </button>
          </div>
        )}
        {done && (
          <div data-share-hide className="contents">
            <p className={cn([hand, "animate-pop mt-2 -rotate-2 text-[30px]"])}>
              {t.hero.yourTurn}
            </p>
            <div className="flex flex-wrap justify-center gap-2">
              <button
                type="button"
                aria-pressed={inking}
                onClick={() => setInking(!inking)}
                className={cn([
                  chip,
                  "border-ink",
                  inking ? "bg-ink text-on-ink" : "text-ink",
                ])}
              >
                {inking ? t.ui.penOn : t.ui.penOff}
              </button>
              <button
                type="button"
                onClick={() => setCanvasKey(canvasKey + 1)}
                className={chip}
              >
                {t.ui.clear}
              </button>
              <button type="button" onClick={start} className={chip}>
                {t.ui.replay}
              </button>
            </div>
          </div>
        )}
      </div>

      <Cards state={state} tags={t.show.tags} />
      <ContactSheet count={state.sheet} label={t.show.sheet} />
      <AgentTerminal state={state} />
      <InkCanvas key={canvasKey} active={inking} />
      {inking && (
        <div className="absolute bottom-6 left-1/2 flex -translate-x-1/2 gap-2">
          <button
            type="button"
            aria-pressed
            onClick={() => setInking(false)}
            className={cn([chip, "border-ink bg-ink text-on-ink"])}
          >
            {t.ui.penOn}
          </button>
          <button
            type="button"
            onClick={() => setCanvasKey(canvasKey + 1)}
            className={chip}
          >
            {t.ui.clear}
          </button>
        </div>
      )}
      {state.flash && (
        <div
          aria-hidden="true"
          className="animate-flash pointer-events-none absolute inset-0 bg-white"
        />
      )}
    </section>
  );
}
