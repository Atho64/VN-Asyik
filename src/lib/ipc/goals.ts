/** Custom reading goals and their validated Rust boundary. */
import { z } from "zod";
import { call } from "./client";

export const goalKindSchema = z.enum(["playtime_minutes", "vns_completed", "manual"]);
export type GoalKind = z.infer<typeof goalKindSchema>;

export const goalPeriodSchema = z.enum(["daily", "weekly", "monthly", "yearly", "all_time"]);
export type GoalPeriod = z.infer<typeof goalPeriodSchema>;

export const readingGoalSchema = z.object({
  meta: z.object({
    id: z.string(),
    updated_at: z.number(),
    device_id_device: z.string(),
    deleted: z.boolean(),
  }),
  title: z.string(),
  kind: goalKindSchema,
  period: goalPeriodSchema,
  target_value: z.number(),
  manual_progress: z.number().default(0),
  unit: z.string().nullable().default(null),
  pinned: z.boolean().default(false),
  created_at: z.number(),
});
export type ReadingGoal = z.infer<typeof readingGoalSchema>;

export const readingGoalViewSchema = z.object({
  goal: readingGoalSchema,
  currentValue: z.number(),
});
export type ReadingGoalView = z.infer<typeof readingGoalViewSchema>;

export interface SaveReadingGoalInput {
  id: string | null;
  title: string;
  kind: GoalKind;
  period: GoalPeriod;
  targetValue: number;
  manualProgress: number;
  unit: string | null;
  pinned: boolean;
  expectedUpdatedAt: number | null;
}

export const goalsApi = {
  list: () => call("reading_goals_list", z.array(readingGoalViewSchema)),
  save: (input: SaveReadingGoalInput) =>
    call("reading_goal_save", readingGoalViewSchema, { input }),
  setPinned: (id: string, pinned: boolean) =>
    call("reading_goal_set_pinned", readingGoalViewSchema, { id, pinned }),
  setProgress: (id: string, progress: number) =>
    call("reading_goal_set_progress", readingGoalViewSchema, { id, progress }),
  remove: (id: string) => call("reading_goal_delete", z.null(), { id }),
};
