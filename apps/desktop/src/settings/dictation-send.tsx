import { Trans, useLingui } from "@lingui/react/macro";
import { useForm } from "@tanstack/react-form";
import { platform } from "@tauri-apps/plugin-os";

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@anlg/ui/components/ui/select";
import { Textarea } from "@anlg/ui/components/ui/textarea";

import { useSetSettingValue } from "~/settings/queries";
import { SETTING_CONTROL_CLASS, SettingRow } from "~/settings/setting-row";
import { useConfigValue } from "~/shared/config";

const MAX_APPS = 200;
const MODES = ["off", "all", "apps"] as const;

/** "Send after pasting": Return after each dictation, everywhere or in the apps listed. */
export function DictationSendSettings() {
  const { t } = useLingui();
  const stored = useConfigValue("dictation_send_after_paste");
  const mode = MODES.find((mode) => mode === stored) ?? "off";
  const setMode = useSetSettingValue("dictation_send_after_paste");
  const labels = {
    off: t`Off`,
    all: t`In every app`,
    apps: t`Only in these apps`,
  };

  return (
    <div className="flex flex-col gap-3">
      <SettingRow
        title={<Trans>Send after pasting</Trans>}
        description={
          <Trans>
            Presses Return after your dictation is inserted, so chats and coding
            agents send it. Only when the whole dictation went into the field
            that still has focus; never in password fields or after editing a
            selection by voice.
          </Trans>
        }
      >
        {(labelProps) => (
          <Select value={mode} onValueChange={setMode}>
            <SelectTrigger {...labelProps} className={SETTING_CONTROL_CLASS}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {MODES.map((option) => (
                <SelectItem key={option} value={option}>
                  {labels[option]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </SettingRow>
      {mode === "apps" && <SendApps />}
    </div>
  );
}

function SendApps() {
  const { t } = useLingui();
  const stored = useConfigValue("dictation_send_after_paste_apps");
  const setStored = useSetSettingValue("dictation_send_after_paste_apps");
  const form = useForm({
    defaultValues: { apps: stored.join("\n") },
    onSubmit: ({ value }) => {
      const apps = value.apps
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean)
        .slice(0, MAX_APPS);
      setStored(JSON.stringify(apps));
    },
  });
  const save = () => void form.handleSubmit();

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        event.stopPropagation();
        save();
      }}
    >
      <form.Field name="apps">
        {(field) => (
          <label className="flex flex-col gap-2 text-sm">
            <span>
              <Trans>Send in these apps</Trans>
            </span>
            <Textarea
              value={field.state.value}
              placeholder={
                platform() === "macos"
                  ? t`One app name or bundle ID per line`
                  : t`One app or program name per line, like Slack or code`
              }
              onChange={(event) => field.handleChange(event.target.value)}
              onBlur={() => {
                field.handleBlur();
                save();
              }}
            />
            {stored.length === 0 && (
              <span className="text-muted-foreground text-xs">
                <Trans>Nothing is sent until you list an app.</Trans>
              </span>
            )}
          </label>
        )}
      </form.Field>
    </form>
  );
}
