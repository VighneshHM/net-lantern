import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useLive } from "../state/live";
import { ApiRequestError } from "../api/client";
import {
  BACKUP_LEFT_MESSAGE,
  fetchDebugInfo,
  resetAllData,
  useHost,
  useSetAutostart,
  useSetUpdateCheck,
  useUpdate,
} from "../api/ops";

const card = "flex flex-col gap-3 rounded-xl border border-border bg-surface p-5";
const heading = "m-0 text-lg font-semibold";
const note = "m-0 text-sm text-text-muted";
const alertText = "m-0 text-sm font-medium text-bad";
const button =
  "min-h-11 rounded-lg border border-border-strong bg-surface px-5 text-sm font-semibold text-text disabled:cursor-not-allowed disabled:opacity-60";

function Check({
  checked,
  disabled,
  onChange,
  label,
}: {
  checked: boolean;
  disabled: boolean;
  onChange: (on: boolean) => void;
  label: string;
}) {
  return (
    <label className="flex min-h-11 cursor-pointer items-center gap-3 py-2">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        className="size-11 shrink-0 accent-accent"
      />
      <span className="font-medium">{label}</span>
    </label>
  );
}

/** Shown only in the desktop app, which is the one host with a start at login switch. */
export function StartWithWindowsCard() {
  const { data } = useHost();
  const setAutostart = useSetAutostart();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);

  if (!data?.autostart.available) return null;

  async function change(on: boolean) {
    setBusy(true);
    setFailed(null);
    try {
      await setAutostart(on);
    } catch (e) {
      if (e instanceof ApiRequestError && e.error.code === "phone_not_allowed") {
        setFailed("This can only be changed on the laptop.");
      } else {
        setFailed(e instanceof Error ? e.message : "Couldn't change this.");
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className={card} aria-label="Start with Windows">
      <h2 className={heading}>Start with Windows</h2>
      <Check
        label="Start Net Lantern when you sign in to Windows"
        checked={data.autostart.enabled === true}
        disabled={busy}
        onChange={(on) => void change(on)}
      />
      <p className={note}>It starts in the tray, without opening a window.</p>
      {failed && (
        <p role="alert" className={alertText}>
          {failed}
        </p>
      )}
    </section>
  );
}

export function UpdatesCard() {
  const { data, error } = useUpdate();
  const setCheck = useSetUpdateCheck();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);

  async function change(on: boolean) {
    setBusy(true);
    setFailed(null);
    try {
      await setCheck(on);
    } catch (e) {
      setFailed(e instanceof Error ? e.message : "Couldn't change this.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className={card} aria-label="Updates">
      <h2 className={heading}>Updates</h2>
      {data && (
        <>
          <Check
            label="Check GitHub for new versions"
            checked={data.setting === true}
            disabled={busy}
            onChange={(on) => void change(on)}
          />
          <p className={note}>
            Net Lantern asks GitHub for the latest version number once a day. Nothing about your
            network is sent.
          </p>
          <p className="m-0 text-sm">
            <span>You have version {data.current}.</span>
            {data.newer && data.latest && (
              <>
                {" "}
                <span>Version {data.latest} is available.</span>
                {data.url?.startsWith("https://github.com/") && (
                  <>
                    {" "}
                    <a
                      href={data.url}
                      target="_blank"
                      rel="noreferrer"
                      data-inline-link
                      className="text-accent underline"
                    >
                      See what's new
                    </a>
                  </>
                )}
              </>
            )}
          </p>
        </>
      )}
      {error && !data && (
        <p role="alert" className={alertText}>
          {error.message}
        </p>
      )}
      {failed && (
        <p role="alert" className={alertText}>
          {failed}
        </p>
      )}
    </section>
  );
}

export function HelpCard() {
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const [text, setText] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);

  async function copy() {
    setBusy(true);
    setCopied(false);
    setText(null);
    setFailed(null);
    let info: string;
    try {
      info = await fetchDebugInfo();
    } catch (e) {
      setFailed(e instanceof Error ? e.message : "Couldn't gather the debug info.");
      setBusy(false);
      return;
    }
    try {
      await navigator.clipboard.writeText(info);
      setCopied(true);
    } catch {
      setText(info);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className={card} aria-label="Help with a problem">
      <h2 className={heading}>Help with a problem</h2>
      <div>
        <button type="button" className={button} disabled={busy} onClick={() => void copy()}>
          Copy debug info
        </button>
      </div>
      <p className={note}>
        Copies your Net Lantern version, Windows version, network state and recent log lines, with
        addresses and names removed. Nothing is sent anywhere.
      </p>
      {copied && <p role="status" className="m-0 text-sm font-medium">Copied — paste it into your bug report.</p>}
      {text !== null && (
        <>
          <textarea
            readOnly
            aria-label="Debug info"
            value={text}
            rows={8}
            className="w-full rounded-lg border border-border bg-surface-2 p-3 font-mono text-xs text-text"
          />
          <p className={note}>Select the text and copy it.</p>
        </>
      )}
      {failed && (
        <p role="alert" className={alertText}>
          {failed}
        </p>
      )}
    </section>
  );
}

const DROPPED = "All data deleted. If this page stops responding, reload it on the laptop.";

function ConfirmDelete({
  onConfirm,
  onCancel,
  busy,
  failed,
}: {
  onConfirm: () => void;
  onCancel: () => void;
  busy: boolean;
  failed: string | null;
}) {
  const title = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    title.current?.focus();
  }, []);

  return (
    <div
      role="alertdialog"
      aria-labelledby="delete-heading"
      className="flex flex-col gap-3 rounded-lg border border-border-strong bg-surface-2 p-4"
    >
      <h3
        id="delete-heading"
        ref={title}
        tabIndex={-1}
        className="m-0 text-base font-semibold focus:outline-none"
      >
        Delete all Net Lantern data? This can't be undone.
      </h3>
      {failed && (
        <p role="alert" className={alertText}>
          {failed}
        </p>
      )}
      <div className="flex flex-wrap gap-3">
        <button
          type="button"
          className="min-h-11 rounded-lg bg-bad px-5 text-sm font-semibold text-accent-ink disabled:cursor-not-allowed disabled:opacity-60"
          disabled={busy}
          onClick={onConfirm}
        >
          Delete all data
        </button>
        <button type="button" className={button} disabled={busy} onClick={onCancel}>
          Cancel
        </button>
      </div>
    </div>
  );
}

