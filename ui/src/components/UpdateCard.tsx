import { useState } from "react";
import { useSetUpdateCheck, useUpdate } from "../api/ops";
import { isReleaseUrl } from "../lib/github";

const DISMISSED_KEY = "nl_dismissed_update";

function readDismissed(): string | null {
  try {
    return localStorage.getItem(DISMISSED_KEY);
  } catch {
    return null;
  }
}

/** A slim banner when a newer version is out; dismissible for that version in this browser. */
export function UpdateBanner() {
  const { data } = useUpdate();
  const [dismissed, setDismissed] = useState<string | null>(readDismissed);

  if (!data || !data.newer || !data.latest || dismissed === data.latest) return null;
  const latest = data.latest;

  function dismiss() {
    setDismissed(latest);
    // The banner is about to unmount; keep keyboard focus on the page.
    document.querySelector("h1")?.focus();
    try {
      localStorage.setItem(DISMISSED_KEY, latest);
    } catch {
      // Without storage the banner hides for this visit only.
    }
  }

  return (
    <div
      role="status"
      className="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-lg border border-border-strong bg-surface px-4 py-1 text-sm text-text"
    >
      <span className="font-medium">Version {latest} is available.</span>
      {isReleaseUrl(data.url) && (
        <a
          href={data.url}
          target="_blank"
          rel="noreferrer"
          className="inline-flex min-h-11 items-center text-accent underline"
        >
          Download from GitHub
        </a>
      )}
      <button
        type="button"
        className="ml-auto inline-flex min-h-11 min-w-11 items-center justify-center rounded-lg border border-border-strong bg-surface-2 px-3 text-sm font-semibold text-text"
        onClick={dismiss}
      >
        Dismiss
      </button>
    </div>
  );
}

const answerButton =
  "min-h-11 rounded-lg border border-border-strong bg-surface-2 px-5 text-sm font-semibold text-text disabled:cursor-not-allowed disabled:opacity-60";

/** Asks once whether to check for new versions; neither answer is the default. */
export function UpdateQuestion() {
  const { data } = useUpdate();
  const setCheck = useSetUpdateCheck();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);

  if (!data || data.setting !== null) return null;

  async function answer(on: boolean) {
    setBusy(true);
    setFailed(null);
    try {
      await setCheck(on);
    } catch (e) {
      setFailed(e instanceof Error ? e.message : "Could not save your answer.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      aria-label="Check for new versions"
      className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-5"
    >
      <h2 className="m-0 text-lg font-semibold">Check for new versions?</h2>
      <p className="m-0 text-text-muted">
        Net Lantern can ask GitHub once a day whether a newer version is out. It sends nothing
        about your network.
      </p>
      <p className="m-0 text-sm text-text-muted">You can change this later in Settings.</p>
      <div className="flex flex-wrap gap-3">
        <button type="button" className={answerButton} disabled={busy} onClick={() => void answer(true)}>
          Yes, check for updates
        </button>
        <button type="button" className={answerButton} disabled={busy} onClick={() => void answer(false)}>
          No thanks
        </button>
      </div>
      {failed !== null && (
        <p role="alert" className="m-0 text-sm font-medium text-bad">
          {failed}
        </p>
      )}
    </section>
  );
}
