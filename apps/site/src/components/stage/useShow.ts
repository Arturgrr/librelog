"use client";

import { useEffect, useRef, useState } from "react";

import type { Dictionary } from "~/i18n/en";
import { play, type Sound } from "~/lib/sound";

type Strings = Dictionary["show"];

export type CardKind = keyof Strings["tags"];

export type ShowCard = {
  id: number;
  slot: number;
  kind: CardKind;
  head: string;
  body: string;
  leaving: boolean;
  hot: boolean;
  tilt: number;
};

// Card slots sit beside the copy, never over it; cards only show on screens wide enough for them.
export const SLOTS = [
  { x: 1.5, y: 130, side: -1 },
  { x: 81.5, y: 120, side: 1 },
  { x: 3, y: 300, side: -1 },
  { x: 82.5, y: 290, side: 1 },
  { x: 1, y: 470, side: -1 },
  { x: 81, y: 460, side: 1 },
];

const IDLE = {
  phase: "idle" as "idle" | "run" | "done",
  notch: "idle" as "idle" | "listen" | "work" | "done" | "rec",
  caption: "",
  cards: [] as ShowCard[],
  flies: [] as {
    id: number;
    word: string;
    x: number;
    y: number;
    delay: number;
  }[],
  strike: false,
  pen: null as "strike" | "hand" | null,
  hand: false,
  applied: false,
  entrar: false,
  sheet: 0,
  term: false,
  termLines: [] as { text: string; tone: "cmd" | "dim" | "ok" | "plain" }[],
  agent: "",
  badge: "",
  select: false,
  arrow: false,
  flash: false,
  circled: false,
  uncapped: false,
};

export type ShowState = typeof IDLE;

const DONE: ShowState = {
  ...IDLE,
  phase: "done",
  strike: true,
  hand: true,
  applied: true,
  entrar: true,
  circled: true,
};

