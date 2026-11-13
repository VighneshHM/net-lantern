import { useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { saveRules, useRules, type RuleSettings } from "../api/alerts";
import { ApiRequestError } from "../api/client";

const RULES: { key: keyof RuleSettings; label: string; hint: string }[] = [
  {
    key: "starred_offline",
    label: "Starred device offline",
    hint: "2 min, phones and tablets 10 min",
  },
  {
    key: "internet_down",
    label: "Internet down",
    hint: "router or internet unreachable for 2 checks; with it off, a router outage no longer holds back the alerts of starred devices",
  },
  { key: "latency_high", label: "Internet latency high", hint: "over 100 ms for 2 checks" },
  {
    key: "packet_loss",
    label: "Packet loss",
    hint: "3 of the last 30 checks to the router or a starred device; phones and tablets are skipped because they drop pings while asleep",
  },
  {
    key: "new_device",
    label: "New device joined",
    hint: "a device appears that was not on the network before",
  },
  {
    key: "notifications",
    label: "Send desktop notifications",
    hint: "show each alert as a desktop notification",
  },
];

/**
 * The six alert switches. Each change shows at once and is saved straight
 * away. Saves go one at a time, each sending the switches as they are when it
 * leaves, so a quick second toggle never undoes the first.
 */
export function RulesCard() {
  const { data, error, isPending } = useRules();
  const qc = useQueryClient();
  const [saveError, setSaveError] = useState<string | null>(null);
  /** The switches as the user last set them, while any save is unfinished. */
  const wanted = useRef<RuleSettings | null>(null);
  const queue = useRef<Promise<void>>(Promise.resolve());
  const unsaved = useRef(0);

  function toggle(key: keyof RuleSettings, value: boolean) {
    if (!data) return;
    setSaveError(null);
    const next = { ...(wanted.current ?? data), [key]: value };
    wanted.current = next;
    qc.setQueryData(["rules"], next);
    unsaved.current += 1;
    queue.current = queue.current.then(async () => {
      try {
        const saved = await saveRules(wanted.current ?? next);
        unsaved.current -= 1;
        if (unsaved.current === 0) {
          wanted.current = null;
          qc.setQueryData(["rules"], saved);
        }
      } catch (e: unknown) {
        unsaved.current -= 1;
        setSaveError(e instanceof Error ? e.message : "Could not save the rules.");
        if (unsaved.current === 0) {
          wanted.current = null;
          // Show what the server holds, not the change that failed.
          void qc.invalidateQueries({ queryKey: ["rules"] });
        }
      }
    });
  }

  let body;
  if (isPending) {
    body = <p className="m-0 text-text-muted">Loading…</p>;
  } else if (error || !data) {
    body =
      error instanceof ApiRequestError && error.error.code === "no_network" ? (
        <p className="m-0 text-text-muted">{error.error.hint ?? error.error.message}</p>
      ) : (
        <p className="m-0 text-text-muted">
          {error instanceof Error ? error.message : "Couldn't load the rules."}
        </p>
      );
  } else {
    body = (
      <ul className="m-0 flex list-none flex-col gap-1 p-0">
        {RULES.map((r) => (
          <li key={r.key}>
            <label className="flex min-h-11 cursor-pointer items-start gap-3 py-2">
              <input
                type="checkbox"
                checked={data[r.key]}
                onChange={(e) => toggle(r.key, e.target.checked)}
                className="mt-0.5 size-5 shrink-0 accent-accent"
              />
              <span className="flex min-w-0 flex-col">
                <span className="font-medium">{r.label}</span>
                <span className="text-sm text-text-muted">{r.hint}</span>
              </span>
            </label>
          </li>
        ))}
      </ul>
    );
  }

  return (
    <section
      aria-label="Alert rules"
      className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-5"
    >
      <h2 className="m-0 text-sm font-medium text-text-muted">Alert rules</h2>
      {body}
      {saveError !== null && (
        <p role="alert" className="m-0 text-sm font-medium text-bad">
          {saveError}
        </p>
      )}
    </section>
  );
}
