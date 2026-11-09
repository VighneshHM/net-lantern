import { useEffect, useState } from "react";
import { ApiRequestError } from "../api/client";
import {
  fetchDiagnosis,
  fetchLatestDiagnosis,
  startDiagnosis,
  type DiagnosisRun,
  type LayerStatus,
} from "../api/diagnose";
import { LayerTrace } from "../components/LayerTrace";
import { formatAgo } from "../lib/time";
import { useLive } from "../state/live";

const card = "flex flex-col gap-2 rounded-xl border border-border bg-surface p-5";

const VERDICT_STATUS: Record<LayerStatus, { text: string; color: string }> = {
  pass: { text: "All clear", color: "text-ok" },
  warn: { text: "Warning", color: "text-warn" },
  fail: { text: "Problem found", color: "text-bad" },
  skipped: { text: "Not checked", color: "text-text-muted" },
};

function checkedText(finishedAt: number): string {
  return `Checked ${formatAgo(finishedAt, Date.now())}`;
}

function VerdictCard({ run }: { run: DiagnosisRun | null }) {
  const verdict = run?.verdict ?? null;
  if (!run || (run.state === "running" && !verdict)) {
    return (
      <section className={card} aria-label="Verdict">
        <p className="m-0 text-text-muted">
          {run
            ? "Checking each step from your Wi-Fi to a website…"
            : "Run a diagnosis to check each step from your Wi-Fi to a website."}
        </p>
      </section>
    );
  }
  if (!verdict) return null;
  const status = VERDICT_STATUS[verdict.status];
  return (
    <section className={card} aria-label="Verdict">
      <span className={`text-sm font-semibold ${status.color}`}>{status.text}</span>
      <h2 className="m-0 text-xl font-semibold">{verdict.headline}</h2>
      {verdict.fix && <p className="m-0">Try this: {verdict.fix}</p>}
      {run.finished_at !== null && (
        <p className="m-0 text-sm text-text-muted">
          {checkedText(run.finished_at)}
        </p>
      )}
    </section>
  );
}

/** Show the latest run unless the live store already has this run, or a newer one, further along. */
function shouldAdopt(current: DiagnosisRun | null, latest: DiagnosisRun): boolean {
  if (!current || latest.id > current.id) return true;
  return latest.id === current.id && current.state === "running" && latest.state !== "running";
}

export function Diagnose() {
  const run = useLive((s) => s.diagnosis);
  const [starting, setStarting] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void fetchLatestDiagnosis().then((latest) => {
      if (cancelled || !latest) return;
      if (shouldAdopt(useLive.getState().diagnosis, latest)) {
        useLive.getState().followDiagnosis(latest);
      }
    });
    return () => {
      cancelled = true;
    };
  }, []);

  async function onRun() {
    setStarting(true);
    setStartError(null);
    try {
      const id = await startDiagnosis();
      // Follow the run before the snapshot call, so events arriving meanwhile land.
      if (useLive.getState().diagnosis?.id !== id) {
        useLive.getState().followDiagnosis({
          id,
          state: "running",
          started_at: Date.now(),
          finished_at: null,
          steps: [],
          verdict: null,
          error: null,
        });
      }
      const fetched = await fetchDiagnosis(id);
      if (fetched) useLive.getState().mergeDiagnosis(fetched);
    } catch (e) {
      setStartError(
        e instanceof ApiRequestError ? e.error.message : "Could not start the diagnosis.",
      );
    } finally {
      setStarting(false);
    }
  }

  const busy = starting || run?.state === "running";

  return (
    <>
      <h1 className="m-0 text-3xl font-bold">Diagnose</h1>
      <section className="flex flex-col gap-4" aria-label="Diagnosis">
        <div>
          <button
            type="button"
            onClick={() => void onRun()}
            disabled={busy}
            className="min-h-11 cursor-pointer rounded-lg bg-accent px-5 text-sm font-semibold text-accent-ink disabled:cursor-not-allowed disabled:opacity-60"
          >
            {busy ? "Diagnosing…" : "Run diagnosis"}
          </button>
        </div>
        {startError && (
          <p role="alert" className="m-0 text-bad">
            {startError}
          </p>
        )}
        {run?.state === "failed" && (
          <p role="alert" className="m-0 text-bad">
            The diagnosis stopped: {run.error ?? "unknown error"}
          </p>
        )}
        <VerdictCard run={run} />
        <LayerTrace run={run} />
      </section>
      {/* The Wi-Fi section (<WifiQuality />) goes here. */}
    </>
  );
}
