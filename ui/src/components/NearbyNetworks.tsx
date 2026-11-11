import { useEffect, useState } from "react";
import { ApiRequestError } from "../api/client";
import { useNearby, useWifi } from "../api/wifi";
import { BAND_LABEL } from "../lib/band";
import { signalLabel } from "../lib/signal";
import { formatAgo } from "../lib/time";
import { CongestionChart, showChart } from "./CongestionChart";
import { WifiError } from "./WifiError";

const card = "flex flex-col gap-3 rounded-xl border border-border bg-surface p-5";

export function NearbyNetworks() {
  const { data, error, isPending, isFetching, refetch } = useNearby();
  const wifi = useWifi();
  // Re-render now and then so "Scanned just now" doesn't stay true for an hour.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, []);
  const mine = wifi.data?.connection?.bssid.toLowerCase() ?? null;

  let body;
  if (isPending) {
    body = <p className="m-0 text-text-muted">Looking for nearby networks…</p>;
  } else if (error) {
    body =
      error instanceof ApiRequestError &&
      error.status === 403 &&
      error.error.code === "location_permission" ? (
        <p className="m-0 text-text-muted">Turn on location access (above) to see nearby networks.</p>
      ) : (
        <WifiError error={error} />
      );
  } else {
    const { networks, congestion } = data;
    const sorted = [...networks].sort((a, b) => b.rssi_dbm - a.rssi_dbm);
    body = (
      <>
        <table className="w-full border-collapse text-left text-sm">
          <thead>
            <tr className="text-text-muted">
              <th className="py-1 pr-3 font-medium">Network</th>
              <th className="py-1 pr-3 font-medium">Signal</th>
              <th className="hidden py-1 pr-3 font-medium sm:table-cell">Band</th>
              <th className="py-1 font-medium">Channel</th>
            </tr>
          </thead>
          <tbody>
            {sorted.map((n) => (
              <tr key={n.bssid} className="border-t border-border">
                <td className="py-1.5 pr-3 break-words">
                  {n.ssid === "" ? "(hidden network)" : n.ssid}
                  {n.bssid.toLowerCase() === mine && <span className="text-text-muted"> (you)</span>}
                </td>
                <td className="py-1.5 pr-3">
                  {n.rssi_dbm} dBm · {signalLabel(n.rssi_dbm)}
                </td>
                <td className="hidden py-1.5 pr-3 sm:table-cell">{BAND_LABEL[n.band]}</td>
                <td className="py-1.5">
                  {n.channel}
                  <span className="text-text-muted sm:hidden"> · {BAND_LABEL[n.band]}</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="m-0 text-sm text-text-muted">Scanned {formatAgo(data.scanned_at, now)}</p>
        <p className="m-0">{congestion.advice}</p>
        <CongestionChart band="2.4ghz" congestion={congestion} />
        {showChart(congestion, "5ghz") && <CongestionChart band="5ghz" congestion={congestion} />}
      </>
    );
  }

  return (
    <section className={card} aria-label="Nearby networks">
      <div className="flex items-center justify-between gap-3">
        <h2 className="m-0 text-lg font-semibold">Nearby networks</h2>
        <button
          type="button"
          onClick={() => void refetch()}
          disabled={isFetching}
          className="min-h-11 cursor-pointer rounded-lg border border-border-strong bg-surface-2 px-5 text-sm font-semibold text-text disabled:cursor-not-allowed disabled:opacity-60"
        >
          {isFetching ? "Scanning…" : "Refresh"}
        </button>
      </div>
      {body}
    </section>
  );
}
