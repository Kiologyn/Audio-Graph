import type { Edge, Node } from "@xyflow/react";
import type { Place, SpaceView } from "./space";

/**
 * Graph node kinds.
 * - input  — an input device (microphone, a cable's recording side): output only
 * - output — an output device (headphones, speakers): input = what to play, output = everything
 *            playing on the device, also what doesn't come through the graph
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
export type NodeSlider = "volume" | "balance" | "delay" | "gate" | "place";

export type AudioNodeData = {
  kind: AudioNodeKind;
  label: string;
  sink: DeviceRef | null;
  source: DeviceRef | null;
  /** id of a cable from the shared cable list (cables.ts); null is an internal cable */
  cable: string | null;
  app: AppRef | null;
  /** A cable's volume: 1 is unchanged, 0 is silence, above 1 is gain */
  volume: number;
  /** What the system's slider of a device or app is set to while the graph runs; null leaves it alone */
  systemVolume: number | null;
  /**
   * Delay in seconds. A device's is a minus, how much earlier it sounds than the rest;
   * a cable's holds back the sound passing through it (from 0)
   */
  delay: number;
  /** A cable's left-right balance from -1 (left only) to 1 (right only) */
  balance: number;
  /** A cable's noise gate in dB: quieter sound is muted; NO_GATE lets everything through */
  gate: number;
  /** A cable's place in space; null leaves its sound as it is */
  place: Place | null;
  /** How the place is looked at on the node */
  spaceView: SpaceView | null;
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

/** A noise gate at this level lets everything through; matches NO_GATE in plan.rs */
export const NO_GATE = -100;

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
   * The most volume the node can have; the graph's limit may be lower. Without this field
   * the node has no volume slider
   */
  maxVolume?: number;
  /** The volume is the system's slider of the device or app, set only while the graph runs */
  systemVolume?: boolean;
  /**
   * The node has a delay slider. earlier: an output's, a minus, how much earlier it sounds than
   * the rest; hold: a cable's, a plus, it holds sound back
   */
  delay?: "earlier" | "hold";
  /** The node has a balance slider: the engine's nodes, not what the system's sliders stand for */
  balance?: boolean;
  /** The node has a noise gate */
  gate?: boolean;
  /** The node has a place in space */
  place?: boolean;
};

export const NODE_KINDS: Record<AudioNodeKind, KindMeta> = {
  input: {
    title: "node_input",
    defaultLabel: "label_microphone",
    hasInput: false,
    hasOutput: true,
    source: { placeholder: "placeholder_no_device", allowDefault: true },
    maxVolume: 1,
    systemVolume: true,
  },
  output: {
    title: "node_output",
    defaultLabel: "label_headphones",
    hasInput: true,
    hasOutput: true,
    sink: { placeholder: "placeholder_no_device", allowDefault: true },
    maxVolume: 1,
    systemVolume: true,
    delay: "earlier",
  },
  app: {
    title: "node_app",
    defaultLabel: "node_app",
    hasInput: true,
    hasOutput: true,
    pickApp: true,
    maxVolume: 1,
    systemVolume: true,
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
    gate: true,
    place: true,
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
  const { maxVolume, balance, delay, gate, place } = NODE_KINDS[kind];
  const has = { volume: maxVolume, balance, delay, gate, place };
  return (["volume", "balance", "delay", "gate", "place"] as const).filter((slider) => has[slider]);
}

/** The order in the add menu: the devices, what connects them, then what the system runs */
export const KIND_ORDER: AudioNodeKind[] = ["input", "output", "cable", "app", "system"];
