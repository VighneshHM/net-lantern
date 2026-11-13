import { Link } from "react-router";
import { useAlerts, type Severity } from "../api/alerts";
import { formatAgo } from "../lib/time";
import { useNow } from "../lib/useNow";

const SEVERITY: Record<Severity, string> = {
  critical: "Critical",
  warning: "Warning",
  info: "Info",
};

const SEVERITY_COLOUR: Record<Severity, string> = {
  critical: "text-bad",
  warning: "text-warn",
  info: "text-accent",
};

/** The five newest active alerts, severity shown as text. */
export function RecentAlerts() {
  const { data } = useAlerts("active");
  const now = useNow();
  const items = data ? [...data].sort((a, b) => b.opened_at - a.opened_at).slice(0, 5) : [];
  return (
    <section
      aria-label="Recent alerts"
      className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-5"
    >
      <h2 className="m-0 text-sm font-medium text-text-muted">Recent alerts</h2>
      {data && items.length === 0 && <p className="m-0 text-text-muted">No active alerts.</p>}
      {items.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-2 p-0">
          {items.map((a) => (
            <li key={a.id} className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
              <span className={`text-sm font-semibold ${SEVERITY_COLOUR[a.severity]}`}>
                {SEVERITY[a.severity]}
              </span>
              <span className="min-w-0 break-words">{a.title}</span>
              <span className="text-sm text-text-muted">{formatAgo(a.opened_at, now)}</span>
            </li>
          ))}
        </ul>
      )}
      <Link
        to="/alerts"
        className="inline-flex min-h-11 items-center self-start text-sm font-semibold text-accent"
      >
        See all alerts
      </Link>
    </section>
  );
}
