import { useMonitor, type Check, type CheckOutcome } from "../api/monitor";
import { useDevices, type Device } from "../api/queries";
import { formatAgo } from "../lib/time";
import { useNow } from "../lib/useNow";

function checkLabel(c: Check): string {
  return c.kind === "tcp" ? `Port ${c.port}` : `HTTP ${c.path}`;
}

function presenceText(d: Device, now: number): string | null {
  if (d.presence_changed_at == null) return null;
  const ago = formatAgo(d.presence_changed_at, now);
  const span = ago.replace(/ ago$/, "");
  if (!d.online) return ago === "just now" ? "offline just now" : `offline since ${span}`;
  return ago === "just now" ? "online for under a minute" : `online for ${span}`;
}

function Outcomes({ outcomes }: { outcomes: CheckOutcome[] }) {
  if (outcomes.length === 0) return null;
  return (
    <ul className="m-0 flex list-none flex-wrap gap-x-4 gap-y-1 p-0 text-sm text-text-muted">
      {outcomes.map((o, i) => (
        <li key={i}>
          {checkLabel(o.check)}: {o.detail}
        </li>
      ))}
    </ul>
  );
}

/** Starred devices with presence and the latest result of each check. */
export function StarredDevices() {
  const devices = useDevices();
  const monitor = useMonitor();
  const now = useNow();
  const starred = (devices.data ?? []).filter((d) => d.starred);
  return (
    <section
      aria-label="Starred devices"
      className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-5"
    >
      <h2 className="m-0 text-sm font-medium text-text-muted">Starred devices</h2>
      {devices.data && starred.length === 0 && (
        <p className="m-0 text-text-muted">
          Star a device on the Devices screen to watch it closely.
        </p>
      )}
      {starred.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-4 p-0">
          {starred.map((d) => {
            const outcomes = monitor.data?.checks.find((c) => c.device_id === d.id)?.outcomes ?? [];
            const since = presenceText(d, now);
            return (
              <li key={d.id} className="flex flex-col gap-1">
                <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                  <span className="min-w-0 font-semibold break-words">{d.display_name}</span>
                  <span className={`text-sm font-semibold ${d.online ? "text-ok" : "text-bad"}`}>
                    {d.online ? "Online" : "Offline"}
                  </span>
                  {since && <span className="text-sm text-text-muted">{since}</span>}
                </div>
                <Outcomes outcomes={outcomes} />
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
