import type { Network } from "../api/queries";
import { networkAddress } from "../lib/ip";

const CATEGORY_LABEL: Record<string, string> = {
  public: "Public",
  private: "Private",
  domain: "Domain",
};

/**
 * Shown on the Dashboard before this network has been scanned. Its one button
 * replaces the scan card's, so the same action is never offered twice.
 */
export function FirstRun({
  network,
  onQuickScan,
}: {
  network: Network;
  onQuickScan: () => void;
}) {
  const subnet = networkAddress(network.ip, network.prefix);
  const name = network.ssid ?? (subnet === null ? network.ip : `${network.ip} · ${subnet}/${network.prefix}`);
  const category = network.category === null ? null : CATEGORY_LABEL[network.category];

  return (
    <section
      aria-labelledby="first-run-heading"
      className="flex flex-col gap-2 rounded-xl border border-border-strong bg-surface p-5"
    >
      <h2 id="first-run-heading" className="m-0 text-lg font-semibold">
        Network detected
      </h2>
      <p className="m-0 font-mono text-base">{name}</p>
      {category && (
        <p className="m-0 text-sm text-text-muted">
          Windows marks this network <strong className="font-semibold text-text">{category}</strong>
        </p>
      )}
      {network.category === "public" && (
        <p className="m-0 text-sm text-text-muted">
          Some features, like phone access, stay off on Public networks.
        </p>
      )}
      <p className="m-0 text-sm">Run your first scan to see the devices on this network.</p>
      <div className="flex flex-wrap items-center gap-3">
        <button
          type="button"
          className="min-h-11 rounded-lg bg-accent px-5 text-sm font-semibold text-accent-ink"
          onClick={onQuickScan}
        >
          Run quick scan
        </button>
      </div>
      <p className="m-0 text-xs text-text-muted">A deep scan asks for your permission first.</p>
    </section>
  );
}
