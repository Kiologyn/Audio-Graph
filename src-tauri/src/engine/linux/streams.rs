//! The device streams on the audio server: a thread per device, run by engine/streams.rs.
//! Each thread has its own server connection: a connection is single-threaded.

use super::super::link::{LinkReader, CHANNELS, RATE};
use super::super::ring::Producer;
use super::super::streams::{lock, Peer};
use crate::pulse::AudioStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// What the server is asked for: recording in packets of this length
const PACKET: Duration = Duration::from_millis(10);
/// and output with this much sound ahead: with less, PulseAudio starts to stutter on weak machines
const AHEAD: Duration = Duration::from_millis(30);
/// How long a thread sleeps at most: it looks at its stop flag this often when no sound comes
const WAKE: Duration = Duration::from_millis(100);

pub(in crate::engine) fn capture_thread(device: &str, links: &Mutex<Vec<(Peer, Producer)>>, stop: &AtomicBool, working: &AtomicBool) -> Result<(), String> {
    let mut stream = AudioStream::record(device, RATE as u32, CHANNELS as u8, PACKET)?;
    working.store(true, Ordering::Relaxed);
    while !stop.load(Ordering::Relaxed) {
        stream.wait(WAKE)?;
        stream.read(|samples| {
            for (_, producer) in lock(links).iter() {
                producer.push(samples);
            }
        })?;
    }
    Ok(())
}

pub(in crate::engine) fn render_thread(device: &str, links: &Mutex<Vec<(Peer, LinkReader)>>, stop: &AtomicBool, working: &AtomicBool) -> Result<(), String> {
    let mut stream = AudioStream::play(device, RATE as u32, CHANNELS as u8, AHEAD, PACKET)?;
    working.store(true, Ordering::Relaxed);
    let mut mix = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        stream.wait(WAKE)?;
        let frames = stream.writable()?;
        if frames == 0 {
            continue;
        }
        mix.clear();
        mix.resize(frames * CHANNELS, 0.0f32);
        for (_, reader) in lock(links).iter_mut() {
            reader.mix_into(&mut mix);
        }
        // the sum of several sources can go out of range
        for s in mix.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        stream.write(&mix)?;
    }
    Ok(())
}
