/**
 * One small icon set, drawn to match the identity motif: 1.5px strokes, square
 * joins, no rounded stylised library look (antislop R-04). Each glyph earns its
 * place by naming something concrete in this app, so there is no decorative set.
 *
 * `currentColor` only; size comes from the caller's font size.
 */

import type { ReactNode } from "react";

export type IconName =
  | "chat"
  | "library"
  | "shelf"
  | "stats"
  | "settings"
  | "search"
  | "close"
  | "plus"
  | "chevronLeft"
  | "clock"
  | "star"
  | "grid"
  | "list"
  | "eye"
  | "eyeOff"
  | "check"
  | "alert"
  | "spark"
  | "send"
  | "stop"
  | "play"
  | "image"
  | "guide"
  | "gamepad"
  | "screen"
  | "trash";

interface IconProps {
  name: IconName;
  size?: number;
  className?: string;
  /** Accessible name. Omit when the surrounding control already labels it. */
  title?: string;
}

const paths: Record<IconName, ReactNode> = {
  chat: (
    <>
      <path d="M3 4.5h18v12H10l-5 4v-4H3z" />
      <path d="M7 9h10M7 12h7" />
    </>
  ),
  library: (
    <>
      <path d="M3 4.5h5v15H3zM10.5 4.5h4v15h-4z" />
      <path d="M17 5.2l3.2-.8 2.3 14.5-3.2.8z" />
    </>
  ),
  shelf: (
    <>
      <path d="M3 5.5h18" />
      <path d="M4.5 5.5v13M19.5 5.5v13" />
      <path d="M7.5 9h4v9.5h-4zM14 7.5h3v11h-3z" />
    </>
  ),
  stats: (
    <>
      <path d="M4 20V10M10 20V4M16 20v-7M22 20H2" />
    </>
  ),
  guide: (
    <>
      <path d="M12 6.8C10 5 7.4 4.4 4 4.7v13.4c3.4-.3 6 .4 8 2.2 2-1.8 4.6-2.5 8-2.2V4.7c-3.4-.3-6 .3-8 2.1z" />
      <path d="M12 6.8v13.5" />
    </>
  ),
  screen: (
    <>
      <path d="M3 5.5h18v11H3z" />
      <path d="M9 20.5h6M12 16.5v4" />
    </>
  ),
  gamepad: (
    <>
      <path d="M7.5 8.5h9a4.5 4.5 0 0 1 4.4 5.4l-.8 3.4a2.6 2.6 0 0 1-4.4 1.2l-1.6-1.6H9.9l-1.6 1.6a2.6 2.6 0 0 1-4.4-1.2l-.8-3.4a4.5 4.5 0 0 1 4.4-5.4z" />
      <path d="M7.7 11.4v3M6.2 12.9h3" />
      <circle cx="16" cy="12.1" r=".9" />
      <circle cx="17.8" cy="13.9" r=".9" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3.2" />
      <path d="M12 2.8v3M12 18.2v3M2.8 12h3M18.2 12h3M5.5 5.5l2.1 2.1M16.4 16.4l2.1 2.1M18.5 5.5l-2.1 2.1M7.6 16.4l-2.1 2.1" />
    </>
  ),
  search: (
    <>
      <circle cx="10.5" cy="10.5" r="6" />
      <path d="M15 15l5 5" />
    </>
  ),
  close: <path d="M6 6l12 12M18 6L6 18" />,
  plus: <path d="M12 5v14M5 12h14" />,
  chevronLeft: <path d="M15 5l-7 7 7 7" />,
  // A filled triangle, as a media control rather than an outline.
  play: <path d="M8 5.5v13l11-6.5z" fill="currentColor" stroke="none" />,
  clock: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7.5V12l3 2" />
    </>
  ),
  star: (
    <path d="M12 3.6l2.6 5.4 5.9.8-4.3 4.1 1.1 5.8L12 17l-5.3 2.7 1.1-5.8L3.5 9.8l5.9-.8z" />
  ),
  grid: (
    <>
      <path d="M4 4h6.5v6.5H4zM13.5 4H20v6.5h-6.5zM4 13.5h6.5V20H4zM13.5 13.5H20V20h-6.5z" />
    </>
  ),
  list: <path d="M4 6.5h16M4 12h16M4 17.5h16" />,
  eye: (
    <>
      <path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z" />
      <circle cx="12" cy="12" r="2.8" />
    </>
  ),
  eyeOff: (
    <>
      <path d="M4 4l16 16" />
      <path d="M9.9 6.2A9.6 9.6 0 0112 6c6 0 9.5 6 9.5 6a17 17 0 01-3 3.6M6.3 8.1A17 17 0 002.5 12S6 18 12 18a9.4 9.4 0 003.5-.7" />
    </>
  ),
  check: <path d="M4.5 12.5l5 5 10-11" />,
  // A four-pointed star: "suggested", not another list or shelf.
  spark: <path d="M12 3l1.9 5.6L20 10l-6.1 1.4L12 17l-1.9-5.6L4 10l6.1-1.4z" />,
  // Arrow into the composer's direction of travel: submit, not "next page".
  send: <path d="M12 19V5M6 11l6-6 6 6" />,
  stop: <rect x="7" y="7" width="10" height="10" fill="currentColor" stroke="none" />,
  alert: (
    <>
      <path d="M12 3.5L22 20H2z" />
      <path d="M12 9.5v4.5M12 17h.01" />
    </>
  ),
  image: (
    <>
      <rect x="3" y="4.5" width="18" height="15" rx="2.5" />
      <circle cx="8.5" cy="9.5" r="1.5" />
      <path d="M3.5 17l5-4.5 4 3.5 3-2.5 5 4" />
    </>
  ),
  trash: (
    <>
      <path d="M4 7h16" />
      <path d="M9.5 7V4.5h5V7" />
      <path d="M6.5 7l.9 12.2h9.2L17.5 7" />
    </>
  ),
};

export function Icon({ name, size = 16, className, title }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="square"
      strokeLinejoin="miter"
      className={className}
      role={title ? "img" : "presentation"}
      aria-hidden={title ? undefined : true}
      focusable="false"
    >
      {title ? <title>{title}</title> : null}
      {paths[name]}
    </svg>
  );
}
