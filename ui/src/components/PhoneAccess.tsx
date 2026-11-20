import { useEffect, useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ApiRequestError } from "../api/client";
import { usePhoneAccess, useSetPhoneAccess, type PhoneStatus } from "../api/settings";

const button =
  "min-h-11 rounded-lg px-5 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-60";
const primary = `${button} bg-accent text-accent-ink`;
const secondary = `${button} border border-border-strong bg-surface text-text`;

/** The firewall prompt is polled every 2 s for at most 60 s (spec §6.12). */
const POLL_MS = 2000;
const POLL_FOR_MS = 60_000;

function PublicNetworkText() {
  return (
    <p className="m-0 text-sm">
      Phone access is paused because Windows marks this network as Public. On a network you trust,
      open <strong>Settings → Network &amp; internet → Wi-Fi → your network</strong> and set it to{" "}
      <strong>Private</strong>.
    </p>
  );
}

/** The phone link as a QR code. The library loads on first use, so it stays out of the main bundle. */
function QrCode({ url }: { url: string }) {
  const [made, setMade] = useState<{ url: string; src: string } | null>(null);
  const src = made?.url === url ? made.src : null;
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const failed = failedUrl === url;
  useEffect(() => {
    let alive = true;
    void import("qrcode")
      .then((qr) => qr.toString(url, { type: "svg", margin: 1 }))
      .then((svg) => {
        if (alive) {
          setMade({ url, src: `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}` });
        }
      })
      .catch(() => {
        // The link is also shown as text, so a failed QR code is not fatal.
        if (alive) setFailedUrl(url);
      });
    return () => {
      alive = false;
    };
  }, [url]);

  return (
    <div className="flex size-48 items-center justify-center self-start rounded-lg bg-[var(--qr-bg)] p-2">
      {src ? (
        <img src={src} alt="QR code for the phone link" className="size-full" />
      ) : failed ? (
        <span className="text-sm text-text-muted">
          The QR code couldn't be drawn — use the link below.
        </span>
      ) : (
        <span className="text-sm text-text-muted">Making the QR code…</span>
      )}
    </div>
  );
}

function FirewallLine({
  verdict,
  timedOut,
}: {
  verdict: PhoneStatus["firewall"];
  timedOut: boolean;
}) {
  switch (verdict) {
    case "allowed":
    case "off":
      return (
        <p data-testid="firewall-line" className="m-0 text-sm text-text-muted">
          Windows Firewall allows phones to connect.
        </p>
      );
    case "not_decided":
      if (timedOut) {
        return (
          <p data-testid="firewall-line" role="alert" className="m-0 text-sm font-medium text-bad">
            Windows Firewall isn't letting phones in yet. If you didn't see a prompt, open{" "}
            <strong>
              Windows Security → Firewall &amp; network protection → Allow an app through firewall
            </strong>
            , find <strong>netlantern.exe</strong> and tick <strong>Private</strong>. Then turn
            phone access off and on.
          </p>
        );
      }
      return (
        <p data-testid="firewall-line" className="m-0 text-sm text-text-muted">
          Waiting for your answer to the Windows Firewall prompt.
        </p>
      );
    case "blocked":
      return (
        <p data-testid="firewall-line" role="alert" className="m-0 text-sm font-medium text-bad">
          Windows Firewall is blocking phones. Open{" "}
          <strong>Windows Security → Firewall &amp; network protection → Allow an app through firewall</strong>
          , find <strong>netlantern.exe</strong>, and tick <strong>Private</strong>. Then turn phone
          access off and on. If it is still blocked, delete the Block rules for{" "}
          <strong>netlantern.exe</strong> in{" "}
          <strong>Windows Defender Firewall → Advanced settings → Inbound Rules</strong>.
        </p>
      );
    default:
      return null;
  }
}

function Body({ children }: { children: ReactNode }) {
  return <div className="flex flex-col gap-3">{children}</div>;
}

