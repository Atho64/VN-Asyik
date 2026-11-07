import { z } from "zod";
import { call } from "./client";
import { companionSnapshotSchema, visionViewSchema } from "./companion";

export const cropSchema = z.object({
  x: z.number(),
  y: z.number(),
  width: z.number(),
  height: z.number(),
});
export type VisionCrop = z.infer<typeof cropSchema>;
const previewSchema = z.object({
  token: z.string(),
  dataUrl: z.string().startsWith("data:image/jpeg;base64,"),
  width: z.number(),
  height: z.number(),
  crop: cropSchema,
  generation: z.number(),
  view: visionViewSchema,
});
export type VisionPreview = z.infer<typeof previewSchema>;
export const visionApi = {
  windows: () =>
    call(
      "companion_vision_windows",
      z.array(z.object({ id: z.string(), title: z.string() })),
    ),
  select: (id: string, window: string, crop: VisionCrop) =>
    call("companion_vision_select", previewSchema, { id, window, crop }),
  preview: (id: string, generation: number) =>
    call("companion_vision_preview", previewSchema, { id, generation }),
  probe: (id: string) => call("companion_vision_probe", companionSnapshotSchema, { id }),
  control: (
    id: string,
    action: "watch" | "quiet" | "stop" | "dismiss",
    generation: number,
    preview: string | null = null,
  ) =>
    call("companion_vision_control", companionSnapshotSchema, {
      id,
      action,
      generation,
      preview,
    }),
  send: (id: string, preview: string, text: string) =>
    call("companion_vision_send", companionSnapshotSchema, { id, preview, text }),
};
