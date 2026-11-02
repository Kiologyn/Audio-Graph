//! Delay calibration: each output plays a short sweep, a microphone hears it, and the time between
//! handing the sweep to the output and hearing it is the output's latency. The microphone's own
//! latency is the same for all outputs, so the differences between outputs are what the delays need.

#[cfg(target_os = "linux")]
mod linux;

use crate::devices::{self, Flow};
use crate::graphs::DeviceRef;
use crate::storage::{err, CmdResult};
use serde::{Deserialize, Serialize};

pub const RATE: usize = 48_000;

/// Silence before the sweep: the streams settle first
const LEAD: f32 = 0.4;
/// Silence after it: the longest latency that can be measured
const TAIL: f32 = 1.2;
/// Measurements per output; the median counts
const TRIES: usize = 3;
/// How far the correlation peak must stand out from the rest to count as heard
const CLEAR: f32 = 8.0;
/// Samples averaged into one for the coarse search
const STEP: usize = 8;

#[cfg(windows)]
use crate::engine::probe::{play_and_record, play_together};
#[cfg(target_os = "linux")]
use linux::{play_and_record, play_together};

#[cfg(not(any(windows, target_os = "linux")))]
fn play_and_record(_: &str, _: &str, _: &[f32]) -> Result<(Vec<f32>, f64), String> {
    Err("not supported".into())
}
#[cfg(not(any(windows, target_os = "linux")))]
fn play_together(_: &[(String, Vec<f32>)]) -> Result<(), String> {
    Err("not supported".into())
}

fn silence(seconds: f32) -> Vec<f32> {
    vec![0.0; (seconds * RATE as f32) as usize]
}

/// A tone gliding from 200 to 2000 Hz in 0.3 s, faded in and out: a sharp correlation peak,
/// and it survives the coarse search's averaging (frequencies well below RATE / STEP / 2)
pub fn sweep() -> Vec<f32> {
    let n = RATE * 3 / 10;
    let (f0, f1) = (200.0f32, 2000.0f32);
    let duration = n as f32 / RATE as f32;
    let fade = RATE / 100;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let phase = std::f32::consts::TAU * (f0 * t + (f1 - f0) * t * t / (2.0 * duration));
            let edge = (i.min(n - 1 - i) as f32 / fade as f32).min(1.0);
            0.5 * edge * phase.sin()
        })
        .collect()
}

