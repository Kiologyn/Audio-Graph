//! A "recording device → output device" link: a buffer between their streams and latency
//! adjustment. Device clocks run slightly differently, so the buffer's reserve slowly
//! grows or shrinks; it is kept near the target by dropping or repeating one frame at a time.
//! An output's delay is a bigger reserve: the sound waits in the buffer that much longer.

use super::plan::{Gain, MAX_DELAY};
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
    // the longest delay and a second for everything else
    let (producer, consumer) = ring((RATE as f32 * (MAX_DELAY + 1.0)) as usize * CHANNELS);
    (
        producer,
        LinkReader {
            ring: consumer,
            primed: false,
            average: 0.0,
            gain: Arc::new([AtomicU32::new(1.0f32.to_bits()), AtomicU32::new(1.0f32.to_bits())]),
            current_gain: [1.0; CHANNELS],
            delay: Arc::new(AtomicU32::new(0)),
            applied_delay: 0,
        },
    )
}

pub struct LinkReader {
    ring: Consumer,
    /// The reserve is filled; after start and after running dry it is built up again first
    primed: bool,
    /// Average reserve over the last ~half second: the instant one jumps with the packets
    average: f32,
    /// The link's volume per channel; the engine thread changes it on the fly, the output stream follows smoothly
    gain: Arc<[AtomicU32; CHANNELS]>,
    current_gain: Gain,
    /// The delay in frames; the engine thread changes it, the output stream applies it at the next packet
    delay: Arc<AtomicU32>,
    applied_delay: usize,
}

impl LinkReader {
    pub fn set_gain(&self, gain: Gain) {
        for (g, v) in self.gain.iter().zip(gain) {
            g.store(v.to_bits(), Ordering::Relaxed);
        }
    }

    /// The output's delay in seconds, at most plan::MAX_DELAY
    pub fn set_delay(&self, seconds: f32) {
        let frames = (seconds.clamp(0.0, MAX_DELAY) * RATE as f32).round() as u32;
        self.delay.store(frames, Ordering::Relaxed);
    }

    /// Mixes the link's sound into out (frames of CHANNELS values)
    pub fn mix_into(&mut self, out: &mut [f32]) {
        let to: Gain = std::array::from_fn(|ch| f32::from_bits(self.gain[ch].load(Ordering::Relaxed)));
        let from = self.current_gain;
        self.current_gain = to;
        self.mix_frames(out, from, to);
    }

