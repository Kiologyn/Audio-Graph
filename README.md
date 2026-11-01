# Audio Graph

A Windows desktop app for routing audio by drawing a graph: connect microphones, headphones, apps and virtual cables with arrows, press Start, and the sound goes where the arrows point.

![Audio Graph: Spotify goes to the headphones and, mixed with the microphone, into Discord](docs/screenshot.png)

The graph above sends Spotify to the headphones and, at the same time, mixes it with the microphone into a virtual cable that Discord uses as its microphone.

## Features

- **Five kinds of nodes.** Input (microphone), Output (headphones, speakers), App, Cable, and System (the Windows default devices).
- **Per-app routing.** An app plays into the device on the right of its node and hears the device on the left. This is the same setting as Windows' *Volume mixer → app output/input device*, but driven by the graph.
- **Mixing.** Several arrows into one node are mixed together. An internal cable is a mix point inside the app and needs no drivers.
- **Volume on every node.** Devices and cables go from 0 to 200%; apps go up to 100% (their volume in the Windows mixer).
- **Several graphs at once.** Run them from the graph list. A running dot shows which ones are active, and it turns red if something is wrong.
- **Problems are shown in place.** Missing devices, loops, an app connected to two outputs, conflicts between running graphs: the node gets a red note, the arrow turns red, and the header lists everything.
- **Everything is put back.** When a graph stops or the app quits, apps get their previous devices and volumes, and Windows gets its previous default devices. If the app is killed, this happens on the next start.
- **Tray.** Closing the window hides it to the tray; graphs keep running. Start, stop and quit from the tray menu.
- **Settings.** Light, dark or system theme; Russian, English, Spanish, Chinese or Japanese; turning the cables of a graph off while it isn't running.

## Requirements

- Windows 11 or Windows 10 version 1803 or later (developed on Windows 11).
- [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/), which is already installed on Windows 11 and up-to-date Windows 10.
- For cables between apps, a virtual audio cable driver such as [VB-Audio Virtual Cable](https://vb-audio.com/Cable/) or [Virtual Audio Cable](https://vac.muzychenko.net/). Each driver line is a pair of devices: a playback device that apps play into and a recording device the sound comes out of. Add this pair once in the **Cables** menu, then use it in any graph.

## How it works

| Connection | Who carries the sound |
| --- | --- |
| App → device or cable | Windows: the app's own output device is set to it |
| Device or cable → App | Windows: the app's own input device is set to it |
| System → … / … → System | Windows: the default output and input devices are switched while the graph runs |
| Between inputs, outputs and cables | The Audio Graph engine: WASAPI shared-mode streams, 48 kHz float stereo, with clock drift correction between devices |

Things that follow from this:

- An app has one output and one input device, so it can't be connected to two places directly. Connect it to a cable and fan out from the cable.
- Windows only accepts a device for an app that has opened audio at least once. Until then, the node says it is waiting, and the device is assigned as soon as the app starts playing.
- Loops (sound that would come back to where it started) are detected, including loops through several running graphs, and their arrows are not played.

## Running and building

You need [Node.js](https://nodejs.org/) 22 or later, [Rust](https://rustup.rs/) (stable, MSVC toolchain) and the [Tauri prerequisites for Windows](https://v2.tauri.app/start/prerequisites/).

```sh
npm install
npm run tauri dev        # or run.bat: development mode, picks up code changes
build-portable.bat       # portable\Audio Graph <version>.exe — a single file, no installer
```

Tests:

```sh
cd src-tauri
cargo test               # the graph planner, mixing buffers, translations
cargo test -- --ignored  # tests on real devices: need Virtual Audio Cable and change device settings
```

The app keeps its data in `%APPDATA%\dev.audiograph.app`: graphs, cables, settings, the journal of changed app devices (`app-routing.json`) and a log (`logs\audio-graph.log`).

## Project layout

| Path | What's there |
| --- | --- |
| `src/` | The interface: React, [React Flow](https://reactflow.dev/) for the canvas, Zustand for state |
| `src/locales/` | Translations, shared by the interface and the Rust side |
| `src-tauri/src/engine/plan.rs` | Turns graphs into a plan: Windows routing, engine streams, problems |
| `src-tauri/src/engine/win/` | Executes the plan: audio streams (`streams.rs`) and Windows device settings (`router.rs`) |
| `src-tauri/src/` | Storage of graphs, cables and settings, device and app lists, the tray |

## Translations

The key of every string is its Russian text, as written in the code. `src/locales/<code>.json` maps it to the other languages: the interface uses `t("…")`, and Rust messages (graph problems, device errors, the tray) use `tr!("…")`. A missing translation falls back to English, then to Russian.

To add a language: create `src/locales/<code>.json`, add it to `LANGUAGES` and `DICTIONARIES` in `src/i18n.ts`, to `Lang` in `src/settings.ts`, and to `Language` in `src-tauri/src/settings.rs` and the dictionary list in `src-tauri/src/i18n.rs`.
