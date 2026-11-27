import Image from "next/image";

import { cn } from "@anlg/utils";

import icon64 from "../assets/icon-64.png";
import icon256 from "../assets/icon-256.png";
import { CookieNotice, CopyButton, Elapsed } from "./ClientBits";
import { NetworkMonitor } from "./NetworkMonitor";
import { PlatformNote } from "./PlatformNote";
import { Reveal } from "./Reveal";
import { button, code, eyebrow, h2, hand, wrap } from "./styles";

import type { Dictionary } from "~/i18n/en";
import { LATEST_RELEASE, REPO } from "~/lib/links";
import { withVersion } from "~/lib/site";

const XATTR = "xattr -dr com.apple.quarantine /Applications/Librelog.app";

const DATA_FOLDERS = [
  "~/Library/Application Support/librelog",
  "~/Library/Application Support/com.arturgrr.librelog",
  "~/Library/Caches/com.arturgrr.librelog",
  "~/Library/WebKit/com.arturgrr.librelog",
  "~/Library/Logs/com.arturgrr.librelog",
];

function Circle({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 100 60"
      preserveAspectRatio="none"
      aria-hidden="true"
      className={cn([
        "pointer-events-none absolute -inset-x-4 -inset-y-3.5 h-[calc(100%+28px)] w-[calc(100%+32px)] overflow-visible group-data-wait/reveal:hidden",
        className,
      ])}
    >
      <path
        className="ink-path [animation-delay:0.4s]"
        strokeWidth={3.5}
        pathLength={1000}
        d="M30 6C2 12 0 52 50 55c44 3 50-30 42-42C84 2 54 0 26 8"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}

export function Privacy({ t }: { t: Dictionary["net"] }) {
  return (
    <section aria-labelledby="net-title" className="py-[110px]">
      <div
        className={cn([
          wrap,
          "grid grid-cols-[repeat(auto-fit,minmax(min(100%,420px),1fr))] items-center gap-12",
        ])}
      >
        <Reveal rise>
          <NetworkMonitor t={t} />
        </Reveal>
        <div className="flex min-w-0 flex-col gap-3.5">
          <p className={eyebrow}>{t.eyebrow}</p>
          <h2 id="net-title" className={h2}>
            {t.title}
          </h2>
          <p>{t.body}</p>
          <p className="text-muted text-sm">{t.caveat}</p>
        </div>
      </div>
    </section>
  );
}

export function Letter({ t }: { t: Dictionary["letter"] }) {
  return (
    <section aria-labelledby="letter-title" className="py-[110px]">
      <div className={wrap}>
        <Reveal rise className="mx-auto max-w-[720px]">
          <article className="border-line bg-surface shadow-card max-tab:px-[22px] max-tab:py-[26px] flex -rotate-[0.6deg] flex-col gap-4 rounded-[14px] border px-[38px] py-[34px] text-[19px]">
            <h2 id="letter-title" className={eyebrow}>
              {t.title}
            </h2>
            {t.paragraphs.map((p) => (
              <p key={p}>{p}</p>
            ))}
            <p className={cn([hand, "self-end text-[34px]"])}>
              —{" "}
              <a
                href="https://github.com/Arturgrr"
                className="text-ink no-underline"
              >
                @Arturgrr
              </a>
            </p>
          </article>
        </Reveal>
      </div>
    </section>
  );
}

export function NotList({ t }: { t: Dictionary["not"] }) {
  return (
    <section
      aria-labelledby="not-title"
      className="border-line bg-surface border-y py-[110px]"
    >
      <div
        className={cn([
          wrap,
          "grid grid-cols-[repeat(auto-fit,minmax(min(100%,420px),1fr))] items-center gap-12",
        ])}
      >
        <div className="flex min-w-0 flex-col gap-3.5">
          <p className={eyebrow}>{t.eyebrow}</p>
          <h2 id="not-title" className={h2}>
            {t.title}
          </h2>
          <p className="text-muted">{t.intro}</p>
        </div>
        <Reveal>
          <ul className="m-0 list-none p-0">
            {t.items.map(([item, reason], i) => (
              <li
                key={item}
                className="border-line flex flex-col gap-0.5 border-t py-3.5 last:border-b"
              >
                <s
                  className={cn([
                    "ink-line font-display self-start text-[23px] leading-[1.25] font-bold no-underline",
                    "group-data-in/reveal:animate-ink group-data-wait/reveal:[background-size:0_4px]",
                  ])}
                  style={{ animationDelay: `${i * 0.12}s` }}
                >
                  {item}
                </s>
                <span className="text-muted text-[15px]">{reason}</span>
              </li>
            ))}
          </ul>
        </Reveal>
      </div>
    </section>
  );
}

const picture =
  "relative flex aspect-[4/3] items-center justify-center overflow-hidden rounded-xl border border-line bg-linear-160 from-sunk to-bg";
const dialog =
  "flex w-[82%] flex-col gap-1.5 rounded-[14px] border border-line bg-card p-4 text-xs leading-snug shadow-card";

export function Install({
  t,
  notMac,
  ui,
  cookie,
}: {
  t: Dictionary["install"];
  notMac: Dictionary["hero"]["notMac"];
  ui: Dictionary["ui"];
  cookie: Dictionary["cookie"];
}) {
  const pictures = [
    <div key="dmg" aria-hidden="true" className={cn([picture, "gap-2.5"])}>
      <Image
        src={icon256}
        width={72}
        height={72}
        alt=""
        className="rounded-2xl"
      />
      <svg viewBox="0 0 120 40" className="w-[90px]">
        <path
          d="M4 22C40 8 80 8 112 20m0 0-10-8m10 8-12 5"
          fill="none"
          stroke="var(--ink)"
          strokeWidth={4}
          strokeLinecap="round"
        />
      </svg>
      <span className="relative flex h-[66px] w-[84px] items-end justify-center rounded-[6px_6px_10px_10px] bg-[#3d8ae6] pb-1.5 text-[10px] font-semibold text-white before:absolute before:-top-2 before:left-0 before:h-3 before:w-9 before:rounded-t before:bg-[#3d8ae6]">
        Applications
      </span>
    </div>,
    <div key="alert" aria-hidden="true" className={picture}>
      <div className={cn([dialog, "items-center text-center"])}>
        <Image src={icon64} width={34} height={34} alt="" />
        <b className="text-[13px]">“Librelog” Not Opened</b>
        <span>Apple could not verify “Librelog” is free of malware.</span>
        <span className="mt-1.5 flex w-full gap-2 font-semibold">
          <span className="bg-sunk flex-1 rounded-[7px] px-2 py-1.5">
            Move to Trash
          </span>
          <span className="bg-agent relative flex-1 rounded-[7px] px-2 py-1.5 text-white">
            Done
            <Circle />
          </span>
        </span>
      </div>
    </div>,
    <div key="settings" aria-hidden="true" className={picture}>
      <div className={dialog}>
        <b className="text-[13px]">Privacy &amp; Security</b>
        <span className="bg-line my-1 block h-px w-full" />
        <span className="text-fg2">
          “Librelog” was blocked to protect your Mac.
        </span>
        <span className="bg-agent relative mt-1.5 self-end rounded-[7px] px-3 py-1.5 font-semibold text-white">
          Open Anyway
          <Circle />
        </span>
      </div>
    </div>,
  ];

  return (
    <section
      id="instalar"
      aria-labelledby="install-title"
      className="pt-[110px] pb-[60px]"
    >
      <div className={cn([wrap, "flex flex-col gap-10"])}>
        <div className="flex max-w-[62ch] flex-col gap-3">
          <p className={eyebrow}>{t.eyebrow}</p>
          <h2 id="install-title" className={h2}>
            {t.title}
          </h2>
          <p className="text-muted">{t.intro}</p>
        </div>
        <ol className="m-0 grid list-none grid-cols-[repeat(auto-fit,minmax(min(100%,300px),1fr))] gap-6 p-0">
          {t.steps.map((step, i) => (
            <li key={step}>
              <Reveal rise className="flex flex-col gap-3">
                {pictures[i]}
                <p>
                  <span className={cn([hand, "mr-1 text-[28px]"])}>
                    {i + 1}
                  </span>{" "}
                  {step}
                </p>
              </Reveal>
            </li>
          ))}
        </ol>
        <div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,420px),1fr))] items-start gap-12">
          <div className="flex min-w-0 flex-col gap-3.5">
            <h3 className="text-lg font-bold">{t.terminalTitle}</h3>
            <div className="flex flex-wrap items-center gap-2.5">
              <pre className={cn([code, "flex-[1_1_300px]"])}>
                <code>{XATTR}</code>
              </pre>
              <CopyButton text={XATTR} label={ui.copy} done={ui.copied} />
            </div>
            <p className="text-muted text-sm">{t.checksum}</p>
            <pre className={code}>
              <code>shasum -a 256 -c SHA256SUMS.txt --ignore-missing</code>
            </pre>
          </div>
          <div className="flex min-w-0 flex-col gap-3.5">
            <h3 className="text-lg font-bold">{t.leaveTitle}</h3>
            <p>{t.leaveBody}</p>
            <pre className={cn([code, "bg-sunk text-fg"])}>
              <code>{DATA_FOLDERS.join("\n")}</code>
            </pre>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-3.5">
          <a
            href={LATEST_RELEASE}
            className={cn([button, "bg-ink text-on-ink hover:text-on-ink"])}
          >
            {withVersion(t.download)}
          </a>
          <span className="text-muted font-mono text-[13px]">{t.dlNote}</span>
        </div>
        <PlatformNote t={notMac} />
        <CookieNotice t={cookie} />
      </div>
    </section>
  );
}