    fn mix_frames(&mut self, out: &mut [f32], from: Gain, to: Gain) {
        let need = out.len() / CHANNELS;
        let delay = self.delay.load(Ordering::Relaxed) as usize;
        if delay != self.applied_delay {
            if delay < self.applied_delay {
                // earlier: the difference is dropped at once
                let excess = (self.applied_delay - delay).min(self.ring.len() / CHANNELS);
                self.ring.skip(excess * CHANNELS);
            } else {
                // later: silence until the bigger reserve has gathered
                self.primed = false;
            }
            self.applied_delay = delay;
            // the clock drift average starts over from the new reserve
            self.average = (self.ring.len() / CHANNELS) as f32;
        }
        let target = TARGET + delay;
        let mut available = self.ring.len() / CHANNELS;
        if !self.primed {
            if available < target {
                return;
            }
            self.primed = true;
            self.average = available as f32;
        }
        if available > target + (MAX - TARGET) {
            self.ring.skip((available - target) * CHANNELS);
            available = target;
            self.average = target as f32;
        }
        self.average += (available as f32 - self.average) * 0.02;

        if available < need {
            // recording lags: give what there is and build the reserve again
            self.ring.mix_into_scaled(&mut out[..available * CHANNELS], from, to);
            self.primed = false;
            return;
        }
        if self.average > (target + SLACK) as f32 && available > need {
            // recording runs fast: drop one frame
            self.ring.skip(CHANNELS);
        } else if self.average < (target - SLACK) as f32 && need > 1 {
            // recording runs slow: repeat the last frame
            let last: [f32; CHANNELS] = std::array::from_fn(|ch| self.ring.peek((need - 2) * CHANNELS + ch));
            let n = (need - 1) * CHANNELS;
            self.ring.mix_into_scaled(&mut out[..n], from, to);
            for ((o, s), g) in out[n..].iter_mut().zip(last).zip(to) {
                *o += s * g;
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

    /// How many frames of silence come out before the sound, packets of 10 ms in and out
    fn frames_before_sound(reader: &mut LinkReader, producer: &Producer) -> usize {
        let mut out = vec![0.0f32; 480 * CHANNELS];
        for packet in 0..1000 {
            producer.push(&vec![0.5f32; 480 * CHANNELS]);
            out.fill(0.0);
            reader.mix_into(&mut out);
            if let Some(i) = out.iter().position(|&s| s != 0.0) {
                return packet * 480 + i / CHANNELS;
            }
        }
        panic!("no sound");
    }

    #[test]
    fn delay_holds_sound_back() {
        let (producer, mut plain) = link();
        let undelayed = frames_before_sound(&mut plain, &producer);
        let (producer, mut reader) = link();
        reader.set_delay(0.25);
        // the sound comes 250 ms later than through a link without a delay
        assert_eq!(frames_before_sound(&mut reader, &producer) - undelayed, ms(250));
        // a steady stream keeps the reserve at the target plus the delay (measured after a read)
        let mut out = vec![0.0f32; 480 * CHANNELS];
        for _ in 0..300 {
            producer.push(&vec![0.5f32; 480 * CHANNELS]);
            reader.mix_into(&mut out);
        }
        assert_eq!(reader.ring.len() / CHANNELS + 480, TARGET + ms(250));
    }

    #[test]
    fn delay_changes_on_the_fly() {
        let (producer, mut reader) = link();
        let mut out = vec![0.0f32; 480 * CHANNELS];
        let mut run = |reader: &mut LinkReader, packets: usize| {
            let mut silent = 0;
            for _ in 0..packets {
                producer.push(&vec![0.5f32; 480 * CHANNELS]);
                out.fill(0.0);
                reader.mix_into(&mut out);
                silent += out.iter().filter(|&&s| s == 0.0).count() / CHANNELS;
            }
            silent
        };
        run(&mut reader, 100);
        let reserve = reader.ring.len() / CHANNELS;
        // later by 100 ms: about that much silence once (in whole 10 ms packets), then the reserve holds it
        reader.set_delay(0.1);
        let silent = run(&mut reader, 100);
        assert!(silent.abs_diff(ms(100)) <= 480, "{silent}");
        assert_eq!(reader.ring.len() / CHANNELS, reserve + ms(100));
        // back to no delay: the excess is dropped, no silence
        reader.set_delay(0.0);
        assert_eq!(run(&mut reader, 100), 0);
        assert_eq!(reader.ring.len() / CHANNELS, reserve);
    }

    #[test]
    fn balance_quiets_one_channel() {
        let (producer, mut reader) = link();
        reader.set_gain([1.0, 0.25]);
        producer.push(&vec![0.5f32; TARGET * 4 * CHANNELS]);
        let mut out = vec![0.0f32; 480 * CHANNELS];
        reader.mix_into(&mut out);
        // the right channel ramps down over the first packet, then stays a quarter of the left
        out.fill(0.0);
        reader.mix_into(&mut out);
        assert!(out.chunks(2).all(|f| f[0] == 0.5 && (f[1] - 0.125).abs() < 1e-6));
    }

    #[test]
    fn gain_ramps_without_jump() {
        let (producer, mut reader) = link();
        producer.push(&vec![0.5f32; TARGET * 4 * CHANNELS]);
        let mut out = vec![0.0f32; 480 * CHANNELS];
        reader.mix_into(&mut out);
        assert!(out.iter().all(|&s| s == 0.5));
        // volume 50%: smoothly from 0.5 to 0.25 over one packet, then a steady 0.25
        reader.set_gain([0.5; CHANNELS]);
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
