import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useMonitor } from "../api/monitor";
import {
  ApiRequestError,
  cancelScan,
  fetchLatestScan,
  refreshScan,
  startScan,
  useNetwork,
  useSetScanConsent,
} from "../api/queries";
import { useAnswerReplacedRouter } from "../api/settings";
import { useWifi } from "../api/wifi";
import { ConsentPrompt } from "../components/ConsentPrompt";
import { FirstRun } from "../components/FirstRun";
import { KpiTiles } from "../components/KpiTiles";
import { LatencyChart } from "../components/LatencyChart";
import { OutdatedBanner } from "../components/OutdatedBanner";
import { RecentAlerts } from "../components/RecentAlerts";
import { RouterPrompt } from "../components/RouterPrompt";
import { StarredDevices } from "../components/StarredDevices";
import { UpdateBanner, UpdateQuestion } from "../components/UpdateCard";
import { networkAddress } from "../lib/ip";
import { useLive, type LiveScan } from "../state/live";

const STAGE_LABEL: Record<string, string> = {
  sweep: "Pinging addresses",
  arp: "Reading the ARP table",
  ports: "Checking common ports",
  discover: "Asking devices their names",
  merge: "Identifying devices",
  port_scan: "Scanning ports",
  banners: "Reading what open ports say",
};

const COUNTED_STAGES = new Set(["sweep", "port_scan", "banners"]);

const ISOLATED_TEXT =
  "This network appears to isolate devices — only the router answered, so other devices can't be seen from here.";

/** What a deep scan fails with when its own consent recheck finds none. */
const NO_CONSENT_TEXT = "Deep scan needs your permission for this network.";

/** What a scan fails with when it finds a known network behind a new router (core NEW_ROUTER_TEXT). */
const NEW_ROUTER_TEXT = "This looks like a network you know with a new router.";

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
      return scan.error ? `The scan stopped: ${scan.error}` : "The scan was cancelled.";
    default: {
      const stage = STAGE_LABEL[scan.stage] ?? scan.stage;
      const head =
        COUNTED_STAGES.has(scan.stage) && scan.total > 0
          ? `${stage} · ${scan.done} of ${scan.total}`
          : `${stage}…`;
      return scan.devicesFound > 0 ? `${head} · ${scan.devicesFound} found so far` : head;
    }
  }
}

function NetworkCard() {
  const { data, error, isPending } = useNetwork();
  const wifi = useWifi();
  const ssid = wifi.data?.connection?.ssid ?? null;

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
      {ssid !== null && <p className="m-0 text-sm text-text-muted">Wi-Fi: {ssid}</p>}
    </section>
  );
}

function MonitoringLine() {
  const { data } = useMonitor();
  if (!data) return null;
  let text: string;
  switch (data.state) {
    case "monitoring":
      text =
        data.session_started_at === null
          ? "Monitoring"
          : `Monitoring since ${new Date(data.session_started_at).toLocaleTimeString([], {
              hour: "numeric",
              minute: "2-digit",
            })}`;
      break;
    case "waiting_for_scan":
      text = "Run a quick scan to start monitoring this network.";
      break;
    default:
      text = "Not monitoring: not connected.";
  }
  return <p className="m-0 text-sm text-text-muted">{text}</p>;
}

