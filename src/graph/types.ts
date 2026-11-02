import type { Edge, Node } from "@xyflow/react";

/**
 * Graph node kinds.
 * - input  — an input device (microphone, a cable's recording side): output only
 * - output — an output device (headphones, speakers): input only
 * - app    — an app: output = its sound, input = its microphone
 * - cable  — a cable: an internal mixer or a cable from the shared list (a pair of system devices)
 * - system — the system's default devices: output = sound of the apps playing into the default
 *            output device, input = what they hear as the default microphone
 */
export type AudioNodeKind = "input" | "output" | "app" | "cable" | "system";

/** A port's binding to a system device. Matches DeviceRef in graphs.rs. */
export type DeviceRef =
  | { type: "default" }
  // the name is remembered to show it while the device is unplugged
  | { type: "device"; id: string; name: string };

/** The app of an app node. Matches AppRef in graphs.rs. */
export type AppRef = {
  /** executable file name: the app is recognized by it even if its folder changed */
  exe: string;
  /** the full path the app was chosen under */
  path: string;
  /** the name, shown while the app isn't running */
  name: string;
};

/** sink is the output device behind the input port, source the recording device behind the output port */
export type DeviceSlotName = "sink" | "source";

export type AudioNodeData = {
  kind: AudioNodeKind;
  label: string;
  sink: DeviceRef | null;
  source: DeviceRef | null;
  /** id of a cable from the shared cable list (cables.ts); null is an internal cable */
  cable: string | null;
  app: AppRef | null;
  /** Volume: 1 is unchanged, 0 is silence, above 1 is gain */
  volume: number;
  /** An output's delay in seconds relative to the other outputs: plus plays later, minus earlier */
  delay: number;
  /** Left-right balance from -1 (left only) to 1 (right only) */
  balance: number;
};

export type AudioNode = Node<AudioNodeData, "audio">;
export type AudioEdge = Edge;

export type DeviceSlot = {
  /** Text while no device is chosen */
  placeholder: string;
  /** Whether "default" can be chosen (follow the system device) */
  allowDefault: boolean;
};

/** Texts are translation keys (i18n.ts) */
type KindMeta = {
  title: string;
  defaultLabel: string;
  hasInput: boolean;
  hasOutput: boolean;
  sink?: DeviceSlot;
  source?: DeviceSlot;
  /** The node chooses a cable from the shared cable list */
  pickCable?: boolean;
  /** The node chooses a running app */
  pickApp?: boolean;
  /** The node shows the current default devices */
  showDefaults?: boolean;
  /**
   * Maximum volume. Only the engine applies gain above 1; an app's volume
   * is its volume in the system mixer, at most 100%. Without this field the node has no volume slider
   */
  maxVolume?: number;
  /** The node has a delay slider (when the graph's delay sliders are on) */
  delay?: boolean;
  /** The node has a balance slider (when the graph's balance sliders are on): the engine's nodes, not apps */
  balance?: boolean;
};

export const NODE_KINDS: Record<AudioNodeKind, KindMeta> = {
  input: {
    title: "node_input",
    defaultLabel: "label_microphone",
    hasInput: false,
    hasOutput: true,
    source: { placeholder: "placeholder_no_device", allowDefault: true },
    maxVolume: 2,
    balance: true,
  },
  output: {
    title: "node_output",
    defaultLabel: "label_headphones",
    hasInput: true,
    hasOutput: false,
    sink: { placeholder: "placeholder_no_device", allowDefault: true },
    maxVolume: 2,
    balance: true,
    delay: true,
  },
  app: {
    title: "node_app",
    defaultLabel: "node_app",
    hasInput: true,
    hasOutput: true,
    pickApp: true,
    maxVolume: 1,
  },
  cable: {
    title: "node_cable",
    defaultLabel: "node_cable",
    hasInput: true,
    hasOutput: true,
    // the device pair is set in the cable list, the node only refers to the cable
    pickCable: true,
    maxVolume: 2,
    balance: true,
  },
  system: {
    title: "node_system",
    defaultLabel: "label_default_devices",
    hasInput: true,
    hasOutput: true,
    showDefaults: true,
  },
};

export const KIND_ORDER: AudioNodeKind[] = ["input", "app", "system", "cable", "output"];
