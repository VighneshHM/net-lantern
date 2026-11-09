import { useQueryClient } from "@tanstack/react-query";

/** Walks the user through Windows' location setting, which gates Wi-Fi details. */
export function LocationPermission() {
  const client = useQueryClient();

  function checkAgain() {
    void client.refetchQueries({ queryKey: ["wifi"] });
    void client.refetchQueries({ queryKey: ["nearby"] });
  }

  return (
    <div className="flex flex-col gap-3">
      <h3 className="m-0 text-lg font-semibold">Windows is hiding Wi-Fi details</h3>
      <p className="m-0">
        Windows only shares Wi-Fi names and signal details with apps that have location access. Net
        Lantern doesn't use your location — it only reads Wi-Fi details.
      </p>
      <ol className="m-0 flex list-decimal flex-col gap-1 pl-6">
        <li>Open Settings &gt; Privacy &amp; security &gt; Location.</li>
        <li>Turn on Location services.</li>
        <li>Turn on Let desktop apps access your location.</li>
      </ol>
      <div className="flex flex-wrap items-center gap-3">
        <a
          href="ms-settings:privacy-location"
          className="inline-flex min-h-11 items-center rounded-lg bg-accent px-5 text-sm font-semibold text-accent-ink no-underline"
        >
          Open location settings
        </a>
        <button
          type="button"
          onClick={checkAgain}
          className="min-h-11 cursor-pointer rounded-lg border border-border-strong bg-surface-2 px-5 text-sm font-semibold text-text"
        >
          Check again
        </button>
      </div>
      <p className="m-0 text-sm text-text-muted">
        Diagnosis still checks your router, internet and DNS without it.
      </p>
    </div>
  );
}
