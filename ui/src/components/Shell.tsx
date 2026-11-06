import type { ReactNode } from "react";
import { NavLink, Outlet } from "react-router";

const iconProps = {
  width: 18,
  height: 18,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 2,
  strokeLinecap: "round",
  strokeLinejoin: "round",
  "aria-hidden": true,
} as const;

const NAV: { to: string; label: string; icon: ReactNode }[] = [
  {
    to: "/",
    label: "Dashboard",
    icon: (
      <svg {...iconProps}>
        <rect x="3" y="3" width="7" height="7" rx="1" />
        <rect x="14" y="3" width="7" height="7" rx="1" />
        <rect x="3" y="14" width="7" height="7" rx="1" />
        <rect x="14" y="14" width="7" height="7" rx="1" />
      </svg>
    ),
  },
  {
    to: "/devices",
    label: "Devices",
    icon: (
      <svg {...iconProps}>
        <rect x="2" y="4" width="14" height="10" rx="1.5" />
        <path d="M6 18h6" />
        <rect x="18" y="8" width="4" height="10" rx="1" />
      </svg>
    ),
  },
  {
    to: "/map",
    label: "Map",
    icon: (
      <svg {...iconProps}>
        <circle cx="12" cy="12" r="2.5" />
        <circle cx="5" cy="5" r="2" />
        <circle cx="19" cy="5" r="2" />
        <circle cx="5" cy="19" r="2" />
        <circle cx="19" cy="19" r="2" />
        <path d="M6.5 6.5l3.7 3.7M17.5 6.5l-3.7 3.7M6.5 17.5l3.7-3.7M17.5 17.5l-3.7-3.7" />
      </svg>
    ),
  },
  {
    to: "/diagnose",
    label: "Diagnose",
    icon: (
      <svg {...iconProps}>
        <path d="M3 12h4l3-8 4 16 3-8h4" />
      </svg>
    ),
  },
  {
    to: "/alerts",
    label: "Alerts",
    icon: (
      <svg {...iconProps}>
        <path d="M6 16V11a6 6 0 0 1 12 0v5l2 2H4z" />
        <path d="M10 20a2 2 0 0 0 4 0" />
      </svg>
    ),
  },
  {
    to: "/settings",
    label: "Settings",
    icon: (
      <svg {...iconProps}>
        <circle cx="12" cy="12" r="3" />
        <path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1" />
      </svg>
    ),
  },
];

const linkBase =
  "flex min-h-11 items-center gap-2.5 rounded-lg px-3 text-sm font-medium no-underline hover:bg-surface-2 hover:text-text";

/**
 * App frame: sidebar on the left on wide screens; below 760 px it stacks above
 * the content and the nav becomes a wrapping row. The brand is not a link.
 */
export function Shell() {
  return (
    <div className="flex min-h-screen flex-col min-[760px]:flex-row">
      <aside className="flex flex-col gap-5 border-b border-border bg-surface px-4 py-5 min-[760px]:w-60 min-[760px]:shrink-0 min-[760px]:border-r min-[760px]:border-b-0">
        <div className="flex min-h-11 items-center gap-2.5 text-lg font-bold text-text">
          <svg {...iconProps} width={24} height={24} className="text-accent">
            <circle cx="12" cy="12" r="2" />
            <path d="M7.8 7.8a6 6 0 0 0 0 8.4M16.2 7.8a6 6 0 0 1 0 8.4M4.9 4.9a10 10 0 0 0 0 14.2M19.1 4.9a10 10 0 0 1 0 14.2" />
          </svg>
          Net Lantern
        </div>
        <nav aria-label="App" className="flex flex-row flex-wrap gap-1 min-[760px]:flex-col">
          {NAV.map(({ to, label, icon }) => (
            <NavLink
              key={to}
              to={to}
              end={to === "/"}
              className={({ isActive }) =>
                `${linkBase} ${
                  isActive
                    ? "bg-surface-2 text-text ring-1 ring-border-strong ring-inset"
                    : "text-text-muted"
                }`
              }
            >
              {icon}
              {label}
            </NavLink>
          ))}
        </nav>
      </aside>
      <main className="flex min-w-0 flex-1 flex-col gap-5 px-4 pt-5 pb-10 min-[760px]:px-8 min-[760px]:pt-7 min-[760px]:pb-12">
        <Outlet />
      </main>
    </div>
  );
}
