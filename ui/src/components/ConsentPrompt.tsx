import { useEffect, useRef, type RefObject } from "react";

const button =
  "min-h-11 rounded-lg px-5 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-60";

/**
 * Asks once per network whether the user may scan it. A card in the page, not
 * a modal. Focus moves to its heading when it opens, and back to `returnFocus`
 * when it closes.
 */
export function ConsentPrompt({
  onConfirm,
  onCancel,
  busy,
  returnFocus,
}: {
  onConfirm: () => void;
  onCancel: () => void;
  busy: boolean;
  returnFocus?: RefObject<HTMLElement | null>;
}) {
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus();
    const back = returnFocus?.current;
    return () => back?.focus();
  }, [returnFocus]);

  return (
    <div
      role="dialog"
      aria-labelledby="consent-heading"
      className="flex flex-col gap-3 rounded-lg border border-border-strong bg-surface-2 p-4"
    >
      <h3
        id="consent-heading"
        ref={heading}
        tabIndex={-1}
        className="m-0 text-base font-semibold focus:outline-none"
      >
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
