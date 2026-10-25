import type { ReactNode } from "react";

interface EmptyStateProps {
  title: string;
  body: string;
  action?: ReactNode;
}

/**
 * Empty state that names the cause and offers the one action that fills it
 * (antislop R-27). No illustration: art here would be decoration.
 */
export function EmptyState({ title, body, action }: EmptyStateProps) {
  return (
    <div className="empty">
      <h2 className="empty__title">{title}</h2>
      <p className="empty__body">{body}</p>
      {action}
    </div>
  );
}
