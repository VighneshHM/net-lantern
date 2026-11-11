import { useState, type FormEvent, type KeyboardEvent } from "react";
import { useDevice, type Device } from "../api/queries";
import { useUpdateDevice, type DevicePatch } from "../api/mutations";
import { DEVICE_TYPE_LABEL } from "../lib/deviceType";
import { formatAgo } from "../lib/time";
import { ChecksEditor } from "./ChecksEditor";

const button =
  "min-h-11 cursor-pointer rounded-lg border border-border-strong bg-surface-2 px-4 text-sm font-semibold text-text";

function typeText(d: Device): string {
  if (d.type === "unknown") return DEVICE_TYPE_LABEL.unknown;
  return `${DEVICE_TYPE_LABEL[d.type]} · ${d.type_confidence >= 0.7 ? "fairly sure" : "best guess"}`;
}

function ago(ms: number): string {
  return formatAgo(ms, Date.now());
}

function statusText(d: Device): string {
  if (!d.online) return `Offline · last seen ${ago(d.last_seen)}`;
  return d.answers_ping ? "Online" : "Online · doesn't answer ping";
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5 sm:flex-row sm:gap-3">
      <dt className="w-28 shrink-0 text-text-muted">{label}</dt>
      <dd className="m-0 break-words">{children}</dd>
    </div>
  );
}

function Loaded({ device: d, onClose }: { device: Device; onClose: () => void }) {
  const update = useUpdateDevice();
  const [draft, setDraft] = useState(d.name ?? "");
  const [error, setError] = useState<string | null>(null);

  // Follow the server's name (a save, a refetch) so the input never goes stale.
  const [seenName, setSeenName] = useState(d.name);
  if (seenName !== d.name) {
    setSeenName(d.name);
    setDraft(d.name ?? "");
  }

  function send(patch: DevicePatch) {
    setError(null);
    update
      .mutateAsync({ id: d.id, patch })
      .catch((e: unknown) =>
        setError(e instanceof Error ? e.message : "Could not update the device."),
      );
  }

  function save(e: FormEvent) {
    e.preventDefault();
    const name = draft.trim();
    send({ name: name === "" ? null : name });
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Escape") {
      setDraft(d.name ?? "");
      setError(null);
    }
  }

  const defaultName = d.hostname ?? d.ip ?? "";

  return (
    <>
      <button type="button" onClick={onClose} className={`${button} self-start min-[760px]:hidden`}>
        Back to list
      </button>
      <button type="button" onClick={onClose} className={`${button} hidden self-end min-[760px]:block`}>
        Close
      </button>
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="m-0 text-xl font-bold break-words">{d.display_name}</h2>
        {d.is_new && (
          <span className="rounded-full border border-border-strong bg-surface-2 px-2 py-0.5 text-xs font-medium">
            New device
          </span>
        )}
      </div>
      <p className="m-0 text-text-muted">{typeText(d)}</p>

      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          aria-pressed={d.starred}
          onClick={() => send({ starred: !d.starred })}
          className={button}
        >
          <span aria-hidden="true">{d.starred ? "★" : "☆"}</span> {d.starred ? "Unstar" : "Star"}
        </button>
        {d.is_new && (
          <button type="button" onClick={() => send({ is_new: false })} className={button}>
            Mark as seen
          </button>
        )}
      </div>

      <form onSubmit={save} className="flex flex-col gap-2">
        <label className="flex flex-col gap-1 text-sm font-medium">
          Name
          <input
            type="text"
            value={draft}
            placeholder={defaultName}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
            className="min-h-11 rounded-lg border border-border-strong bg-surface px-3 font-normal text-text"
          />
        </label>
        <div className="flex flex-wrap gap-2">
          <button type="submit" className={button}>
            Save
          </button>
          {d.name !== null && (
            <button type="button" onClick={() => send({ name: null })} className={button}>
              Use default name
            </button>
          )}
        </div>
        {error !== null && (
          <p role="alert" className="m-0 text-sm font-medium text-bad">
            {error}
          </p>
        )}
      </form>

      <ChecksEditor device={d} />

      <dl className="m-0 flex flex-col gap-2 text-sm">
        <Row label="Vendor">
          {d.private_mac ? "Private address — vendor hidden" : (d.vendor ?? "Unknown")}
        </Row>
        <Row label="IP">
          <span className="font-mono">{d.ip ?? "—"}</span>
        </Row>
        <Row label="MAC">
          <span className="font-mono">{d.mac ?? "—"}</span>
        </Row>
        <Row label="Latency">{d.rtt_ms === null ? "—" : `${d.rtt_ms} ms`}</Row>
        <Row label="Status">{statusText(d)}</Row>
        <Row label="First seen">{ago(d.first_seen)}</Row>
        <Row label="Last seen">{ago(d.last_seen)}</Row>
        <Row label="Open ports">
          {d.open_ports.length > 0 ? d.open_ports.join(", ") : "none of the common ports"}
        </Row>
        <Row label="Announced as">
          {d.services.length > 0 ? (
            <ul className="m-0 list-none p-0">
              {d.services.map((s, i) => (
                <li key={i}>{`${s.source}: ${s.value}`}</li>
              ))}
            </ul>
          ) : (
            "Nothing announced"
          )}
        </Row>
      </dl>

      <p className="m-0 text-sm text-text-muted">
        Don't recognise it? Net Lantern can't block devices. Check your router's client list, or
        change the Wi-Fi password.
      </p>
    </>
  );
}

export function DeviceDetails({ id, onClose }: { id: number; onClose: () => void }) {
  const { data, error, isPending } = useDevice(id);

  let body;
  if (isPending) {
    body = <p className="m-0 text-text-muted">Loading…</p>;
  } else if (error || !data) {
    body = (
      <>
        <button type="button" onClick={onClose} className={`${button} self-start`}>
          Close
        </button>
        <p className="m-0 text-text-muted">Device not found.</p>
      </>
    );
  } else {
    body = <Loaded key={data.id} device={data} onClose={onClose} />;
  }

  return (
    <aside
      aria-label="Device details"
      className="flex w-full flex-col gap-4 rounded-xl border border-border bg-surface p-4 min-[760px]:w-96 min-[760px]:shrink-0"
    >
      {body}
    </aside>
  );
}
