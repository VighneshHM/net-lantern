import { useMemo, useState } from "react";
import { useSearchParams } from "react-router";
import { useDevices, type Device } from "../api/queries";
import { useUpdateDevice } from "../api/mutations";
import { DeviceDetails } from "../components/DeviceDetails";
import { OutdatedBanner } from "../components/OutdatedBanner";
import { DEVICE_TYPE_LABEL } from "../lib/deviceType";
import { compareIp } from "../lib/ip";
import { formatAgo } from "../lib/time";

const th = "px-3 py-2 text-left text-xs font-semibold tracking-wide text-text-muted uppercase";
const td = "px-3 py-2 align-middle whitespace-nowrap";
const hideOnPhone = "hidden sm:table-cell";
const badge =
  "rounded-full border border-border-strong bg-surface-2 px-2 py-0.5 text-xs font-medium text-text";

type Filter = "all" | "online" | "starred" | "new";
const FILTERS: { id: Filter; label: string; test: (d: Device) => boolean }[] = [
  { id: "all", label: "All", test: () => true },
  { id: "online", label: "Online", test: (d) => d.online },
  { id: "starred", label: "Starred", test: (d) => d.starred },
  { id: "new", label: "New", test: (d) => d.is_new },
];

function compareDevices(a: Device, b: Device): number {
  if (a.online !== b.online) return a.online ? -1 : 1;
  if (a.ip === null || b.ip === null) return a.ip === b.ip ? 0 : a.ip === null ? 1 : -1;
  return compareIp(a.ip, b.ip);
}

function matches(d: Device, q: string): boolean {
  return [d.display_name, d.ip, d.mac, d.vendor, DEVICE_TYPE_LABEL[d.type]].some(
    (v) => v !== null && v.toLowerCase().includes(q),
  );
}

function statusText(d: Device): string {
  if (!d.online) return `Offline · last seen ${formatAgo(d.last_seen, Date.now())}`;
  return d.answers_ping ? "Online" : "Online · doesn't answer ping";
}

function DeviceRow({
  d,
  onStar,
  onOpen,
}: {
  d: Device;
  onStar: (d: Device) => void;
  onOpen: (d: Device) => void;
}) {
  return (
    <tr className={`border-t border-border ${d.online ? "" : "text-text-muted"}`}>
      <td className={td}>
        <button
          type="button"
          aria-label={`${d.starred ? "Unstar" : "Star"} ${d.display_name}`}
          aria-pressed={d.starred}
          onClick={() => onStar(d)}
          className="min-h-11 min-w-11 cursor-pointer rounded-lg text-lg"
        >
          <span aria-hidden="true">{d.starred ? "★" : "☆"}</span>
        </button>
      </td>
      <td className={td}>
        <div className="flex flex-col items-start">
          <span className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => onOpen(d)}
              className="min-h-11 cursor-pointer text-left font-medium"
            >
              {d.display_name}
            </button>
            {d.is_new && <span className={badge}>New</span>}
            {d.is_self && <span className={badge}>This device</span>}
            {d.is_gateway && <span className={badge}>Router</span>}
          </span>
          {d.vendor && <span className="text-xs text-text-muted">{d.vendor}</span>}
          {d.ip && <span className="font-mono text-xs text-text-muted sm:hidden">{d.ip}</span>}
        </div>
      </td>
      <td className={`${td} ${hideOnPhone} font-mono`}>{d.ip ?? "—"}</td>
      <td className={`${td} ${hideOnPhone} font-mono`}>
        {d.private_mac ? "Private address" : (d.mac ?? "—")}
      </td>
      <td className={td}>{DEVICE_TYPE_LABEL[d.type]}</td>
      <td className={`${td} ${hideOnPhone}`}>{d.rtt_ms === null ? "—" : `${d.rtt_ms} ms`}</td>
      <td className={td}>
        <span className="flex items-center gap-2">
          <span
            aria-hidden="true"
            className={`size-2 rounded-full ${d.online ? (d.answers_ping ? "bg-ok" : "bg-warn") : "bg-text-muted"}`}
          />
          {statusText(d)}
        </span>
      </td>
    </tr>
  );
}

