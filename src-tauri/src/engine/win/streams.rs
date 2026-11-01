//! Engine streams on WASAPI: every recording device and every output device
//! gets its own stream. Recording hands packets to all links, output mixes its links.

use super::super::link::{link, LinkReader, CHANNELS, RATE};
use super::super::plan::{Gain, Stream, LOOPBACK_PREFIX};
use super::super::ring::Producer;
use super::super::{DeviceError, StreamStatus};
use crate::com::{wide, ComGuard};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::{
    IAudioCaptureClient, IAudioClient, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    DEVICE_STATE_ACTIVE, WAVEFORMATEX,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, WaitForSingleObject,
};

/// A failed stream (device gone or busy) restarts no more often than this
const RETRY: Duration = Duration::from_secs(2);
/// The device buffer, in hundreds of nanoseconds
const BUFFER_HNS: i64 = 200_000;

/// The engine format; AUTOCONVERTPCM asks Windows to convert it to the device format
const FORMAT: WAVEFORMATEX = WAVEFORMATEX {
    wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
    nChannels: CHANNELS as u16,
    nSamplesPerSec: RATE as u32,
    nAvgBytesPerSec: (RATE * CHANNELS * 4) as u32,
    nBlockAlign: (CHANNELS * 4) as u16,
    wBitsPerSample: 32,
    cbSize: 0,
};
const FLAGS: u32 =
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // a panic in one audio thread must not bring down the others
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(super) fn describe(e: windows::core::Error) -> String {
    match e.code() {
        AUDCLNT_E_DEVICE_INVALIDATED => tr!("error_device_disconnected"),
        AUDCLNT_E_DEVICE_IN_USE => tr!("error_device_exclusive"),
        _ => e.message(),
    }
}

/// A device's link list; changes on the fly, the stream reads it on every packet
type Links<T> = Arc<Mutex<Vec<(String, T)>>>;
type Body<T> = fn(&str, &Mutex<Vec<(String, T)>>, &AtomicBool, &AtomicBool) -> Result<(), String>;

/// The stream of one device
struct Worker<T> {
    links: Links<T>,
    stop: Arc<AtomicBool>,
    /// The device is open and sound flows
    working: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), String>>>,
    started: Option<Instant>,
    error: Option<String>,
}

impl<T: Send + 'static> Worker<T> {
    fn new() -> Self {
        Self {
            links: Arc::new(Mutex::new(Vec::new())),
            stop: Arc::new(AtomicBool::new(false)),
            working: Arc::new(AtomicBool::new(false)),
            thread: None,
            started: None,
            error: None,
        }
    }

    /// Starts the stream if it isn't running; a failed one at most once per RETRY
    fn supervise(&mut self, device: &str, body: Body<T>) {
        if self.thread.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        if let Some(thread) = self.thread.take() {
            self.error = match thread.join() {
                Ok(result) => result.err(),
                Err(_) => Some(tr!("error_stream_crashed")),
            };
        }
        if self.started.is_some_and(|t| t.elapsed() < RETRY) {
            return;
        }
        let (device, links, stop, working) = (device.to_string(), self.links.clone(), self.stop.clone(), self.working.clone());
        self.started = Some(Instant::now());
        self.working.store(false, Ordering::Relaxed);
        self.thread = std::thread::Builder::new()
            .name(format!("audio {device}"))
            .spawn(move || body(&device, &links, &stop, &working))
            .ok();
    }

    /// The next supervise may start a failed stream at once
    fn retry_now(&mut self) {
        self.started = None;
    }

    fn error(&self) -> Option<String> {
        if self.working.load(Ordering::Relaxed) {
            None
        } else {
            self.error.clone()
        }
    }

    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn device_error<T: Send + 'static>(device: &str, worker: Option<&Worker<T>>) -> Option<DeviceError> {
    worker.and_then(|w| w.error()).map(|message| DeviceError {
        // a loopback's errors belong to its output device: the interface finds the node by it
        device: device.strip_prefix(LOOPBACK_PREFIX).unwrap_or(device).to_string(),
        message,
    })
}

#[derive(Default)]
pub struct Streams {
    captures: HashMap<String, Worker<Producer>>,
    renders: HashMap<String, Worker<LinkReader>>,
}

