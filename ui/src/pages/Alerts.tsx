import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ackAlert, useAlerts, type Alert, type AlertsFilter, type Severity } from "../api/alerts";
import { ApiRequestError } from "../api/client";
import { IncidentTimeline } from "../components/IncidentTimeline";
import { NotificationPreview } from "../components/NotificationPreview";
import { RulesCard } from "../components/RulesCard";
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

const TABS: { filter: AlertsFilter; label: string }[] = [
  { filter: "active", label: "Active" },
  { filter: "resolved", label: "Resolved" },
  { filter: "all", label: "All" },
];

const button =
  "min-h-11 cursor-pointer rounded-lg border border-border-strong bg-surface-2 px-4 text-sm font-semibold text-text";

function AlertRow({
  alert: a,
  now,
  onOpen,
  onAck,
  note,
}: {
  alert: Alert;
  now: number;
  onOpen: () => void;
  onAck: () => void;
  note: string | undefined;
}) {
  let when = formatAgo(a.opened_at, now);
  if (a.resolved_at !== null) when += ` · resolved ${formatAgo(a.resolved_at, now)}`;
  else if (a.state === "acked") when += " · acknowledged";
  return (
    <li className="flex flex-col gap-1 rounded-xl border border-border bg-surface p-4">
      <span className={`text-sm font-semibold ${SEVERITY_COLOUR[a.severity]}`}>
        {SEVERITY[a.severity]}
      </span>
      <button
        type="button"
        onClick={onOpen}
        className="min-h-11 cursor-pointer self-start border-0 bg-transparent p-0 text-left text-base font-semibold break-words text-text underline"
      >
        {a.title}
      </button>
      {a.detail !== "" && <span className="text-sm break-words">{a.detail}</span>}
      <span className="text-sm text-text-muted">{when}</span>
      {a.suppressed && (
        <span className="text-sm text-text-muted">Held back while the router is down</span>
      )}
      {a.state === "open" && (
        <button type="button" onClick={onAck} className={`${button} mt-1 self-start`}>
          Acknowledge
        </button>
      )}
      {note !== undefined && (
        <p role="alert" className="m-0 text-sm font-medium text-bad">
          {note}
        </p>
      )}
    </li>
  );
}

export function Alerts() {
  const [filter, setFilter] = useState<AlertsFilter>("active");
  const [selected, setSelected] = useState<number | null>(null);
  const [notes, setNotes] = useState<Record<number, string>>({});
  const { data, error, isPending } = useAlerts(filter);
  const qc = useQueryClient();
  const now = useNow();

  function ack(id: number) {
    setNotes((n) => Object.fromEntries(Object.entries(n).filter(([k]) => Number(k) !== id)));
    ackAlert(id).then(
      () => {
        void qc.invalidateQueries({ queryKey: ["alerts"] });
        void qc.invalidateQueries({ queryKey: ["incident"] });
      },
      (e: unknown) => {
        const resolved = e instanceof ApiRequestError && e.status === 409;
        const msg = resolved
          ? "Already resolved."
          : e instanceof Error
            ? e.message
            : "Could not acknowledge.";
        setNotes((n) => ({ ...n, [id]: msg }));
        if (resolved) void qc.invalidateQueries({ queryKey: ["alerts"] });
      },
    );
  }

  const sorted = data ? [...data].sort((a, b) => b.opened_at - a.opened_at) : [];

  return (
    <>
      <h1 className="m-0 text-3xl font-bold">Alerts</h1>
      <div className="flex flex-wrap gap-2">
        {TABS.map((t) => (
          <button
            key={t.filter}
            type="button"
            aria-pressed={filter === t.filter}
            onClick={() => setFilter(t.filter)}
            className={`${button} aria-pressed:border-accent aria-pressed:bg-accent aria-pressed:text-accent-ink`}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_24rem]">
        <div className="flex min-w-0 flex-col gap-3">
          {isPending && <p className="m-0 text-text-muted">Loading…</p>}
          {error && <p className="m-0 text-text-muted">Couldn't load alerts.</p>}
          {data && sorted.length === 0 && <p className="m-0 text-text-muted">No alerts here.</p>}
          {sorted.length > 0 && (
            <ul className="m-0 flex list-none flex-col gap-3 p-0">
              {sorted.map((a) => (
                <AlertRow
                  key={a.id}
                  alert={a}
                  now={now}
                  note={notes[a.id]}
                  onOpen={() => setSelected(a.incident_id)}
                  onAck={() => ack(a.id)}
                />
              ))}
            </ul>
          )}
        </div>
        {selected !== null && <IncidentTimeline id={selected} />}
      </div>
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <RulesCard />
        <NotificationPreview />
      </div>
    </>
  );
}
