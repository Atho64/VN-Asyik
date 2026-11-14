/**
 * Canvas-rendered wrapped share cards (`post` 1080x1350, `story` 1080x1920) for theme-independent
 * output; draws local data URLs only because remote URLs taint the canvas.
 */

import type { Locale } from "@/i18n";
import type { YearReview } from "@/lib/ipc/types";
import { formatDuration } from "@/lib/time";

export type ShareCardFormat = "post" | "story";

export const SHARE_CARD_SIZES: Record<
  ShareCardFormat,
  { width: number; height: number }
> = {
  post: { width: 1080, height: 1350 },
  story: { width: 1080, height: 1920 },
};

const MARGIN = 72;
const DAY_MS = 86_400_000;

const DISPLAY_FONT =
  '"Inter Tight", Inter, "Segoe UI", "Noto Sans JP", "Yu Gothic", "Meiryo", system-ui, sans-serif';

const INK = "#f5f7fb";
const MUTED = "rgba(245, 247, 251, 0.64)";
const FAINT = "rgba(245, 247, 251, 0.42)";
const FALLBACK_ACCENT = "#8b7cf6";

export interface ShareCardFigure {
  value: string;
  label: string;
}

export interface ShareCardGame {
  title: string;
  countedMs: number;
}

export interface ShareCardFact {
  label: string;
  value: string;
}

/** Everything the renderer needs, already localized by the caller. */
export interface ShareCardModel {
  /** The year or `ALL TIME` badge in the top-right. */
  badge: string;
  /** Small caps above the total, e.g. `YEAR IN REVIEW`. */
  eyebrow: string;
  totalMs: number;
  /** `84 days read · 212 sessions`, already interpolated. */
  subline: string;
  figures: ShareCardFigure[];
  topGames: ShareCardGame[];
  facts: ShareCardFact[];
  /** Reading time per local hour, for the story rhythm chart. */
  hourMs: number[];
  /** The localized peak-hour sentence, e.g. `Most around 20:00 · 3h 20m`. */
  hourPeak: string;
  accent: string;
  /** A cached, pre-blurred local image, or `null` for the gradient alone. */
  backdropUrl: string | null;
  /** File name stem, without the format suffix or extension. */
  filename: string;
}

/** Static labels the model does not carry. */
export interface ShareCardLabels {
  totalTime: string;
  mostRead: string;
  footer: string;
  hourTitle: string;
}

export interface ShareCardOptions {
  model: ShareCardModel;
  labels: ShareCardLabels;
  locale: Locale;
  format: ShareCardFormat;
}

/** The file name a saved card gets. */
export function shareCardFilename(
  model: ShareCardModel,
  format: ShareCardFormat,
): string {
  return format === "story"
    ? `${model.filename}-story.png`
    : `${model.filename}.png`;
}

/** The PNG data URL the Rust save/share commands accept. */
export function shareCardDataUrl(canvas: HTMLCanvasElement): string {
  return canvas.toDataURL("image/png");
}

/** The compact hero duration, e.g. `127h 34m` / `127 j 34 mnt`. */
export function shareDuration(ms: number, locale: Locale): string {
  return formatDuration(ms, locale);
}

/** The busiest month's name, or `null` for a quiet year. */
export function shareCardMonth(review: YearReview, locale: Locale): string | null {
  if (!review.busiestMonth) return null;
  return new Intl.DateTimeFormat(locale === "id" ? "id-ID" : "en-GB", {
    month: "long",
  }).format(new Date(2024, review.busiestMonth.month - 1, 1));
}

/** The busiest local day, rendered as a short date. */
export function shareCardDay(review: YearReview, locale: Locale): string | null {
  if (!review.busiestDay) return null;
  // Day indexes encode a local calendar date as-if-UTC, the same convention as
  // the heatmap, so rendering in UTC shows the date the reader experienced.
  return new Intl.DateTimeFormat(locale === "id" ? "id-ID" : "en-GB", {
    day: "numeric",
    month: "short",
    timeZone: "UTC",
  }).format(new Date(review.busiestDay.day * DAY_MS));
}

type Ctx = CanvasRenderingContext2D;

function setFont(ctx: Ctx, size: number, weight: number): void {
  ctx.font = `${weight} ${size}px ${DISPLAY_FONT}`;
}

function roundedRect(
  ctx: Ctx,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
): void {
  const r = Math.max(0, Math.min(radius, width / 2, height / 2));
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + width, y, x + width, y + height, r);
  ctx.arcTo(x + width, y + height, x, y + height, r);
  ctx.arcTo(x, y + height, x, y, r);
  ctx.arcTo(x, y, x + width, y, r);
  ctx.closePath();
}

