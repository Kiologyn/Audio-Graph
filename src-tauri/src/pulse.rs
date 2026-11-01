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
use pa::mainloop::standard::Mainloop;
use pa::operation::{Operation, State as OperationState};
use pa::proplist::Proplist;
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
    /// For a source: it is the monitor of an output device (what plays into it), not an input device
    pub monitor_of: Option<u32>,
}

/// A stream of an app or module: playback (sink input) or recording (source output)
#[derive(Clone, Debug)]
pub struct Stream {
    pub index: u32,
    /// The device the stream is connected to: a sink or source index
    pub device: u32,
    /// Set for app streams; module streams (loopbacks) have none
    pub client: Option<u32>,
    pub owner_module: Option<u32>,
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
        owner_module: i.owner_module,
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
        owner_module: i.owner_module,
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

/// A stream's volume slider position: 1.0 is 100%
pub fn mixer_level(v: &ChannelVolumes) -> f32 {
    v.avg().0 as f32 / Volume::NORMAL.0 as f32
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

impl Drop for Pulse {
    fn drop(&mut self) {
        self.context.disconnect();
    }
}