export function Dashboard() {
  const scan = useLive((s) => s.scan);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [asking, setAsking] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [routerAsk, setRouterAsk] = useState(false);
  const [answering, setAnswering] = useState(false);
  const [routerError, setRouterError] = useState<string | null>(null);
  // The scan a router question interrupted, restarted in the same mode once answered.
  const pendingMode = useRef<"quick" | "deep" | null>(null);
  const [stoppingId, setStoppingId] = useState<number | null>(null);
  const network = useNetwork().data;
  const setScanConsent = useSetScanConsent();
  const answerRouter = useAnswerReplacedRouter();
  const latest = useQuery({ queryKey: ["latestScan"], queryFn: fetchLatestScan, retry: false });
  const queryClient = useQueryClient();
  const deepButton = useRef<HTMLButtonElement>(null);
  const quickButton = useRef<HTMLButtonElement>(null);
  const running = scan?.state === "running";
  const busy = starting || running;
  const stopping = running && stoppingId === scan.id;
  // Before this network's first scan: no saved network yet, or no scan for it.
  const firstRun =
    network !== undefined &&
    !busy &&
    scan === null &&
    (network.network_id === null || (!latest.isPending && latest.data === null));

  // A deep scan that found, at its start, that this network has no consent
  // (another router, or consent taken back) asks again, once per scan.
  useEffect(() => {
    let handled: number | null = null;
    let ended: number | null = null;
    return useLive.subscribe(({ scan: s }) => {
      // A finished scan makes the cached "no scan yet" answer stale.
      if (s && s.state !== "running" && ended !== s.id) {
        ended = s.id;
        void queryClient.invalidateQueries({ queryKey: ["latestScan"] });
      }
      if (s?.state !== "failed" || handled === s.id) return;
      if (s.error === NO_CONSENT_TEXT) {
        handled = s.id;
        setAsking(true);
        void queryClient.invalidateQueries({ queryKey: ["network"] });
      } else if (s.error === NEW_ROUTER_TEXT) {
        handled = s.id;
        setRouterAsk(true);
        void queryClient.invalidateQueries({ queryKey: ["network"] });
      }
    });
  }, [queryClient]);

  async function runScan(mode: "quick" | "deep") {
    setError(null);
    setStarting(true);
    try {
      const id = await startScan(mode);
      useLive.getState().attach(id);
      void refreshScan(id);
    } catch (e) {
      if (e instanceof ApiRequestError && e.error.code === "consent_needed") {
        setAsking(true);
      } else if (e instanceof ApiRequestError && e.error.code === "network_decision_needed") {
        askRouter(mode);
      } else {
        setError(e instanceof Error ? e.message : "Could not start the scan.");
      }
    } finally {
      setStarting(false);
    }
  }

  function askRouter(mode: "quick" | "deep") {
    pendingMode.current = mode;
    setRouterError(null);
    setRouterAsk(true);
    void queryClient.invalidateQueries({ queryKey: ["network"] });
  }

  async function answerRouterQuestion(keep: boolean) {
    setAnswering(true);
    setRouterError(null);
    try {
      await answerRouter(keep);
    } catch (e) {
      if (e instanceof ApiRequestError && e.error.code === "no_decision_pending") {
        // Already answered elsewhere: the question is gone, so carry on.
        void queryClient.invalidateQueries({ queryKey: ["network"] });
      } else {
        setRouterError(e instanceof Error ? e.message : "Could not save your answer.");
        return;
      }
    } finally {
      setAnswering(false);
    }
    setRouterAsk(false);
    void queryClient.invalidateQueries({ queryKey: ["devices"] });
    const mode = pendingMode.current;
    pendingMode.current = null;
    if (mode !== null) {
      await runScan(mode);
    } else if (useLive.getState().scan?.error === NEW_ROUTER_TEXT) {
      // The failed scan's message is obsolete now.
      useLive.setState({ scan: null });
    }
  }

  function deepScan() {
    if (network?.scan_consent === false) {
      setAsking(true);
      return;
    }
    void runScan("deep");
  }

  async function confirmConsent() {
    setConfirming(true);
    try {
      await setScanConsent(true);
      setAsking(false);
    } catch (e) {
      setAsking(false);
      if (e instanceof ApiRequestError && e.error.code === "network_decision_needed") {
        askRouter("deep");
      } else {
        setError(e instanceof Error ? e.message : "Could not save your answer.");
      }
      return;
    } finally {
      setConfirming(false);
    }
    await runScan("deep");
  }

  async function stop(id: number) {
    setError(null);
    setStoppingId(id);
    try {
      await cancelScan(id);
    } catch (e) {
      // 409: it had just finished, and its done event is on its way.
      if (!(e instanceof ApiRequestError && e.error.code === "scan_not_running")) {
        setStoppingId(null);
        setError(e instanceof Error ? e.message : "Could not stop the scan.");
      }
    }
  }

  return (
    <>
      <h1 className="m-0 text-3xl font-bold">Dashboard</h1>
      <OutdatedBanner />
      <UpdateBanner />
      {(routerAsk || network?.replaced_router != null) && (
        <RouterPrompt
          label={network?.replaced_router?.label ?? null}
          busy={answering}
          error={routerError}
          returnFocus={pendingMode.current === "deep" ? deepButton : quickButton}
          onAnswer={(keep) => void answerRouterQuestion(keep)}
        />
      )}
      <NetworkCard />
      {firstRun && <FirstRun network={network} onQuickScan={() => void runScan("quick")} />}
      <UpdateQuestion />

      <section className={card} aria-label="Scan">
        {!firstRun && (
          <div className="flex flex-wrap items-center gap-3">
            <button
              ref={quickButton}
              type="button"
              className={`${button} bg-accent text-accent-ink`}
              disabled={busy}
              onClick={() => void runScan("quick")}
            >
              {busy ? "Scanning…" : "Quick scan"}
            </button>
            <button
              type="button"
              className={`${button} border border-border-strong bg-surface-2 text-text`}
              ref={deepButton}
              disabled={busy}
              onClick={deepScan}
            >
              Deep scan
            </button>
            {running && (
              <button
                type="button"
                className={`${button} border border-border-strong bg-surface text-text`}
                disabled={stopping}
                onClick={() => void stop(scan.id)}
              >
                {stopping ? "Stopping…" : "Stop scan"}
              </button>
            )}
          </div>
        )}

        {asking && (
          <ConsentPrompt
            busy={confirming}
            returnFocus={deepButton}
            onConfirm={() => void confirmConsent()}
            onCancel={() => setAsking(false)}
          />
        )}

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

      <MonitoringLine />
      <KpiTiles />
      <LatencyChart />
      <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
        <StarredDevices />
        <RecentAlerts />
      </div>
    </>
  );
}
