import { Trans, useLingui } from "@lingui/react/macro";
import { useForm } from "@tanstack/react-form";
import { useState } from "react";

import type { Replacement } from "@anlg/plugin-dictation";
import {
  ArrowRight,
  Check,
  MinusCircle,
  PencilSimple,
  Plus,
  X,
} from "@anlg/ui/components/icons";
import { Button } from "@anlg/ui/components/ui/button";
import { Input } from "@anlg/ui/components/ui/input";

// The bounds dictation applies replacements within (dictation-core `replace.rs`).
const MAX_REPLACEMENTS = 100;
const MAX_FROM_LENGTH = 100;
const MAX_TO_LENGTH = 2_000;

type Problem = "duplicate" | "fromTooLong" | "toTooLong" | "full";

export function DictionaryReplacements({
  replacements,
  onSave,
}: {
  replacements: Replacement[];
  onSave: (value: string) => void;
}) {
  const { t } = useLingui();
  const save = (next: Replacement[]) => onSave(JSON.stringify(next));

  const form = useForm({
    defaultValues: { from: "", to: "" },
    onSubmit: ({ value }) => {
      const next = { from: value.from.trim(), to: value.to.trim() };
      if (!next.from || !next.to || problem(replacements, next, null)) return;
      save([...replacements, next]);
      form.reset();
    },
  });

  return (
    <section className="flex flex-col gap-3">
      <div>
        <h3 className="text-sm font-medium">
          <Trans>Replacements</Trans>
        </h3>
        <p className="text-muted-foreground text-xs">
          <Trans>
            When dictation hears a phrase, it writes your text instead, like "my
            email" for your address.
          </Trans>
        </p>
      </div>
      <form
        className="flex flex-col gap-1"
        onSubmit={(event) => {
          event.preventDefault();
          event.stopPropagation();
          void form.handleSubmit();
        }}
      >
        <div className="flex items-center gap-2">
          <form.Field name="from">
            {(field) => (
              <Input
                className="h-8 min-w-0 flex-1"
                placeholder={t`When I say`}
                aria-label={t`When I say`}
                value={field.state.value}
                onChange={(event) => field.handleChange(event.target.value)}
                onBlur={field.handleBlur}
              />
            )}
          </form.Field>
          <ArrowRight
            className="text-muted-foreground size-4 shrink-0"
            aria-hidden="true"
          />
          <form.Field name="to">
            {(field) => (
              <Input
                className="h-8 min-w-0 flex-1"
                placeholder={t`Write`}
                aria-label={t`Write`}
                value={field.state.value}
                onChange={(event) => field.handleChange(event.target.value)}
                onBlur={field.handleBlur}
              />
            )}
          </form.Field>
          <form.Subscribe selector={(state) => state.values}>
            {(values) => (
              <Button
                type="submit"
                variant="outline"
                size="sm"
                disabled={
                  !values.from.trim() ||
                  !values.to.trim() ||
                  problem(replacements, values, null) !== null
                }
              >
                <Plus className="size-3.5" aria-hidden="true" />
                <Trans>Add</Trans>
              </Button>
            )}
          </form.Subscribe>
        </div>
        <form.Subscribe selector={(state) => state.values}>
          {(values) => (
            <ProblemMessage problem={problem(replacements, values, null)} />
          )}
        </form.Subscribe>
      </form>
      {replacements.length > 0 && (
        <div className="bg-card shadow-raised divide-border-muted divide-y overflow-hidden rounded-lg">
          {replacements.map((replacement) => (
            <ReplacementRow
              key={replacement.from}
              replacement={replacement}
              replacements={replacements}
              onEdit={(next) =>
                save(
                  replacements.map((entry) =>
                    entry.from === replacement.from ? next : entry,
                  ),
                )
              }
              onRemove={() =>
                save(
                  replacements.filter(
                    (entry) => entry.from !== replacement.from,
                  ),
                )
              }
            />
          ))}
        </div>
      )}
    </section>
  );
}

function ReplacementRow({
  replacement,
  replacements,
  onEdit,
  onRemove,
}: {
  replacement: Replacement;
  replacements: Replacement[];
  onEdit: (next: Replacement) => void;
  onRemove: () => void;
}) {
  const { t } = useLingui();
  const [editing, setEditing] = useState(false);
  const { from, to } = replacement;

  if (editing) {
    return (
      <EditReplacement
        replacement={replacement}
        replacements={replacements}
        onSave={(next) => {
          onEdit(next);
          setEditing(false);
        }}
        onCancel={() => setEditing(false)}
      />
    );
  }

  return (
    <div className="group flex min-h-12 items-center justify-between gap-3 py-3 pr-3 pl-4">
      <span className="flex min-w-0 items-center gap-2 text-sm">
        <span className="truncate">{from}</span>
        <ArrowRight
          className="text-muted-foreground size-3.5 shrink-0"
          aria-hidden="true"
        />
        <span className="truncate">{to}</span>
      </span>
      <div className="flex items-center">
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="text-muted-foreground hover:text-foreground size-7 shrink-0 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
          onClick={() => setEditing(true)}
          aria-label={t`Edit replacement: ${from}`}
        >
          <PencilSimple className="size-4" />
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="text-muted-foreground hover:text-foreground size-7 shrink-0 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
          onClick={onRemove}
          aria-label={t`Remove replacement: ${from}`}
        >
          <MinusCircle className="size-4" />
        </Button>
      </div>
    </div>
  );
}