/** Truncate to fit, keeping an ellipsis so the cut is visible. */
export function fitText(ctx: Ctx, text: string, maxWidth: number): string {
  if (ctx.measureText(text).width <= maxWidth) return text;
  let low = 0;
  let high = text.length;
  while (low < high) {
    const mid = Math.ceil((low + high) / 2);
    const candidate = `${text.slice(0, mid)}…`;
    if (ctx.measureText(candidate).width <= maxWidth) low = mid;
    else high = mid - 1;
  }
  return low > 0 ? `${text.slice(0, low)}…` : "…";
}

function accentOf(value: string): string {
  return /^#[0-9a-f]{6}$/i.test(value.trim()) ? value.trim() : FALLBACK_ACCENT;
}

function rgba(hex: string, alpha: number): string {
  const match = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!match?.[1]) return `rgba(139, 124, 246, ${alpha})`;
  const value = match[1];
  return `rgba(${Number.parseInt(value.slice(0, 2), 16)}, ${Number.parseInt(
    value.slice(2, 4),
    16,
  )}, ${Number.parseInt(value.slice(4, 6), 16)}, ${alpha})`;
}

function loadImage(url: string): Promise<HTMLImageElement | null> {
  return new Promise((resolve) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => resolve(null);
    image.src = url;
  });
}

function drawCover(
  ctx: Ctx,
  image: HTMLImageElement,
  width: number,
  height: number,
): void {
  const scale = Math.max(width / image.width, height / image.height);
  const drawWidth = image.width * scale;
  const drawHeight = image.height * scale;
  ctx.drawImage(
    image,
    (width - drawWidth) / 2,
    (height - drawHeight) / 2,
    drawWidth,
    drawHeight,
  );
}

function drawBackground(
  ctx: Ctx,
  width: number,
  height: number,
  accent: string,
  backdrop: HTMLImageElement | null,
): void {
  const base = ctx.createLinearGradient(0, 0, 0, height);
  base.addColorStop(0, "#0b1020");
  base.addColorStop(0.55, "#0a0d18");
  base.addColorStop(1, "#070910");
  ctx.fillStyle = base;
  ctx.fillRect(0, 0, width, height);

  if (backdrop) {
    ctx.save();
    ctx.globalAlpha = 0.34;
    drawCover(ctx, backdrop, width, height);
    ctx.restore();
  }

  // A soft accent glow gives the card the same "the cover is the light source"
  // identity the app uses, without needing a sharp cover on the canvas.
  const glow = ctx.createRadialGradient(
    width * 0.82,
    height * 0.08,
    0,
    width * 0.82,
    height * 0.08,
    width * 0.75,
  );
  glow.addColorStop(0, rgba(accent, 0.38));
  glow.addColorStop(1, rgba(accent, 0));
  ctx.fillStyle = glow;
  ctx.fillRect(0, 0, width, height);

  const veil = ctx.createLinearGradient(0, 0, 0, height);
  veil.addColorStop(0, "rgba(7, 9, 16, 0.38)");
  veil.addColorStop(0.45, "rgba(7, 9, 16, 0.72)");
  veil.addColorStop(1, "rgba(7, 9, 16, 0.96)");
  ctx.fillStyle = veil;
  ctx.fillRect(0, 0, width, height);
}

function drawTopBar(
  ctx: Ctx,
  width: number,
  model: ShareCardModel,
  accent: string,
  y: number,
): void {
  setFont(ctx, 30, 700);
  ctx.fillStyle = INK;
  ctx.textAlign = "left";
  ctx.textBaseline = "alphabetic";
  ctx.fillText("VN ASYIK", MARGIN, y);

  setFont(ctx, 30, 600);
  ctx.fillStyle = MUTED;
  ctx.textAlign = "right";
  ctx.fillText(model.badge, width - MARGIN, y);

  ctx.fillStyle = accent;
  roundedRect(ctx, MARGIN, y + 28, 54, 5, 3);
  ctx.fill();
}

interface HeroLayout {
  eyebrowY: number;
  labelY: number;
  heroY: number;
  sublineY: number;
  maxSize: number;
}

function drawHero(
  ctx: Ctx,
  width: number,
  model: ShareCardModel,
  labels: ShareCardLabels,
  locale: Locale,
  accent: string,
  layout: HeroLayout,
): void {
  const contentWidth = width - MARGIN * 2;

  setFont(ctx, 26, 700);
  ctx.fillStyle = accent;
  ctx.textAlign = "left";
  ctx.fillText(model.eyebrow, MARGIN, layout.eyebrowY);

  setFont(ctx, 24, 500);
  ctx.fillStyle = MUTED;
  ctx.fillText(labels.totalTime, MARGIN, layout.labelY);

  const duration = shareDuration(model.totalMs, locale);
  let size = layout.maxSize;
  setFont(ctx, size, 750);
  while (ctx.measureText(duration).width > contentWidth && size > 74) {
    size -= 4;
    setFont(ctx, size, 750);
  }
  ctx.fillStyle = INK;
  ctx.fillText(fitText(ctx, duration, contentWidth), MARGIN, layout.heroY);

  setFont(ctx, 30, 500);
  ctx.fillStyle = MUTED;
  ctx.fillText(model.subline, MARGIN, layout.sublineY);
}

