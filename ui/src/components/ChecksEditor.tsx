import { useId, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { saveChecks } from "../api/alerts";
import type { Check } from "../api/monitor";
import type { Device } from "../api/queries";

const button =
  "min-h-11 cursor-pointer rounded-lg border border-border-strong bg-surface-2 px-4 text-sm font-semibold text-text";
const field = "min-h-11 rounded-lg border border-border-strong bg-surface px-3 font-normal text-text";

function describeCheck(c: Check): string {
  if (c.kind === "tcp") return `TCP port ${c.port}`;
  return `${c.https ? "HTTPS" : "HTTP"} port ${c.port}, path ${c.path}`;
}

/** Service checks for a starred device. Renders nothing for other devices. */
export function ChecksEditor({ device }: { device: Device }) {
  const qc = useQueryClient();
  const baseId = useId();
  const checks = device.checks ?? [];
  const [adding, setAdding] = useState(false);
  const [kind, setKind] = useState<"tcp" | "http">("tcp");
  const [port, setPort] = useState("80");
  const [path, setPath] = useState("/");
  const [https, setHttps] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  if (!device.starred) return null;

  async function send(next: Check[]): Promise<boolean> {
    setError(null);
    setBusy(true);
    try {
      await saveChecks(device.id, next);
      void qc.invalidateQueries({ queryKey: ["device", device.id] });
      void qc.invalidateQueries({ queryKey: ["devices"] });
      void qc.invalidateQueries({ queryKey: ["monitor"] });
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save the checks.");
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function save(e: FormEvent) {
    e.preventDefault();
    const p = Number.parseInt(port, 10);
    const c: Check = kind === "tcp" ? { kind, port: p } : { kind, port: p, path, https };
    if (await send([...checks, c])) setAdding(false);
  }

  return (
    <section aria-label="Service checks" className="flex flex-col gap-2">
      <h3 className="m-0 text-sm font-semibold">Service checks</h3>
      {checks.length === 0 && (
        <p className="m-0 text-sm text-text-muted">No service checks yet.</p>
      )}
      {checks.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-2 p-0">
          {checks.map((c, i) => (
            <li key={i} className="flex flex-wrap items-center justify-between gap-2 text-sm">
              <span id={`${baseId}-${i}`} className="min-w-0 break-words">
                {describeCheck(c)}
              </span>
              <button
                type="button"
                aria-describedby={`${baseId}-${i}`}
                disabled={busy}
                onClick={() => void send(checks.filter((_, j) => j !== i))}
                className={button}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
      {adding ? (
        <form onSubmit={(e) => void save(e)} className="flex flex-col gap-2">
          <label className="flex flex-col gap-1 text-sm font-medium">
            Kind
            <select
              value={kind}
              onChange={(e) => setKind(e.target.value as "tcp" | "http")}
              className={field}
            >
              <option value="tcp">TCP port</option>
              <option value="http">HTTP</option>
            </select>
          </label>
          <label className="flex flex-col gap-1 text-sm font-medium">
            Port
            <input
              type="number"
              inputMode="numeric"
              value={port}
              onChange={(e) => setPort(e.target.value)}
              className={field}
            />
          </label>
          {kind === "http" && (
            <>
              <label className="flex flex-col gap-1 text-sm font-medium">
                Path
                <input
                  type="text"
                  value={path}
                  onChange={(e) => setPath(e.target.value)}
                  className={field}
                />
              </label>
              <label className="flex min-h-11 items-center gap-3 text-sm font-medium">
                <input
                  type="checkbox"
                  checked={https}
                  onChange={(e) => setHttps(e.target.checked)}
                  className="size-5 accent-accent"
                />
                HTTPS
              </label>
            </>
          )}
          <div className="flex flex-wrap gap-2">
            <button type="submit" disabled={busy} className={button}>
              Save check
            </button>
            <button
              type="button"
              onClick={() => {
                setAdding(false);
                setError(null);
              }}
              className={button}
            >
              Cancel
            </button>
          </div>
        </form>
      ) : (
        <button type="button" onClick={() => setAdding(true)} className={`${button} self-start`}>
          Add check
        </button>
      )}
      {error !== null && (
        <p role="alert" className="m-0 text-sm font-medium text-bad">
          {error}
        </p>
      )}
      <p className="m-0 text-sm text-text-muted">
        Checks run every 30 s against this device's own address. Ping is always on for starred
        devices.
      </p>
    </section>
  );
}