/** The phone access card's contents: Off, On and Paused. */
export function PhoneAccess() {
  const phone = usePhoneAccess();
  const setPhone = useSetPhoneAccess();
  const queryClient = useQueryClient();
  const [explaining, setExplaining] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [timedOut, setTimedOut] = useState(false);
  const allowButton = useRef<HTMLButtonElement>(null);

  const firewall = phone.data?.state === "on" ? phone.data.firewall : null;
  const { refetch } = phone;
  useEffect(() => {
    if (firewall !== "not_decided") return;
    const poll = setInterval(() => void refetch(), POLL_MS);
    const stop = setTimeout(() => {
      clearInterval(poll);
      setTimedOut(true);
    }, POLL_FOR_MS);
    return () => {
      clearInterval(poll);
      clearTimeout(stop);
      // A new verdict, or phone access turned off and on, starts a fresh wait.
      setTimedOut(false);
    };
  }, [firewall, refetch]);

  async function change(enabled: boolean) {
    setBusy(true);
    setError(null);
    try {
      await setPhone(enabled);
      setExplaining(false);
    } catch (e) {
      if (enabled && e instanceof ApiRequestError && e.error.code === "public_network") {
        // The setting is saved; it starts once the network is Private.
        queryClient.setQueryData<PhoneStatus>(["phone"], (old) => ({
          state: "paused",
          reason: "public_network",
          url: null,
          firewall: null,
          category: "public",
          demo: old?.demo ?? false,
        }));
        setExplaining(false);
      } else {
        setError(e instanceof Error ? e.message : "Phone access couldn't be changed.");
      }
    } finally {
      setBusy(false);
    }
  }

  async function copy(url: string) {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
    } catch {
      setError("Couldn't copy the link. Select it and copy it by hand.");
    }
  }

  if (phone.isPending) return <p className="m-0 text-text-muted">Checking phone access…</p>;
  if (phone.error && !phone.data) {
    return (
      <p role="alert" className="m-0 text-sm font-medium text-bad">
        {phone.error instanceof Error ? phone.error.message : "Couldn't read phone access."}
      </p>
    );
  }

  const status = phone.data;
  const staleLine = phone.error && (
    <p role="alert" className="m-0 text-sm font-medium text-bad">
      Couldn't refresh phone access status.
    </p>
  );
  const errorLine = error && (
    <p role="alert" className="m-0 text-sm font-medium text-bad">
      {error}
    </p>
  );
  const turnOff = (
    <div>
      <button type="button" className={secondary} disabled={busy} onClick={() => void change(false)}>
        Turn off phone access
      </button>
    </div>
  );

  if (status.state === "on" && status.url) {
    return (
      <Body>
        <QrCode url={status.url} />
        <p className="m-0 text-sm text-text-muted">Scan this with your phone's camera.</p>
        <div className="flex flex-wrap items-center gap-3">
          <code className="break-all text-sm">{status.url}</code>
          <button type="button" className={secondary} onClick={() => void copy(status.url ?? "")}>
            Copy link
          </button>
          {copied && (
            <span role="status" className="text-sm text-text-muted">
              Copied
            </span>
          )}
        </div>
        {status.demo && (
          <p className="m-0 text-sm text-text-muted">
            Demo mode: this link works on this laptop only.
          </p>
        )}
        <FirewallLine verdict={status.firewall} timedOut={timedOut} />
        {staleLine}
        {errorLine}
        {turnOff}
      </Body>
    );
  }

  if (status.state === "paused") {
    return (
      <Body>
        {status.reason === "not_connected" ? (
          <p className="m-0 text-sm">
            Phone access is paused until this laptop is connected to a network.
          </p>
        ) : status.reason === "public_network" ? (
          <PublicNetworkText />
        ) : status.reason === "bind_failed" ? (
          <p className="m-0 text-sm">
            Phone access is paused. Net Lantern couldn't listen on this computer's network address;
            it will keep trying.
          </p>
        ) : (
          <p className="m-0 text-sm">Phone access is paused.</p>
        )}
        {staleLine}
        {errorLine}
        {turnOff}
      </Body>
    );
  }

  return (
    <Body>
      {staleLine}
      <p className="m-0 text-sm text-text-muted">
        Open this dashboard on your phone over Wi-Fi. Your phone and this laptop must be on the same
        network.
      </p>
      {explaining ? (
        <div
          role="group"
          aria-label="Before you turn on phone access"
          className="flex flex-col gap-3 rounded-lg border border-border-strong bg-surface-2 p-4"
        >
          <p className="m-0 text-sm">
            Windows may ask whether Net Lantern can use the network. Choose <strong>Allow</strong>{" "}
            for <strong>Private networks</strong>. This needs an administrator's approval once.
            Anyone with the QR code can open the dashboard until you turn phone access off.
          </p>
          {errorLine}
          <div className="flex flex-wrap gap-3">
            <button
              type="button"
              className={primary}
              disabled={busy}
              onClick={() => void change(true)}
            >
              Continue
            </button>
            <button
              type="button"
              className={secondary}
              disabled={busy}
              onClick={() => {
                setExplaining(false);
                setError(null);
                requestAnimationFrame(() => allowButton.current?.focus());
              }}
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <div>
          <button
            ref={allowButton}
            type="button"
            className={primary}
            onClick={() => setExplaining(true)}
          >
            Allow phone access
          </button>
        </div>
      )}
    </Body>
  );
}