impl Streams {
    /// Brings streams in line with the plan on the fly: new links are added, extra ones removed
    pub fn apply(&mut self, plan: &[Stream]) -> Vec<StreamStatus> {
        // link → volume and delay
        let wanted: BTreeMap<(&str, &str), (Gain, f32)> = plan
            .iter()
            .flat_map(|s| s.sources.iter().map(|src| ((src.device.as_str(), s.sink.as_str()), (src.gain, src.delay))))
            .collect();

        for (device, w) in &self.captures {
            lock(&w.links).retain(|(sink, _)| wanted.contains_key(&(device.as_str(), sink.as_str())));
        }
        for (device, w) in &self.renders {
            let mut links = lock(&w.links);
            links.retain(|(source, _)| wanted.contains_key(&(source.as_str(), device.as_str())));
            // volume and delay change on the fly without recreating the link
            for (source, reader) in links.iter() {
                let (gain, delay) = wanted[&(source.as_str(), device.as_str())];
                reader.set_gain(gain);
                reader.set_delay(delay);
            }
        }
        for (&(source, sink), &(gain, delay)) in &wanted {
            let capture = self.captures.entry(source.to_string()).or_insert_with(Worker::new);
            if lock(&capture.links).iter().any(|(s, _)| s == sink) {
                continue;
            }
            let (producer, reader) = link();
            reader.set_gain(gain);
            reader.set_delay(delay);
            lock(&capture.links).push((sink.to_string(), producer));
            let render = self.renders.entry(sink.to_string()).or_insert_with(Worker::new);
            lock(&render.links).push((source.to_string(), reader));
        }

        // devices without links are closed
        let unused: Vec<String> = self.captures.iter().filter(|(_, w)| lock(&w.links).is_empty()).map(|(d, _)| d.clone()).collect();
        for device in unused {
            self.captures.remove(&device).unwrap().shutdown();
        }
        let unused: Vec<String> = self.renders.iter().filter(|(_, w)| lock(&w.links).is_empty()).map(|(d, _)| d.clone()).collect();
        for device in unused {
            self.renders.remove(&device).unwrap().shutdown();
        }

        for (device, w) in &mut self.captures {
            w.supervise(device, capture_thread);
        }
        for (device, w) in &mut self.renders {
            w.supervise(device, render_thread);
        }

        plan.iter()
            .map(|s| StreamStatus {
                sink: s.sink.clone(),
                sources: s.sources.iter().map(|src| src.device.clone()).collect(),
                errors: device_error(&s.sink, self.renders.get(&s.sink))
                    .into_iter()
                    .chain(s.sources.iter().filter_map(|src| device_error(&src.device, self.captures.get(&src.device))))
                    .collect(),
            })
            .collect()
    }

    pub fn retry_now(&mut self) {
        self.captures.values_mut().for_each(Worker::retry_now);
        self.renders.values_mut().for_each(Worker::retry_now);
    }

    pub fn shutdown(&mut self) {
        for (_, w) in self.captures.drain() {
            w.shutdown();
        }
        for (_, w) in self.renders.drain() {
            w.shutdown();
        }
    }
}

/// The event WASAPI wakes the thread with when a packet is due
pub(super) struct Event(pub(super) HANDLE);

impl Drop for Event {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// An audio thread gets the multimedia scheduler priority, like audio software
struct ProAudio(Option<HANDLE>);

impl ProAudio {
    fn enter() -> Self {
        let mut index = 0;
        Self(unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut index) }.ok())
    }
}

impl Drop for ProAudio {
    fn drop(&mut self) {
        if let Some(h) = self.0 {
            let _ = unsafe { AvRevertMmThreadCharacteristics(h) };
        }
    }
}

/// Opens the device and sets up a stream in the engine format.
/// "loopback:<output device>" opens the output device to record what is played into it
pub(super) fn open(device: &str) -> Result<(IAudioClient, Event), String> {
    let (device, flags) = match device.strip_prefix(LOOPBACK_PREFIX) {
        Some(render) => (render, FLAGS | AUDCLNT_STREAMFLAGS_LOOPBACK),
        None => (device, FLAGS),
    };
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(describe)?;
        let id = wide(device);
        let mm = enumerator.GetDevice(PCWSTR(id.as_ptr())).map_err(|_| tr!("error_device_not_found"))?;
        if mm.GetState().map_err(describe)? != DEVICE_STATE_ACTIVE {
            return Err(tr!("error_device_disabled"));
        }
        let client: IAudioClient = mm.Activate(CLSCTX_ALL, None).map_err(describe)?;
        let event = Event(CreateEventW(None, false, false, None).map_err(describe)?);
        client
            .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER_HNS, 0, &FORMAT, None)
            .map_err(describe)?;
        client.SetEventHandle(event.0).map_err(describe)?;
        Ok((client, event))
    }
}

fn capture_thread(device: &str, links: &Mutex<Vec<(String, Producer)>>, stop: &AtomicBool, working: &AtomicBool) -> Result<(), String> {
    let _com = ComGuard::init();
    let _priority = ProAudio::enter();
    let (client, event) = open(device)?;
    let result = unsafe { capture_loop(&client, &event, links, stop, working) };
    let _ = unsafe { client.Stop() };
    result.map_err(describe)
}

