import { useState } from "react";
import { Link } from "react-router";
import { useNetwork, useSetScanConsent } from "../api/queries";
import { PhoneAccess } from "../components/PhoneAccess";

const card = "flex flex-col gap-3 rounded-xl border border-border bg-surface p-5";
const heading = "m-0 text-lg font-semibold";
const button =
  "min-h-11 rounded-lg border border-border-strong bg-surface px-5 text-sm font-semibold text-text disabled:cursor-not-allowed disabled:opacity-60";

/** How long each kind of data is kept (spec §6.12). */
const DATA_KEPT = [
  { label: "Raw samples", kept: "24 hours" },
  { label: "5-minute averages", kept: "30 days" },
  { label: "Online/offline history", kept: "90 days" },
  { label: "Logs", kept: "7 days" },
] as const;

const CATEGORY = { public: "Public", private: "Private", domain: "Domain" } as const;

function ThisNetwork() {
  const { data, error, isPending } = useNetwork();
  const setConsent = useSetScanConsent();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);

  async function withdraw() {
    setBusy(true);
    setFailed(null);
    try {
      await setConsent(false);
    } catch (e) {
      setFailed(e instanceof Error ? e.message : "Couldn't withdraw permission.");
    } finally {
      setBusy(false);
    }
  }

  if (isPending) return <p className="m-0 text-text-muted">Checking your network…</p>;
  if (error) {
    return (
      <p role="alert" className="m-0 text-sm font-medium text-bad">
        {error.message}
      </p>
    );
  }
  return (
    <>
      <p className="m-0">
        <span className="font-medium">{data.ssid ?? "Unknown name"}</span>{" "}
        <span className="text-text-muted">·</span> <span>{data.ip}</span>
      </p>
      <p className="m-0 text-sm text-text-muted">
        {data.category ? (
          <>
            Windows marks it <strong className="text-text">{CATEGORY[data.category]}</strong>
          </>
        ) : (
          "Windows' network type couldn't be read."
        )}
      </p>
      {data.scan_consent ? (
        <div className="flex flex-wrap items-center gap-3">
          <p className="m-0 text-sm">Deep scans allowed on this network</p>
          <button type="button" className={button} disabled={busy} onClick={() => void withdraw()}>
            Withdraw permission
          </button>
        </div>
      ) : (
        <p className="m-0 text-sm text-text-muted">
          Deep scans not allowed yet; you'll be asked before the first one.
        </p>
      )}
      {failed && (
        <p role="alert" className="m-0 text-sm font-medium text-bad">
          {failed}
        </p>
      )}
    </>
  );
}

export default function Settings() {
  return (
    <>
      <h1 className="m-0 text-3xl font-bold">Settings</h1>
      <section className={card} aria-label="Phone access">
        <h2 className={heading}>Phone access</h2>
        <PhoneAccess />
      </section>
      <section className={card} aria-label="This network">
        <h2 className={heading}>This network</h2>
        <ThisNetwork />
      </section>
      <section className={card} aria-label="Alerts and notifications">
        <h2 className={heading}>Alerts and notifications</h2>
        <p className="m-0 text-sm text-text-muted">
          Alert rules and desktop notifications are on the{" "}
          <Link to="/alerts" data-inline-link className="text-accent underline">
            Alerts screen
          </Link>
          .
        </p>
      </section>
      <section className={card} aria-label="Data kept">
        <h2 className={heading}>Data kept</h2>
        <ul className="m-0 flex list-none flex-col gap-1 p-0 text-sm">
          {DATA_KEPT.map((row) => (
            <li key={row.label}>
              {row.label}: <span className="text-text-muted">{row.kept}</span>
            </li>
          ))}
        </ul>
      </section>
    </>
  );
}