function drawDivider(ctx: Ctx, width: number, y: number): void {
  ctx.fillStyle = "rgba(255, 255, 255, 0.12)";
  ctx.fillRect(MARGIN, y, width - MARGIN * 2, 1);
}

function drawFigure(
  ctx: Ctx,
  x: number,
  y: number,
  value: string,
  label: string,
  maxWidth: number,
): void {
  setFont(ctx, 62, 700);
  ctx.fillStyle = INK;
  ctx.textAlign = "left";
  ctx.fillText(fitText(ctx, value, maxWidth), x, y);

  setFont(ctx, 23, 500);
  ctx.fillStyle = MUTED;
  ctx.fillText(fitText(ctx, label, maxWidth), x, y + 38);
}

function drawFigures(
  ctx: Ctx,
  width: number,
  model: ShareCardModel,
  y: number,
): void {
  const column = (width - MARGIN * 2) / 3;
  model.figures.slice(0, 3).forEach((figure, index) => {
    drawFigure(
      ctx,
      MARGIN + index * column,
      y,
      figure.value,
      figure.label,
      column - 24,
    );
  });
}

function drawTopGames(
  ctx: Ctx,
  width: number,
  model: ShareCardModel,
  labels: ShareCardLabels,
  locale: Locale,
  accent: string,
  limit: number,
  startY: number,
  rowHeight: number,
): void {
  if (model.topGames.length === 0) return;

  const contentWidth = width - MARGIN * 2;
  setFont(ctx, 24, 700);
  ctx.fillStyle = accent;
  ctx.textAlign = "left";
  ctx.fillText(labels.mostRead, MARGIN, startY - 52);

  const top = model.topGames[0];
  const max = top && top.countedMs > 0 ? top.countedMs : 1;
  const barWidth = contentWidth - 62;
  const titleWidth = barWidth - 220;

  model.topGames.slice(0, limit).forEach((game, index) => {
    const y = startY + index * rowHeight;
    setFont(ctx, 28, 700);
    ctx.fillStyle = accent;
    ctx.textAlign = "left";
    ctx.fillText(String(index + 1).padStart(2, "0"), MARGIN, y);

    setFont(ctx, 33, 600);
    ctx.fillStyle = INK;
    ctx.textAlign = "left";
    ctx.fillText(fitText(ctx, game.title, titleWidth), MARGIN + 62, y);

    setFont(ctx, 27, 500);
    ctx.fillStyle = MUTED;
    ctx.textAlign = "right";
    ctx.fillText(shareDuration(game.countedMs, locale), width - MARGIN, y);

    const filled = Math.max(4, (game.countedMs / max) * barWidth);
    ctx.fillStyle = "rgba(255, 255, 255, 0.10)";
    roundedRect(ctx, MARGIN + 62, y + 14, barWidth, 7, 4);
    ctx.fill();
    ctx.fillStyle = accent;
    roundedRect(ctx, MARGIN + 62, y + 14, filled, 7, 4);
    ctx.fill();
  });
}

function drawBottomFacts(
  ctx: Ctx,
  width: number,
  model: ShareCardModel,
  startY: number,
): void {
  const columnWidth = (width - MARGIN * 2) / 2;
  model.facts.slice(0, 4).forEach((fact, index) => {
    const x = MARGIN + (index % 2) * columnWidth;
    const y = startY + Math.floor(index / 2) * 92;
    setFont(ctx, 21, 600);
    ctx.fillStyle = FAINT;
    ctx.textAlign = "left";
    ctx.fillText(fact.label.toUpperCase(), x, y);
    setFont(ctx, 32, 600);
    ctx.fillStyle = INK;
    ctx.fillText(fitText(ctx, fact.value, columnWidth - 32), x, y + 38);
  });
}

