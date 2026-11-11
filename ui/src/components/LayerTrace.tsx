import { LAYERS, type DiagnosisRun, type LayerName, type LayerStatus } from "../api/diagnose";

const DEFAULT_TITLE: Record<LayerName, string> = {
  link: "Wi-Fi link",
  router: "Router",
  internet: "Internet",
  dns: "DNS",
  website: "Website",
};

const STATUS: Record<LayerStatus, { text: string; color: string }> = {
  pass: { text: "Passed", color: "text-ok" },
  warn: { text: "Warning", color: "text-warn" },
  fail: { text: "Failed", color: "text-bad" },
  skipped: { text: "Not checked", color: "text-text-muted" },
};

const NOT_CHECKED = STATUS.skipped;

/** One row per layer, in run order; layers without a result yet read as pending. */
export function LayerTrace({ run }: { run: DiagnosisRun | null }) {
  const running = run?.state === "running";
  const firstMissing = LAYERS.findIndex((l) => !run?.steps.some((s) => s.layer === l));
  return (
    <ol aria-label="Diagnosis steps" className="m-0 flex list-none flex-col gap-2 p-0">
      {LAYERS.map((layer, i) => {
        const step = run?.steps.find((s) => s.layer === layer);
        let status = NOT_CHECKED;
        if (step) status = STATUS[step.status];
        else if (running) {
          status = { text: i === firstMissing ? "Checking…" : "Waiting", color: "text-text-muted" };
        }
        return (
          <li
            key={layer}
            className="flex flex-col gap-1 rounded-xl border border-border bg-surface p-4"
          >
            <div className="flex flex-wrap items-baseline justify-between gap-2">
              <span className="font-semibold">{step?.title ?? DEFAULT_TITLE[layer]}</span>
              <span className={`text-sm font-semibold ${status.color}`}>{status.text}</span>
            </div>
            {step?.explanation && <p className="m-0 text-sm [overflow-wrap:anywhere]">{step.explanation}</p>}
            {step && step.evidence.length > 0 && (
              <ul className="m-0 list-disc pl-5 text-sm text-text-muted">
                {step.evidence.map((e, j) => (
                  <li key={j} className="[overflow-wrap:anywhere]">
                    {e}
                  </li>
                ))}
              </ul>
            )}
            {step?.fix && <p className="m-0 text-sm text-accent [overflow-wrap:anywhere]">Try this: {step.fix}</p>}
          </li>
        );
      })}
    </ol>
  );
}
