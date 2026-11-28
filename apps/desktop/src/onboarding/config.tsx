import { platform } from "@tauri-apps/plugin-os";

import type { SectionStatus } from "./shared";

import { captureAvailable } from "~/capture/system";
import { isAppStoreBuild } from "~/shared/app-store";

export type OnboardingStep =
  | "permissions"
  | "transcription"
  | "intelligence"
  | "dictation"
  | "capture"
  | "final";

const STEPS_MACOS: OnboardingStep[] = [
  "permissions",
  "transcription",
  "intelligence",
  "dictation",
  "capture",
  "final",
];
const STEPS_OTHER: OnboardingStep[] = [
  "transcription",
  "intelligence",
  "dictation",
  "final",
];

const SAVED_STEP_KEY = "librelog.onboarding-step";
const NO_TRANSCRIPTION_KEY = "librelog.onboarding-no-transcription";

export function getOnboardingSteps(): OnboardingStep[] {
  if (platform() !== "macos") {
    // Capture also runs on Linux, before the last step.
    return captureAvailable()
      ? [...STEPS_OTHER.slice(0, -1), "capture", "final"]
      : STEPS_OTHER;
  }
  // Capture is not in the App Store build.
  return isAppStoreBuild()
    ? STEPS_MACOS.filter((step) => step !== "capture")
    : STEPS_MACOS;
}

export function getInitialStep(): OnboardingStep {
  const steps = getOnboardingSteps();
  const saved = readSavedStep();
  return saved && steps.includes(saved) ? saved : steps[0];
}

// Choices live in settings, so a restart can land after a step whose choice
// was cleared in the meantime; that step is shown again until it is made.
export function limitToMissingSelection(
  step: OnboardingStep,
  missing: OnboardingStep | null,
): OnboardingStep {
  const steps = getOnboardingSteps();
  return missing && steps.indexOf(step) > steps.indexOf(missing)
    ? missing
    : step;
}

export function saveOnboardingStep(step: OnboardingStep | null) {
  try {
    if (step) {
      localStorage.setItem(SAVED_STEP_KEY, step);
    } else {
      localStorage.removeItem(SAVED_STEP_KEY);
      localStorage.removeItem(NO_TRANSCRIPTION_KEY);
    }
  } catch (error) {
    console.error("Failed to save the onboarding step", error);
  }
}

// Kept across a restart, so someone who went on without a transcription
// model isn't sent back to choose one.
export function saveContinuedWithoutTranscription(value: boolean) {
  try {
    if (value) {
      localStorage.setItem(NO_TRANSCRIPTION_KEY, "1");
    } else {
      localStorage.removeItem(NO_TRANSCRIPTION_KEY);
    }
  } catch (error) {
    console.error("Failed to save the transcription choice", error);
  }
}

export function readContinuedWithoutTranscription() {
  try {
    return localStorage.getItem(NO_TRANSCRIPTION_KEY) === "1";
  } catch {
    return false;
  }
}

function readSavedStep(): OnboardingStep | null {
  try {
    return localStorage.getItem(SAVED_STEP_KEY) as OnboardingStep | null;
  } catch {
    return null;
  }
}

export function getPrevStep(
  currentStep: OnboardingStep,
): OnboardingStep | null {
  const steps = getOnboardingSteps();
  const idx = steps.indexOf(currentStep);
  return idx > 0 ? steps[idx - 1] : null;
}

export function getNextStep(
  currentStep: OnboardingStep,
): OnboardingStep | null {
  const steps = getOnboardingSteps();
  const idx = steps.indexOf(currentStep);
  return idx < steps.length - 1 ? steps[idx + 1] : null;
}

export function getStepStatus(
  step: OnboardingStep,
  currentStep: OnboardingStep,
): SectionStatus | null {
  const steps = getOnboardingSteps();
  const stepIdx = steps.indexOf(step);
  if (stepIdx === -1) return null;
  const currentIdx = steps.indexOf(currentStep);
  if (stepIdx < currentIdx) return "completed";
  if (stepIdx === currentIdx) return "active";
  return "upcoming";
}
