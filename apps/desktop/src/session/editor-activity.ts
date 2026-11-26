import type { EditorView } from "prosemirror-view";

import { flushDatabaseWrites } from "~/db/write-queue";

const mountedCanonicalEditors = new Map<
  string,
  Map<
    EditorView,
    { flushPendingChanges: () => void; flushQueuedChange: () => void }
  >
>();

export function registerCanonicalSessionEditor(
  sessionId: string,
  view: EditorView,
  flushPendingChanges: () => void,
  flushQueuedChange: () => void,
) {
  const editors = mountedCanonicalEditors.get(sessionId) ?? new Map();
  editors.set(view, { flushPendingChanges, flushQueuedChange });
  mountedCanonicalEditors.set(sessionId, editors);
}

export function unregisterCanonicalSessionEditor(
  sessionId: string,
  view: EditorView,
) {
  const editors = mountedCanonicalEditors.get(sessionId);
  if (!editors) return;
  editors.delete(view);
  if (editors.size === 0) mountedCanonicalEditors.delete(sessionId);
}

export async function flushCanonicalSessionEditorChanges(
  sessionId: string,
): Promise<void> {
  const editors = mountedCanonicalEditors.get(sessionId);
  if (editors) {
    for (const editor of editors.values()) {
      editor.flushPendingChanges();
    }
  }
  await flushDatabaseWrites([`session:${sessionId}`]);
}

/** Hands every mounted editor's queued local edit to the database write queue. */
export function flushAllCanonicalSessionEditors() {
  for (const editors of mountedCanonicalEditors.values()) {
    for (const editor of editors.values()) {
      try {
        editor.flushQueuedChange();
      } catch (error) {
        console.error("Failed to flush a note editor", error);
      }
    }
  }
}
