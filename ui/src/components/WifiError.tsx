import { ApiRequestError } from "../api/client";

/**
 * A Wi-Fi read that failed for a reason other than location access: the
 * server's message with its hint underneath, or a plain sentence when the
 * failure is not an API error.
 */
export function WifiError({ error }: { error: Error }) {
  if (!(error instanceof ApiRequestError)) {
    return <p className="m-0 text-text-muted">Couldn't read Wi-Fi details.</p>;
  }
  return (
    <>
      <p className="m-0 text-text-muted">{error.error.message}</p>
      {error.error.hint && <p className="m-0 text-sm">{error.error.hint}</p>}
    </>
  );
}