type Outcome = { kind: "done"; text: string } | { kind: "warning"; text: string };

export function DeleteDataCard() {
  const queryClient = useQueryClient();
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const opener = useRef<HTMLButtonElement>(null);
  const wasAsking = useRef(false);

  // The opener is unmounted while the dialog is open, so focus goes back to
  // the freshly mounted button when the dialog closes (cancel or finished).
  useEffect(() => {
    if (wasAsking.current && !asking) opener.current?.focus();
    wasAsking.current = asking;
  }, [asking]);

  function cleared(result: Outcome) {
    setOutcome(result);
    setAsking(false);
    // The data.reset event does this too; doing it here covers a dropped socket.
    useLive.getState().reset();
    void queryClient.invalidateQueries();
  }

  async function confirm() {
    setBusy(true);
    setFailed(null);
    try {
      await resetAllData();
      cleared({ kind: "done", text: "All data deleted." });
    } catch (e) {
      if (e instanceof ApiRequestError && e.status === 0) {
        // A phone is cut off when the reset turns phone access off: probably done.
        cleared({ kind: "done", text: DROPPED });
      } else if (e instanceof ApiRequestError && e.error.message === BACKUP_LEFT_MESSAGE) {
        cleared({
          kind: "warning",
          text: BACKUP_LEFT_MESSAGE,
        });
      } else {
        setFailed(e instanceof Error ? e.message : "Couldn't delete the data.");
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className={card} aria-label="Delete all data">
      <h2 className={heading}>Delete all data</h2>
      <p className={note}>
        Removes every network, device, name, history and setting Net Lantern has stored on this
        computer. Log files are kept.
      </p>
      {asking ? (
        <ConfirmDelete
          busy={busy}
          failed={failed}
          onConfirm={() => void confirm()}
          onCancel={() => {
            setAsking(false);
            setFailed(null);
          }}
        />
      ) : (
        <div>
          <button
            ref={opener}
            type="button"
            className={button}
            onClick={() => {
              setOutcome(null);
              setAsking(true);
            }}
          >
            Delete all data…
          </button>
        </div>
      )}
      {outcome && (
        <p
          role="status"
          className={`m-0 text-sm font-medium ${outcome.kind === "warning" ? "text-warn" : ""}`}
        >
          {outcome.text}
        </p>
      )}
    </section>
  );
}
