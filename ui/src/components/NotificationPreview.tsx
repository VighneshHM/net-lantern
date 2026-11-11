import { useAlerts } from "../api/alerts";

const SAMPLE = { title: "Office printer is offline", detail: "Not seen for 2 min." };

/** A toast-like sample of the desktop notification. It only explains; it changes no setting. */
export function NotificationPreview() {
  const { data } = useAlerts("active");
  const newest =
    data && data.length > 0 ? data.reduce((a, b) => (b.opened_at > a.opened_at ? b : a)) : null;
  const shown = newest ?? SAMPLE;
  return (
    <section
      aria-label="Notification preview"
      className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-5"
    >
      <h2 className="m-0 text-sm font-medium text-text-muted">
        What a desktop notification looks like
      </h2>
      <div className="flex flex-col gap-0.5 rounded-lg border border-border-strong bg-surface-2 p-3">
        <span className="text-xs font-medium text-text-muted">Net Lantern</span>
        <span className="font-semibold break-words">{shown.title}</span>
        <span className="text-sm break-words">{shown.detail}</span>
      </div>
      <p className="m-0 text-sm text-text-muted">
        If notifications don't appear, check Windows Settings → System → Notifications.
      </p>
    </section>
  );
}
