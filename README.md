# Audio Graph

Route audio by drawing it. Connect microphones, headphones, apps and virtual cables with arrows, press Start, and the sound goes where the arrows point. For Windows and Linux.

![Audio Graph: Spotify goes to the headphones and, mixed with the microphone, into Discord](docs/screenshot.png)

*Spotify plays in the headphones and, mixed with the microphone, goes into Discord through a virtual cable.*

> [!IMPORTANT]
> **Windows needs a virtual cable driver.** Without one, no sound can go through a cable to or from an app, and that is what most graphs do. [What to install](#virtual-cables-on-windows)

## What it's for

- **Music in a call:** mix an app with your microphone and send it to Discord, OBS or any other app.
- **Two devices at once:** hear the same sound on headphones and speakers.
- **Each app on its own device:** the game in the headphones, the music on the speakers.
- **Devices in sync:** delay speakers to match Bluetooth headphones, measured automatically with a microphone.
- **Less background noise:** a noise gate mutes everything quieter than a threshold you set.
- **Hearing yourself:** send the microphone to your headphones.

## Quick start

1. Download the app from [Releases](https://github.com/Kiologyn/Audio-Graph/releases).
   - Windows: `setup.msi`, or `portable.exe` with no installation
   - Linux: `.AppImage`, `.deb` or `.rpm`
2. **On Windows:** install a [virtual cable driver](#virtual-cables-on-windows), then press *Find cables* in the **Cables** menu.
3. Add nodes with the *Add node* button or a right click on the canvas, and drag arrows between them.
4. Press **Start**. When the graph stops, everything goes back to how it was.

## Virtual cables on Windows

An app can only play into and listen to a device the system knows, and only a driver can add one. A virtual cable is a pair of such devices: apps play into one, and the sound comes out of the other.

Audio Graph can't create these devices by itself, as it does on Linux: Microsoft requires drivers to be signed through it before Windows will load them, so the cables have to come from a signed driver that you install.

| Connection | Without a driver | With a driver |
| --- | :---: | :---: |
| App → output device | ✅ | ✅ |
| Microphone → app | ✅ | ✅ |
| Microphone → cable → output device | ✅ | ✅ |
| **App → cable** | ❌ | ✅ |
| **Cable → app** | ❌ | ✅ |

**Set it up once:**

1. Install [VB-CABLE](https://vb-audio.com/Cable/) or [Virtual Audio Cable](https://vac.muzychenko.net/).
2. Press *Find cables* in the **Cables** menu of Audio Graph. It adds every cable it finds.
3. Choose that cable in a Cable node, in any graph.

On Linux there is nothing to install: the graph creates its cables itself while it runs.

## Nodes

| Node | What it is | Settings |
| --- | --- | --- |
| **Input** | a microphone | volume |
| **Output** | headphones, speakers; its arrow out carries everything playing on the device | volume, delay |
| **App** | one program | volume |
| **Cable** | mixes everything that comes in and passes it on | volume, balance, delay, noise gate |
| **System** | the system's default devices | |

The arrow in a node's corner opens its settings, and a checked one stays visible on the node.

Sound works as with wires and a mixer: every node processes what reaches it. Cables in a row act one after another, and several paths to one node add up.

## Features

**Sound**
- Noise gate on cables, with a level meter on its slider
- Delays up to 10 minutes, and automatic calibration of outputs
- Cable volume up to 1000%, balance
- Volume of devices and apps is the system's own slider, in step with it while the graph runs

**Graphs**
- Several graphs running at once
- Export to a file and import, with the cables they use
- Undo and redo
- Problems shown on the node and the arrow: missing devices, loops, conflicts between graphs

**Safe to use**
- A graph changes the system only while it runs: devices and volumes return when it stops, even after a crash
- Unplugged devices are picked up again when they return

**App**
- Tray: graphs keep running with the window closed
- Starts with the system and starts chosen graphs (installed version only)
- Remembers the window and each graph's view
- Light and dark themes; English, Russian, Spanish, Chinese, Japanese

## Good to know

- **An app has one output.** To send it to two places, connect it to a cable and fan out from the cable.
- **An app gets its device once it opens audio.** Until then its node says it is waiting.
- **Sound the system carries skips Audio Graph.** With an app or the System node connected straight to an output, delay doesn't apply. Put a cable between them; the node tells you when this happens.
- **On Windows an app can't listen to an output device.** Put a cable between them.
- **On Linux a cable is a real device while the graph runs,** so other programs can use it too.
- **Your data** is in `%APPDATA%\dev.audiograph.app` on Windows and `~/.local/share/dev.audiograph.app` on Linux.

## Requirements

**Windows**
- Windows 10 version 1803 or later, or Windows 11
- A [virtual cable driver](#virtual-cables-on-windows)

**Linux**
- PipeWire with `pipewire-pulse`, or PulseAudio

## Building from source

| | Windows | Linux |
| --- | --- | --- |
| Install the build tools | `scripts\windows\setup.bat` | `scripts/linux/setup.sh` |
| Start in development mode | `scripts\windows\dev.bat` | `scripts/linux/dev.sh` |
| Build into `release/<version>/` | `scripts\windows\build.bat` | `scripts/linux/build.sh` |
| Build the Linux version in WSL | `scripts\windows\build-linux.bat` | |

*setup* covers Debian/Ubuntu, Fedora and Arch. On other distributions, install [Node.js](https://nodejs.org/) 22+, [Rust](https://rustup.rs/), the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) and libpulse by hand.
