import { useAlerts } from "../api/alerts";
import { useMonitor } from "../api/monitor";
import { useDevices } from "../api/queries";

const NONE = "—";

function ms(v: number | null | undefined, haveTick: boolean): string {
  if (!haveTick) return NONE;
  return v === null || v === undefined ? "No reply" : `${Math.round(v)} ms`;
}

function Tile({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex min-w-0 flex-col gap-1 rounded-xl border border-border bg-surface p-4">
      <span className="text-sm text-text-muted">{label}</span>
      <span className="text-xl font-semibold break-words">{value}</span>
    </div>
  );
}

/** Five headline numbers, each a text label above a text value. */
export function KpiTiles() {
  const monitor = useMonitor();
  const devices = useDevices();
  const alerts = useAlerts("active");
  const last = monitor.data?.last ?? null;
  const haveTick = last !== null;

  const online = devices.data ? devices.data.filter((d) => d.online).length : null;
  return (
    <section aria-label="Key numbers" className="grid grid-cols-2 gap-3 lg:grid-cols-5">
      <Tile
        label="Devices online"
        value={online === null || !devices.data ? NONE : `${online} of ${devices.data.length}`}
      />
      <Tile label="Router" value={ms(last?.gateway_rtt_ms, haveTick)} />
      <Tile label="Internet" value={ms(last?.internet_rtt_ms, haveTick)} />
      <Tile
        label="Router loss (15 min)"
        value={last ? `${last.router_lost} of ${last.router_window}` : NONE}
      />
      <Tile label="Active alerts" value={alerts.data ? String(alerts.data.length) : NONE} />
    </section>
  );
}
