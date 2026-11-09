import { ApiRequestError } from "../api/client";
import { useWifi, type WifiConnection } from "../api/wifi";
import { BAND_LABEL } from "../lib/band";
import { signalLabel } from "../lib/signal";
import { LocationPermission } from "./LocationPermission";

const card = "flex flex-col gap-3 rounded-xl border border-border bg-surface p-5";

function linkSpeed(c: WifiConnection): string | null {
  const parts = [
    c.rx_rate_mbps === null ? null : `${c.rx_rate_mbps} Mbps down`,
    c.tx_rate_mbps === null ? null : `${c.tx_rate_mbps} Mbps up`,
  ].filter((p): p is string => p !== null);
  return parts.length > 0 ? parts.join(" · ") : null;
}

function Details({ c }: { c: WifiConnection }) {
  const rows: [string, string | null][] = [
    ["Network", c.ssid],
    ["Signal", `${c.rssi_dbm} dBm · ${signalLabel(c.rssi_dbm)}`],
    ["Band", BAND_LABEL[c.band]],
    ["Channel", c.width_mhz === null ? String(c.channel) : `${c.channel} · ${c.width_mhz} MHz wide`],
    ["Standard", c.phy],
    ["Link speed", linkSpeed(c)],
    ["Security", c.security],
    ["BSSID", c.bssid],
  ];
  return (
    <dl className="m-0 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
      {rows
        .filter((r): r is [string, string] => r[1] !== null)
        .map(([k, v]) => (
          <div key={k} className="contents">
            <dt className="text-text-muted">{k}</dt>
            <dd className="m-0 min-w-0 break-words">{v}</dd>
          </div>
        ))}
    </dl>
  );
}

export function WifiQuality() {
  const { data, error, isPending } = useWifi();

  let body;
  if (isPending) {
    body = <p className="m-0 text-text-muted">Checking your Wi-Fi…</p>;
  } else if (error) {
    body =
      error instanceof ApiRequestError &&
      error.status === 403 &&
      error.error.code === "location_permission" ? (
        <LocationPermission />
      ) : (
        <p className="m-0 text-text-muted">{error.message}</p>
      );
  } else if (data.interface_kind === "ethernet") {
    body = <p className="m-0">Connected by cable — Wi-Fi details don't apply.</p>;
  } else if (!data.connection) {
    body = <p className="m-0">Not connected to Wi-Fi.</p>;
  } else {
    body = <Details c={data.connection} />;
  }

  return (
    <section className={card} aria-label="Wi-Fi quality">
      <h2 className="m-0 text-lg font-semibold">Wi-Fi quality</h2>
      {body}
    </section>
  );
}
