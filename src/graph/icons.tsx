import type { AudioNodeKind } from "./types";

const kindPaths: Record<AudioNodeKind, string> = {
  // microphone
  input:
    "M12 3a3 3 0 0 0-3 3v5a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3Zm-6 8a6 6 0 0 0 12 0M12 17v4m-3 0h6",
  // headphones
  output: "M4 15v-3a8 8 0 0 1 16 0v3M4 15h3v5H5a1 1 0 0 1-1-1v-4Zm16 0h-3v5h2a1 1 0 0 0 1-1v-4Z",
  // app window
  app: "M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1Zm-1 4h18",
  // jack plug with a wire
  cable: "M9 2v5M6 7h6v6a3 3 0 0 1-6 0ZM9 16v1a4 4 0 0 0 4 4h1a4 4 0 0 0 4-4V9a3 3 0 0 1 3-3",
  // monitor: the whole system
  system: "M4 4h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1ZM8 20h8M12 16v4",
};

const uiPaths = {
  chevron: "m6 9 6 6 6-6",
  pencil: "M4 20h4L19 9l-4-4L4 16v4ZM13.5 6.5l4 4",
  trash: "M4 7h16M10 11v6M14 11v6M6 7l1 12a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-12M9 7V4h6v3",
  plus: "M12 5v14M5 12h14",
  check: "m5 12 5 5 9-10",
  // two nodes joined by an edge
  graph: "M3 3h6v6H3ZM15 15h6v6h-6ZM9 6h4a5 5 0 0 1 5 5v4",
  play: "M7 5v14l11-7Z",
  stop: "M7 7h10v10H7Z",
  warning: "M12 4 2.8 19.5h18.4ZM12 10v4.5M12 17.2v.1",
  // speaker with sound waves
  volume: "M11 5 6 9H2v6h4l5 4V5ZM15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14",
  // gear
  settings:
    "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z",
};

function Svg({ d, size }: { d: string; size: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={d} />
    </svg>
  );
}

export function KindIcon({ kind, size = 14 }: { kind: AudioNodeKind; size?: number }) {
  return <Svg d={kindPaths[kind]} size={size} />;
}

export function Icon({ name, size = 14 }: { name: keyof typeof uiPaths; size?: number }) {
  return <Svg d={uiPaths[name]} size={size} />;
}