unsafe fn capture_loop(
    client: &IAudioClient,
    event: &Event,
    links: &Mutex<Vec<(String, Producer)>>,
    stop: &AtomicBool,
    working: &AtomicBool,
) -> windows::core::Result<()> {
    let capture: IAudioCaptureClient = client.GetService()?;
    client.Start()?;
    working.store(true, Ordering::Relaxed);
    let mut silence: Vec<f32> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        WaitForSingleObject(event.0, 100);
        while capture.GetNextPacketSize()? > 0 {
            let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
            capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
            let n = frames as usize * CHANNELS;
            let samples: &[f32] = if data.is_null() || flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                silence.resize(n.max(silence.len()), 0.0);
                &silence[..n]
            } else {
                std::slice::from_raw_parts(data as *const f32, n)
            };
            for (_, producer) in lock(links).iter() {
                producer.push(samples);
            }
            capture.ReleaseBuffer(frames)?;
        }
    }
    Ok(())
}

fn render_thread(device: &str, links: &Mutex<Vec<(String, LinkReader)>>, stop: &AtomicBool, working: &AtomicBool) -> Result<(), String> {
    let _com = ComGuard::init();
    let _priority = ProAudio::enter();
    let (client, event) = open(device)?;
    let result = unsafe { render_loop(&client, &event, links, stop, working) };
    let _ = unsafe { client.Stop() };
    result.map_err(describe)
}

