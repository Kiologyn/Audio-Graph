//! The Linux audio server through libpulse. PulseAudio and PipeWire (pipewire-pulse) speak
//! the same protocol, so the code is the same too.
//!
//! The wrapper is synchronous: every call spins the event loop until the server answers, but no
//! longer than TIMEOUT: a hung server must not stop the engine. A connection is single-threaded (that's
//! how libpulse works): the engine has its own, device and app lists open one per request.

use libpulse_binding as pa;
use pa::callbacks::ListResult;
use pa::context::introspect::{SinkInputInfo, SourceOutputInfo};
use pa::context::{Context, FlagSet, State};
use pa::def::BufferAttr;
use pa::error::{Code, PAErr};
use pa::mainloop::standard::Mainloop;
use pa::operation::{Operation, State as OperationState};
use pa::proplist::Proplist;
use pa::sample::{Format, Spec};
use pa::stream::{FlagSet as StreamFlags, PeekResult, SeekMode, State as StreamState, Stream as PaStream};
use pa::time::MicroSeconds;
use pa::volume::{ChannelVolumes, Volume, VolumeLinear};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How long to wait for the server. It usually answers in milliseconds; longer means it hung
const TIMEOUT: Duration = Duration::from_secs(3);

pub struct Pulse {
    mainloop: Mainloop,
    context: Context,
    /// The server didn't answer in time: the connection is no longer usable
    broken: bool,
}

/// An output (sink) or input (source) device
#[derive(Clone, Debug)]
pub struct Device {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub volume: ChannelVolumes,
    /// For a source: it is the monitor of an output device (what plays into it), not an input device
    pub monitor_of: Option<u32>,
}

/// A stream of an app or module: playback (sink input) or recording (source output)
#[derive(Clone, Debug)]
pub struct Stream {
    pub index: u32,
    /// The device the stream is connected to: a sink or source index
    pub device: u32,
    /// Set for app streams; module streams have none
    pub client: Option<u32>,
    pub corked: bool,
    pub volume: ChannelVolumes,
    pub volume_writable: bool,
    /// The app's executable file name (application.process.binary)
    pub binary: Option<String>,
    pub pid: Option<u32>,
    pub app_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub index: u32,
    pub name: String,
    pub argument: String,
}

#[derive(Clone, Debug)]
pub struct Client {
    pub binary: Option<String>,
    pub pid: Option<u32>,
    pub app_name: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Server {
    pub name: String,
    pub version: String,
    pub default_sink: Option<String>,
    pub default_source: Option<String>,
}

fn props(list: &Proplist) -> (Option<String>, Option<u32>, Option<String>) {
    (
        list.get_str("application.process.binary"),
        list.get_str("application.process.id").and_then(|p| p.parse().ok()),
        list.get_str("application.name"),
    )
}

fn sink_input(i: &SinkInputInfo) -> Stream {
    let (binary, pid, app_name) = props(&i.proplist);
    Stream {
        index: i.index,
        device: i.sink,
        client: i.client,
        corked: i.corked,
        volume: i.volume,
        volume_writable: i.volume_writable,
        binary,
        pid,
        app_name,
    }
}

fn source_output(i: &SourceOutputInfo) -> Stream {
    let (binary, pid, app_name) = props(&i.proplist);
    Stream {
        index: i.index,
        device: i.source,
        client: i.client,
        corked: i.corked,
        volume: i.volume,
        volume_writable: i.volume_writable,
        binary,
        pid,
        app_name,
    }
}

/// Volume as an amplitude factor: 0.5 is half as loud, 2 twice as loud
pub fn linear(channels: u8, gain: f32) -> ChannelVolumes {
    let mut v = ChannelVolumes::default();
    v.set(channels.max(1), Volume::from(VolumeLinear(f64::from(gain.max(0.0)))));
    v
}

/// Volume as a mixer slider position (100% is unchanged)
pub fn mixer(channels: u8, level: f32) -> ChannelVolumes {
    let mut v = ChannelVolumes::default();
    v.set(channels.max(1), Volume((level.max(0.0) * Volume::NORMAL.0 as f32).round() as u32));
    v
}

/// The same volume at another slider position: the channels keep their balance
pub fn rescaled(volume: &ChannelVolumes, level: f32) -> ChannelVolumes {
    let mut v = *volume;
    v.scale(Volume((level.max(0.0) * Volume::NORMAL.0 as f32).round() as u32));
    v
}

/// Where a slider stands when the channels differ: the loudest one, rescaled moves it
pub fn slider_level(v: &ChannelVolumes) -> f32 {
    v.max().0 as f32 / Volume::NORMAL.0 as f32
}

fn describe(e: pa::error::PAErr) -> String {
    e.to_string().unwrap_or_else(|| format!("{e:?}"))
}

/// A list answer: items until the end comes; an error instead of the end fails the whole request
struct Collected<T> {
    items: Vec<T>,
    failed: bool,
}

impl<T> Collected<T> {
    fn shared() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self { items: Vec::new(), failed: false }))
    }
}

