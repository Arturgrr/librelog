// Every sound is synthesized on the spot: the page downloads no audio and plays nothing before a click.

export type Sound =
  | "pop"
  | "start"
  | "stop"
  | "scratch"
  | "scratchLong"
  | "card"
  | "chime"
  | "shutter";

let context: AudioContext | null = null;

export function play(kind: Sound) {
  try {
    context ??= new AudioContext();
    const ac = context;
    const t = ac.currentTime;
    const tone = (
      f0: number,
      f1: number,
      dur: number,
      vol: number,
      at = 0,
      type: OscillatorType = "sine",
    ) => {
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
    const noise = (
      dur: number,
      vol: number,
      freq: number,
      q: number,
      at = 0,
    ) => {
      const length = Math.floor(ac.sampleRate * dur);
      const buffer = ac.createBuffer(1, length, ac.sampleRate);
      const data = buffer.getChannelData(0);
      for (let i = 0; i < length; i++)
        data[i] = (Math.random() * 2 - 1) * (0.6 + 0.4 * Math.sin(i / 900));
      const source = ac.createBufferSource();
      const filter = ac.createBiquadFilter();
      const g = ac.createGain();
      source.buffer = buffer;
      filter.type = "bandpass";
      filter.frequency.value = freq;
      filter.Q.value = q;
      g.gain.setValueAtTime(vol, t + at);
      g.gain.exponentialRampToValueAtTime(0.0001, t + at + dur);
      source.connect(filter).connect(g).connect(ac.destination);
      source.start(t + at);
    };
    switch (kind) {
      case "pop":
        tone(180, 60, 0.18, 0.4, 0, "triangle");
        noise(0.08, 0.3, 2500, 1);
        break;
      case "start":
        tone(660, 990, 0.14, 0.18);
        break;
      case "stop":
        tone(880, 520, 0.16, 0.18);
        break;
      case "scratch":
        noise(0.85, 0.22, 3200, 0.8);
        break;
      case "scratchLong":
        noise(2.4, 0.16, 2800, 0.7);
        break;
      case "card":
        tone(1200, 900, 0.06, 0.05);
        break;
      case "chime":
        tone(784, 784, 0.5, 0.15);
        tone(1175, 1175, 0.6, 0.12, 0.13);
        break;
      case "shutter":
        noise(0.05, 0.5, 4000, 1.2);
        noise(0.07, 0.4, 2500, 1.2, 0.09);
        break;
    }
  } catch {}
}

export const storage = {
  get(key: string) {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  },
  set(key: string, value: string) {
    try {
      localStorage.setItem(key, value);
    } catch {}
  },
};
