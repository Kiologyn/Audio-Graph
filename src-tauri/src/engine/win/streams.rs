//! The device streams on WASAPI: a thread per device, run by engine/streams.rs.

use super::super::link::{CHANNELS, RATE};
use super::super::mixer::Mixer;
use super::super::plan::LOOPBACK_PREFIX;
use super::super::streams::{lock, Recipients};
use crate::com::{wide, ComGuard};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
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

pub(super) fn describe(e: windows::core::Error) -> String {
    match e.code() {
        AUDCLNT_E_DEVICE_INVALIDATED => tr!("error_device_disconnected"),
        AUDCLNT_E_DEVICE_IN_USE => tr!("error_device_exclusive"),
        _ => e.message(),
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

/// A stream of silence on an output device, for as long as it is held
struct KeepPlaying(IAudioClient, #[allow(dead_code)] Event);

impl KeepPlaying {
    /// While a device plays nothing, its loopback gives no packets: a link would lose the pauses,
    /// and a delay would shrink by each of them. Started once, the stream keeps the device going
    fn start(device: &str) -> Result<Self, String> {
        let (client, event) = open(device)?;
        unsafe {
            let render: IAudioRenderClient = client.GetService().map_err(describe)?;
            let size = client.GetBufferSize().map_err(describe)?;
            render.GetBuffer(size).map_err(describe)?;
            render.ReleaseBuffer(size, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32).map_err(describe)?;
            client.Start().map_err(describe)?;
        }
        Ok(Self(client, event))
    }
}

impl Drop for KeepPlaying {
    fn drop(&mut self) {
        let _ = unsafe { self.0.Stop() };
    }
}

pub(in crate::engine) fn capture_thread(device: &str, links: &Mutex<Recipients>, stop: &AtomicBool, working: &AtomicBool) -> Result<(), String> {
    let _com = ComGuard::init();
    let _priority = ProAudio::enter();
    let _playing = device.strip_prefix(LOOPBACK_PREFIX).map(KeepPlaying::start).transpose()?;
    let (client, event) = open(device)?;
    let result = unsafe { capture_loop(&client, &event, links, stop, working) };
    let _ = unsafe { client.Stop() };
    result.map_err(describe)
}

unsafe fn capture_loop(
    client: &IAudioClient,
    event: &Event,
    links: &Mutex<Recipients>,
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

pub(in crate::engine) fn render_thread(device: &str, mixer: &Mutex<Mixer>, stop: &AtomicBool, working: &AtomicBool) -> Result<(), String> {
    let _com = ComGuard::init();
    let _priority = ProAudio::enter();
    let (client, event) = open(device)?;
    let result = unsafe { render_loop(&client, &event, mixer, stop, working) };
    let _ = unsafe { client.Stop() };
    result.map_err(describe)
}

unsafe fn render_loop(
    client: &IAudioClient,
    event: &Event,
    mixer: &Mutex<Mixer>,
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
        lock(mixer).play(out);
        let data = render.GetBuffer(frames)?;
        std::ptr::copy_nonoverlapping(out.as_ptr(), data as *mut f32, out.len());
        render.ReleaseBuffer(frames, 0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::levels::Levels;
    use super::super::super::link::link;
    use super::super::super::plan::{Gain, Stream};
    use super::super::super::ring::ring;
    use super::super::super::streams::{Streams, Worker};
    use super::*;
    use crate::devices::{list, DeviceState, Flow};
    use std::time::{Duration, Instant};

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

    /// Nothing plays into the VAC input, yet its loopback keeps giving packets: the pauses stay in the link.
    /// cargo test silent_loopback_keeps_time -- --ignored --nocapture
    #[test]
    #[ignore]
    fn silent_loopback_keeps_time() {
        let _com = ComGuard::init();
        let device = format!("{LOOPBACK_PREFIX}{}", vac(Flow::Render));
        let (producer, reader) = ring(RATE * CHANNELS * 4);
        let links = Mutex::new(vec![((String::new(), 0), producer)]);
        let (stop, working) = (AtomicBool::new(false), AtomicBool::new(false));
        let frames = std::thread::scope(|s| {
            let thread = s.spawn(|| capture_thread(&device, &links, &stop, &working));
            std::thread::sleep(Duration::from_secs(2));
            stop.store(true, Ordering::Relaxed);
            thread.join().unwrap().unwrap();
            reader.len() / CHANNELS
        });
        println!("{frames} frames in 2 s");
        assert!(frames > RATE * 3 / 2, "{frames}");
    }

    /// Turns the VAC Line 1 recording device off and on under a running link (Line 1 → Line 2):
    /// the stream must come back by itself. Changes the system state for a moment:
    /// cargo test device_comes_back -- --ignored --nocapture
    #[test]
    #[ignore]
    fn device_comes_back() {
        let _com = ComGuard::init();
        let _defaults = crate::devices::win::KeepDefaults::new();
        let line = |n: &str, flow: Flow| {
            list()
                .unwrap()
                .into_iter()
                .find(|d| d.name.starts_with(&format!("Line {n} (Virtual Audio Cable")) && d.flow == flow && d.state == DeviceState::Active)
                .expect("VAC lines 1 and 2 must be on")
                .id
        };
        let (source, sink) = (line("1", Flow::Capture), line("2", Flow::Render));
        let plan = [Stream::through(&sink, true, &[(&source, [1.0; 2], 0.0)])];
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

    /// A tone from VAC Line 1 goes into Line 2 in two layers: the left channel at once, the right one
    /// 300 ms later. Only VAC lines sound; Line 2 is turned on for the test if it is off:
    /// cargo test vac_layers -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vac_layers() {
        let _com = ComGuard::init();
        let _defaults = crate::devices::win::KeepDefaults::new();
        let find = |n: &str, flow: Flow| {
            let devices = list().unwrap();
            let device = devices.into_iter().find(|d| d.name.starts_with(&format!("Line {n} (Virtual Audio Cable")) && d.flow == flow);
            device.expect("no such VAC line")
        };
        let line2 = [find("2", Flow::Render), find("2", Flow::Capture)];
        let off: Vec<String> = line2.iter().filter(|d| d.state != DeviceState::Active).map(|d| d.id.clone()).collect();
        if !off.is_empty() {
            crate::devices::set_enabled(&off, true).unwrap();
            std::thread::sleep(Duration::from_millis(500));
        }

        let source = find("1", Flow::Capture).id;
        let plan = [Stream::through(&find("2", Flow::Render).id, true, &[(&source, [1.0, 0.0], 0.0), (&source, [0.0, 1.0], 0.3)])];
        let mut streams = Streams::default();
        streams.apply(&plan);

        let mut capture = Worker::<Recipients>::new();
        let (heard_in, heard) = ring(RATE * CHANNELS);
        lock(&capture.links).push((("test".into(), 0), heard_in));
        capture.supervise(&find("2", Flow::Capture).id, capture_thread);
        let mut render = Worker::<Mixer>::new();
        let (tone, reader) = link();
        lock(&render.links).links.push((("test".into(), 0), reader));
        render.supervise(&find("1", Flow::Render).id, render_thread);
        std::thread::sleep(Duration::from_millis(400));

        let started = Instant::now();
        let (mut phase, mut onset) = (0usize, [None; 2]);
        let mut buf = vec![0.0f32; RATE * CHANNELS];
        while started.elapsed() < Duration::from_millis(1500) {
            let due = (started.elapsed().as_secs_f64() * RATE as f64) as usize;
            let packet: Vec<f32> = (phase..due)
                .flat_map(|i| {
                    let s = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin();
                    [s, s]
                })
                .collect();
            phase = due;
            tone.push(&packet);
            std::thread::sleep(Duration::from_millis(5));
            buf.fill(0.0);
            let n = heard.mix_into(&mut buf);
            for (i, s) in buf[..n].iter().enumerate() {
                if onset[i % 2].is_none() && s.abs() > 0.05 {
                    onset[i % 2] = Some(started.elapsed());
                }
            }
        }
        render.shutdown();
        capture.shutdown();
        streams.shutdown();
        if !off.is_empty() {
            crate::devices::set_enabled(&off, false).unwrap();
        }
        let [left, right] = onset.map(|o| o.expect("a channel stayed silent").as_secs_f32());
        println!("left after {left:.3} s, right after {right:.3} s");
        assert!((right - left - 0.3).abs() < 0.05, "{left} {right}");
    }

    /// Plays a tone through a link with the delay and volume; returns when it was heard and how loud per channel
    fn tone_through(render_id: &str, capture_id: &str, delay: f32, gain: Gain) -> (Duration, [f64; 2]) {
        let mut capture = Worker::<Recipients>::new();
        let (heard_in, heard) = ring(RATE * CHANNELS);
        lock(&capture.links).push((("test".into(), 0), heard_in));
        capture.supervise(capture_id, capture_thread);

        let mut render = Worker::<Mixer>::new();
        let (tone, mut reader) = link();
        reader.set_delay(delay);
        reader.stage = 1;
        let mut mixer = lock(&render.links);
        mixer.set_stages(&Stream::through("", true, &[("", gain, 0.0)]).stages, &Levels::default());
        mixer.links.push((("test".into(), 0), reader));
        drop(mixer);
        render.supervise(render_id, render_thread);
        std::thread::sleep(Duration::from_millis(300));
        assert!(capture.error().is_none() && render.error().is_none(), "{:?} {:?}", capture.error(), render.error());

        // a 440 Hz tone, as much of it as the time passed asks for: a sleep lasts longer than asked,
        // and fixed packets would come slower than a real recording; the cable output is listened to all this time
        let started = Instant::now();
        let (mut phase, mut onset, mut energy, mut count) = (0usize, None, [0.0f64; 2], 0usize);
        let mut buf = vec![0.0f32; RATE * CHANNELS];
        let delay = Duration::from_secs_f32(delay);
        while started.elapsed() < Duration::from_millis(1500) + delay {
            let due = (started.elapsed().as_secs_f64() * RATE as f64) as usize;
            let packet: Vec<f32> = (phase..due)
                .flat_map(|i| {
                    let s = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin();
                    [s, s]
                })
                .collect();
            phase = due;
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
