import { useEffect, useState } from "react";
import { api, asUiError, events } from "../api";
import type { Navigate } from "../App";
import { bytes } from "../format";
import type { DetectedVolume, UiError } from "../types";
import { Banner, Button, ErrorBanner } from "./ui";

/**
 * Cards and drives connected while MediaFlow runs. Only a suggestion: the
 * import starts after the user picks it and reviews the plan.
 */
export function DetectedCards({ go }: { go: Navigate }) {
  const [cards, setCards] = useState<DetectedVolume[]>([]);
  const [dismissed, setDismissed] = useState<string[]>([]);
  const [error, setError] = useState<UiError | null>(null);

  useEffect(() => {
    let alive = true;
    let un: (() => void) | undefined;
    api
      .detectedVolumes()
      .then((v) => alive && setCards(v))
      .catch(() => {
        /* not available (e.g. older backend): nothing to show */
      });
    void events
      .onVolumeDetected((v) => setCards((c) => [...c.filter((x) => x.mount_point !== v.mount_point), v]))
      .then((u) => (alive ? (un = u) : u()));
    return () => {
      alive = false;
      un?.();
    };
  }, []);

  const visible = cards.filter((c) => !dismissed.includes(c.id));
  if (visible.length === 0 && !error) return null;

  const use = async (c: DetectedVolume) => {
    setError(null);
    try {
      const g = await api.useDetectedVolume(c.id);
      setDismissed((d) => [...d, c.id]);
      go({ page: "import", source: { id: g.id, label: c.name } });
    } catch (e) {
      setError(asUiError(e));
    }
  };

  return (
    <div className="detected">
      <ErrorBanner error={error} onDismiss={() => setError(null)} />
      {visible.map((c) => (
        <Banner
          key={c.id}
          tone="info"
          title={`${c.name} connected`}
          actions={
            <>
              <Button variant="ghost" onClick={() => setDismissed((d) => [...d, c.id])}>
                Not now
              </Button>
              <Button variant="primary" icon="import" onClick={() => void use(c)}>
                Import from this card
              </Button>
            </>
          }
        >
          {bytes(c.total_bytes)} · <span className="mono-path">{c.mount_point}</span>. You will review the plan before anything is copied.
        </Banner>
      ))}
    </div>
  );
}
