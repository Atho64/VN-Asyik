import { useEffect } from "react";
import { CompanionScreen } from "./CompanionScreen";
import { connectCompanion } from "./store";

export function CompanionOverlay() {
  useEffect(connectCompanion, []);
  return (
    <main className="companion-overlay">
      <CompanionScreen overlay />
    </main>
  );
}