function createShow(
  t: Strings,
  setState: (update: (s: ShowState) => ShowState) => void,
  soundOn: () => boolean,
) {
  let timers: number[] = [];
  let phase: ShowState["phase"] = "idle";
  let uid = 0;

  const patch = (p: Partial<ShowState>) => setState((s) => ({ ...s, ...p }));
  const at = (ms: number, fn: () => void) =>
    timers.push(window.setTimeout(fn, ms));
  const clear = () => {
    timers.forEach(clearTimeout);
    timers = [];
  };
  const sfx = (kind: Sound) => {
    if (soundOn()) play(kind);
  };
  const stream = (
    text: string,
    start: number,
    step: number,
    field: "caption" | "agent" = "caption",
  ) => {
    const words = text.split(" ");
    words.forEach((_, i) =>
      at(start + i * step, () => {
        const shown = words.slice(0, i + 1).join(" ");
        patch(field === "caption" ? { caption: shown } : { agent: shown });
      }),
    );
  };

  const addCard = (slot: number, [kind, head, body]: readonly string[]) => {
    const id = ++uid;
    const place = slot % SLOTS.length;
    const card: ShowCard = {
      id,
      slot: place,
      kind: kind as CardKind,
      head,
      body,
      leaving: false,
      hot: false,
      tilt: Math.round((Math.random() * 6 - 3) * 10) / 10,
    };
    setState((s) => ({
      ...s,
      cards: [
        ...s.cards.map((c) => (c.slot === place ? { ...c, leaving: true } : c)),
        card,
      ],
    }));
    at(480, () =>
      setState((s) => ({
        ...s,
        cards: s.cards.filter(
          (c) => !(c.leaving && c.slot === place && c.id < id),
        ),
      })),
    );
    sfx("card");
  };
  const editCard = (slot: number, p: Partial<ShowCard>) =>
    setState((s) => ({
      ...s,
      cards: s.cards.map((c) =>
        c.slot === slot && !c.leaving ? { ...c, ...p } : c,
      ),
    }));
  const sweep = () => {
    setState((s) => ({
      ...s,
      cards: s.cards.map((c) => ({ ...c, leaving: true })),
    }));
    at(480, () =>
      setState((s) => ({ ...s, cards: s.cards.filter((c) => !c.leaving) })),
    );
  };
  const fly = (text: string) => {
    const words = text.split(" ").map((word, i) => ({
      id: ++uid,
      word,
      x: 38 + ((i * 37) % 26),
      y: 640 + ((i * 53) % 90),
      delay: i * 0.16,
    }));
    patch({ flies: words });
    at(1200 + words.length * 160, () => patch({ flies: [] }));
  };
  const termLine = (
    text: string,
    tone: ShowState["termLines"][number]["tone"],
  ) => setState((s) => ({ ...s, termLines: [...s.termLines, { text, tone }] }));

  const run = () => {
    clear();
    phase = "run";
    setState(() => ({ ...IDLE, phase: "run" }));
    const A = at;

    // Act 1: a meeting, and voices that become people.
    A(350, () => {
      patch({ notch: "listen" });
      sfx("start");
    });
    stream(t.meet1, 500, 150);
    A(700, () => addCard(0, t.cards.meet1));
    stream(t.meet2, 2000, 150);
    A(2100, () => addCard(1, t.cards.meet2));
    A(3600, () => {
      patch({ notch: "work", caption: t.separating });
      sfx("stop");
    });
    A(4100, () => addCard(2, t.cards.separating));
    A(5500, () => {
      editCard(0, { head: t.rafael });
      editCard(1, { head: t.marina });
      editCard(2, { head: t.cards.voices[1], body: t.cards.voices[2] });
      patch({ notch: "done", caption: t.recognized });
    });
    A(6600, () => patch({ notch: "idle", caption: "" }));

    // Act 2: dictation. Words go into the notch, and the text lands whole once it is transcribed.
    A(7200, () => {
      patch({ notch: "listen", caption: t.hold });
      sfx("start");
      addCard(3, t.cards.slack);
      fly(t.dictated);
    });
    A(9200, () => {
      patch({ notch: "work", caption: t.transcribing });
      sfx("stop");
    });
    A(10100, () => {
      editCard(3, { body: t.dictated, hot: true });
      patch({ notch: "done", caption: t.pasted });
    });
    A(11000, () => {
      editCard(3, { hot: false });
      patch({ notch: "idle", caption: "" });
    });

    // Act 3: Annotate. The pen strikes the headline and writes the real one.
    A(11500, () => patch({ notch: "rec" }));
    stream(t.narr1, 11600, 160);
    A(12700, () => {
      patch({ strike: true, pen: "strike" });
      sfx("scratch");
    });
    A(13800, () => patch({ pen: null }));
    stream(t.narr2, 14000, 130);
    A(14300, () => {
      patch({ hand: true, pen: "hand" });
      sfx("scratchLong");
    });
    A(16900, () => patch({ pen: null }));
    stream(t.narr3, 17100, 140);
    A(17900, () => {
      patch({ entrar: true });
      sfx("scratch");
    });
    A(19000, () => {
      patch({ notch: "work", caption: t.sheetCaption, sheet: 1 });
      sfx("stop");
    });
    A(19400, () => patch({ sheet: 2 }));
    A(19800, () => patch({ sheet: 3 }));
    A(20400, () => patch({ notch: "done", caption: t.ready }));

    // Act 4: the agent reads the contact sheet and applies the fix.
    A(21200, () => {
      patch({ notch: "idle", caption: "", term: true, badge: t.thinking });
      termLine("› ⌥⇧A", "cmd");
    });
    A(21800, () => {
      termLine(t.termPath, "dim");
      termLine(t.termSummary, "dim");
    });
    stream(t.agent, 22800, 110, "agent");
    A(25400, () => {
      patch({ agent: "", badge: t.appliedBadge, applied: true });
      termLine(t.agent, "plain");
      termLine(t.applied, "ok");
      sfx("chime");
    });

    // Act 5: everything at once, then a screen capture wipes it.
    const states = ["listen", "work", "done"] as const;
    t.burst.forEach((card, i) =>
      A(26600 + i * 380, () => {
        addCard(i, card);
        patch({
          notch: states[i % 3],
          caption: t.burstCaptions[i % t.burstCaptions.length],
        });
      }),
    );
    A(31400, () => {
      sweep();
      patch({ notch: "idle", caption: t.screen });
    });
    A(32000, () => {
      patch({ select: true, caption: t.drag });
      sfx("card");
    });
    A(33100, () => patch({ arrow: true }));
    A(34100, () => {
      patch({ flash: true, caption: t.copied });
      sfx("shutter");
    });
    A(34700, () =>
      patch({
        flash: false,
        select: false,
        arrow: false,
        term: false,
        sheet: 0,
        circled: true,
      }),
    );
    A(35600, () => {
      phase = "done";
      patch({ phase: "done", caption: "" });
    });
  };

  const finish = () => {
    clear();
    phase = "done";
    setState(() => DONE);
  };

  const start = () => {
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return finish();
    if (phase !== "idle") return run();
    clear();
    patch({ uncapped: true });
    sfx("pop");
    at(700, run);
  };

  return { start, finish, clear, phase: () => phase };
}

export function useShow(t: Strings, soundOn: boolean) {
  const [state, setState] = useState<ShowState>(IDLE);
  const sound = useRef(soundOn);
  const [show] = useState(() => createShow(t, setState, () => sound.current));

  useEffect(() => {
    sound.current = soundOn;
  }, [soundOn]);

  useEffect(() => {
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) show.finish();
    return show.clear;
  }, [show]);

  return { state, show };
}
