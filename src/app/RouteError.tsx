import { Link, useRouteError } from "react-router";

import { useI18n } from "@/app/useI18n";

/** Readable text for anything thrown, including plain objects. */
function describe(error: unknown): string | null {
  if (error === null || error === undefined) return null;
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try {
    const text = JSON.stringify(error);
    return text && text !== "{}" ? text : String(error);
  } catch {
    return String(error);
  }
}

/**
 * Route-level failure. Says what happened and offers a way back, instead of a
 * blank screen (antislop R-27).
 */
export function RouteError() {
  const error = useRouteError();
  const { t } = useI18n();
  // A non-Error thrown by a failed IPC schema parse stringifies to
  // "[object Object]", which tells the user nothing and cost real time to
  // diagnose. Prefer a message, then JSON, then the type.
  const message = describe(error) ?? t("common.unknown");

  return (
    <div className="app">
      <div className="app__content">
        <main className="app__main">
          <div className="empty">
            <h1 className="empty__title">{t("common.error")}</h1>
            <p className="empty__body">{message}</p>
            <Link className="btn btn--secondary" to="/">
              {t("detail.back")}
            </Link>
          </div>
        </main>
      </div>
    </div>
  );
}
