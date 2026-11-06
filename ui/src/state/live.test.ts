import { beforeEach, describe, expect, it } from "vitest";
import { resetLive } from "../test/render";
import { useLive, type LiveScan } from "./live";

function snapshot(id: number, state: string): LiveScan {
  return { id, state, stage: "merge", done: 254, total: 254, devicesFound: 13 };
}

beforeEach(() => {
  resetLive();
});

describe("setScan", () => {
  it("loads a snapshot when no scan is followed yet", () => {
    useLive.getState().setScan(snapshot(4, "done"));
    expect(useLive.getState().scan).toEqual(snapshot(4, "done"));
  });

  it("updates the followed scan from its own snapshot", () => {
    useLive.getState().attach(2);
    useLive.getState().setScan({ ...snapshot(2, "running"), stage: "arp", devicesFound: 0 });
    expect(useLive.getState().scan).toMatchObject({ id: 2, state: "running", stage: "arp" });
  });

  it("ignores a late snapshot of another scan while a scan is followed", () => {
    useLive.getState().attach(2);
    // A resync reload of the previous scan lands after scan 2 started.
    useLive.getState().setScan(snapshot(1, "done"));
    expect(useLive.getState().scan).toMatchObject({ id: 2, state: "running" });
  });

  it("does not undo a finish with an older running snapshot of the same scan", () => {
    useLive.getState().attach(3);
    useLive.getState().applyEvent({
      seq: 9,
      api_version: 1,
      type: "scan.done",
      data: { scan_id: 3, state: "done", devices_found: 13 },
    });
    useLive.getState().setScan(snapshot(3, "running"));
    expect(useLive.getState().scan).toMatchObject({ id: 3, state: "done" });
  });
});
