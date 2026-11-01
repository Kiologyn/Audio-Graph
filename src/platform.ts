import { invoke } from "@tauri-apps/api/core";

/** The OS the app runs on (std::env::consts::OS in Rust) */
export type Platform = "windows" | "linux" | "macos";

let current: Platform = "windows";
let installed = false;

/** Learn the OS before the first render: it decides how cables work in the interface */
export async function loadPlatform() {
  try {
    [current, installed] = await Promise.all([invoke<Platform>("platform"), invoke<boolean>("autostart_supported")]);
  } catch {
    // outside Tauri (checks in a browser): as on Windows
  }
}

export const platform = () => current;

/** The app is installed, not a portable file or an AppImage: only then it can start with the system */
export const autostartSupported = () => installed;

/**
 * Cables are virtual devices the graph creates itself (Linux): a cable node has no choice
 * from a list, and there is no cable list or cable power. On Windows cables are device pairs
 * of a virtual cable driver
 */
export const dynamicCables = () => current === "linux";

/** Device names of a node's virtual cable: input and output. Matches virtual_device_names in plan.rs */
export function virtualDeviceIds(nodeId: string): [string, string] {
  const id = nodeId.replace(/[^A-Za-z0-9-]/g, "");
  return [`audiograph.${id}`, `audiograph.${id}.out`];
}

/** The name the system shows for a virtual cable's devices. Matches description in engine/linux.rs */
export function virtualDeviceName(label: string): string {
  const clean = label.replace(/["'\\]/g, "").replace(/\p{Cc}/gu, "").trim();
  return clean ? `${clean} (Audio Graph)` : "Audio Graph";
}
