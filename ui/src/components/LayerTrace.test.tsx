import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { DiagnosisRun, LayerName, LayerResult, LayerStatus } from "../api/diagnose";
import { LayerTrace } from "./LayerTrace";

function step(layer: LayerName, status: LayerStatus = "pass", over: Partial<LayerResult> = {}): LayerResult {
  return { layer, title: `T ${layer}`, status, finding: "f", evidence: [], explanation: "", fix: null, ...over };
}
function run(over: Partial<DiagnosisRun> = {}): DiagnosisRun {
  return { id: 1, state: "running", started_at: 1, finished_at: null, steps: [], verdict: null, error: null, ...over };
}
const items = () => {
  const ol = screen.getByRole("list", { name: "Diagnosis steps" });
  return within(ol)
    .getAllByRole("listitem")
    .filter((li) => li.parentElement === ol);
};

describe("LayerTrace", () => {
  it("lists the five layers with default titles when there is no run", () => {
    render(<LayerTrace run={null} />);
    const titles = ["Wi-Fi link", "Router", "Internet", "DNS", "Website"];
    expect(items()).toHaveLength(5);
    items().forEach((li, i) => {
      expect(li).toHaveTextContent(titles[i]);
      expect(li).toHaveTextContent("Not checked");
    });
  });

  it("shows status words, explanation, evidence and fix for a step", () => {
    render(
      <LayerTrace
        run={run({
          state: "done",
          steps: [
            step("link", "warn", {
              explanation: "Weak signal.",
              evidence: ["Signal 20%", "Channel 6"],
              fix: "Move closer.",
            }),
            step("router", "fail"),
          ],
        })}
      />,
    );
    const [a, b, c] = items();
    expect(a).toHaveTextContent("T link");
    expect(a).toHaveTextContent("Warning");
    expect(a).toHaveTextContent("Weak signal.");
    expect(within(a).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "Signal 20%",
      "Channel 6",
    ]);
    expect(a).toHaveTextContent("Try this: Move closer.");
    expect(b).toHaveTextContent("Failed");
    expect(c).toHaveTextContent("Not checked");
  });

  it("shows Checking and Waiting while running", () => {
    render(<LayerTrace run={run({ steps: [step("link")] })} />);
    expect(items()[0]).toHaveTextContent("Passed");
    expect(items()[1]).toHaveTextContent("Checking…");
    expect(items()[4]).toHaveTextContent("Waiting");
  });
});
