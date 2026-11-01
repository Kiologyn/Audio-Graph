//! The calibration signal through the audio server: blocking streams of libpulse-simple.

use super::RATE;
use libpulse_binding::def::BufferAttr;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Samples per write and read: 10 ms
const CHUNK: usize = RATE / 100;
/// What the playback buffer holds: like the engine's loopbacks, a few tens of milliseconds
const BUFFERED: usize = RATE * 30 / 1000;
const BYTES: usize = 4;

fn spec() -> Spec {
    Spec {
        format: Format::F32le,
        channels: 1,
        rate: RATE as u32,
    }
}

fn stream(direction: Direction, device: &str) -> Result<Simple, String> {
    let attr = BufferAttr {
        maxlength: u32::MAX,
        tlength: (BUFFERED * BYTES) as u32,
        prebuf: u32::MAX,
        minreq: (CHUNK * BYTES) as u32,
        fragsize: (CHUNK * BYTES) as u32,
    };
    Simple::new(None, "Audio Graph", direction, Some(device), "delay calibration", &spec(), None, Some(&attr))
        .map_err(|e| e.to_string().unwrap_or_default())
}

/// Plays sound into output while recording mic. Returns the recording and where in it, in samples,
/// the sound's start was handed to the output (as with the Windows version)
pub fn play_and_record(output: &str, mic: &str, sound: &[f32]) -> Result<(Vec<f32>, f64), String> {
    let stop = Arc::new(AtomicBool::new(false));
    let recorder = {
        let (mic, stop) = (mic.to_string(), stop.clone());
        std::thread::spawn(move || record(&mic, &stop))
    };
    std::thread::sleep(Duration::from_millis(100));
    let played = play(output, sound);
    std::thread::sleep(Duration::from_millis(100));
    stop.store(true, Ordering::Relaxed);
    let (recording, first) = recorder.join().map_err(|_| tr!("error_stream_crashed"))??;
    let handed = played?;
    let at = if handed >= first {
        (handed - first).as_secs_f64()
    } else {
        -(first - handed).as_secs_f64()
    };
    Ok((recording, at * RATE as f64))
}

/// The recording and when its first sample was recorded
fn record(mic: &str, stop: &AtomicBool) -> Result<(Vec<f32>, Instant), String> {
    let s = stream(Direction::Record, mic)?;
    let mut recording = Vec::new();
    let mut buf = vec![0u8; CHUNK * BYTES];
    let mut first = None;
    while !stop.load(Ordering::Relaxed) {
        s.read(&mut buf).map_err(|e| e.to_string().unwrap_or_default())?;
        recording.extend(buf.chunks_exact(BYTES).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        // a read returns as soon as a chunk is recorded: its last sample is from about now.
        // Taken once the stream runs steadily, not from the first reads
        if first.is_none() && recording.len() >= RATE / 5 {
            first = Some(Instant::now() - Duration::from_secs_f64(recording.len() as f64 / RATE as f64));
        }
    }
    Ok((recording, first.ok_or_else(|| tr!("error_calibration_no_recording"))?))
}

/// Plays the sound; returns when its first sample was handed to the output the way the engine
/// hands it: into a buffer kept a few tens of milliseconds full
fn play(output: &str, sound: &[f32]) -> Result<Instant, String> {
    let s = stream(Direction::Playback, output)?;
    let mut handed = None;
    let mut at = 0;
    // after the sound, the buffer is let play out
    let total = sound.len() + RATE / 5;
    while at < total {
        let chunk: Vec<u8> = (at..at + CHUNK)
            .flat_map(|i| sound.get(i).copied().unwrap_or(0.0).to_le_bytes())
            .collect();
        s.write(&chunk).map_err(|e| e.to_string().unwrap_or_default())?;
        at += CHUNK;
        // a write returns once there is room: the buffer is full up to the chunk's end. Taken once
        // the stream runs steadily (the sound starts with silence)
        if handed.is_none() && at >= RATE / 5 {
            handed = Some(Instant::now() - Duration::from_secs_f64(at as f64 / RATE as f64));
        }
    }
    let _ = s.drain();
    handed.ok_or_else(|| tr!("error_calibration_no_recording"))
}

/// Plays sounds into several outputs at once, each from the same moment
pub fn play_together(outputs: &[(String, Vec<f32>)]) -> Result<(), String> {
    let threads: Vec<_> = outputs
        .iter()
        .cloned()
        .map(|(device, sound)| std::thread::spawn(move || play(&device, &sound).map(|_| ())))
        .collect();
    for t in threads {
        t.join().map_err(|_| tr!("error_stream_crashed"))??;
    }
    Ok(())
}