/// A short tick for listening whether outputs play together: 20 ms of 1 kHz
fn tick() -> Vec<f32> {
    let n = RATE / 50;
    (0..n)
        .map(|i| {
            let edge = (i.min(n - 1 - i) as f32 / (RATE / 1000) as f32).min(1.0);
            0.6 * edge * (std::f32::consts::TAU * 1000.0 * i as f32 / RATE as f32).sin()
        })
        .collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Where the signal starts in the recording and how clearly it stands out; None if it isn't there.
/// First a coarse search over averaged samples, then the exact sample near the coarse peak
pub fn find(recording: &[f32], signal: &[f32]) -> Option<(usize, f32)> {
    let coarse = |x: &[f32]| x.chunks(STEP).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect::<Vec<_>>();
    let (s, r) = (coarse(signal), coarse(recording));
    if r.len() <= s.len() {
        return None;
    }
    // a microphone may invert the sound: the peak's sign doesn't matter
    let correlation: Vec<f32> = (0..=r.len() - s.len()).map(|k| dot(&s, &r[k..]).abs()).collect();
    let (peak, &height) = correlation.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    let rms = (correlation.iter().map(|c| c * c).sum::<f32>() / correlation.len() as f32).sqrt();
    let clarity = if rms > 0.0 { height / rms } else { 0.0 };
    if clarity < CLEAR {
        return None;
    }
    let from = (peak * STEP).saturating_sub(2 * STEP);
    let to = (peak * STEP + 2 * STEP).min(recording.len().saturating_sub(signal.len()));
    let exact = (from..=to).max_by(|&a, &b| dot(signal, &recording[a..]).abs().total_cmp(&dot(signal, &recording[b..]).abs()))?;
    Some((exact, clarity))
}

/// The id of a device as the system has it now: "default" is the current default
fn device_id(device: &DeviceRef, flow: Flow) -> CmdResult<String> {
    match device {
        DeviceRef::Default => {
            let (render, capture) = devices::defaults();
            match flow {
                Flow::Render => render,
                Flow::Capture => capture,
            }
            .ok_or_else(|| tr!("error_calibration_no_default"))
        }
        DeviceRef::Device { id, name } => {
            let list = devices::list().unwrap_or_default();
            Ok(devices::find(&list, flow, id, name).map_or_else(|| id.clone(), |d| d.id.clone()))
        }
    }
}

/// One measurement: seconds from handing the sweep to the output until the microphone hears it
fn measure_once(output: &str, mic: &str) -> CmdResult<f32> {
    latency(|sound| play_and_record(output, mic, sound))
}

/// The latency of one measurement. play plays the sound and records meanwhile; it returns the recording
/// and where in it, in samples, the sound's start was handed to the output
fn latency(play: impl FnOnce(&[f32]) -> Result<(Vec<f32>, f64), String>) -> CmdResult<f32> {
    let signal = sweep();
    let sound: Vec<f32> = [silence(LEAD), signal.clone(), silence(TAIL)].concat();
    let (recording, at) = play(&sound)?;
    let (onset, _) = find(&recording, &signal).ok_or_else(|| tr!("error_calibration_not_heard"))?;
    let expected = at + (LEAD * RATE as f32) as f64;
    Ok(((onset as f64 - expected) / RATE as f64) as f32)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    /// Seconds from the engine handing sound to the output until the microphone hears it (the median)
    pub latency: f32,
    /// How far the tries differ, in seconds: a big spread means an unreliable measurement
    pub spread: f32,
}

/// Measures an output's latency with a microphone
#[tauri::command]
pub async fn calibrate_measure(output: DeviceRef, mic: DeviceRef) -> CmdResult<Measurement> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(windows)]
        let _com = crate::com::ComGuard::init();
        let (output, mic) = (device_id(&output, Flow::Render)?, device_id(&mic, Flow::Capture)?);
        let mut tries = (0..TRIES).map(|_| measure_once(&output, &mic)).collect::<CmdResult<Vec<f32>>>()?;
        tries.sort_by(f32::total_cmp);
        Ok(Measurement {
            latency: tries[TRIES / 2],
            spread: tries[TRIES - 1] - tries[0],
        })
    })
    .await
    .map_err(err)?
}

#[derive(Deserialize)]
pub struct PreviewOutput {
    device: DeviceRef,
    /// Seconds the output's tick waits
    delay: f32,
}