unsafe fn render_loop(
    client: &IAudioClient,
    event: &Event,
    links: &Mutex<Vec<(String, LinkReader)>>,
    stop: &AtomicBool,
    working: &AtomicBool,
) -> windows::core::Result<()> {
    let render: IAudioRenderClient = client.GetService()?;
    let size = client.GetBufferSize()?;
    // before the start the device buffer is filled with silence so the beginning doesn't click
    render.GetBuffer(size)?;
    render.ReleaseBuffer(size, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
    client.Start()?;
    working.store(true, Ordering::Relaxed);
    let mut mix = vec![0.0f32; size as usize * CHANNELS];
    while !stop.load(Ordering::Relaxed) {
        WaitForSingleObject(event.0, 100);
        let frames = size - client.GetCurrentPadding()?;
        if frames == 0 {
            continue;
        }
        let out = &mut mix[..frames as usize * CHANNELS];
        out.fill(0.0);
        for (_, reader) in lock(links).iter_mut() {
            reader.mix_into(out);
        }
        // the sum of several sources can go out of range
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        let data = render.GetBuffer(frames)?;
        std::ptr::copy_nonoverlapping(out.as_ptr(), data as *mut f32, out.len());
        render.ReleaseBuffer(frames, 0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::ring::ring;
    use super::*;
    use crate::devices::{list, DeviceState, Flow};

    /// Line 1 of Virtual Audio Cable: one line, so the tone played into its input comes out of its output
    fn vac(flow: Flow) -> String {
        list()
            .unwrap()
            .into_iter()
            .find(|d| d.name.starts_with("Line 1 (Virtual Audio Cable") && d.flow == flow && d.state == DeviceState::Active)
            .expect("no Virtual Audio Cable")
            .id
    }

    /// Plays a tone into the VAC input (Line 1) and listens to its output: checks recording, playback
    /// and the full latency. Only someone listening to that cable hears it:
    /// cargo test vac_loopback -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vac_loopback() {
        tone_through(&vac(Flow::Render), &vac(Flow::Capture), 0.0, [1.0; 2]);
    }

    /// The same with the balance to the left: the right channel comes a quarter as loud
    /// cargo test vac_balanced -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vac_balanced() {
        let (_, [left, right]) = tone_through(&vac(Flow::Render), &vac(Flow::Capture), 0.0, [1.0, 0.25]);
        assert!((right / left - 0.25).abs() < 0.02, "{left} {right}");
    }

    /// The same with an output delay of 300 ms: the tone comes that much later
    /// cargo test vac_delayed -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vac_delayed() {
        let (plain, _) = tone_through(&vac(Flow::Render), &vac(Flow::Capture), 0.0, [1.0; 2]);
        let (delayed, _) = tone_through(&vac(Flow::Render), &vac(Flow::Capture), 0.3, [1.0; 2]);
        let extra = delayed.as_secs_f32() - plain.as_secs_f32();
        println!("the delay added {:.0} ms", extra * 1000.0);
        assert!((extra - 0.3).abs() < 0.03, "{extra}");
    }

    /// The same, but listens to the VAC input itself through WASAPI loopback, as for a cable without an output:
    /// cargo test vac_input_loopback -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vac_input_loopback() {
        let render_id = vac(Flow::Render);
        tone_through(&render_id, &format!("{LOOPBACK_PREFIX}{render_id}"), 0.0, [1.0; 2]);
    }

    /// Turns the VAC Line 1 recording device off and on under a running link (Line 1 → Line 2):
    /// the stream must come back by itself. Changes the system state for a moment:
    /// cargo test device_comes_back -- --ignored --nocapture
    #[test]
    #[ignore]
    fn device_comes_back() {
        let _com = ComGuard::init();
        let line = |n: &str, flow: Flow| {
            list()
                .unwrap()
                .into_iter()
                .find(|d| d.name.starts_with(&format!("Line {n} (Virtual Audio Cable")) && d.flow == flow && d.state == DeviceState::Active)
                .expect("VAC lines 1 and 2 must be on")
                .id
        };
        let (source, sink) = (line("1", Flow::Capture), line("2", Flow::Render));
        let plan = [Stream {
            sink: sink.clone(),
            sources: vec![super::super::super::plan::Source { device: source.clone(), gain: [1.0; 2], delay: 0.0 }],
            output: true,
        }];
        let mut streams = Streams::default();
        let errors = |streams: &mut Streams| streams.apply(&plan)[0].errors.clone();
        let wait = |streams: &mut Streams, broken: bool| {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(10) {
                if errors(streams).is_empty() != broken {
                    return started.elapsed();
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            panic!("no change in 10 s");
        };
        wait(&mut streams, false);
        crate::devices::set_enabled(&[source.clone()], false).unwrap();
        let broke = wait(&mut streams, true);
        println!("off: an error after {broke:?}: {:?}", errors(&mut streams));
        crate::devices::set_enabled(&[source.clone()], true).unwrap();
        let back = wait(&mut streams, false);
        println!("on: working again after {back:?} by retrying");

        // the same, but told that devices changed, as the device watch does
        crate::devices::set_enabled(&[source.clone()], false).unwrap();
        wait(&mut streams, true);
        crate::devices::set_enabled(&[source.clone()], true).unwrap();
        streams.retry_now();
        let back = wait(&mut streams, false);
        println!("on: working again after {back:?} when told");
        assert!(back < Duration::from_millis(1000), "{back:?}");
        streams.shutdown();
    }

    /// Plays a tone through a link with the delay and volume; returns when it was heard and how loud per channel
    fn tone_through(render_id: &str, capture_id: &str, delay: f32, gain: Gain) -> (Duration, [f64; 2]) {
        let mut capture = Worker::<Producer>::new();
        let (heard_in, heard) = ring(RATE * CHANNELS);
        lock(&capture.links).push(("test".into(), heard_in));
        capture.supervise(capture_id, capture_thread);

        let mut render = Worker::<LinkReader>::new();
        let (tone, reader) = link();
        reader.set_delay(delay);
        reader.set_gain(gain);
        lock(&render.links).push(("test".into(), reader));
        render.supervise(render_id, render_thread);
        std::thread::sleep(Duration::from_millis(300));
        assert!(capture.error().is_none() && render.error().is_none(), "{:?} {:?}", capture.error(), render.error());

        // a 440 Hz tone in 10 ms packets; the cable output is listened to all this time
        let started = Instant::now();
        let (mut phase, mut onset, mut energy, mut count) = (0usize, None, [0.0f64; 2], 0usize);
        let mut buf = vec![0.0f32; RATE * CHANNELS];
        let delay = Duration::from_secs_f32(delay);
        while started.elapsed() < Duration::from_millis(1500) + delay {
            let packet: Vec<f32> = (0..480)
                .flat_map(|i| {
                    let s = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * (phase + i) as f32 / RATE as f32).sin();
                    [s, s]
                })
                .collect();
            phase += 480;
            tone.push(&packet);
            std::thread::sleep(Duration::from_millis(10));
            buf.fill(0.0);
            let n = heard.mix_into(&mut buf);
            if onset.is_none() && buf[..n].iter().any(|s| s.abs() > 0.05) {
                onset = Some(started.elapsed());
            }
            if started.elapsed() > Duration::from_millis(700) + delay {
                for (i, &s) in buf[..n].iter().enumerate() {
                    energy[i % 2] += (s as f64) * (s as f64);
                }
                count += n / 2;
            }
        }
        let rms = energy.map(|e| (e / count.max(1) as f64).sqrt());
        println!("sound at the cable output after {onset:?}, level (RMS) {rms:.3?}, expected ~0.354 times the volume");
        render.shutdown();
        capture.shutdown();
        assert!(onset.is_some() && rms[0].max(rms[1]) > 0.1, "the tone didn't arrive");
        (onset.unwrap(), rms)
    }
}
