import { useQuery } from "@tanstack/react-query";
import { apiFetch, unwrap } from "./client";

export interface WifiConnection {
  ssid: string;
  bssid: string;
  rssi_dbm: number;
  channel: number;
  band: "2.4ghz" | "5ghz" | "6ghz";
  width_mhz: number | null;
  phy: string;
  rx_rate_mbps: number | null;
  tx_rate_mbps: number | null;
  security: string;
}

export interface WifiStatus {
  interface_kind: "wifi" | "ethernet" | "other" | null;
  connection: WifiConnection | null;
}

export interface NearbyNetwork {
  ssid: string;
  bssid: string;
  rssi_dbm: number;
  channel: number;
  band: "2.4ghz" | "5ghz" | "6ghz";
  width_mhz: number | null;
}

export interface ChannelLoad {
  band: string;
  channel: number;
  networks: number;
}

export interface CurrentLoad {
  band: string;
  channel: number;
  overlapping: number;
  crowded: boolean;
  own_5ghz_visible: boolean;
  suggested_channel: number | null;
}

export interface Congestion {
  channels: ChannelLoad[];
  current: CurrentLoad | null;
  advice: string;
}

export interface NearbyScan {
  scanned_at: number;
  networks: NearbyNetwork[];
  congestion: Congestion;
}

export function useWifi() {
  return useQuery({
    queryKey: ["wifi"],
    queryFn: async () => unwrap(await apiFetch<WifiStatus>("/api/wifi")),
    retry: false,
  });
}

/**
 * A nearby-networks scan, run each time a screen using it opens (spec §6.7;
 * the server reuses a scan younger than 10 s); Refresh calls `refetch()`.
 */
export function useNearby() {
  return useQuery({
    queryKey: ["nearby"],
    queryFn: async () => unwrap(await apiFetch<NearbyScan>("/api/wifi/scan", { method: "POST" })),
    staleTime: Infinity,
    refetchOnMount: "always",
    retry: false,
  });
}
