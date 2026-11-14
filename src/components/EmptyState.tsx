import type { ReactNode } from "react";

interface EmptyStateProps {
  title: string;
  body: string;
  action?: ReactNode;
}

/** Empty state naming the cause and offering a single action to populate it. */
export function EmptyState({ title, body, action }: EmptyStateProps) {
  return (
    <div className="empty">
      <h2 className="empty__title">{title}</h2>
      <p className="empty__body">{body}</p>
      {action}
    </div>
  );
}