impl Pulse {
    /// Connects to the server from the environment (PULSE_SERVER, the session socket)
    pub fn connect(client_name: &str) -> Result<Self, String> {
        let mainloop = Mainloop::new().ok_or("libpulse: mainloop")?;
        let mut proplist = Proplist::new().ok_or("libpulse: proplist")?;
        let _ = proplist.set_str("application.name", client_name);
        let _ = proplist.set_str("application.icon_name", "audio-card");
        let mut context = Context::new_with_proplist(&mainloop, client_name, &proplist).ok_or("libpulse: context")?;
        // no autospawn: starting the server is the session's job, not the app's
        context.connect(None, FlagSet::NOAUTOSPAWN, None).map_err(describe)?;
        let mut pulse = Self { mainloop, context, broken: false };
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match pulse.context.get_state() {
                State::Ready => return Ok(pulse),
                State::Failed | State::Terminated => return Err(pulse.error()),
                _ => {}
            }
            pulse.turn(deadline)?;
        }
    }

    pub fn is_ready(&self) -> bool {
        !self.broken && self.context.get_state() == State::Ready
    }

    fn error(&self) -> String {
        describe(self.context.errno())
    }

    /// One turn of the event loop: waits for events, but not past the deadline
    fn turn(&mut self, deadline: Instant) -> Result<(), String> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            self.broken = true;
            return Err(tr!("error_audio_server_not_responding"));
        }
        self.mainloop.prepare(Some(MicroSeconds(left.as_micros() as u64))).map_err(describe)?;
        self.mainloop.poll().map_err(describe)?;
        self.mainloop.dispatch().map_err(describe)?;
        Ok(())
    }

    /// Waits for the server to answer a request; no answer in time cancels the request
    fn wait<F: ?Sized>(&mut self, mut op: Operation<F>) -> Result<(), String> {
        let deadline = Instant::now() + TIMEOUT;
        while op.get_state() == OperationState::Running {
            if let Err(e) = self.turn(deadline) {
                // a cancelled request won't call its answer handler later
                op.cancel();
                return Err(e);
            }
            if !self.is_ready() {
                op.cancel();
                return Err(self.error());
            }
        }
        if op.get_state() == OperationState::Cancelled {
            return Err(self.error());
        }
        Ok(())
    }

    /// A request with a yes/no answer: false is a server error
    fn confirm(&mut self, start: impl FnOnce(&mut Self, Box<dyn FnMut(bool)>) -> Operation<dyn FnMut(bool)>) -> Result<(), String> {
        let ok = Rc::new(RefCell::new(false));
        let done = ok.clone();
        let op = start(self, Box::new(move |success| *done.borrow_mut() = success));
        self.wait(op)?;
        let success = *ok.borrow();
        if success {
            Ok(())
        } else {
            Err(self.error())
        }
    }

    /// A list: the handler collects the server's answer, an error at the end of the list fails the request
    fn collect<T: Clone, F: ?Sized>(&mut self, out: Rc<RefCell<Collected<T>>>, op: Operation<F>) -> Result<Vec<T>, String> {
        self.wait(op)?;
        let collected = out.borrow();
        if collected.failed {
            return Err(self.error());
        }
        Ok(collected.items.clone())
    }

    pub fn server(&mut self) -> Result<Server, String> {
        let out: Rc<RefCell<Option<Server>>> = Rc::new(RefCell::new(None));
        let dst = out.clone();
        let op = self.context.introspect().get_server_info(move |i| {
            *dst.borrow_mut() = Some(Server {
                name: i.server_name.as_deref().unwrap_or_default().to_string(),
                version: i.server_version.as_deref().unwrap_or_default().to_string(),
                default_sink: i.default_sink_name.as_deref().map(str::to_string),
                default_source: i.default_source_name.as_deref().map(str::to_string),
            })
        });
        self.wait(op)?;
        let server = out.borrow().clone();
        server.ok_or_else(|| self.error())
    }

    pub fn sinks(&mut self) -> Result<Vec<Device>, String> {
        let out = Collected::shared();
        let dst = out.clone();
        let op = self.context.introspect().get_sink_info_list(move |r| match r {
            ListResult::Item(i) => dst.borrow_mut().items.push(Device {
                index: i.index,
                name: i.name.as_deref().unwrap_or_default().to_string(),
                description: i.description.as_deref().unwrap_or_default().to_string(),
                volume: i.volume,
                monitor_of: None,
            }),
            ListResult::Error => dst.borrow_mut().failed = true,
            ListResult::End => {}
        });
        self.collect(out, op)
    }

    pub fn sources(&mut self) -> Result<Vec<Device>, String> {
        let out = Collected::shared();
        let dst = out.clone();
        let op = self.context.introspect().get_source_info_list(move |r| match r {
            ListResult::Item(i) => dst.borrow_mut().items.push(Device {
                index: i.index,
                name: i.name.as_deref().unwrap_or_default().to_string(),
                description: i.description.as_deref().unwrap_or_default().to_string(),
                volume: i.volume,
                monitor_of: i.monitor_of_sink,
            }),
            ListResult::Error => dst.borrow_mut().failed = true,
            ListResult::End => {}
        });
        self.collect(out, op)
    }

    pub fn sink_inputs(&mut self) -> Result<Vec<Stream>, String> {
        let out = Collected::shared();
        let dst = out.clone();
        let op = self.context.introspect().get_sink_input_info_list(move |r| match r {
            ListResult::Item(i) => dst.borrow_mut().items.push(sink_input(i)),
            ListResult::Error => dst.borrow_mut().failed = true,
            ListResult::End => {}
        });
        self.collect(out, op)
    }

    pub fn source_outputs(&mut self) -> Result<Vec<Stream>, String> {
        let out = Collected::shared();
        let dst = out.clone();
        let op = self.context.introspect().get_source_output_info_list(move |r| match r {
            ListResult::Item(i) => dst.borrow_mut().items.push(source_output(i)),
            ListResult::Error => dst.borrow_mut().failed = true,
            ListResult::End => {}
        });
        self.collect(out, op)
    }

    pub fn clients(&mut self) -> Result<Vec<Client>, String> {
        let out = Collected::shared();
        let dst = out.clone();
        let op = self.context.introspect().get_client_info_list(move |r| match r {
            ListResult::Item(i) => {
                let (binary, pid, app_name) = props(&i.proplist);
                dst.borrow_mut().items.push(Client { binary, pid, app_name });
            }
            ListResult::Error => dst.borrow_mut().failed = true,
            ListResult::End => {}
        });
        self.collect(out, op)
    }

    pub fn modules(&mut self) -> Result<Vec<Module>, String> {
        let out = Collected::shared();
        let dst = out.clone();
        let op = self.context.introspect().get_module_info_list(move |r| match r {
            ListResult::Item(i) => dst.borrow_mut().items.push(Module {
                index: i.index,
                name: i.name.as_deref().unwrap_or_default().to_string(),
                argument: i.argument.as_deref().unwrap_or_default().to_string(),
            }),
            ListResult::Error => dst.borrow_mut().failed = true,
            ListResult::End => {}
        });
        self.collect(out, op)
    }

    /// Loads a server module; returns its index
    pub fn load_module(&mut self, name: &str, argument: &str) -> Result<u32, String> {
        let out = Rc::new(RefCell::new(u32::MAX));
        let dst = out.clone();
        let op = self.context.introspect().load_module(name, argument, move |index| *dst.borrow_mut() = index);
        self.wait(op)?;
        let index = *out.borrow();
        // PA_INVALID_INDEX: the server rejected the arguments or the device doesn't exist
        if index == u32::MAX {
            Err(self.error())
        } else {
            Ok(index)
        }
    }

    pub fn unload_module(&mut self, index: u32) -> Result<(), String> {
        self.confirm(|p, done| p.context.introspect().unload_module(index, done))
    }

    pub fn move_sink_input(&mut self, index: u32, sink: &str) -> Result<(), String> {
        self.confirm(|p, done| p.context.introspect().move_sink_input_by_name(index, sink, Some(done)))
    }

    pub fn move_source_output(&mut self, index: u32, source: &str) -> Result<(), String> {
        self.confirm(|p, done| p.context.introspect().move_source_output_by_name(index, source, Some(done)))
    }

    pub fn set_sink_volume(&mut self, name: &str, volume: &ChannelVolumes) -> Result<(), String> {
        self.confirm(|p, done| p.context.introspect().set_sink_volume_by_name(name, volume, Some(done)))
    }

    pub fn set_source_volume(&mut self, name: &str, volume: &ChannelVolumes) -> Result<(), String> {
        self.confirm(|p, done| p.context.introspect().set_source_volume_by_name(name, volume, Some(done)))
    }

    pub fn set_sink_input_volume(&mut self, index: u32, volume: &ChannelVolumes) -> Result<(), String> {
        self.confirm(|p, done| p.context.introspect().set_sink_input_volume(index, volume, Some(done)))
    }

    pub fn set_default_sink(&mut self, name: &str) -> Result<(), String> {
        self.confirm(|p, done| p.context.set_default_sink(name, done))
    }

    pub fn set_default_source(&mut self, name: &str) -> Result<(), String> {
        self.confirm(|p, done| p.context.set_default_source(name, done))
    }
}