/// Ticks on several outputs at once, each later by its delay: played together they sound as one
#[tauri::command]
pub async fn calibrate_preview(outputs: Vec<PreviewOutput>) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(windows)]
        let _com = crate::com::ComGuard::init();
        let sounds = outputs
            .iter()
            .map(|o| {
                let delay = o.delay.clamp(0.0, 2.0);
                let sound = [silence(0.2 + delay), tick(), silence(0.3)].concat();
                device_id(&o.device, Flow::Render).map(|id| (id, sound))
            })
            .collect::<CmdResult<Vec<_>>>()?;
        play_together(&sounds)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Noise that is the same on every run
    fn noise(n: usize, level: f32) -> Vec<f32> {
        let mut x = 12345u32;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                ((x >> 16) as f32 / 32768.0 - 1.0) * level
            })
            .collect()
    }

    #[test]
    fn finds_the_sweep_in_noise() {
        let signal = sweep();
        let mut recording = noise(RATE * 2, 0.05);
        // quieter, inverted, with an echo 30 ms later
        for (i, s) in signal.iter().enumerate() {
            recording[12_345 + i] -= 0.2 * s;
            recording[12_345 + RATE * 3 / 100 + i] -= 0.05 * s;
        }
        let (at, clarity) = find(&recording, &signal).expect("not found");
        println!("found at {at}, clarity {clarity:.1}");
        assert_eq!(at, 12_345);
    }

    #[test]
    fn noise_alone_isnt_heard() {
        assert_eq!(find(&noise(RATE * 2, 0.2), &sweep()), None);
    }

    #[test]
    fn sweep_is_found_clipped_and_off_center() {
        let signal = sweep();
        // a microphone overloaded to clipping, with a constant offset
        let mut recording = vec![0.3; RATE];
        for (i, s) in signal.iter().enumerate() {
            recording[777 + i] += (4.0 * s).clamp(-1.0, 1.0);
        }
        assert_eq!(find(&recording, &signal).map(|f| f.0), Some(777));
    }

    #[test]
    fn sweep_is_found_at_the_edges_of_the_recording() {
        let signal = sweep();
        let at_start: Vec<f32> = [signal.clone(), silence(0.5)].concat();
        assert_eq!(find(&at_start, &signal).map(|f| f.0), Some(0));
        let at_end: Vec<f32> = [silence(0.5), signal.clone()].concat();
        assert_eq!(find(&at_end, &signal).map(|f| f.0), Some(RATE / 2));
        // a recording too short to hold it
        assert_eq!(find(&signal[..1000], &signal), None);
    }

    /// A pretend output and microphone: the recording started `before` seconds before the sound was handed
    /// to the output (negative: after it), and the microphone hears the sound `latency` seconds after that
    fn simulated(before: f64, latency: f64) -> impl FnOnce(&[f32]) -> Result<(Vec<f32>, f64), String> {
        move |sound: &[f32]| {
            let at = before * RATE as f64;
            let shift = (at + latency * RATE as f64).round() as isize;
            let mut recording = noise(sound.len() + RATE, 0.02);
            for (i, s) in sound.iter().enumerate() {
                if let Some(r) = recording.get_mut((i as isize + shift) as usize) {
                    *r += 0.3 * s;
                }
            }
            Ok((recording, at))
        }
    }

    #[test]
    fn latency_is_from_handing_over_to_hearing() {
        for (before, expected) in [(0.25, 0.037), (0.0, 0.180), (-0.1, 0.250)] {
            let measured = latency(simulated(before, expected)).unwrap();
            assert!((measured as f64 - expected).abs() < 0.0005, "{before}: {measured} instead of {expected}");
        }
        // a microphone that hears nothing of it
        let deaf = |sound: &[f32]| Ok((noise(sound.len() + RATE, 0.02), 0.0));
        assert!(latency(deaf).is_err());
        // a failing device is reported as it is
        assert_eq!(latency(|_: &[f32]| Err("device busy".to_string())), Err("device busy".to_string()));
    }

    /// Two outputs heard by one "microphone" (a mix sink's monitor), one of them 200 ms later through
    /// a loopback: the measured latencies differ by that much. Needs a test audio server (PULSE_SERVER):
    /// cargo test measures_two_outputs -- --ignored --nocapture
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn measures_two_outputs() {
        use std::process::Command;
        let pactl = |args: &[&str]| String::from_utf8(Command::new("pactl").args(args).output().unwrap().stdout).unwrap();
        let mut modules = Vec::new();
        for sink in ["agcal.mix", "agcal.a", "agcal.b"] {
            modules.push(pactl(&["load-module", "module-null-sink", &format!("sink_name={sink}")]));
        }
        for (sink, latency) in [("agcal.a", 30), ("agcal.b", 230)] {
            modules.push(pactl(&[
                "load-module",
                "module-loopback",
                &format!("source={sink}.monitor"),
                "sink=agcal.mix",
                &format!("latency_msec={latency}"),
            ]));
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
        let measure = |sink: &str| {
            let tries: Vec<f32> = (0..3).map(|_| measure_once(sink, "agcal.mix.monitor").unwrap()).collect();
            println!("{sink}: {tries:?}");
            tries[1]
        };
        let (a, b) = (measure("agcal.a"), measure("agcal.b"));
        for m in modules.iter().rev() {
            pactl(&["unload-module", m.trim()]);
        }
        println!("difference: {:.0} ms", (b - a) * 1000.0);
        assert!((b - a - 0.2).abs() < 0.03, "{a} {b}");
    }

    /// Plays the sweep into VAC Line 1 and records Line 1's output: the cable's latency.
    /// cargo test measures_vac -- --ignored --nocapture
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn measures_vac() {
        let _com = crate::com::ComGuard::init();
        let line = |flow: Flow| {
            devices::list()
                .unwrap()
                .into_iter()
                .find(|d| d.name.starts_with("Line 1 (Virtual Audio Cable") && d.flow == flow && d.state == devices::DeviceState::Active)
                .expect("VAC Line 1 must be on")
                .id
        };
        let tries: Vec<f32> = (0..3).map(|_| measure_once(&line(Flow::Render), &line(Flow::Capture)).unwrap()).collect();
        println!("Line 1 latency: {tries:?}");
        assert!(tries.iter().all(|t| (0.0..0.2).contains(t)));
        assert!(tries.iter().cloned().fold(0.0, f32::max) - tries.iter().cloned().fold(1.0, f32::min) < 0.005);
    }
}
