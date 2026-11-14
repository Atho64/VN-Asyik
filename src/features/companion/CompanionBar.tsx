import { useEffect } from "react";
import { Link, useLocation } from "react-router";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { companionApi } from "@/lib/ipc/companion";
import { companionError } from "./errors";
import { connectCompanion, performAction, useCompanionStore } from "./store";
import { OpenOverlayButton } from "./OverlayActions";

/** Mounted beside the routed page. It observes Rust; it does not own its task. */
export function CompanionBar() {
  const { t } = useI18n();
  const { pathname } = useLocation();
  const snapshot = useCompanionStore((state) => state.snapshot);
  const pending = useCompanionStore((state) => state.actionPending);
  const actionError = useCompanionStore((state) => state.error);
  useEffect(connectCompanion, []);
  const session = snapshot?.session;
  if (!snapshot || !session || snapshot.lifecycle === "off" || pathname === "/companion")
    return null;
  const error = actionError ?? snapshot.error;
  return (
    <aside className="companion-bar" aria-label={t("companion.title")}>
      <Link className="companion-bar__link" to="/companion">
        <Icon name="chat" />
        <span>
          <strong>{session.gameTitle ?? t("companion.removedGame")}</strong>
          <small>
            {error
              ? companionError(t, error)
              : t(
                  snapshot.busy
                    ? "companion.replying"
                    : `companion.state.${snapshot.lifecycle}`,
                )}
          </small>
        </span>
      </Link>
      <div className="companion__actions">
        <OpenOverlayButton compact />
        <Button
          variant="ghost"
          size="icon"
          aria-label={t(
            snapshot.lifecycle === "paused" ? "companion.resume" : "companion.pause",
          )}
          title={t(
            snapshot.lifecycle === "paused" ? "companion.resume" : "companion.pause",
          )}
          disabled={pending}
          onClick={() =>
            void performAction(() =>
              companionApi.control(
                session.id,
                snapshot.lifecycle === "paused" ? "resume" : "pause",
              ),
            )
          }
        >
          <Icon name={snapshot.lifecycle === "paused" ? "play" : "pause"} />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label={t("companion.end")}
          title={t("companion.end")}
          disabled={pending}
          onClick={() =>
            void performAction(() => companionApi.control(session.id, "end"))
          }
        >
          <Icon name="stop" />
        </Button>
      </div>
    </aside>
  );
}