/// Properties of the engine's streams: WirePlumber must not apply a saved volume or device
/// to them (otherwise one stream's volume carries over to others)
const STREAM_PROPS: [(&str, &str); 3] = [("audiograph.link", "1"), ("state.restore-props", "false"), ("state.restore-target", "false")];
/// Streams stay on their device: when it goes, the stream ends instead of sounding somewhere else
const STREAM_FLAGS: StreamFlags = StreamFlags::DONT_MOVE.union(StreamFlags::ADJUST_LATENCY);
/// A sample of the float format the streams use
const SAMPLE: usize = size_of::<f32>();
/// "Leave it to the server" in buffer sizes
const SERVER_DECIDES: u32 = u32::MAX;

/// A recording or playback stream on its own server connection, in float samples.
/// There are no callbacks: whoever owns it waits for events and asks what can be read or written
pub struct AudioStream {
    // before the connection: the stream goes first
    stream: PaStream,
    pulse: Pulse,
    channels: u8,
    /// What came from the server and doesn't make whole frames yet
    bytes: Vec<u8>,
    samples: Vec<f32>,
}

impl AudioStream {
    fn open(device: &str, rate: u32, channels: u8, connect: impl FnOnce(&mut PaStream) -> Result<(), PAErr>) -> Result<Self, String> {
        let mut pulse = Pulse::connect("Audio Graph")?;
        let spec = Spec { format: Format::FLOAT32NE, channels, rate };
        let mut props = Proplist::new().ok_or("libpulse: proplist")?;
        for (key, value) in STREAM_PROPS {
            let _ = props.set_str(key, value);
        }
        let mut stream = PaStream::new_with_proplist(&mut pulse.context, "link", &spec, None, &mut props).ok_or_else(|| pulse.error())?;
        connect(&mut stream).map_err(describe)?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match stream.get_state() {
                StreamState::Ready => break,
                StreamState::Failed | StreamState::Terminated => {
                    let error = pulse.context.errno();
                    return Err(if error.0.abs() == Code::NoEntity as i32 {
                        tr!("error_device_not_found")
                    } else {
                        format!("{device}: {}", describe(error))
                    });
                }
                _ => {}
            }
            pulse.turn(deadline)?;
        }
        Ok(Self { stream, pulse, channels, bytes: Vec::new(), samples: Vec::new() })
    }

    /// Records from a source in packets of about this length
    pub fn record(device: &str, rate: u32, channels: u8, packet: Duration) -> Result<Self, String> {
        let attr = BufferAttr {
            maxlength: SERVER_DECIDES,
            tlength: SERVER_DECIDES,
            prebuf: SERVER_DECIDES,
            minreq: SERVER_DECIDES,
            fragsize: bytes(packet, rate, channels),
        };
        Self::open(device, rate, channels, |s| s.connect_record(Some(device), Some(&attr), STREAM_FLAGS))
    }

    /// Plays into a sink with this much sound ahead, asking for more in packets of about this length.
    /// The stream's own volume stays at 100%: the volume is in what is written
    pub fn play(device: &str, rate: u32, channels: u8, ahead: Duration, packet: Duration) -> Result<Self, String> {
        let attr = BufferAttr {
            maxlength: SERVER_DECIDES,
            tlength: bytes(ahead, rate, channels),
            // no waiting for a full buffer, at the start or after running dry: a link always has
            // something to write, and under load this clicks less than waiting (measured)
            prebuf: 0,
            minreq: bytes(packet, rate, channels),
            fragsize: SERVER_DECIDES,
        };
        let volume = linear(channels, 1.0);
        let flags = STREAM_FLAGS | StreamFlags::START_UNMUTED;
        Self::open(device, rate, channels, |s| s.connect_playback(Some(device), Some(&attr), flags, Some(&volume), None))
    }

    /// Sleeps until the server has something, at most for this long. A stream that ended is an error:
    /// its device is gone or the server went away
    pub fn wait(&mut self, at_most: Duration) -> Result<(), String> {
        self.pulse.turn(Instant::now() + at_most)?;
        match self.stream.get_state() {
            StreamState::Ready => Ok(()),
            _ => Err(tr!("error_device_disconnected")),
        }
    }

    fn frame(&self) -> usize {
        SAMPLE * self.channels as usize
    }

    /// Hands over all that was recorded by now, in whole frames
    pub fn read(&mut self, mut each: impl FnMut(&[f32])) -> Result<(), String> {
        loop {
            match self.stream.peek().map_err(describe)? {
                PeekResult::Empty => return Ok(()),
                PeekResult::Data(data) => self.bytes.extend_from_slice(data),
                // the server has no sound for that part
                PeekResult::Hole(size) => self.bytes.resize(self.bytes.len() + size, 0),
            }
            self.stream.discard().map_err(describe)?;
            let whole = self.bytes.len() / self.frame() * self.frame();
            self.samples.clear();
            self.samples.extend(self.bytes[..whole].chunks_exact(SAMPLE).map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])));
            self.bytes.drain(..whole);
            each(&self.samples);
        }
    }

    /// How many frames the server wants now
    pub fn writable(&self) -> Result<usize, String> {
        let bytes = self.stream.writable_size().ok_or_else(|| self.pulse.error())?;
        Ok(bytes / self.frame())
    }

    pub fn write(&mut self, samples: &[f32]) -> Result<(), String> {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_ne_bytes()).collect();
        self.stream.write_copy(&bytes, 0, SeekMode::Relative).map_err(describe)
    }
}

impl Drop for AudioStream {
    fn drop(&mut self) {
        let _ = self.stream.disconnect();
    }
}

/// A length of sound in bytes of the float format
fn bytes(length: Duration, rate: u32, channels: u8) -> u32 {
    (length.as_secs_f64() * rate as f64) as u32 * SAMPLE as u32 * channels as u32
}

impl Drop for Pulse {
    fn drop(&mut self) {
        self.context.disconnect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The volumes as amplitude factors, to a thousandth (the server keeps them on a cubic scale)
    fn factors(v: &ChannelVolumes) -> Vec<f64> {
        v.get().iter().map(|&v| (VolumeLinear::from(v).0 * 1000.0).round() / 1000.0).collect()
    }

    #[test]
    fn linear_volume_is_an_amplitude_factor() {
        assert_eq!(factors(&linear(2, 0.5)), [0.5, 0.5]);
        assert_eq!(factors(&linear(0, 1.0)), [1.0]);
        assert_eq!(factors(&linear(1, 2.0)), [2.0]);
    }
}
