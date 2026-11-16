(() => {
  const S = JSON.parse(document.getElementById("strings").textContent);
  const stage = document.querySelector(".stage");
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");
  const store = {
    get(key) {
      try {
        return localStorage.getItem(key);
      } catch {
        return null;
      }
    },
    set(key, value) {
      try {
        localStorage.setItem(key, value);
      } catch {}
    },
  };

  console.info(S.console);

  // Sounds are synthesized on the spot: the page downloads no audio and plays nothing before a click.
  const sound = {
    on: store.get("librelog-sound") !== "off",
    ctx: null,
    play(kind) {
      if (!this.on) return;
      try {
        this.ctx ??= new AudioContext();
        const ac = this.ctx;
        const t = ac.currentTime;
        const tone = (f0, f1, dur, vol, type = "sine", at = 0) => {
          const o = ac.createOscillator();
          const g = ac.createGain();
          o.type = type;
          o.frequency.setValueAtTime(f0, t + at);
          o.frequency.exponentialRampToValueAtTime(f1, t + at + dur);
          g.gain.setValueAtTime(0.0001, t + at);
          g.gain.exponentialRampToValueAtTime(vol, t + at + 0.015);
          g.gain.exponentialRampToValueAtTime(0.0001, t + at + dur);
          o.connect(g).connect(ac.destination);
          o.start(t + at);
          o.stop(t + at + dur + 0.02);
        };
        const noise = (dur, vol, freq, q, at = 0) => {
          const len = Math.floor(ac.sampleRate * dur);
          const buf = ac.createBuffer(1, len, ac.sampleRate);
          const d = buf.getChannelData(0);
          for (let i = 0; i < len; i++)
            d[i] = (Math.random() * 2 - 1) * (0.6 + 0.4 * Math.sin(i / 900));
          const src = ac.createBufferSource();
          const bp = ac.createBiquadFilter();
          const g = ac.createGain();
          src.buffer = buf;
          bp.type = "bandpass";
          bp.frequency.value = freq;
          bp.Q.value = q;
          g.gain.setValueAtTime(vol, t + at);
          g.gain.exponentialRampToValueAtTime(0.0001, t + at + dur);
          src.connect(bp).connect(g).connect(ac.destination);
          src.start(t + at);
        };
        const sounds = {
          pop: () => {
            tone(180, 60, 0.18, 0.4, "triangle");
            noise(0.08, 0.3, 2500, 1);
          },
          start: () => tone(660, 990, 0.14, 0.18),
          stop: () => tone(880, 520, 0.16, 0.18),
          scratch: () => noise(0.85, 0.22, 3200, 0.8),
          scratchLong: () => noise(2.4, 0.16, 2800, 0.7),
          card: () => tone(1200, 900, 0.06, 0.05),
          chime: () => {
            tone(784, 784, 0.5, 0.15);
            tone(1175, 1175, 0.6, 0.12, "sine", 0.13);
          },
          shutter: () => {
            noise(0.05, 0.5, 4000, 1.2);
            noise(0.07, 0.4, 2500, 1.2, 0.09);
          },
        };
        sounds[kind]?.();
      } catch {}
    },
  };

  const flag = (name, value = true) => {
    if (value === false || value == null) delete stage.dataset[name];
    else stage.dataset[name] = value === true ? "" : String(value);
  };

  // The show: a scripted revision of this page, in five acts.
  const caption = stage.querySelector(".caption");
  const layer = stage.querySelector(".layer");
  const termLines = stage.querySelector(".term__lines");
  const termAgent = stage.querySelector(".term__agent");
  const termBadge = stage.querySelector(".term__badge");
  const title = stage.querySelector(".swap__title");
  const T = S.show;

  const sentencesOf = (text) =>
    text
      .trim()
      .match(/[^.!?]+[.!?]*/g)
      .map((x) => x.trim());
  let wordIndex = 0;
  const titleSentences = sentencesOf(title.textContent);
  title.textContent = "";
  for (const sentence of titleSentences) {
    const line = document.createElement("span");
    line.className = "line";
    sentence.split(/\s+/).forEach((word, i, words) => {
      const span = document.createElement("span");
      span.className = "w";
      span.style.setProperty("--i", String(wordIndex++));
      span.textContent = word;
      line.append(span, i < words.length - 1 ? " " : "");
    });
    title.append(line, " ");
  }

  const ink = stage.querySelector(".swap__ink");
  const sentences = sentencesOf(ink.textContent);
  ink.textContent = "";
  sentences.forEach((line, i) => {
    const span = document.createElement("span");
    span.className = "swap__line";
    span.style.setProperty("--i", String(i));
    span.textContent = line;
    ink.append(span);
  });

  let timers = [];
  const at = (ms, fn) => timers.push(setTimeout(fn, ms));
  const clearTimers = () => {
    timers.forEach(clearTimeout);
    timers = [];
  };
  const stream = (text, startMs, step, el = caption) => {
    const words = text.split(" ");
    words.forEach((_, i) =>
      at(
        startMs + i * step,
        () => (el.textContent = words.slice(0, i + 1).join(" ")),
      ),
    );
  };

  const SLOTS = [
    { x: 1.5, y: 130, side: -1 },
    { x: 81.5, y: 120, side: 1 },
    { x: 3, y: 300, side: -1 },
    { x: 82.5, y: 290, side: 1 },
    { x: 1, y: 470, side: -1 },
    { x: 81, y: 460, side: 1 },
  ];
  const cards = new Map();

  const removeCard = (el) => {
    el.classList.add("is-out");
    setTimeout(() => el.remove(), 480);
  };

  const addCard = (slot, [kind, head, body]) => {
    const s = SLOTS[slot % SLOTS.length];
    const old = cards.get(slot % SLOTS.length);
    if (old) removeCard(old);
    const el = document.createElement("div");
    el.className = "card";
    el.style.cssText = `left:${s.x}%;top:${s.y}px;--r:${(Math.random() * 6 - 3).toFixed(1)}deg;--dx:${s.side * 220}px`;
    const tag = document.createElement("span");
    tag.className = `tag tag--${kind}`;
    tag.textContent = T.tags[kind];
    const b = document.createElement("b");
    b.textContent = head;
    const p = document.createElement("p");
    p.textContent = body;
    el.append(tag, b, p);
    layer.append(el);
    cards.set(slot % SLOTS.length, el);
    sound.play("card");
    return el;
  };

  const editCard = (slot, head, body) => {
    const el = cards.get(slot);
    if (!el) return;
    if (head != null) el.querySelector("b").textContent = head;
    if (body != null) el.querySelector("p").textContent = body;
    return el;
  };

  const sweep = () => {
    cards.forEach(removeCard);
    cards.clear();
  };

  const fly = (text) => {
    text.split(" ").forEach((w, i) => {
      const el = document.createElement("span");
      el.className = "fly";
      el.textContent = w;
      el.style.cssText = `--sx:${38 + ((i * 37) % 26)}%;--sy:${640 + ((i * 53) % 90)}px;animation-delay:${(i * 0.16).toFixed(2)}s`;
      layer.append(el);
      setTimeout(() => el.remove(), 1200 + i * 160);
    });
  };

  const termLine = (text, cls) => {
    const div = document.createElement("div");
    div.className = cls;
    div.textContent = text;
    termLines.append(div);
  };

  const reset = (phase) => {
    clearTimers();
    sweep();
    layer.querySelectorAll(".fly").forEach((el) => el.remove());
    termLines.textContent = "";
    termAgent.textContent = "";
    termBadge.textContent = "";
    caption.textContent = "";
    for (const name of [
      "strike",
      "pen",
      "hand",
      "applied",
      "entrar",
      "sheet",
      "term",
      "select",
      "arrow",
      "flash",
      "circled",
      "uncapped",
    ])
      flag(name, false);
    flag("notch", "idle");
    flag("phase", phase);
  };

  const finish = () => {
    reset("done");
    for (const name of ["strike", "hand", "applied", "entrar", "circled"])
      flag(name);
  };

  const run = () => {
    reset("run");
    const A = at;
    A(350, () => {
      flag("notch", "listen");
      sound.play("start");
    });
    stream(T.meet1, 500, 150);
    A(700, () => addCard(0, T.cards.meet1));
    stream(T.meet2, 2000, 150);
    A(2100, () => addCard(1, T.cards.meet2));
    A(3600, () => {
      flag("notch", "work");
      caption.textContent = T.separating;
      sound.play("stop");
    });
    A(4100, () => addCard(2, T.cards.sep));
    A(5500, () => {
      editCard(0, T.cards.rafael);
      editCard(1, T.cards.marina);
      editCard(2, ...T.cards.voices);
      flag("notch", "done");
      caption.textContent = T.recognized;
    });
    A(6600, () => {
      flag("notch", "idle");
      caption.textContent = "";
    });

    A(7200, () => {
      flag("notch", "listen");
      caption.textContent = T.hold;
      sound.play("start");
      addCard(3, T.cards.slack);
      fly(T.dictated);
    });
    A(9200, () => {
      flag("notch", "work");
      caption.textContent = T.transcribing;
      sound.play("stop");
    });
    A(10100, () => {
      editCard(3, null, T.dictated)?.classList.add("is-hot");
      flag("notch", "done");
      caption.textContent = T.pasted;
    });
    A(11000, () => {
      cards.get(3)?.classList.remove("is-hot");
      flag("notch", "idle");
      caption.textContent = "";
    });

    A(11500, () => flag("notch", "rec"));
    stream(T.narr1, 11600, 160);
    A(12700, () => {
      flag("strike");
      flag("pen", "strike");
      sound.play("scratch");
    });
    A(13800, () => flag("pen", false));
    stream(T.narr2, 14000, 130);
    A(14300, () => {
      flag("hand");
      flag("pen", "hand");
      sound.play("scratchLong");
    });
    A(16900, () => flag("pen", false));
    stream(T.narr3, 17100, 140);
    A(17900, () => {
      flag("entrar");
      sound.play("scratch");
    });
    A(19000, () => {
      flag("notch", "work");
      caption.textContent = T.sheet;
      flag("sheet", 1);
      sound.play("stop");
    });
    A(19400, () => flag("sheet", 2));
    A(19800, () => flag("sheet", 3));
    A(20400, () => {
      flag("notch", "done");
      caption.textContent = T.ready;
    });

    A(21200, () => {
      flag("notch", "idle");
      caption.textContent = "";
      flag("term");
      termBadge.textContent = T.thinking;
      termLine("› ⌥⇧A", "is-cmd");
    });
    A(21800, () => {
      termLine(T.termPath, "is-dim");
      termLine(T.termSummary, "is-dim");
    });
    stream(T.agent, 22800, 110, termAgent);
    A(25400, () => {
      termAgent.textContent = "";
      termLine(T.agent, "");
      termLine(T.applied, "is-ok");
      termBadge.textContent = T.appliedBadge;
      flag("applied");
      sound.play("chime");
    });

    const states = ["listen", "work", "done"];
    T.burst.forEach((card, i) =>
      A(26600 + i * 380, () => {
        addCard(i, card);
        flag("notch", states[i % 3]);
        caption.textContent = T.burstCaptions[i % T.burstCaptions.length];
      }),
    );
    A(31400, () => {
      sweep();
      flag("notch", "idle");
      caption.textContent = T.screen;
    });
    A(32000, () => {
      flag("select");
      caption.textContent = T.drag;
      sound.play("card");
    });
    A(33100, () => flag("arrow"));
    A(34100, () => {
      flag("flash");
      caption.textContent = T.copied;
      sound.play("shutter");
    });
    A(34700, () => {
      for (const name of ["flash", "select", "arrow", "term", "sheet"])
        flag(name, false);
      flag("circled");
    });
    A(35600, () => {
      flag("phase", "done");
      caption.textContent = "";
    });
  };

  const start = () => {
    if (reduced.matches) return finish();
    if (scrollY > stage.offsetTop + 8)
      scrollTo({ top: stage.offsetTop, behavior: "smooth" });
    if (stage.dataset.phase === "idle") {
      flag("uncapped");
      sound.play("pop");
      clearTimers();
      at(700, run);
    } else {
      run();
    }
  };

  // Free drawing, like Annotate: the strokes are kept normalized so they survive a resize.
  const canvas = stage.querySelector(".ink");
  const ctx = canvas.getContext("2d");
  let strokes = [];
  let current = null;
  const inkColor = () =>
    getComputedStyle(document.documentElement)
      .getPropertyValue("--ink")
      .trim() || "#c42323";
  const fit = () => {
    const r = stage.getBoundingClientRect();
    const dpr = Math.min(devicePixelRatio || 1, 2);
    canvas.width = Math.round(r.width * dpr);
    canvas.height = Math.round(r.height * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    redraw();
  };
  const redraw = () => {
    const r = stage.getBoundingClientRect();
    ctx.clearRect(0, 0, r.width, r.height);
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.lineWidth = 4;
    ctx.strokeStyle = inkColor();
    for (const s of current ? [...strokes, current] : strokes) {
      ctx.beginPath();
      s.forEach(([x, y], i) =>
        i
          ? ctx.lineTo(x * r.width, y * r.height)
          : ctx.moveTo(x * r.width, y * r.height),
      );
      ctx.stroke();
    }
  };
  const point = (e) => {
    const r = canvas.getBoundingClientRect();
    return [(e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height];
  };
  canvas.addEventListener("pointerdown", (e) => {
    canvas.setPointerCapture(e.pointerId);
    current = [point(e)];
  });
  canvas.addEventListener("pointermove", (e) => {
    if (!current) return;
    current.push(point(e));
    redraw();
  });
  const endStroke = () => {
    if (current && current.length > 1) strokes.push(current);
    current = null;
    redraw();
  };
  canvas.addEventListener("pointerup", endStroke);
  canvas.addEventListener("pointercancel", endStroke);
  new ResizeObserver(fit).observe(stage);

  const penButton = stage.querySelector('[data-action="pen"]');
  const setPen = (on) => {
    flag("inking", on);
    penButton.setAttribute("aria-pressed", String(on));
    penButton.textContent = on ? S.ui.penOn : S.ui.penOff;
  };

  const soundButton = stage.querySelector('[data-action="sound"]');
  const paintSound = () => {
    soundButton.setAttribute("aria-pressed", String(sound.on));
    soundButton.textContent = sound.on ? S.ui.soundOn : S.ui.soundOff;
  };
  paintSound();

  const actions = {
    start,
    replay: () => {
      setPen(false);
      start();
    },
    skip: () => {
      finish();
      scrollTo({
        top: stage.offsetTop,
        behavior: reduced.matches ? "auto" : "smooth",
      });
    },
    sound: () => {
      sound.on = !sound.on;
      store.set("librelog-sound", sound.on ? "on" : "off");
      paintSound();
    },
    pen: () => setPen(!("inking" in stage.dataset)),
    clear: () => {
      strokes = [];
      redraw();
    },
    cookie: (button) => {
      button.closest(".cookie").hidden = true;
      store.set("librelog-cookie", "dismissed");
    },
  };
  document.addEventListener("click", (e) => {
    const button = e.target.closest("[data-action]");
    if (button) actions[button.dataset.action]?.(button);
  });

  // ⌥⇧S is Annotate's shortcut in the app; here it starts the show, then picks up the pen.
  addEventListener("keydown", (e) => {
    if (e.altKey && e.shiftKey && e.code === "KeyS") {
      e.preventDefault();
      if (stage.dataset.phase === "idle") start();
      else if (stage.dataset.phase === "done") actions.pen();
    } else if (e.key === "Escape") {
      if ("inking" in stage.dataset) setPen(false);
      else if (stage.dataset.phase === "run") finish();
    }
  });

  if (reduced.matches) finish();

  // Story: the window follows the chapter in the middle of the viewport.
  const story = document.querySelector(".story");
  const chapters = [...story.querySelectorAll(".chapter")];
  const tabs = [...story.querySelectorAll("[data-tab]")];
  const show = (i) => {
    if (story.dataset.ch === String(i)) return;
    story.dataset.ch = String(i);
    chapters.forEach((c, j) => c.classList.toggle("is-active", i === j));
    tabs.forEach((t, j) => t.setAttribute("aria-selected", String(i === j)));
  };
  chapters[0].classList.add("is-active");
  const chapterObserver = new IntersectionObserver(
    (entries) =>
      entries.forEach(
        (e) => e.isIntersecting && show(chapters.indexOf(e.target)),
      ),
    { rootMargin: "-45% 0px -45% 0px" },
  );
  chapters.forEach((c) => chapterObserver.observe(c));
  tabs.forEach((t, i) =>
    t.addEventListener("click", () => {
      show(i);
      if (matchMedia("(min-width: 981px)").matches)
        chapters[i].scrollIntoView({
          behavior: reduced.matches ? "auto" : "smooth",
          block: "center",
        });
    }),
  );

  // Rolling zeros, the pen crossing out lines, and blocks that rise into place.
  const onView = new IntersectionObserver(
    (entries) =>
      entries.forEach((e) => {
        if (!e.isIntersecting) return;
        e.target.classList.add("is-in");
        onView.unobserve(e.target);
      }),
    { threshold: 0.15 },
  );
  document
    .querySelectorAll(".zeros, .ink-line, .rise, .step")
    .forEach((el) => onView.observe(el));

  // The network monitor only ever shows the three addresses the app knows.
  const net = document.querySelector(".net");
  const rows = net.querySelector(".net__rows");
  const paintNet = () => {
    const n = rows.children.length;
    net.dataset.count = String(n);
    net.querySelector('[data-net="count"]').textContent = String(n);
    net.querySelector('[data-net="hosts"]').textContent = String(n);
  };
  net.addEventListener("click", (e) => {
    const add = e.target.closest("[data-net-add]");
    if (add) {
      const key = add.dataset.netAdd;
      if (rows.querySelector(`[data-host="${key}"]`)) return;
      const [host, why] = S.net[key];
      const row = document.createElement("div");
      row.className = "net__row";
      row.dataset.host = key;
      const h = document.createElement("span");
      h.className = "mono";
      h.textContent = host;
      const w = document.createElement("span");
      w.textContent = why;
      row.append(h, w);
      rows.append(row);
      paintNet();
    } else if (e.target.closest("[data-net-reset]")) {
      rows.textContent = "";
      paintNet();
    }
  });
  paintNet();

  document.querySelectorAll("[data-copy]").forEach((button) =>
    button.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(button.dataset.copy);
        button.textContent = S.ui.copied;
        setTimeout(() => (button.textContent = S.ui.copy), 2000);
      } catch {}
    }),
  );

  // Downloads are Apple-silicon Macs only; say so instead of offering a file that won't open.
  const ua = navigator.userAgent;
  if (
    /Windows|Android|CrOS|iPhone|Linux(?!.*Mac)/i.test(ua) &&
    !/Macintosh/.test(ua)
  ) {
    document
      .querySelectorAll(".platform-note")
      .forEach((el) => (el.hidden = false));
  }

  if (store.get("librelog-cookie") !== "dismissed")
    document.querySelector(".cookie").hidden = false;

  document
    .querySelectorAll("[data-lang]")
    .forEach((a) =>
      a.addEventListener("click", () =>
        store.set("librelog-lang", a.dataset.lang),
      ),
    );

  const elapsed = document.querySelector("[data-elapsed]");
  const began = Date.now();
  const tick = () => {
    const s = Math.floor((Date.now() - began) / 1000);
    elapsed.textContent = S.elapsed.replace(
      "{t}",
      `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`,
    );
  };
  tick();
  setInterval(tick, 1000);
})();
