interface ProgressRingProps {
  /** 0..1. Values outside the range are clamped, never wrapped. */
  progress: number;
  size?: number;
  strokeWidth?: number;
  label?: string;
  /** Accessible description, e.g. "42% of the reported length". */
  title?: string;
}

/**
 * Circular progress. Used on cover cards to show playtime against the length
 * VNDB reports, which is the only progress signal this app can honestly compute.
 */
export function ProgressRing({
  progress,
  size = 28,
  strokeWidth = 2.5,
  label,
  title,
}: ProgressRingProps) {
  const clamped = Math.max(0, Math.min(1, Number.isFinite(progress) ? progress : 0));
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const offset = circumference * (1 - clamped);

  return (
    <span
      className="ring"
      style={{ width: size, height: size }}
      role="img"
      aria-label={title ?? `${Math.round(clamped * 100)}%`}
    >
      <svg className="ring__svg" width={size} height={size} aria-hidden="true">
        <circle
          className="ring__track"
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          strokeWidth={strokeWidth}
        />
        <circle
          className="ring__value"
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          strokeWidth={strokeWidth}
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          strokeLinecap="round"
        />
      </svg>
      {label ? <span className="ring__label">{label}</span> : null}
    </span>
  );
}
