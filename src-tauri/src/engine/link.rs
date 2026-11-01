//! A "recording device → output device" link: a buffer between their streams and latency
//! adjustment. Device clocks run slightly differently, so the buffer's reserve slowly
//! grows or shrinks; it is kept near the target by dropping or repeating one frame at a time.

use super::ring::{ring, Consumer, Producer};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// The audio format inside the engine; Windows converts to each device's format itself
pub const CHANNELS: usize = 2;
pub const RATE: usize = 48_000;

const fn ms(ms: usize) -> usize {
    RATE * ms / 1000
}

/// Reserve in the link buffer, in frames: covers uneven recording packets
const TARGET: usize = ms(20);
/// How far the average reserve may drift from the target before adjustment starts
const SLACK: usize = ms(8);
/// If more has piled up (for example, the output device restarted), the excess is dropped at once
const MAX: usize = ms(120);

pub fn link() -> (Producer, LinkReader) {
    let (producer, consumer) = ring(RATE * CHANNELS);
    (
        producer,
        LinkReader {
            ring: consumer,
            primed: false,
            average: 0.0,
            gain: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            current_gain: 1.0,
        },
    )
}

pub struct LinkReader {
    ring: Consumer,
    /// The reserve is filled; after start and after running dry it is built up again first
    primed: bool,
    /// Average reserve over the last ~half second: the instant one jumps with the packets
    average: f32,
    /// The link's volume; the engine thread changes it on the fly, the output stream follows smoothly
    gain: Arc<AtomicU32>,
    current_gain: f32,
}

impl LinkReader {
    pub fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// Mixes the link's sound into out (frames of CHANNELS values)
    pub fn mix_into(&mut self, out: &mut [f32]) {
        let (from, to) = (self.current_gain, f32::from_bits(self.gain.load(Ordering::Relaxed)));
        self.current_gain = to;
        self.mix_frames(out, from, to);
    }

    fn mix_frames(&mut self, out: &mut [f32], from: f32, to: f32) {
        let need = out.len() / CHANNELS;
        let mut available = self.ring.len() / CHANNELS;
        if !self.primed {
            if available < TARGET {
                return;
            }
            self.primed = true;
            self.average = available as f32;
        }
        if available > MAX {
            self.ring.skip((available - TARGET) * CHANNELS);
            available = TARGET;
            self.average = TARGET as f32;
        }
        self.average += (available as f32 - self.average) * 0.02;

        if available < need {
            // recording lags: give what there is and build the reserve again
            self.ring.mix_into_scaled(&mut out[..available * CHANNELS], from, to);
            self.primed = false;
            return;
        }
        if self.average > (TARGET + SLACK) as f32 && available > need {
            // recording runs fast: drop one frame
            self.ring.skip(CHANNELS);
        } else if self.average < (TARGET - SLACK) as f32 && need > 1 {
            // recording runs slow: repeat the last frame
            let last: [f32; CHANNELS] = std::array::from_fn(|ch| self.ring.peek((need - 2) * CHANNELS + ch));
            let n = (need - 1) * CHANNELS;
            self.ring.mix_into_scaled(&mut out[..n], from, to);
            for (o, s) in out[n..].iter_mut().zip(last) {
                *o += s * to;
            }
            return;
        }
        self.ring.mix_into_scaled(out, from, to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recording and playback in 10 ms packets; every tenth recording packet is longer
    /// or shorter by drift frames (1 frame per 4800 is about 200 ppm, real clocks drift less)
    fn simulate(drift: isize, seconds: usize) -> (usize, usize) {
        let (producer, mut reader) = link();
        let mut out = vec![0.0f32; 480 * CHANNELS];
        let (mut silent, mut level) = (0, 0);
        for i in 0..seconds * 100 {
            let frames = if i % 10 == 0 { 480 + drift } else { 480 } as usize;
            producer.push(&vec![0.5f32; frames * CHANNELS]);
            level = reader.ring.len() / CHANNELS;
            out.fill(0.0);
            reader.mix_into(&mut out);
            if i > 100 {
                silent += out.iter().filter(|&&s| s == 0.0).count() / CHANNELS;
            }
        }
        (silent, level)
    }

    #[test]
    fn steady_without_gaps() {
        let (silent, level) = simulate(0, 10);
        assert_eq!(silent, 0);
        assert_eq!(level, TARGET);
    }

    #[test]
    fn follows_clock_drift() {
        // a minute without adjustment would drift by 6000 frames, yet the reserve stays at the target
        for drift in [1, -1, 5, -5] {
            let (silent, level) = simulate(drift, 60);
            assert_eq!(silent, 0, "drift {drift}");
            assert!(level.abs_diff(TARGET) <= SLACK + 50, "drift {drift}: reserve {level}");
        }
    }

    #[test]
    fn gain_ramps_without_jump() {
        let (producer, mut reader) = link();
        producer.push(&vec![0.5f32; TARGET * 4 * CHANNELS]);
        let mut out = vec![0.0f32; 480 * CHANNELS];
        reader.mix_into(&mut out);
        assert!(out.iter().all(|&s| s == 0.5));
        // volume 50%: smoothly from 0.5 to 0.25 over one packet, then a steady 0.25
        reader.set_gain(0.5);
        out.fill(0.0);
        reader.mix_into(&mut out);
        assert_eq!(out[0], 0.5);
        assert!((out[out.len() - 1] - 0.25).abs() < 1e-6);
        assert!(out.windows(2).all(|w| w[1] <= w[0]));
        out.fill(0.0);
        reader.mix_into(&mut out);
        assert!(out.iter().all(|&s| (s - 0.25).abs() < 1e-6));
    }
}
