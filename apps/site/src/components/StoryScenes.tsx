import { cn } from "@anlg/utils";

import { Bars, Spinner } from "./stage/Notch";
import { eyebrow, hand } from "./styles";

import type { Dictionary } from "~/i18n/en";

type Story = Dictionary["story"];

const pill =
  "inline-block whitespace-nowrap rounded-full bg-sunk px-[7px] py-1 font-mono text-[11px] font-semibold leading-none text-fg";
const bar = "block h-2 w-4/5 rounded bg-sunk";

// Each scene loops while its chapter is on screen; with reduced motion it rests on its last frame.
export function SayScene({ t }: { t: Story["s1"] }) {
  return (
    <div className="from-sunk to-bg absolute inset-0 bg-linear-160">
      <div className="bg-notch absolute top-0 left-1/2 grid h-[30px] -translate-x-1/2 animate-[s1n_5s_infinite] place-items-center overflow-hidden rounded-b-[14px] motion-reduce:w-[110px] motion-reduce:animate-none">
        <span className="flex animate-[s1b_5s_infinite] items-center gap-2 [grid-area:1/1] motion-reduce:opacity-0">
          <span className="bg-mic-bright size-2.5 rounded-full" />
          <Bars />
        </span>
        <span className="animate-[s1s_5s_infinite] [grid-area:1/1] motion-reduce:opacity-0">
          <Spinner />
        </span>
        <span className="animate-[s1c_5s_infinite] text-[15px] font-extrabold text-[#4fcf85] [grid-area:1/1]">
          ✓
        </span>
      </div>
      <span
        className={cn([
          hand,
          "text-mic absolute inset-x-0 bottom-[70px] animate-[s1w_5s_infinite] text-center text-3xl motion-reduce:opacity-0",
        ])}
      >
        {t.spoken}
      </span>
      <div className="border-line bg-card absolute inset-x-[8%] top-16 bottom-9 flex flex-col rounded-[10px] border">
        <div className="border-line border-b px-3.5 py-2.5 font-mono text-[13px] font-semibold">
          {t.channel}
        </div>
        <p className="flex-1 px-3.5 py-3 text-sm">
          <b>Rafael</b> <span className="text-muted">{t.question}</span>
        </p>
        <div className="border-line2 m-2.5 min-h-[46px] rounded-lg border-[1.5px] px-3 py-2.5 text-sm">
          <span className="inline-block animate-[s1t_5s_infinite]">
            {t.typed}
          </span>
        </div>
      </div>
    </div>
  );
}

export function MeetScene({ t }: { t: Story["s2"] }) {
  return (
    <div className="absolute inset-0 grid grid-cols-2">
      <div className="flex min-w-0 flex-col gap-2.5 p-[18px] text-[13px]">
        <span className={eyebrow}>{t.notes}</span>
        <p className="text-fg2 animate-[s2m_9s_infinite] font-mono text-[13px] leading-[1.7]">
          {t.memo.map((line) => (
            <span key={line} className="block">
              {line}
            </span>
          ))}
        </p>
        <span className={eyebrow}>{t.transcript}</span>
        <p className="flex flex-wrap items-center gap-2">
          <span className="inline-grid">
            <span
              className={cn([
                pill,
                "animate-[s2a_9s_infinite] [grid-area:1/1] motion-reduce:opacity-0",
              ])}
            >
              {t.speaker}
            </span>
            <span
              className={cn([
                pill,
                "bg-agent animate-[s2b_9s_infinite] text-white [grid-area:1/1]",
              ])}
            >
              {t.prof}
            </span>
          </span>
          {t.line1}
        </p>
        <p className="flex flex-wrap items-center gap-2">
          <span className={pill}>{t.me}</span>
          {t.line2}
        </p>
        <span className="text-muted animate-[s2l_9s_infinite] font-mono text-xs motion-reduce:opacity-0">
          {t.separating}
        </span>
      </div>
      <div className="border-line bg-bg flex min-w-0 flex-col gap-2.5 border-l p-[18px]">
        <span className={eyebrow}>{t.summaryLabel}</span>
        <div className="flex animate-[s2s_9s_infinite] flex-col gap-2 text-sm">
          <b className="font-display text-lg leading-tight">{t.summaryTitle}</b>
          {t.summary.map((line) => (
            <span key={line}>{line}</span>
          ))}
          <span className="text-ok font-mono">{t.exported}</span>
        </div>
      </div>
    </div>
  );
}

