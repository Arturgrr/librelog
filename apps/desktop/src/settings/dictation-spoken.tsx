import { Trans } from "@lingui/react/macro";

import { useSetSettingValue } from "~/settings/queries";
import { SettingSwitchRow } from "~/settings/setting-row";
import { useConfigValue } from "~/shared/config";

/**
 * Spoken commands applied on this computer without an AI model, each off until
 * turned on: a phrase like "new line" can also be meant as words.
 */
export function DictationSpokenSettings() {
  const fillers = useConfigValue("dictation_remove_fillers");
  const setFillers = useSetSettingValue("dictation_remove_fillers");
  const formatting = useConfigValue("dictation_spoken_formatting");
  const setFormatting = useSetSettingValue("dictation_spoken_formatting");
  const symbols = useConfigValue("dictation_spoken_symbols");
  const setSymbols = useSetSettingValue("dictation_spoken_symbols");

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h3 className="mb-1 text-sm font-medium">
          <Trans>Spoken commands</Trans>
        </h3>
        <p className="text-muted-foreground text-xs">
          <Trans>
            Applied instantly on this computer, without AI, when you dictate in
            English or Portuguese.
          </Trans>
        </p>
      </div>
      <SettingSwitchRow
        title={<Trans>Remove hesitations</Trans>}
        description={
          <Trans>
            Drops sounds like "um", "uh", "éé" and "hã" from what you dictate.
            Real words, such as "é" in "ele é bom", are kept.
          </Trans>
        }
        checked={fillers}
        onChange={setFillers}
      />
      <SettingSwitchRow
        title={<Trans>Spoken formatting</Trans>}
        description={
          <Trans>
            Say "new line", "new paragraph", "open quote" … "close quote" or
            "open parenthesis" … "close parenthesis" ("nova linha", "novo
            parágrafo", "abre aspas" … "fecha aspas", "abre parênteses" … "fecha
            parênteses") to type them.
          </Trans>
        }
        checked={formatting}
        onChange={setFormatting}
      />
      <SettingSwitchRow
        title={<Trans>Spoken symbols</Trans>}
        description={
          <Trans>
            Say "at sign" or "arroba" for @, "hashtag" for #, "underline" in
            Portuguese for _, and "dot com" or "ponto com" for .com, joined to
            the words next to them.
          </Trans>
        }
        checked={symbols}
        onChange={setSymbols}
      />
    </div>
  );
}