function drawHourChart(
  ctx: Ctx,
  width: number,
  model: ShareCardModel,
  labels: ShareCardLabels,
  accent: string,
  startY: number,
): void {
  if (!model.hourMs.some((ms) => ms > 0)) return;

  const contentWidth = width - MARGIN * 2;
  setFont(ctx, 24, 700);
  ctx.fillStyle = accent;
  ctx.textAlign = "left";
  ctx.fillText(labels.hourTitle, MARGIN, startY);

  const chartTop = startY + 30;
  const chartHeight = 150;
  const gap = 4;
  const barWidth = (contentWidth - gap * 23) / 24;
  const max = Math.max(1, ...model.hourMs);

  model.hourMs.forEach((ms, index) => {
    const x = MARGIN + index * (barWidth + gap);
    ctx.fillStyle = "rgba(255, 255, 255, 0.08)";
    roundedRect(ctx, x, chartTop, barWidth, chartHeight, 4);
    ctx.fill();

    const height = Math.max(3, (ms / max) * chartHeight);
    ctx.fillStyle = accent;
    roundedRect(ctx, x, chartTop + chartHeight - height, barWidth, height, 4);
    ctx.fill();
  });

  setFont(ctx, 20, 500);
  ctx.fillStyle = FAINT;
  ctx.textAlign = "left";
  ctx.fillText("00", MARGIN, chartTop + chartHeight + 28);
  ctx.textAlign = "center";
  ctx.fillText("06", MARGIN + contentWidth * 0.25, chartTop + chartHeight + 28);
  ctx.fillText("12", MARGIN + contentWidth * 0.5, chartTop + chartHeight + 28);
  ctx.fillText("18", MARGIN + contentWidth * 0.75, chartTop + chartHeight + 28);
  ctx.textAlign = "right";
  ctx.fillText("23", MARGIN + contentWidth, chartTop + chartHeight + 28);

  setFont(ctx, 22, 500);
  ctx.fillStyle = MUTED;
  ctx.textAlign = "left";
  ctx.fillText(model.hourPeak, MARGIN, chartTop + chartHeight + 66);
}

function drawFooter(
  ctx: Ctx,
  width: number,
  labels: ShareCardLabels,
  accent: string,
  lineY: number,
  textY: number,
): void {
  ctx.fillStyle = "rgba(255, 255, 255, 0.10)";
  ctx.fillRect(MARGIN, lineY, width - MARGIN * 2, 1);

  setFont(ctx, 22, 500);
  ctx.fillStyle = FAINT;
  ctx.textAlign = "left";
  ctx.fillText(labels.footer, MARGIN, textY);

  setFont(ctx, 22, 700);
  ctx.fillStyle = accent;
  ctx.textAlign = "right";
  ctx.fillText("VN ASYIK", width - MARGIN, textY);
}

function drawPostCard(
  ctx: Ctx,
  width: number,
  height: number,
  model: ShareCardModel,
  labels: ShareCardLabels,
  locale: Locale,
  accent: string,
  backdrop: HTMLImageElement | null,
): void {
  drawBackground(ctx, width, height, accent, backdrop);
  drawTopBar(ctx, width, model, accent, 104);
  drawHero(ctx, width, model, labels, locale, accent, {
    eyebrowY: 214,
    labelY: 268,
    heroY: 404,
    sublineY: 464,
    maxSize: 138,
  });
  drawDivider(ctx, width, 520);
  drawFigures(ctx, width, model, 600);
  drawTopGames(ctx, width, model, labels, locale, accent, 3, 800, 92);
  drawBottomFacts(ctx, width, model, 1118);
  drawFooter(ctx, width, labels, accent, 1272, 1318);
}

function drawStoryCard(
  ctx: Ctx,
  width: number,
  height: number,
  model: ShareCardModel,
  labels: ShareCardLabels,
  locale: Locale,
  accent: string,
  backdrop: HTMLImageElement | null,
): void {
  drawBackground(ctx, width, height, accent, backdrop);
  drawTopBar(ctx, width, model, accent, 130);
  drawHero(ctx, width, model, labels, locale, accent, {
    eyebrowY: 280,
    labelY: 350,
    heroY: 520,
    sublineY: 600,
    maxSize: 160,
  });
  drawDivider(ctx, width, 680);
  drawFigures(ctx, width, model, 800);
  drawTopGames(ctx, width, model, labels, locale, accent, 5, 1010, 110);
  drawHourChart(ctx, width, model, labels, accent, 1540);
  drawFooter(ctx, width, labels, accent, 1820, 1860);
}

/**
 * Draw the full card. The caller owns the canvas element and its CSS size; the
 * backing store is always the selected format's canonical size.
 */
export async function drawShareCard(
  canvas: HTMLCanvasElement,
  options: ShareCardOptions,
): Promise<void> {
  const { width, height } = SHARE_CARD_SIZES[options.format];
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas 2D is unavailable");

  const accent = accentOf(options.model.accent);
  const backdrop = options.model.backdropUrl
    ? await loadImage(options.model.backdropUrl)
    : null;

  ctx.clearRect(0, 0, width, height);
  if (options.format === "story") {
    drawStoryCard(
      ctx,
      width,
      height,
      options.model,
      options.labels,
      options.locale,
      accent,
      backdrop,
    );
  } else {
    drawPostCard(
      ctx,
      width,
      height,
      options.model,
      options.labels,
      options.locale,
      accent,
      backdrop,
    );
  }
}