export function ShowScene({ t }: { t: Story["s3"] }) {
  return (
    <div className="bg-bg absolute inset-0">
      <div className="border-line bg-card absolute inset-x-[6%] top-6 flex h-60 flex-col gap-3 rounded-[10px] border p-[18px]">
        <i className="bg-line block h-3 w-[45%] rounded" />
        <i className={bar} />
        <i className={cn([bar, "w-2/3"])} />
        <span className="bg-btn text-btn-fg relative mt-auto self-end rounded-lg px-[18px] py-2.5 text-[13px] font-semibold">
          {t.save}
          <svg
            viewBox="0 0 200 100"
            preserveAspectRatio="none"
            className="absolute -top-[22px] -left-[30px] h-[calc(100%+44px)] w-[calc(100%+60px)] overflow-visible"
          >
            <path
              pathLength={1000}
              d="M40 10C0 30 0 90 100 94c90 4 104-50 90-70C176 4 110 0 60 12"
              className="stroke-ink animate-[s3c_6s_infinite] fill-none [stroke-width:3.5] [stroke-dasharray:1000] [stroke-linecap:round]"
            />
          </svg>
        </span>
      </div>
      <div className="absolute bottom-24 left-[6%] flex gap-2">
        {["1", "2", "3"].map((n, i) => (
          <span
            key={n}
            className={cn([
              hand,
              "border-ink bg-card flex h-[46px] w-[66px] animate-[s3f_6s_infinite] items-end rounded-md border-[1.5px] px-1.5 py-[3px] text-xl",
            ])}
            style={{ animationDelay: `${i * 0.25}s` }}
          >
            {n}
          </span>
        ))}
      </div>
      <div className="bg-term text-term-fg absolute inset-x-[6%] bottom-[18px] rounded-lg px-3 py-2.5 font-mono text-xs leading-normal">
        <span className="text-amber">›</span>{" "}
        <span className="animate-[s3t_6s_infinite]">{t.pasted}</span>
      </div>
    </div>
  );
}

export function CaptureScene({ t }: { t: Story["s4"] }) {
  return (
    <div className="from-sunk to-bg absolute inset-0 bg-linear-160">
      <div className="border-line bg-card absolute top-10 left-[8%] flex h-[300px] w-[62%] flex-col gap-3 rounded-[10px] border p-[18px]">
        <i className="bg-line block h-3 w-[45%] rounded" />
        <i className={bar} />
        <i className={cn([bar, "w-2/3"])} />
        <span className="text-fg2 mt-3.5 text-[13px]">
          {t.client}{" "}
          <span className="relative">
            marina@acme.com
            <span className="bg-faint/80 absolute -inset-x-1 -inset-y-[3px] animate-[s4b_6s_infinite] rounded-sm backdrop-blur-sm" />
          </span>
        </span>
      </div>
      <div className="absolute inset-0 animate-[s4d_6s_infinite] bg-[#0b0c0e]/20" />
      <div className="absolute top-[70px] left-[12%] animate-[s4r_6s_infinite] border-2 border-white bg-white/10 outline outline-1 outline-[#16181d] motion-reduce:h-[42%] motion-reduce:w-[46%]" />
      <svg
        viewBox="0 0 300 200"
        className="absolute top-[120px] right-[8%] h-[150px] w-[220px] overflow-visible"
      >
        <path
          pathLength={400}
          d="M280 20C200 30 120 80 40 150m0 0 8-30m-8 30 30-6"
          className="stroke-ink animate-[s4a_6s_infinite] fill-none [stroke-width:5] [stroke-dasharray:400] [stroke-linecap:round]"
        />
      </svg>
      <span className="absolute right-5 bottom-5 animate-[s4k_6s_infinite] rounded-lg bg-[#16181d] px-3 py-2 font-mono text-xs text-[#f2f3f5]">
        {t.copied}
      </span>
    </div>
  );
}
