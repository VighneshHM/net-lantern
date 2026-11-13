import type { Device } from "../api/queries";
import type { Alert } from "../api/alerts";
import type { MonitorStatus, MonitorTick } from "../api/monitor";

export function device(over: Partial<Device> = {}): Device {
  return {
    id: 1,
    ip: "192.168.1.10",
    mac: "aa:bb:cc:dd:ee:01",
    private_mac: false,
    vendor: null,
    hostname: null,
    name: null,
    display_name: "Device",
    type: "unknown",
    type_confidence: 0,
    starred: false,
    is_new: false,
    online: true,
    answers_ping: true,
    rtt_ms: 3,
    is_self: false,
    is_gateway: false,
    first_seen: 1_000,
    last_seen: 2_000,
    services: [],
    open_ports: [],
    ...over,
  };
}

export function tick(over: Partial<MonitorTick> = {}): MonitorTick {
  return {
    ts: 1_000,
    network_id: 1,
    gateway_rtt_ms: 3,
    internet_rtt_ms: null,
    router_lost: 0,
    router_window: 30,
    dns_ms: null,
    wifi_rssi_dbm: null,
    ...over,
  };
}

export function monitorStatus(over: Partial<MonitorStatus> = {}): MonitorStatus {
  return {
    state: "monitoring",
    network_id: 1,
    session_started_at: 1_000,
    last: tick(),
    checks: [],
    ...over,
  };
}

export function alert(over: Partial<Alert> = {}): Alert {
  return {
    id: 1,
    rule: "internet_down",
    key: "internet_down",
    severity: "critical",
    title: "Internet is down",
    detail: "",
    suppressed: false,
    state: "open",
    incident_id: 1,
    opened_at: 1_000,
    acked_at: null,
    resolved_at: null,
    ...over,
  };
}
