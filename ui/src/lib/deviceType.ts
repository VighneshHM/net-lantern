export type DeviceType =
  | "router"
  | "laptop"
  | "phone"
  | "tablet"
  | "printer"
  | "tv"
  | "speaker"
  | "camera"
  | "nas"
  | "console"
  | "iot"
  | "unknown";

export const DEVICE_TYPE_LABEL: Record<DeviceType, string> = {
  router: "Router",
  laptop: "Computer",
  phone: "Phone",
  tablet: "Tablet",
  printer: "Printer",
  tv: "TV",
  speaker: "Speaker",
  camera: "Camera",
  nas: "NAS",
  console: "Game console",
  iot: "Smart device",
  unknown: "Unknown",
};