function EditReplacement({
  replacement,
  replacements,
  onSave,
  onCancel,
}: {
  replacement: Replacement;
  replacements: Replacement[];
  onSave: (next: Replacement) => void;
  onCancel: () => void;
}) {
  const { t } = useLingui();
  const form = useForm({
    defaultValues: replacement,
    onSubmit: ({ value }) => {
      const next = { from: value.from.trim(), to: value.to.trim() };
      if (!next.from || !next.to) return;
      if (problem(replacements, next, replacement.from)) return;
      onSave(next);
    },
  });
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  };

  return (
    <form
      className="flex flex-col gap-1 py-2 pr-3 pl-4"
      onSubmit={(event) => {
        event.preventDefault();
        event.stopPropagation();
        void form.handleSubmit();
      }}
    >
      <div className="flex min-h-8 items-center gap-2">
        <form.Field name="from">
          {(field) => (
            <Input
              autoFocus
              className="h-7 min-w-0 flex-1"
              aria-label={t`When I say`}
              value={field.state.value}
              onChange={(event) => field.handleChange(event.target.value)}
              onKeyDown={onKeyDown}
            />
          )}
        </form.Field>
        <ArrowRight
          className="text-muted-foreground size-3.5 shrink-0"
          aria-hidden="true"
        />
        <form.Field name="to">
          {(field) => (
            <Input
              className="h-7 min-w-0 flex-1"
              aria-label={t`Write`}
              value={field.state.value}
              onChange={(event) => field.handleChange(event.target.value)}
              onKeyDown={onKeyDown}
            />
          )}
        </form.Field>
        <form.Subscribe selector={(state) => state.values}>
          {(values) => (
            <Button
              type="submit"
              variant="ghost"
              size="icon"
              className="text-muted-foreground hover:text-foreground size-7 shrink-0"
              disabled={
                !values.from.trim() ||
                !values.to.trim() ||
                problem(replacements, values, replacement.from) !== null
              }
              aria-label={t`Save`}
            >
              <Check className="size-4" />
            </Button>
          )}
        </form.Subscribe>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="text-muted-foreground hover:text-foreground size-7 shrink-0"
          onClick={onCancel}
          aria-label={t`Cancel`}
        >
          <X className="size-4" />
        </Button>
      </div>
      <form.Subscribe selector={(state) => state.values}>
        {(values) => (
          <ProblemMessage
            problem={problem(replacements, values, replacement.from)}
          />
        )}
      </form.Subscribe>
    </form>
  );
}

function ProblemMessage({ problem }: { problem: Problem | null }) {
  if (!problem) return null;
  return (
    <p role="alert" className="text-destructive text-xs">
      {problem === "duplicate" ? (
        <Trans>There is already a replacement for this phrase.</Trans>
      ) : problem === "fromTooLong" ? (
        <Trans>The phrase can be at most 100 characters.</Trans>
      ) : problem === "toTooLong" ? (
        <Trans>The text to write can be at most 2,000 characters.</Trans>
      ) : (
        <Trans>You can keep up to 100 replacements.</Trans>
      )}
    </p>
  );
}

/** Why `value` can't replace the one said as `editing` (null when adding), if it can't. */
function problem(
  replacements: Replacement[],
  value: Replacement,
  editing: string | null,
): Problem | null {
  const from = value.from.trim();
  const key = from.toLocaleLowerCase();
  const duplicate = replacements.some(
    (entry) =>
      entry.from !== editing && entry.from.trim().toLocaleLowerCase() === key,
  );
  if (from && duplicate) return "duplicate";
  if ([...from].length > MAX_FROM_LENGTH) return "fromTooLong";
  if ([...value.to.trim()].length > MAX_TO_LENGTH) return "toTooLong";
  if (!editing && replacements.length >= MAX_REPLACEMENTS) return "full";
  return null;
}

/** Terms suggested from the person's own corrections, kept until they add or dismiss them. */
export function DictionarySuggestions({
  suggestions,
  onAdd,
  onDismiss,
}: {
  suggestions: string[];
  onAdd: (term: string) => void;
  onDismiss: (term: string) => void;
}) {
  const { t } = useLingui();
  if (suggestions.length === 0) return null;

  return (
    <section className="flex flex-col gap-3">
      <div>
        <h3 className="text-sm font-medium">
          <Trans>Suggested from your corrections</Trans>
        </h3>
        <p className="text-muted-foreground text-xs">
          <Trans>
            Words you fixed after dictating. Add them so dictation spells them
            right next time.
          </Trans>
        </p>
      </div>
      <div className="bg-card shadow-raised divide-border-muted divide-y overflow-hidden rounded-lg">
        {suggestions.map((term) => (
          <div
            key={term}
            className="flex min-h-12 items-center justify-between gap-3 py-3 pr-3 pl-4"
          >
            <span className="text-sm">{term}</span>
            <div className="flex items-center gap-1">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => onAdd(term)}
                aria-label={t`Add ${term}`}
              >
                <Trans>Add</Trans>
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => onDismiss(term)}
                aria-label={t`Dismiss ${term}`}
              >
                <Trans>Dismiss</Trans>
              </Button>
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}
