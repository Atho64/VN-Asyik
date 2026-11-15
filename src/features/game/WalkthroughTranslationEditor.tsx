import { useState } from "react";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import type { WalkthroughRoute } from "@/lib/ipc/types";

interface Props {
  original: WalkthroughRoute[];
  initial: WalkthroughRoute[];
  saving: boolean;
  error: string | null;
  onSave: (texts: string[]) => void;
  onCancel: () => void;
}

export function WalkthroughTranslationEditor({
  original,
  initial,
  saving,
  error,
  onSave,
  onCancel,
}: Props) {
  const { t } = useI18n();
  const [draft, setDraft] = useState(() =>
    original.map((route, index) => {
      const translated = initial[index];
      return [
        translated?.title ?? route.title,
        ...route.items.map(
          (item, itemIndex) => translated?.items[itemIndex]?.text ?? item.text,
        ),
      ];
    }),
  );
  const [selected, setSelected] = useState(0);
  const route = original[selected];
  const values = draft[selected];
  if (!route || !values) return null;
  const setText = (index: number, text: string) =>
    setDraft((current) =>
      current.map((texts, routeIndex) =>
        routeIndex === selected
          ? texts.map((value, textIndex) => (textIndex === index ? text : value))
          : texts,
      ),
    );
  const valid = draft
    .flat()
    .every(
      (text) => text.trim().length > 0 && new TextEncoder().encode(text).length <= 8192,
    );

  return (
    <form
      className="walkthrough__editor"
      aria-label={t("walkthrough.editTranslation")}
      aria-busy={saving}
      onSubmit={(event) => {
        event.preventDefault();
        if (valid) onSave(draft.flat());
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && !saving) {
          event.stopPropagation();
          onCancel();
        }
      }}
    >
      <div className="walkthrough__editor-heading">
        <h3>{t("walkthrough.editTranslation")}</h3>
        <p>{t("walkthrough.editHint")}</p>
      </div>
      <label className="walkthrough__editor-route">
        <span>{t("walkthrough.routes")}</span>
        <select
          className="select"
          value={selected}
          disabled={saving}
          onChange={(event) => setSelected(Number(event.target.value))}
        >
          {original.map((entry, index) => (
            <option key={entry.id} value={index}>
              {entry.title}
            </option>
          ))}
        </select>
      </label>
      {[
        { id: route.id, source: route.title, label: t("walkthrough.routeTitle") },
        ...route.items.map((item, index) => ({
          id: item.id,
          source: item.text,
          label: t("walkthrough.editStep", {
            number: String(index + 1),
            kind: t(`walkthrough.kind.${item.kind}`),
          }),
        })),
      ].map((entry, index) => (
        <div className="walkthrough__editor-line" key={entry.id}>
          <div className="walkthrough__editor-source">
            <span>{t("walkthrough.langOriginal")}</span>
            <p>{entry.source}</p>
          </div>
          <label>
            <span>{entry.label}</span>
            <textarea
              value={values[index] ?? ""}
              rows={2}
              maxLength={8192}
              disabled={saving}
              onChange={(event) => setText(index, event.target.value)}
              required
            />
          </label>
        </div>
      ))}
      {error ? (
        <p className="walkthrough__editor-error" role="alert">
          {error}
        </p>
      ) : null}
      <div className="walkthrough__translation-actions">
        <Button type="submit" variant="primary" disabled={saving || !valid}>
          {saving ? t("walkthrough.savingEdits") : t("walkthrough.saveEdits")}
        </Button>
        <Button onClick={onCancel} disabled={saving}>
          {t("walkthrough.cancelEdits")}
        </Button>
      </div>
    </form>
  );
}
