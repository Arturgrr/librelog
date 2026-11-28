/**
 * The narration's lost and back notices for one recording at a time. A
 * flapping microphone raises the lost notice once, and the back notice only
 * after the microphone stayed back for `settleMs`.
 */
export function narrationNotices(
  show: { lost: () => void; back: () => void },
  settleMs = 2_000,
) {
  let lost = false;
  let pending: ReturnType<typeof setTimeout> | undefined;
  const cancel = () => {
    clearTimeout(pending);
    pending = undefined;
  };
  return {
    heard(heard: boolean) {
      cancel();
      if (!heard) {
        if (!lost) show.lost();
        lost = true;
        return;
      }
      if (!lost) return;
      pending = setTimeout(() => {
        pending = undefined;
        lost = false;
        show.back();
      }, settleMs);
    },
    reset() {
      cancel();
      lost = false;
    },
  };
}
