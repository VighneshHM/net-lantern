import { useIncident, type TimelineKind } from "../api/alerts";
import { ApiRequestError } from "../api/client";

const KIND_LABEL: Record<TimelineKind, string> = {
  last_good: "Last good check",
  detected: "Detected",
  alerted: "Alert raised",
  affected: "Affected",
  diagnosed: "Diagnosed",
  acknowledged: "Acknowledged",
  recovered: "Recovered",
};

function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function dateTime(ms: number): string {
  return new Date(ms).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** One incident: when it opened and closed, what happened in order, and what else it affected. */
export function IncidentTimeline({ id }: { id: number }) {
  const { data, error, isPending } = useIncident(id);

  let body;
  if (isPending) {
    body = <p className="m-0 text-text-muted">Loading…</p>;
  } else if (error || !data) {
    body = (
      <p className="m-0 text-text-muted">
        {error instanceof ApiRequestError ? error.error.message : "Couldn't load the incident."}
      </p>
    );
  } else {
    const others = data.alerts.filter((a) => a.id !== data.root.id);
    body = (
      <>
        <p className="m-0 text-sm text-text-muted">
          Opened {dateTime(data.opened_at)}
          {data.resolved_at !== null
            ? ` · resolved ${dateTime(data.resolved_at)}`
            : " · still open"}
        </p>
        <ol aria-label="Incident timeline" className="m-0 flex list-none flex-col gap-3 p-0">
          {data.timeline.map((e, i) => (
            <li key={i} className="flex flex-col gap-0.5 text-sm">
              <span className="text-text-muted">
                <span className="font-mono">{clock(e.ts)}</span> ·{" "}
                <span className="font-semibold text-text">{KIND_LABEL[e.kind]}</span>
              </span>
              <span className="break-words">{e.text}</span>
            </li>
          ))}
        </ol>
        {others.length > 0 && (
          <>
            <h3 className="m-0 text-sm font-semibold">Also affected</h3>
            <ul className="m-0 flex list-none flex-col gap-1 p-0 text-sm">
              {others.map((a) => (
                <li key={a.id} className="break-words">
                  {a.title}
                </li>
              ))}
            </ul>
          </>
        )}
      </>
    );
  }

  return (
    <section
      aria-label="Incident details"
      className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4"
    >
      <h2 className="m-0 text-lg font-bold">Incident</h2>
      {body}
    </section>
  );
}
