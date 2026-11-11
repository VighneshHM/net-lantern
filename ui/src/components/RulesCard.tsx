import { useState } from "react";
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
    hint: "router or internet unreachable for 2 checks",
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

/** The six alert switches. Each change is saved at once. */
export function RulesCard() {
  const { data, error, isPending } = useRules();
  const qc = useQueryClient();
  const [saveError, setSaveError] = useState<string | null>(null);

  function toggle(key: keyof RuleSettings, value: boolean) {
    if (!data) return;
    setSaveError(null);
    saveRules({ ...data, [key]: value }).then(
      (saved) => qc.setQueryData(["rules"], saved),
      (e: unknown) => setSaveError(e instanceof Error ? e.message : "Could not save the rules."),
    );
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
