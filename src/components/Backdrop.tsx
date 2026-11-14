import { useEffect, useRef, useState } from "react";

/**
 * Fixed non-scrolling backdrop that crossfades blurred cover layers so outgoing
 * artwork stays visible until the incoming image paints (avoiding empty flashes).
 */

const FOCUS_EVENT = "vn:focus-art";

interface FocusDetail {
  url: string | null;
}

export function Backdrop() {
  const [layers, setLayers] = useState<[string | null, string | null]>([null, null]);
  const [active, setActive] = useState<0 | 1>(0);
  const activeRef = useRef<0 | 1>(0);

  useEffect(() => {
    const onFocusArt = (event: Event): void => {
      const detail = (event as CustomEvent<FocusDetail>).detail;
      const url = detail?.url ?? null;
      if (url === null) {
        // Clearing can revoke explicit art. Remove both buffers immediately so
        // an outgoing image cannot survive in a crossfade or reappear later.
        setLayers([null, null]);
        activeRef.current = 0;
        setActive(0);
        return;
      }
      const target = activeRef.current === 0 ? 1 : 0;
      setLayers((current) => {
        const next: [string | null, string | null] = [current[0], current[1]];
        next[target] = url;
        return next;
      });
      activeRef.current = target;
      setActive(target);
    };

    window.addEventListener(FOCUS_EVENT, onFocusArt);
    return () => window.removeEventListener(FOCUS_EVENT, onFocusArt);
  }, []);

  const hasArt = layers[0] !== null || layers[1] !== null;

  return (
    <div className="backdrop" aria-hidden="true">
      {hasArt ? (
        <>
          {layers.map((layer, index) =>
            layer ? (
              <div
                key={`${index}:${layer}`}
                className="backdrop__layer"
                data-visible={index === active ? "true" : "false"}
                style={{
                  backgroundImage: `url("${layer}")`,
                  filter: "blur(30px) saturate(1.25)",
                }}
              />
            ) : null,
          )}
          <div className="backdrop__scrim" />
          <div className="backdrop__grain" />
          <div className="backdrop__bloom" />
        </>
      ) : (
        <div className="backdrop__fallback" />
      )}
    </div>
  );
}

/** Ask the backdrop to show this artwork. Passing `null` clears it. */
export function focusArt(url: string | null): void {
  window.dispatchEvent(new CustomEvent<FocusDetail>(FOCUS_EVENT, { detail: { url } }));
}
