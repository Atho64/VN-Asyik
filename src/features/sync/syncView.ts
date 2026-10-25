/**
 * View logic for the sync screen, kept out of the component so the counting rules
 * are testable.
 */

import type { PullResult, SyncRow } from "@/lib/ipc/types";

export type Resolution = "keep_local" | "keep_remote" | "skip";

/** Decisions keyed by `vn_id`, then field key. */
export type DecisionMap = Record<string, Record<string, Resolution>>;

export interface GroupedConflict {
  row: SyncRow;
  fieldCount: number;
}

/** Pair each row with how many fields it needs decided. */
export function groupConflicts(pull: PullResult): GroupedConflict[] {
  return pull.conflicts.map((row) => ({
    row,
    fieldCount: row.conflicts.length,
  }));
}

/**
 * How many fields still have no decision.
 *
 * `skip` counts as decided: it is an explicit "leave both alone", and a screen
 * that treated it as undecided would trap the user forever.
 */
export function undecidedCount(pull: PullResult, decisions: DecisionMap): number {
  let total = 0;
  for (const row of pull.conflicts) {
    const chosen = decisions[row.vn_id] ?? {};
    for (const conflict of row.conflicts) {
      if (!chosen[conflict.field]) total += 1;
    }
  }
  return total;
}

/**
 * Apply one resolution to every remaining field.
 *
 * The conflict screen offers this because reviewing forty fields one by one is
 * not a reasonable thing to ask of anyone; explicit per-field choice stays
 * available afterwards.
 */
export function decideAll(pull: PullResult, resolution: Resolution): DecisionMap {
  const next: DecisionMap = {};
  for (const row of pull.conflicts) {
    next[row.vn_id] = {};
    for (const conflict of row.conflicts) {
      next[row.vn_id]![conflict.field] = resolution;
    }
  }
  return next;
}

export interface PullSummary {
  entries: number;
  conflicts: number;
  inSync: number;
  notImported: number;
  clean: boolean;
}

export function summarize(pull: PullResult): PullSummary {
  return {
    entries: pull.entries,
    conflicts: pull.conflicts.length,
    inSync: pull.in_sync,
    notImported: pull.not_imported.length,
    clean: pull.conflicts.length === 0,
  };
}

/** Build the payload the backend expects from the decision map. */
export function toDecisionsPayload(
  decisions: DecisionMap,
): Array<{ vn_id: string; fields: Array<{ field: string; resolution: string }> }> {
  return Object.entries(decisions).map(([vnId, fields]) => ({
    vn_id: vnId,
    fields: Object.entries(fields).map(([field, resolution]) => ({
      field,
      resolution,
    })),
  }));
}
