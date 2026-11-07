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

/** A node's sliders. They are in its menu; a pinned one stays on the node too */
export type NodeSlider = "volume" | "balance" | "delay";

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
  /**
   * Delay in seconds. An output's is relative to the other outputs: plus plays later, minus earlier;
   * a cable's holds back the sound passing through it (from 0)
   */
  delay: number;
  /** Left-right balance from -1 (left only) to 1 (right only) */
  balance: number;
  /** The sliders that stay on the node */
  pinned: NodeSlider[];
};

export type AudioNode = Node<AudioNodeData, "audio">;
export type AudioEdge = Edge;

export type DeviceSlot = {
  /** Text while no device is chosen */
  placeholder: string;
  /** Whether "default" can be chosen (follow the system device) */
  allowDefault: boolean;
};

/** The widest volume limit of a graph; matches VOLUME_LIMITS in graphs.rs */
export const MAX_VOLUME_LIMIT = 10;

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
   * The most volume the node can have; the graph's limit may be lower. Only the engine applies gain
   * above 1; an app's volume is its volume in the system mixer, at most 100%. Without this field the node
   * has no volume slider
   */
  maxVolume?: number;
  /**
   * The node has a delay slider. relative: an output's, from minus the range to plus, relative
   * to the other outputs; hold: a cable's, from 0, it only holds sound back
   */
  delay?: "relative" | "hold";
  /** The node has a balance slider: the engine's nodes, not apps */
  balance?: boolean;
};

export const NODE_KINDS: Record<AudioNodeKind, KindMeta> = {
  input: {
    title: "node_input",
    defaultLabel: "label_microphone",
    hasInput: false,
    hasOutput: true,
    source: { placeholder: "placeholder_no_device", allowDefault: true },
    maxVolume: MAX_VOLUME_LIMIT,
    balance: true,
  },
  output: {
    title: "node_output",
    defaultLabel: "label_headphones",
    hasInput: true,
    hasOutput: false,
    sink: { placeholder: "placeholder_no_device", allowDefault: true },
    maxVolume: MAX_VOLUME_LIMIT,
    balance: true,
    delay: "relative",
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
    maxVolume: MAX_VOLUME_LIMIT,
    balance: true,
    delay: "hold",
  },
  system: {
    title: "node_system",
    defaultLabel: "label_default_devices",
    hasInput: true,
    hasOutput: true,
    showDefaults: true,
  },
};

/** The sliders of a node kind, in the order they are shown */
export function nodeSliders(kind: AudioNodeKind): NodeSlider[] {
  const { maxVolume, balance, delay } = NODE_KINDS[kind];
  const has = { volume: maxVolume, balance, delay };
  return (["volume", "balance", "delay"] as const).filter((slider) => has[slider]);
}

/** The order in the add menu: the devices, what connects them, then what the system runs */
export const KIND_ORDER: AudioNodeKind[] = ["input", "output", "cable", "app", "system"];
