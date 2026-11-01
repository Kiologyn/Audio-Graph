//! A test signal for delay calibration: played into an output device while a microphone records.

use super::super::link::{CHANNELS, RATE};
use super::streams::{describe, open};
use crate::com::ComGuard;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows::Win32::Media::Audio::{IAudioCaptureClient, IAudioRenderClient, AUDCLNT_BUFFERFLAGS_SILENT};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::WaitForSingleObject;

/// Now on the clock of WASAPI timestamps: hundreds of nanoseconds of the performance counter
fn now_hns() -> u64 {
    let (mut count, mut frequency) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut count);
        let _ = QueryPerformanceFrequency(&mut frequency);
    }
    (count as u128 * 10_000_000 / frequency.max(1) as u128) as u64
}

/// Plays sound (mono) into output while recording mic. Returns the recording (mono) and where in it,
/// in samples, the sound's start was handed to the output the way the engine hands it sound
pub fn play_and_record(output: &str, mic: &str, sound: &[f32]) -> Result<(Vec<f32>, f64), String> {
    let stop = Arc::new(AtomicBool::new(false));
    let recorder = {
        let (mic, stop) = (mic.to_string(), stop.clone());
        std::thread::spawn(move || record(&mic, &stop))
    };
    // the recording runs before the sound starts
    std::thread::sleep(Duration::from_millis(100));
    let played = play(output, sound);
    std::thread::sleep(Duration::from_millis(100));
    stop.store(true, Ordering::Relaxed);
    let (recording, first) = recorder.join().map_err(|_| tr!("error_stream_crashed"))??;
    let handed = played?;
    let at = (handed as f64 - first as f64) / 1e7 * RATE as f64;
    Ok((recording, at))
}

/// The recording and the time of its first sample
fn record(mic: &str, stop: &AtomicBool) -> Result<(Vec<f32>, u64), String> {
    let _com = ComGuard::init();
    let (client, event) = open(mic)?;
    let mut recording = Vec::new();
    let mut first = None;
    let result = unsafe {
        (|| -> windows::core::Result<()> {
            let capture: IAudioCaptureClient = client.GetService()?;
            client.Start()?;
            while !stop.load(Ordering::Relaxed) {
                WaitForSingleObject(event.0, 100);
                while capture.GetNextPacketSize()? > 0 {
                    let (mut data, mut frames, mut flags, mut time) = (std::ptr::null_mut(), 0u32, 0u32, 0u64);
                    capture.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut time))?;
                    first.get_or_insert(time);
                    let n = frames as usize;
                    if data.is_null() || flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                        recording.extend(std::iter::repeat_n(0.0, n));
                    } else {
                        let samples = std::slice::from_raw_parts(data as *const f32, n * CHANNELS);
                        recording.extend(samples.chunks_exact(CHANNELS).map(|f| f.iter().sum::<f32>() / CHANNELS as f32));
                    }
                    capture.ReleaseBuffer(frames)?;
                }
            }
            Ok(())
        })()
    };
    let _ = unsafe { client.Stop() };
    result.map_err(describe)?;
    Ok((recording, first.ok_or_else(|| tr!("error_calibration_no_recording"))?))
}

/// Plays the sound; returns when its first sample was handed to the device (see below)
fn play(output: &str, sound: &[f32]) -> Result<u64, String> {
    let _com = ComGuard::init();
    let (client, event) = open(output)?;
    let mut handed = 0;
    let result = unsafe {
        (|| -> windows::core::Result<()> {
            let render: IAudioRenderClient = client.GetService()?;
            let size = client.GetBufferSize()?;
            render.GetBuffer(size)?;
            render.ReleaseBuffer(size, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
            client.Start()?;
            let mut at = 0;
            // after the sound, the device buffer is let play out
            while at < sound.len() + RATE / 5 {
                WaitForSingleObject(event.0, 100);
                let frames = (size - client.GetCurrentPadding()?) as usize;
                if frames == 0 {
                    continue;
                }
                // the engine keeps the device buffer full: a sample waits a buffer's length after it is
                // written. The first write goes into an empty buffer, so it counts as written that much earlier
                if at == 0 {
                    handed = now_hns().saturating_sub(size as u64 * 10_000_000 / RATE as u64);
                }
                let data = render.GetBuffer(frames as u32)? as *mut f32;
                let out = std::slice::from_raw_parts_mut(data, frames * CHANNELS);
                for (i, frame) in out.chunks_exact_mut(CHANNELS).enumerate() {
                    frame.fill(sound.get(at + i).copied().unwrap_or(0.0));
                }
                render.ReleaseBuffer(frames as u32, 0)?;
                at += frames;
            }
            Ok(())
        })()
    };
    let _ = unsafe { client.Stop() };
    result.map_err(describe)?;
    Ok(handed)
}

/// Plays sounds into several outputs at once, each from the same moment
pub fn play_together(outputs: &[(String, Vec<f32>)]) -> Result<(), String> {
    let threads: Vec<_> = outputs
        .iter()
        .cloned()
        .map(|(device, sound)| std::thread::spawn(move || play(&device, &sound)))
        .collect();
    for t in threads {
        t.join().map_err(|_| tr!("error_stream_crashed"))??;
    }
    Ok(())
}
