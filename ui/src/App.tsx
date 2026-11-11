import { useEffect, useState } from "react";
import { BrowserRouter, Route, Routes } from "react-router";
import { useLiveEvents } from "./api/events";
import { ensureSession } from "./api/session";
import { Shell } from "./components/Shell";
import { Dashboard } from "./pages/Dashboard";
import { Diagnose } from "./pages/Diagnose";
import { Devices } from "./pages/Devices";
import { NeedsLaunch } from "./pages/NeedsLaunch";
import { Placeholder } from "./pages/Placeholder";

/** The route table, without a router, so tests can mount it in a MemoryRouter. */
export function AppRoutes() {
  return (
    <Routes>
      <Route element={<Shell />}>
        <Route index element={<Dashboard />} />
        <Route path="devices" element={<Devices />} />
        <Route path="map" element={<Placeholder title="Map" phase={5} />} />
        <Route path="diagnose" element={<Diagnose />} />
        <Route path="alerts" element={<Placeholder title="Alerts" phase={3} />} />
        <Route path="settings" element={<Placeholder title="Settings" phase={6} />} />
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
