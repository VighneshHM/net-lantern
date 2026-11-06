import { useLive } from "../state/live";

/** Shown on every page that has live data when the server speaks a newer API than this build. */
export function OutdatedBanner() {
  const outdated = useLive((s) => s.outdated);
  if (!outdated) return null;
  return (
    <p
      role="status"
      className="m-0 rounded-lg border border-warn bg-surface px-4 py-3 text-sm font-medium text-text"
    >
      This dashboard is out of date — reload
    </p>
  );
}
