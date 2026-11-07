import { useState } from "react";
import { ApiRequestError, refreshScan, startScan, useNetwork } from "../api/queries";
import { OutdatedBanner } from "../components/OutdatedBanner";
import { networkAddress } from "../lib/ip";
import { useLive, type LiveScan } from "../state/live";

const STAGE_LABEL: Record<string, string> = {
  sweep: "Pinging addresses",
  arp: "Reading the ARP table",
  ports: "Checking common ports",
  discover: "Asking devices their names",
  merge: "Identifying devices",
};

const ISOLATED_TEXT =
  "This network appears to isolate devices — only the router answered, so other devices can't be seen from here.";

const card = "flex flex-col gap-2 rounded-xl border border-border bg-surface p-5";
const button =
  "min-h-11 rounded-lg px-5 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-60";

function percent(scan: LiveScan): number {
  if (scan.state === "done") return 100;
  if (scan.total <= 0) return 0;
  return Math.min(100, Math.floor((scan.done / scan.total) * 100));
}

function scanSummary(scan: LiveScan): string {
  switch (scan.state) {
    case "done":
      return `${scan.devicesFound} ${scan.devicesFound === 1 ? "device" : "devices"} found`;
    case "failed":
      return scan.error ? `The scan failed: ${scan.error}` : "The scan failed.";
    case "cancelled":
      return "The scan was cancelled.";
    default: {
      const stage = STAGE_LABEL[scan.stage] ?? scan.stage;
      const head =
        scan.stage === "sweep" && scan.total > 0
          ? `${stage} · ${scan.done} of ${scan.total}`
          : `${stage}…`;
      return scan.devicesFound > 0 ? `${head} · ${scan.devicesFound} found so far` : head;
    }
  }
}

function NetworkCard() {
  const { data, error, isPending } = useNetwork();

  if (isPending) {
    return (
      <section className={card} aria-label="Network">
        <p className="m-0 text-text-muted">Checking your network…</p>
      </section>
    );
  }
  if (error) {
    const e = error instanceof ApiRequestError ? error : null;
    if (e?.status === 503) {
      return (
        <section className={card} aria-label="Network">
          <h2 className="m-0 text-lg font-semibold">Not connected</h2>
          <p className="m-0 text-text-muted">{e.error.hint ?? e.error.message}</p>
        </section>
      );
    }
    return (
      <section className={card} aria-label="Network">
        <h2 className="m-0 text-lg font-semibold">Can't read your network</h2>
        <p className="m-0 text-text-muted">{error.message}</p>
      </section>
    );
  }

  const net = networkAddress(data.ip, data.prefix);
  const parts = [
    data.ip,
    net === null ? null : `${net}/${data.prefix}`,
    data.gateway ? `gateway ${data.gateway}` : null,
  ].filter((p): p is string => Boolean(p));
  return (
    <section className={card} aria-label="Network">
      <h2 className="m-0 text-sm font-medium text-text-muted">Your network</h2>
      <p className="m-0 font-mono text-lg">{parts.join(" · ")}</p>
    </section>
  );
}

export function Dashboard() {
  const scan = useLive((s) => s.scan);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const busy = starting || scan?.state === "running";

  async function quickScan() {
    setError(null);
    setStarting(true);
    try {
      const id = await startScan("quick");
      useLive.getState().attach(id);
      void refreshScan(id);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not start the scan.");
    } finally {
      setStarting(false);
    }
  }

  return (
    <>
      <h1 className="m-0 text-3xl font-bold">Dashboard</h1>
      <OutdatedBanner />
      <NetworkCard />

      <section className={card} aria-label="Scan">
        <div className="flex flex-wrap items-center gap-3">
          <button
            type="button"
            className={`${button} bg-accent text-accent-ink`}
            disabled={busy}
            onClick={() => void quickScan()}
          >
            {busy ? "Scanning…" : "Quick scan"}
          </button>
          <div className="flex flex-col">
            <button
              type="button"
              className={`${button} border border-border-strong bg-surface-2 text-text`}
              disabled
              aria-describedby="deep-scan-note"
            >
              Deep scan
            </button>
            <span id="deep-scan-note" className="text-xs text-text-muted">
              Arrives in phase 4
            </span>
          </div>
        </div>

        {error !== null && (
          <p role="alert" className="m-0 text-sm font-medium text-bad">
            {error}
          </p>
        )}

        {scan && (
          <div className="flex flex-col gap-2">
            <div
              role="progressbar"
              aria-label="Scan progress"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={percent(scan)}
              className="h-2.5 overflow-hidden rounded-full bg-surface-2"
            >
              <div className="h-full bg-accent" style={{ width: `${percent(scan)}%` }} />
            </div>
            <p className="m-0 text-sm text-text-muted">{scanSummary(scan)}</p>
            {scan.state === "done" && scan.isolated && (
              <p className="m-0 text-sm font-medium text-text">{ISOLATED_TEXT}</p>
            )}
          </div>
        )}
      </section>
    </>
  );
}