export function Footer({ t }: { t: Dictionary["footer"] }) {
  const links = [
    [t.releases, `${REPO}/releases`],
    [t.docs, `${REPO}/tree/main/docs`],
    [t.issues, `${REPO}/issues`],
    ["AGPL-3.0", `${REPO}/blob/main/LICENSE`],
    [t.fonts, `${REPO}/blob/main/THIRD_PARTY_NOTICES.md#fonts`],
  ];
  return (
    <footer className="text-muted pt-10 pb-16 text-sm">
      <div
        className={cn([
          wrap,
          "border-line flex flex-wrap items-center justify-between gap-[18px] border-t pt-[22px]",
        ])}
      >
        <p>
          {t.credits.before}
          <a href="https://github.com/fastrepl/anarlog">{t.credits.link}</a>
          {t.credits.after}
        </p>
        <p className="font-mono text-[13px]">
          <Elapsed template={t.elapsed} fallback={t.noJs} />
        </p>
        <p className={cn([hand, "text-[28px]"])}>{t.sign}</p>
      </div>
      <div className={cn([wrap, "mt-3.5 flex flex-wrap gap-[18px]"])}>
        {links.map(([label, href]) => (
          <a key={href} href={href}>
            {label}
          </a>
        ))}
      </div>
    </footer>
  );
}
