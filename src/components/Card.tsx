import type { ReactNode } from "react";

interface CardProps {
  children: ReactNode;
  className?: string;
}

/** Surface for a grouped block of real content. No shadow: only sheets float. */
export function Card({ children, className }: CardProps) {
  return (
    <section className={className ? `card ${className}` : "card"}>
      <div className="card__body">{children}</div>
    </section>
  );
}

interface CardHeaderProps {
  title: ReactNode;
  aside?: ReactNode;
}

export function CardHeader({ title, aside }: CardHeaderProps) {
  return (
    <header className="card__header">
      <h2 className="h3">{title}</h2>
      {aside}
    </header>
  );
}
