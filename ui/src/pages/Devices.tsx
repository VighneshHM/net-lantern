import { useMemo } from "react";
import { useDevices, type Device } from "../api/queries";
import { OutdatedBanner } from "../components/OutdatedBanner";
import { compareIp } from "../lib/ip";

const th = "px-3 py-2 text-left text-xs font-semibold tracking-wide text-text-muted uppercase";
const td = "px-3 py-3 align-middle whitespace-nowrap";
const badge =
  "rounded-full border border-border-strong bg-surface-2 px-2 py-0.5 text-xs font-medium text-text";

function DeviceRow({ d }: { d: Device }) {
  return (
    <tr className="border-t border-border">
      <td className={td}>
        <span className="flex items-center gap-2">
          <span>{d.ip}</span>
          {d.is_self && <span className={badge}>This device</span>}
          {d.is_gateway && <span className={badge}>Router</span>}
        </span>
      </td>
      <td className={`${td} font-mono`}>{d.ip}</td>
      <td className={`${td} font-mono`}>{d.private_mac ? "Private address" : (d.mac ?? "—")}</td>
      <td className={td}>{d.rtt_ms === null ? "—" : `${d.rtt_ms} ms`}</td>
      <td className={td}>
        <span className="flex items-center gap-2">
          <span
            aria-hidden="true"
            className={`size-2 rounded-full ${d.answers_ping ? "bg-ok" : "bg-warn"}`}
          />
          {d.answers_ping ? "Online" : "Online · doesn't answer ping"}
        </span>
      </td>
    </tr>
  );
}

export function Devices() {
  const { data, error, isPending } = useDevices();
  const devices = useMemo(() => (data ? [...data].sort((a, b) => compareIp(a.ip, b.ip)) : []), [data]);

  let body;
  if (isPending) {
    body = <p className="m-0 text-text-muted">Loading devices…</p>;
  } else if (error) {
    body = <p className="m-0 text-text-muted">Couldn't load devices: {error.message}</p>;
  } else if (devices.length === 0) {
    body = (
      <p className="m-0 text-text-muted">No devices yet — run a Quick scan from the Dashboard.</p>
    );
  } else {
    body = (
      <div className="overflow-x-auto rounded-xl border border-border bg-surface">
        <table className="w-full border-collapse text-sm">
          <thead>
            <tr>
              <th scope="col" className={th}>Name</th>
              <th scope="col" className={th}>IP</th>
              <th scope="col" className={th}>MAC</th>
              <th scope="col" className={th}>Latency</th>
              <th scope="col" className={th}>Status</th>
            </tr>
          </thead>
          <tbody>
            {devices.map((d) => (
              <DeviceRow key={d.ip} d={d} />
            ))}
          </tbody>
        </table>
      </div>
    );
  }

  return (
    <>
      <h1 className="m-0 text-3xl font-bold">Devices</h1>
      <OutdatedBanner />
      {body}
    </>
  );
}
