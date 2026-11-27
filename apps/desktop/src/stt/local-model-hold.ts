import { commands as localStt } from "@anlg/plugin-local-stt";

import { isLocalFileSttModel, isOnDeviceSttModel } from "~/stt/capabilities";

/**
 * Starts loading the on-device model `conn` transcribes with and keeps it
 * loaded for `holdMs` at most, so a recording transcribed when it stops
 * doesn't wait for the model to load. It never waits for the load, and a
 * cloud model holds nothing. Returns the release, which may be called more
 * than once.
 */
export function holdLocalModel(
  conn: { provider: string; model: string } | null,
  holdMs: number,
): () => void {
  if (!conn) return () => {};
  const model = isLocalFileSttModel(conn.provider, conn.model)
    ? null
    : isOnDeviceSttModel(conn.provider, conn.model)
      ? conn.model
      : undefined;
  if (model === undefined) return () => {};
  const hold = localStt
    .holdModel(model, holdMs)
    .then((result) => {
      if (result.status === "ok") return result.data;
      console.warn("[stt] could not hold the local model", result.error);
      return null;
    })
    .catch((error: unknown) => {
      console.warn("[stt] could not hold the local model", error);
      return null;
    });
  let released = false;
  return () => {
    if (released) return;
    released = true;
    void hold
      .then((id) => (id ? localStt.releaseModelHold(id) : undefined))
      .catch((error: unknown) =>
        console.warn("[stt] could not release the local model", error),
      );
  };
}

// Renewed well inside each hold, so a hold never lapses between renewals.
const RENEWING_HOLD_MS = 10 * 60_000;
const RENEW_EVERY_MS = 5 * 60_000;

/**
 * Keeps the on-device model `conn` transcribes with loaded until the release
 * is called, however long that takes: a dictation has no time limit, and one
 * hold lasts at most 30 minutes. Each renewal takes a new hold before letting
 * the last one go.
 */
export function holdLocalModelUntilReleased(
  conn: { provider: string; model: string } | null,
): () => void {
  let release = holdLocalModel(conn, RENEWING_HOLD_MS);
  const renewal = setInterval(() => {
    const previous = release;
    release = holdLocalModel(conn, RENEWING_HOLD_MS);
    previous();
  }, RENEW_EVERY_MS);
  return () => {
    clearInterval(renewal);
    release();
  };
}