export function Devices() {
  const { data, error, isPending } = useDevices();
  const update = useUpdateDevice();
  const [query, setQuery] = useState("");
  const [starError, setStarError] = useState<{ id: number; message: string } | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [params, setParams] = useSearchParams();
  const rawId = params.get("device");
  const selected = rawId !== null && /^\d+$/.test(rawId) ? Number(rawId) : null;

  function select(id: number | null) {
    setParams(
      (p) => {
        const next = new URLSearchParams(p);
        if (id === null) next.delete("device");
        else next.set("device", String(id));
        return next;
      },
      { replace: false },
    );
  }

  const sorted = useMemo(() => (data ? [...data].sort(compareDevices) : []), [data]);
  const q = query.trim().toLowerCase();
  const active = FILTERS.find((f) => f.id === filter)!;
  const shown = sorted.filter((d) => active.test(d) && (q === "" || matches(d, q)));

  function toggleStar(d: Device) {
    // Only a retry of the same device clears its error; another device's star must not hide it.
    setStarError((e) => (e?.id === d.id ? null : e));
    // Each call reports its own failure; the shared mutation state only tracks the latest one.
    update
      .mutateAsync({ id: d.id, patch: { starred: !d.starred } })
      .catch((e: unknown) =>
        setStarError({
          id: d.id,
          message: e instanceof Error ? e.message : "Could not update the device.",
        }),
      );
  }

  let body;
  if (isPending) {
    body = <p className="m-0 text-text-muted">Loading devices…</p>;
  } else if (error) {
    body = <p className="m-0 text-text-muted">Couldn't load devices: {error.message}</p>;
  } else if (sorted.length === 0) {
    body = (
      <p className="m-0 text-text-muted">No devices yet — run a Quick scan from the Dashboard.</p>
    );
  } else {
    body = (
      <>
        <div className="flex flex-wrap items-center gap-3">
          <label className="flex flex-col gap-1 text-sm">
            <span className="sr-only">Search devices</span>
            <input
              type="search"
              aria-label="Search devices"
              placeholder="Search devices"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="min-h-11 w-64 max-w-full rounded-lg border border-border-strong bg-surface px-3 text-text"
            />
          </label>
          <div className="flex flex-wrap gap-2">
            {FILTERS.map((f) => (
              <button
                key={f.id}
                type="button"
                aria-pressed={filter === f.id}
                onClick={() => setFilter(f.id)}
                className={`min-h-11 cursor-pointer rounded-lg border px-4 text-sm font-semibold ${
                  filter === f.id
                    ? "border-accent bg-accent text-accent-ink"
                    : "border-border-strong bg-surface-2 text-text"
                }`}
              >
                {f.label} <span>{sorted.filter(f.test).length}</span>
              </button>
            ))}
          </div>
        </div>
        {starError !== null && (
          <p role="alert" className="m-0 text-sm font-medium text-bad">
            {starError.message}
          </p>
        )}
        <div className="flex flex-col gap-4 min-[760px]:flex-row min-[760px]:items-start">
          <div className="order-2 min-w-0 flex-1 min-[760px]:order-1">
            {shown.length === 0 ? (
              <p className="m-0 text-text-muted">
                {q !== "" ? `No devices match “${query.trim()}”.` : "No devices in this view."}
              </p>
            ) : (
              <div className="overflow-x-auto rounded-xl border border-border bg-surface">
                <table className="w-full border-collapse text-sm">
                  <thead>
                    <tr>
                      <th scope="col" className={th}>Star</th>
                      <th scope="col" className={th}>Name</th>
                      <th scope="col" className={`${th} ${hideOnPhone}`}>IP</th>
                      <th scope="col" className={`${th} ${hideOnPhone}`}>MAC</th>
                      <th scope="col" className={th}>Type</th>
                      <th scope="col" className={`${th} ${hideOnPhone}`}>Latency</th>
                      <th scope="col" className={th}>Status</th>
                    </tr>
                  </thead>
                  <tbody>
                    {shown.map((d) => (
                      <DeviceRow key={d.id} d={d} onStar={toggleStar} onOpen={(x) => select(x.id)} />
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
          {selected !== null && (
            <div className="order-1 min-[760px]:order-2">
              <DeviceDetails id={selected} onClose={() => select(null)} />
            </div>
          )}
        </div>
      </>
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
