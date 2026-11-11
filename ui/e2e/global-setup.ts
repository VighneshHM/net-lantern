import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import readline from "node:readline";

const PORT = 47850;
const STARTUP_TIMEOUT_MS = 30_000;
const READY_PREFIX = "Net Lantern running at ";

const uiDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// NETLANTERN_EXE (an absolute path) overrides the default debug build, for machines where
// that file is locked by a running netlantern.exe.
const overridePath = process.env.NETLANTERN_EXE;
const exePath =
  overridePath !== undefined && overridePath !== ""
    ? path.resolve(overridePath)
    : path.resolve(uiDir, "..", "target", "debug", "netlantern.exe");

/** Start `netlantern.exe --demo` and wait for its "running at" line. The URL carries a
 * one-time token, so it goes into process.env.NETLANTERN_URL and is never printed. */
export default async function globalSetup(): Promise<() => Promise<void>> {
  if (!existsSync(exePath)) {
    const hint =
      overridePath !== undefined && overridePath !== ""
        ? "NETLANTERN_EXE points to a file that does not exist."
        : "run `cargo build -p netlantern` first, or set NETLANTERN_EXE to the exe's absolute path.";
    throw new Error(`${exePath} not found - ${hint}`);
  }

  const child = spawn(exePath, ["--demo", "--no-browser", "--time-scale", "60", "--port", String(PORT)], {
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });

  let exited = false;
  const exitPromise = new Promise<void>((resolve) => {
    child.once("exit", () => {
      exited = true;
      resolve();
    });
  });

  let stderrTail = "";
  child.stderr!.on("data", (chunk: Buffer) => {
    stderrTail = (stderrTail + chunk.toString()).slice(-2000);
  });

  const stop = async () => {
    if (!exited && child.pid !== undefined) {
      spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
    }
    await exitPromise;
  };

  try {
    process.env.NETLANTERN_URL = await readUrl(child, () => stderrTail);
  } catch (err) {
    await stop();
    throw err;
  }
  return stop;
}

function readUrl(child: ChildProcess, stderr: () => string): Promise<string> {
  return new Promise((resolve, reject) => {
    const lines = readline.createInterface({ input: child.stdout! });
    const timer = setTimeout(() => {
      fail(new Error(`Net Lantern did not print its URL within ${STARTUP_TIMEOUT_MS / 1000} s.`));
    }, STARTUP_TIMEOUT_MS);

    const cleanup = () => {
      clearTimeout(timer);
      lines.close();
    };
    const fail = (err: Error) => {
      cleanup();
      reject(err);
    };

    lines.on("line", (line) => {
      if (line.includes("already running")) {
        fail(
          new Error(
            "another Net Lantern is running for this Windows user - close it and retry.",
          ),
        );
      } else if (line.startsWith(READY_PREFIX)) {
        cleanup();
        resolve(line.slice(READY_PREFIX.length).trim());
      }
    });
    child.once("exit", (code) => {
      fail(
        new Error(
          `netlantern.exe exited early (code ${code}) before printing its URL.\nstderr tail:\n${stderr()}`,
        ),
      );
    });
  });
}
