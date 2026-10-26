import { useEffect, useRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import type { Tone } from "../format";
import type { UiError } from "../types";
import { Icon, type IconName } from "./Icon";

export function Button({
  variant = "secondary",
  icon,
  children,
  className = "",
  ...rest
}: { variant?: "primary" | "secondary" | "danger" | "ghost"; icon?: IconName } & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button type="button" className={`btn btn-${variant} ${className}`} {...rest}>
      {icon && <Icon name={icon} />}
      {children && <span>{children}</span>}
    </button>
  );
}

export function Badge({ tone, children }: { tone: Tone; children: ReactNode }) {
  return <span className={`badge tone-${tone}`}>{children}</span>;
}

const toneIcon: Record<Tone, IconName> = { ok: "check", warn: "warn", bad: "error", info: "info", neutral: "info" };

export function Banner({ tone, title, children, actions }: { tone: Tone; title: ReactNode; children?: ReactNode; actions?: ReactNode }) {
  return (
    <div className={`banner tone-${tone}`} role={tone === "bad" ? "alert" : "status"}>
      <Icon name={toneIcon[tone]} size={18} className="banner-icon" />
      <div className="banner-body">
        <div className="banner-title">{title}</div>
        {children && <div className="banner-text">{children}</div>}
      </div>
      {actions && <div className="banner-actions">{actions}</div>}
    </div>
  );
}

export function ErrorBanner({ error, onDismiss }: { error: UiError | null; onDismiss?: () => void }) {
  if (!error) return null;
  return (
    <Banner tone="bad" title="Something went wrong" actions={onDismiss && <Button variant="ghost" icon="x" aria-label="Dismiss" onClick={onDismiss} />}>
      {error.message}
    </Banner>
  );
}

export function Empty({ icon, title, children }: { icon: IconName; title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      <Icon name={icon} size={28} />
      <div className="empty-title">{title}</div>
      {children && <div className="empty-text">{children}</div>}
    </div>
  );
}

export function ProgressBar({ value, label, tone = "info" }: { value: number | null; label: string; tone?: Tone }) {
  const pct = value == null ? null : Math.max(0, Math.min(1, value)) * 100;
  return (
    <div
      className={`progress tone-${tone}`}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct == null ? undefined : Math.round(pct)}
    >
      <div className="progress-fill" style={{ width: pct == null ? "0%" : `${pct}%` }} />
    </div>
  );
}

/** Storage meter: used vs. needed vs. free. */
export function SpaceMeter({ total, free, needed }: { total: number | null; free: number | null; needed?: number }) {
  if (total == null || free == null || total <= 0) return <div className="meter meter-unknown" aria-hidden="true" />;
  const used = (total - free) / total;
  const need = needed ? Math.min(needed / total, 1 - used) : 0;
  const over = needed != null && needed > free;
  return (
    <div className="meter" aria-hidden="true">
      <div className="meter-used" style={{ width: `${used * 100}%` }} />
      {need > 0 && <div className={`meter-need ${over ? "over" : ""}`} style={{ width: `${need * 100}%` }} />}
    </div>
  );
}

export function PageHeader({ title, subtitle, actions }: { title: string; subtitle?: ReactNode; actions?: ReactNode }) {
  return (
    <header className="page-header">
      <div>
        <h1>{title}</h1>
        {subtitle && <p className="subtitle">{subtitle}</p>}
      </div>
      {actions && <div className="page-actions">{actions}</div>}
    </header>
  );
}

export function Section({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="section">
      <div className="section-head">
        <h2>{title}</h2>
        {aside}
      </div>
      {children}
    </section>
  );
}

export function Confirm({
  open,
  title,
  body,
  confirmLabel,
  danger,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: string;
  body: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) {
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
    } else if (!open && d.open) {
      if (typeof d.close === "function") d.close();
      else d.removeAttribute("open");
    }
  }, [open]);
  return (
    <dialog ref={ref} className="dialog" aria-labelledby="confirm-title" onCancel={(e) => (e.preventDefault(), onCancel())}>
      {open && (
        <>
          <h2 id="confirm-title">{title}</h2>
          <div className="dialog-body">{body}</div>
          <div className="dialog-actions">
            <Button onClick={onCancel} autoFocus>
              Keep going
            </Button>
            <Button variant={danger ? "danger" : "primary"} onClick={onConfirm}>
              {confirmLabel}
            </Button>
          </div>
        </>
      )}
    </dialog>
  );
}

export function Stat({ label, value, tone, hint }: { label: string; value: ReactNode; tone?: Tone; hint?: ReactNode }) {
  return (
    <div className={`stat ${tone ? `tone-${tone}` : ""}`}>
      <div className="stat-value">{value}</div>
      <div className="stat-label">{label}</div>
      {hint && <div className="stat-hint">{hint}</div>}
    </div>
  );
}

export function Mono({ children, title }: { children: ReactNode; title?: string }) {
  return (
    <span className="mono" title={title}>
      {children}
    </span>
  );
}
