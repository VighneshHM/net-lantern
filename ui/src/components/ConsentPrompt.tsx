const button =
  "min-h-11 rounded-lg px-5 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-60";

/** Asks once per network whether the user may scan it. A card in the page, not a modal. */
export function ConsentPrompt({
  onConfirm,
  onCancel,
  busy,
}: {
  onConfirm: () => void;
  onCancel: () => void;
  busy: boolean;
}) {
  return (
    <div
      role="dialog"
      aria-labelledby="consent-heading"
      className="flex flex-col gap-3 rounded-lg border border-border-strong bg-surface-2 p-4"
    >
      <h3 id="consent-heading" className="m-0 text-base font-semibold">
        Is this a network you own or have permission to scan?
      </h3>
      <p className="m-0 text-sm text-text-muted">
        A deep scan checks 100 common ports on every device and reads what they say about
        themselves. Only scan networks you own or have permission to scan.
      </p>
      <div className="flex flex-wrap gap-3">
        <button
          type="button"
          className={`${button} bg-accent text-accent-ink`}
          disabled={busy}
          onClick={onConfirm}
        >
          Yes, I have permission
        </button>
        <button
          type="button"
          className={`${button} border border-border-strong bg-surface text-text`}
          disabled={busy}
          onClick={onCancel}
        >
          Not now
        </button>
      </div>
    </div>
  );
}
