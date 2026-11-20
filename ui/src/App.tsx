import { lazy, Suspense, useEffect, useState } from "react";
import { BrowserRouter, Route, Routes } from "react-router";
import { useLiveEvents } from "./api/events";
import { ensureSession } from "./api/session";
import { Shell } from "./components/Shell";
import { Alerts } from "./pages/Alerts";
import { Dashboard } from "./pages/Dashboard";
import { Diagnose } from "./pages/Diagnose";
import { Devices } from "./pages/Devices";
import { NeedsLaunch } from "./pages/NeedsLaunch";

const NetworkMap = lazy(() => import("./pages/NetworkMap"));
const Settings = lazy(() => import("./pages/Settings"));

/** The route table, without a router, so tests can mount it in a MemoryRouter. */
export function AppRoutes() {
  return (
    <Routes>
      <Route element={<Shell />}>
        <Route index element={<Dashboard />} />
        <Route path="devices" element={<Devices />} />
        <Route
          path="map"
          element={
            <Suspense fallback={<p className="m-0 text-text-muted">Loading the map…</p>}>
              <NetworkMap />
            </Suspense>
          }
        />
        <Route path="diagnose" element={<Diagnose />} />
        <Route path="alerts" element={<Alerts />} />
        <Route
          path="settings"
          element={
            <Suspense fallback={<p className="m-0 text-text-muted">Loading settings…</p>}>
              <Settings />
            </Suspense>
          }
        />
      </Route>
    </Routes>
  );
}

/** The part of the app that needs a valid session: it owns the one live-event socket. */
function Authenticated() {
  useLiveEvents();
  return (
    <BrowserRouter>
      <AppRoutes />
    </BrowserRouter>
  );
}

export default function App() {
  const [state, setState] = useState<"checking" | "ok" | "needs-launch">("checking");

  useEffect(() => {
    let cancelled = false;
    void ensureSession()
      // A failed check is not a session problem: show the dashboard, which
      // reports its own errors, rather than "Connecting…" forever.
      .catch(() => "ok" as const)
      .then((s) => {
        if (!cancelled) setState(s);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (state === "checking") {
    return <p className="m-0 p-6 text-text-muted">Connecting…</p>;
  }
  if (state === "needs-launch") return <NeedsLaunch />;
  return <Authenticated />;
}
