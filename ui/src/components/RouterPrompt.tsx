import { useEffect, useRef, type RefObject } from "react";

const button =
  "min-h-11 rounded-lg px-5 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-60";

/**
 * Asks whether a known network with a new router is the same network. A card
 * in the page, not a modal. Focus moves to its heading when it opens, and back
 * to `returnFocus` when it closes.
 */
export function RouterPrompt({
  label,
  busy,
  onAnswer,
  returnFocus,
}: {
  /** The known network's name; null when only the server's refusal told us to ask. */
  label: string | null;
  busy: boolean;
  onAnswer: (keep: boolean) => void;
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
      role="alertdialog"
      aria-labelledby="router-heading"
      className="flex flex-col gap-3 rounded-xl border border-border-strong bg-surface-2 p-5"
    >
      <h2
        id="router-heading"
        ref={heading}
        tabIndex={-1}
        className="m-0 text-base font-semibold focus:outline-none"
      >
        This looks like{" "}
        {label === null ? "a network you know" : <strong className="font-bold">{label}</strong>}{" "}
        with a new router. Keep your devices and names?
      </h2>
      <div className="flex flex-wrap gap-3">
        <button
          type="button"
          className={`${button} bg-accent text-accent-ink`}
          disabled={busy}
          onClick={() => onAnswer(true)}
        >
          Keep my devices
        </button>
        <button
          type="button"
          className={`${button} border border-border-strong bg-surface text-text`}
          disabled={busy}
          onClick={() => onAnswer(false)}
        >
          It's a different network
        </button>
      </div>
    </div>
  );
}
