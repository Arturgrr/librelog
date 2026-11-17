import { t } from "@lingui/core/macro";
import { useCallback } from "react";

import { commands as fsSyncCommands } from "@anlg/plugin-fs-sync";
import { toast } from "@anlg/ui/components/ui/toast";

import { getEnhancerService } from "~/services/enhancer";
import { LOCAL_DIARIZATION_TARGET } from "~/stt/capabilities";
import { useListener } from "~/stt/contexts";
import { getTranscriptRecord } from "~/stt/queries";
import { isStoppedTranscriptionError, useRunBatch } from "~/stt/useRunBatch";

export function useRegenerateTranscript(sessionId: string) {
  const runBatch = useRunBatch(sessionId);
  const handleBatchFailed = useListener((state) => state.handleBatchFailed);

  return useCallback(async () => {
    const result = await fsSyncCommands.audioPath(sessionId);
    if (result.status === "error") {
      toast.error(t`Recording not found. It may have been deleted.`, {
        id: `transcript-regenerate-audio-missing-${sessionId}`,
      });
      return;
    }

    const audioPath = result.data;

    try {
      await runBatch(audioPath, {
        promotion: { scope: "whole_session" },
      });
      await getEnhancerService()?.queueAutoEnhanceIfSummaryEmpty(sessionId);
    } catch (error) {
      if (isStoppedTranscriptionError(error)) {
        return;
      }
      const msg = error instanceof Error ? error.message : String(error);
      handleBatchFailed(sessionId, msg);
      toast.error(t`Re-transcription failed`, {
        id: `transcript-regenerate-failed-${sessionId}`,
        description: msg,
      });
    }
  }, [handleBatchFailed, runBatch, sessionId]);
}

// Separates the voices of a note's only transcript from its kept recording
// without transcribing again, so a note made with a live-only model can be
// fixed without downloading a batch model. The transcript is left as it was
// when only one voice is found.
export function useSeparateSpeakers(sessionId: string) {
  const runBatch = useRunBatch(sessionId);
  const handleBatchFailed = useListener((state) => state.handleBatchFailed);

  return useCallback(
    async (transcriptId: string) => {
      const audio = await fsSyncCommands.audioPath(sessionId);
      if (audio.status === "error") {
        toast.error(t`Recording not found. It may have been deleted.`, {
          id: `transcript-separate-audio-missing-${sessionId}`,
        });
        return;
      }

      try {
        const transcript = await getTranscriptRecord(transcriptId);
        if (!transcript?.words.length) return;
        const result = await runBatch(audio.data, {
          ...LOCAL_DIARIZATION_TARGET,
          diarizeWords: transcript.words,
          phase: "refining_speakers",
          // The note's only capture starts the recording.
          promotion: {
            scope: "current_capture",
            audioOffsetMs: 0,
            replaceTranscriptId: transcript.id,
            startedAt: transcript.startedAt,
          },
        });
        if (result.separated === false) {
          toast.info(t`Only one voice was found`, {
            id: `transcript-separate-one-voice-${sessionId}`,
            description: t`The transcript was not changed.`,
          });
        }
      } catch (error) {
        if (isStoppedTranscriptionError(error)) {
          return;
        }
        const msg = error instanceof Error ? error.message : String(error);
        handleBatchFailed(sessionId, msg);
        toast.error(t`Speakers could not be separated`, {
          id: `transcript-separate-failed-${sessionId}`,
          description: msg,
        });
      }
    },
    [handleBatchFailed, runBatch, sessionId],
  );
}
