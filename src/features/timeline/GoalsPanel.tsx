import { useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { EmptyState } from "@/components/EmptyState";
import { ProgressRing } from "@/components/ProgressRing";
import {
  goalsApi,
  type GoalKind,
  type GoalPeriod,
  type ReadingGoalView,
} from "@/lib/ipc/goals";
import { MS_PER_MINUTE, formatDurationLong } from "@/lib/time";
import { requestAchievementsRefresh } from "@/lib/ipc/achievements";

interface GoalDraft {
  id: string | null;
  expectedUpdatedAt: number | null;
  title: string;
  kind: GoalKind;
  period: GoalPeriod;
  hours: string;
  minutes: string;
  targetCount: string;
  manualProgress: string;
  unit: string;
  pinned: boolean;
}

const EMPTY_DRAFT: GoalDraft = {
  id: null,
  expectedUpdatedAt: null,
  title: "",
  kind: "vns_completed",
  period: "yearly",
  hours: "2",
  minutes: "0",
  targetCount: "10",
  manualProgress: "0",
  unit: "",
  pinned: false,
};

function draftFromView(view: ReadingGoalView): GoalDraft {
  const { goal } = view;
  const totalMinutes = goal.kind === "playtime_minutes" ? goal.target_value : 120;
  return {
    id: goal.meta.id,
    expectedUpdatedAt: goal.meta.updated_at,
    title: goal.title,
    kind: goal.kind,
    period: goal.period,
    hours: String(Math.floor(totalMinutes / 60)),
    minutes: String(totalMinutes % 60),
    targetCount: String(goal.kind === "playtime_minutes" ? 10 : goal.target_value),
    manualProgress: String(goal.manual_progress),
    unit: goal.unit ?? "",
    pinned: goal.pinned,
  };
}

function periodLabelKey(period: GoalPeriod) {
  switch (period) {
    case "daily":
      return "goals.periodDaily" as const;
    case "weekly":
      return "goals.periodWeekly" as const;
    case "monthly":
      return "goals.periodMonthly" as const;
    case "yearly":
      return "goals.periodYearly" as const;
    case "all_time":
      return "goals.periodAllTime" as const;
  }
}

function kindLabelKey(kind: GoalKind) {
  switch (kind) {
    case "playtime_minutes":
      return "goals.kindPlaytime" as const;
    case "vns_completed":
      return "goals.kindVns" as const;
    case "manual":
      return "goals.kindManual" as const;
  }
}

function formatGoalAmount(
  kind: GoalKind,
  value: number,
  unit: string | null,
  locale: string,
  vnUnit: string,
): string {
  if (kind === "playtime_minutes") {
    if (value <= 0) return formatDurationLong(0, locale);
    return formatDurationLong(value * MS_PER_MINUTE, locale);
  }
  const suffix = kind === "vns_completed" ? vnUnit : (unit ?? "").trim();
  return suffix ? `${value} ${suffix}` : String(value);
}

/** Custom reading goals panel with automatic playtime/VN tracking and manual targets. */
export function GoalsPanel() {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);

  const [items, setItems] = useState<ReadingGoalView[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [draft, setDraft] = useState<GoalDraft | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const load = () => {
    setFailed(false);
    goalsApi
      .list()
      .then((rows) => setItems(rows))
      .catch(() => setFailed(true));
  };

  useEffect(() => {
    let cancelled = false;
    goalsApi
      .list()
      .then((rows) => {
        if (!cancelled) setItems(rows);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const applyPreset = (
    title: string,
    kind: GoalKind,
    period: GoalPeriod,
    target: number,
  ) => {
    setFormError(null);
    setDraft({
      ...EMPTY_DRAFT,
      title,
      kind,
      period,
      hours: kind === "playtime_minutes" ? String(Math.floor(target / 60)) : "2",
      minutes: kind === "playtime_minutes" ? String(target % 60) : "0",
      targetCount: kind === "playtime_minutes" ? "10" : String(target),
    });
  };

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!draft) return;

    const title = draft.title.trim();
    if (!title) {
      setFormError(t("goals.validationTitle"));
      return;
    }

    const targetValue =
      draft.kind === "playtime_minutes"
        ? Math.max(0, Number.parseInt(draft.hours || "0", 10) || 0) * 60 +
          Math.max(0, Number.parseInt(draft.minutes || "0", 10) || 0)
        : Number.parseInt(draft.targetCount || "0", 10) || 0;

    if (targetValue <= 0) {
      setFormError(t("goals.validationTarget"));
      return;
    }

    const manualProgress =
      draft.kind === "manual"
        ? Math.max(0, Number.parseInt(draft.manualProgress || "0", 10) || 0)
        : 0;

    setSaving(true);
    setFormError(null);
    try {
      await goalsApi.save({
        id: draft.id,
        expectedUpdatedAt: draft.expectedUpdatedAt,
        title,
        kind: draft.kind,
        period: draft.period,
        targetValue,
        manualProgress,
        unit: draft.kind === "manual" && draft.unit.trim() ? draft.unit.trim() : null,
        pinned: draft.pinned,
      });
      requestAchievementsRefresh();
      setDraft(null);
      pushToast("success", t("goals.saved"));
      load();
    } catch {
      setFormError(t("common.error"));
    } finally {
      setSaving(false);
    }
  };

  const handleTogglePin = async (view: ReadingGoalView) => {
    const id = view.goal.meta.id;
    setBusyId(id);
    try {
      await goalsApi.setPinned(id, !view.goal.pinned);
      load();
    } catch {
      pushToast("error", t("common.error"));
    } finally {
      setBusyId(null);
    }
  };

  const handleStepProgress = async (view: ReadingGoalView, delta: number) => {
    const id = view.goal.meta.id;
    const next = Math.max(0, view.currentValue + delta);
    if (next === view.currentValue) return;
    setBusyId(id);
    try {
      const updated = await goalsApi.setProgress(id, next);
      setItems((prev) =>
        prev ? prev.map((row) => (row.goal.meta.id === id ? updated : row)) : prev,
      );
    } catch {
      pushToast("error", t("common.error"));
    } finally {
      setBusyId(null);
    }
  };

  const handleDelete = async (id: string) => {
    setBusyId(id);
    try {
      await goalsApi.remove(id);
      setConfirmDeleteId(null);
      setItems((prev) => (prev ? prev.filter((row) => row.goal.meta.id !== id) : prev));
      pushToast("info", t("goals.deleted"));
    } catch {
      pushToast("error", t("common.error"));
    } finally {
      setBusyId(null);
    }
  };

  if (failed && items === null) {
    return (
      <EmptyState
        title={t("goals.errorTitle")}
        body={t("common.error")}
        action={<Button onClick={load}>{t("common.retry")}</Button>}
      />
    );
  }

  if (items === null) {
    return (
      <div aria-busy="true" aria-label={t("goals.loading")}>
        <div className="skeleton" style={{ height: 120 }} />
      </div>
    );
  }

  const reachedCount = items.filter(
    (row) => row.currentValue >= row.goal.target_value,
  ).length;

  return (
    <section className="goals" aria-label={t("timeline.tabGoals")}>
      <div className="goals__bar">
        <span className="caption">
          {items.length > 0
            ? t("goals.summary", { reached: reachedCount, total: items.length })
            : ""}
        </span>
        {draft === null ? (
          <Button
            variant="primary"
            onClick={() => {
              setFormError(null);
              setDraft({ ...EMPTY_DRAFT });
            }}
          >
            {t("goals.new")}
          </Button>
        ) : null}
      </div>

      {draft !== null ? (
        <form className="reading__form" onSubmit={(e) => void handleSubmit(e)}>
          {draft.id === null ? (
            <div>
              <span className="caption">{t("goals.presetsLabel")}</span>
              <div className="goals__presets">
                <Button
                  type="button"
                  onClick={() =>
                    applyPreset(
                      t("goals.presetDaily2h"),
                      "playtime_minutes",
                      "daily",
                      120,
                    )
                  }
                >
                  {t("goals.presetDaily2h")}
                </Button>
                <Button
                  type="button"
                  onClick={() =>
                    applyPreset(
                      t("goals.presetWeekly10h"),
                      "playtime_minutes",
                      "weekly",
                      600,
                    )
                  }
                >
                  {t("goals.presetWeekly10h")}
                </Button>
                <Button
                  type="button"
                  onClick={() =>
                    applyPreset(
                      t("goals.presetYearly10vn"),
                      "vns_completed",
                      "yearly",
                      10,
                    )
                  }
                >
                  {t("goals.presetYearly10vn")}
                </Button>
                <Button
                  type="button"
                  onClick={() =>
                    applyPreset(
                      t("goals.presetMonthly2vn"),
                      "vns_completed",
                      "monthly",
                      2,
                    )
                  }
                >
                  {t("goals.presetMonthly2vn")}
                </Button>
              </div>
            </div>
          ) : null}

          <label className="field">
            <span className="field__label">{t("goals.fieldTitle")}</span>
            <input
              className="input"
              type="text"
              maxLength={120}
              value={draft.title}
              placeholder={t("goals.titlePlaceholder")}
              onChange={(e) => setDraft({ ...draft, title: e.target.value })}
            />
          </label>

          <div className="goals__form-grid">
            <label className="field">
              <span className="field__label">{t("goals.fieldKind")}</span>
              <select
                className="select"
                value={draft.kind}
                onChange={(e) =>
                  setDraft({ ...draft, kind: e.target.value as GoalKind })
                }
              >
                <option value="playtime_minutes">{t("goals.kindPlaytime")}</option>
                <option value="vns_completed">{t("goals.kindVns")}</option>
                <option value="manual">{t("goals.kindManual")}</option>
              </select>
            </label>

            <label className="field">
              <span className="field__label">{t("goals.fieldPeriod")}</span>
              <select
                className="select"
                value={draft.period}
                onChange={(e) =>
                  setDraft({ ...draft, period: e.target.value as GoalPeriod })
                }
              >
                <option value="daily">{t("goals.periodDaily")}</option>
                <option value="weekly">{t("goals.periodWeekly")}</option>
                <option value="monthly">{t("goals.periodMonthly")}</option>
                <option value="yearly">{t("goals.periodYearly")}</option>
                <option value="all_time">{t("goals.periodAllTime")}</option>
              </select>
              <span className="field__hint">{t("goals.periodHint")}</span>
            </label>
          </div>

          {draft.kind === "playtime_minutes" ? (
            <div className="goals__form-grid">
              <label className="field">
                <span className="field__label">{t("goals.fieldHours")}</span>
                <input
                  className="input"
                  type="number"
                  min={0}
                  max={10000}
                  value={draft.hours}
                  onChange={(e) => setDraft({ ...draft, hours: e.target.value })}
                />
              </label>
              <label className="field">
                <span className="field__label">{t("goals.fieldMinutes")}</span>
                <input
                  className="input"
                  type="number"
                  min={0}
                  max={59}
                  value={draft.minutes}
                  onChange={(e) => setDraft({ ...draft, minutes: e.target.value })}
                />
              </label>
            </div>
          ) : (
            <div className="goals__form-grid">
              <label className="field">
                <span className="field__label">{t("goals.fieldTarget")}</span>
                <input
                  className="input"
                  type="number"
                  min={1}
                  max={1000000}
                  value={draft.targetCount}
                  onChange={(e) => setDraft({ ...draft, targetCount: e.target.value })}
                />
              </label>
              {draft.kind === "manual" ? (
                <label className="field">
                  <span className="field__label">{t("goals.fieldUnit")}</span>
                  <input
                    className="input"
                    type="text"
                    maxLength={32}
                    value={draft.unit}
                    placeholder={t("goals.unitPlaceholder")}
                    onChange={(e) => setDraft({ ...draft, unit: e.target.value })}
                  />
                </label>
              ) : null}
            </div>
          )}

          {draft.kind === "manual" ? (
            <label className="field">
              <span className="field__label">{t("goals.fieldProgress")}</span>
              <input
                className="input"
                type="number"
                min={0}
                max={1000000}
                value={draft.manualProgress}
                onChange={(e) => setDraft({ ...draft, manualProgress: e.target.value })}
              />
            </label>
          ) : null}

          <label className="reading__check">
            <input
              type="checkbox"
              checked={draft.pinned}
              onChange={(e) => setDraft({ ...draft, pinned: e.target.checked })}
            />
            <span>{t("goals.fieldPin")}</span>
          </label>

          {formError ? (
            <p className="notice notice--danger" role="alert">
              {formError}
            </p>
          ) : null}

          <div className="reading__actions">
            <Button type="submit" variant="primary" disabled={saving}>
              {draft.id ? t("goals.update") : t("goals.save")}
            </Button>
            <Button type="button" onClick={() => setDraft(null)} disabled={saving}>
              {t("common.cancel")}
            </Button>
          </div>
        </form>
      ) : null}

      {items.length === 0 && draft === null ? (
        <EmptyState
          title={t("goals.emptyTitle")}
          body={t("goals.emptyBody")}
          action={
            <Button
              variant="primary"
              onClick={() => {
                setFormError(null);
                setDraft({ ...EMPTY_DRAFT });
              }}
            >
              {t("goals.new")}
            </Button>
          }
        />
      ) : (
        <ul className="goals__list">
          {items.map((view) => {
            const { goal, currentValue } = view;
            const id = goal.meta.id;
            const ratio =
              goal.target_value > 0
                ? Math.min(1, Math.max(0, currentValue / goal.target_value))
                : 0;
            const percent = Math.round(ratio * 100);
            const reached = currentValue >= goal.target_value;
            const remaining = Math.max(0, goal.target_value - currentValue);
            const vnUnit = t("goals.unitVn");
            const currentText = formatGoalAmount(
              goal.kind,
              currentValue,
              goal.unit,
              locale,
              vnUnit,
            );
            const targetText = formatGoalAmount(
              goal.kind,
              goal.target_value,
              goal.unit,
              locale,
              vnUnit,
            );
            const remainingText = formatGoalAmount(
              goal.kind,
              remaining,
              goal.unit,
              locale,
              vnUnit,
            );
            const busy = busyId === id;

            return (
              <li
                key={id}
                className={`goals__card${reached ? " goals__card--reached" : ""}`}
              >
                <div className="goals__head">
                  <ProgressRing
                    progress={ratio}
                    size={48}
                    strokeWidth={3.5}
                    label={`${percent}%`}
                    title={`${goal.title}: ${percent}%`}
                  />
                  <div className="goals__title-wrap">
                    <h3 className="goals__title">{goal.title}</h3>
                    <div className="goals__badges">
                      <Chip>{t(periodLabelKey(goal.period))}</Chip>
                      <span className="caption">{t(kindLabelKey(goal.kind))}</span>
                      {goal.pinned ? <Chip>{t("goals.pinned")}</Chip> : null}
                      {reached ? <Chip>{t("goals.reached")}</Chip> : null}
                    </div>
                  </div>
                </div>

                <div className="goals__progress-row">
                  <span className="goals__figure">
                    {currentText} / {targetText}
                  </span>
                  <span className="caption">
                    {reached
                      ? t("goals.reached")
                      : t("goals.remaining", { left: remainingText })}
                  </span>
                </div>

                <div className="goals__track" aria-hidden="true">
                  <div className="goals__fill" style={{ width: `${percent}%` }} />
                </div>

                <div className="goals__footer">
                  {goal.kind === "manual" ? (
                    <div className="goals__stepper">
                      <Button
                        type="button"
                        disabled={busy || currentValue <= 0}
                        aria-label={t("goals.decrease")}
                        onClick={() => void handleStepProgress(view, -1)}
                      >
                        -1
                      </Button>
                      <Button
                        type="button"
                        disabled={busy}
                        aria-label={t("goals.increase")}
                        onClick={() => void handleStepProgress(view, 1)}
                      >
                        +1
                      </Button>
                    </div>
                  ) : null}

                  <div className="goals__actions">
                    <Button
                      type="button"
                      disabled={busy}
                      onClick={() => void handleTogglePin(view)}
                    >
                      {goal.pinned ? t("goals.unpin") : t("goals.pin")}
                    </Button>
                    <Button
                      type="button"
                      disabled={busy}
                      onClick={() => {
                        setFormError(null);
                        setConfirmDeleteId(null);
                        setDraft(draftFromView(view));
                      }}
                    >
                      {t("goals.editAction")}
                    </Button>
                    {confirmDeleteId === id ? (
                      <>
                        <Button
                          type="button"
                          variant="danger"
                          disabled={busy}
                          onClick={() => void handleDelete(id)}
                        >
                          {t("goals.confirmDelete")}
                        </Button>
                        <Button
                          type="button"
                          disabled={busy}
                          onClick={() => setConfirmDeleteId(null)}
                        >
                          {t("common.cancel")}
                        </Button>
                      </>
                    ) : (
                      <Button
                        type="button"
                        disabled={busy}
                        onClick={() => setConfirmDeleteId(id)}
                      >
                        {t("goals.deleteAction")}
                      </Button>
                    )}
                  </div>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
