//! A "recording device → output device" link: a buffer between their streams and latency
//! adjustment. Device clocks run slightly differently, so the buffer's reserve slowly
//! grows or shrinks; it is kept near the target by playing a packet a frame longer or shorter.
//! A delay is a bigger reserve: the sound waits in the buffer that much longer, and the buffer
//! grows with it (Streams::apply), so a link only takes the memory its delay needs.

use super::plan::MAX_DELAY;
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

/// How many values a link's buffer holds for a delay: the delay and a second for everything else.
/// In steps of up to a third: dragging a delay slider must not move the buffer on every change
pub fn capacity_for(seconds: f32) -> usize {
    let needed = (seconds.max(0.0) + 1.0).ceil() as usize;
    let step = (needed.next_power_of_two() / 4).max(1);
    needed.div_ceil(step) * step * RATE * CHANNELS
}

/// A link without a delay
pub fn link() -> (Producer, LinkReader) {
    let (producer, consumer) = ring(capacity_for(0.0));
    (
        producer,
        LinkReader {
            ring: consumer,
            primed: false,
            average: 0.0,
            delay: Arc::new(AtomicU32::new(0)),
            applied_delay: 0,
            stage: 0,
        },
    )
}

pub struct LinkReader {
    ring: Consumer,
    /// The reserve is filled; after start and after running dry it is built up again first
    primed: bool,
    /// Average reserve over the last ~half second: the instant one jumps with the packets
    average: f32,
    /// The delay in frames; the engine thread changes it, the output stream applies it at the next packet
    delay: Arc<AtomicU32>,
    applied_delay: usize,
    /// The stage of the output device's mix it goes into
    pub stage: usize,
}

impl LinkReader {
    /// The link's delay in seconds, at most plan::MAX_DELAY
    pub fn set_delay(&self, seconds: f32) {
        let frames = (seconds.clamp(0.0, MAX_DELAY) * RATE as f32).round() as u32;
        self.delay.store(frames, Ordering::Relaxed);
    }

    pub fn capacity(&self) -> usize {
        self.ring.capacity()
    }

    /// Moves the link to a buffer made for this delay, and sets the delay: what the buffer holds
    /// goes along, so the sound continues. Neither stream may run meanwhile
    pub fn resize(&mut self, producer: &mut Producer, (writer, reader): (Producer, Consumer), delay: f32) {
        self.set_delay(delay);
        self.apply_delay();
        // a smaller buffer keeps the newest sound
        self.ring.skip(self.ring.len().saturating_sub(reader.capacity()));
        self.ring.move_to(&writer);
        *producer = writer;
        self.ring = reader;
    }

    /// Brings the reserve to a changed delay; returns the delay in frames
    fn apply_delay(&mut self) -> usize {
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
        delay
    }

    /// Mixes the link's sound into out (frames of CHANNELS values)
    pub fn mix_into(&mut self, out: &mut [f32]) {
        let need = out.len() / CHANNELS;
        let target = TARGET + self.apply_delay();
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
            self.ring.mix_into(&mut out[..available * CHANNELS]);
            self.primed = false;
            return;
        }
        if self.average > (target + SLACK) as f32 && available > need + 1 {
            // recording runs fast: the packet takes in a frame more
            self.mix_stretched(out, need + 1);
            return;
        } else if self.average < (target - SLACK) as f32 && need > 1 {
            // recording runs slow: a frame less
            self.mix_stretched(out, need - 1);
            return;
        }
        self.ring.mix_into(out);
    }

    /// Plays this many frames of the buffer as the whole packet: the correction is spread over it
    /// (a pitch change of a few cents at most), a frame dropped or repeated at once would click.
    /// The buffer must hold a frame more than that
    fn mix_stretched(&mut self, out: &mut [f32], frames: usize) {
        let need = out.len() / CHANNELS;
        let step = frames as f32 / need as f32;
        for (i, frame) in out.chunks_exact_mut(CHANNELS).enumerate() {
            let at = i as f32 * step;
            let (whole, part) = (at as usize * CHANNELS, at.fract());
            for (ch, o) in frame.iter_mut().enumerate() {
                let (a, b) = (self.ring.peek(whole + ch), self.ring.peek(whole + CHANNELS + ch));
                *o += a + (b - a) * part;
            }
        }
        self.ring.skip(frames * CHANNELS);
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

    /// The biggest jump between neighbouring frames of a 440 Hz sine that comes through a link
    /// while its clock drifts: a frame dropped or repeated at once would stand out
    #[test]
    fn drift_correction_doesnt_click() {
        for extra in [1isize, -1] {
            let (producer, mut reader) = link();
            let mut out = vec![0.0f32; 480 * CHANNELS];
            let mut phase = 0usize;
            let mut played: Vec<f32> = Vec::new();
            for i in 0..3000 {
                let frames = (480 + if i % 2 == 0 { extra } else { 0 }) as usize;
                let packet: Vec<f32> = (phase..phase + frames)
                    .flat_map(|n| [((n as f64 * 440.0 * std::f64::consts::TAU / RATE as f64).sin() * 0.5) as f32; CHANNELS])
                    .collect();
                phase += frames;
                producer.push(&packet);
                out.fill(0.0);
                reader.mix_into(&mut out);
                played.extend(out.iter().step_by(CHANNELS).copied());
            }
            // the sine's own curve: k x[n] = x[n - 1] + x[n + 1]; what is left of it are clicks
            let k = 2.0 * (440.0 * std::f32::consts::TAU / RATE as f32).cos();
            let click = played[9600..].windows(3).map(|w| (w[2] - k * w[1] + w[0]).abs()).fold(0.0f32, f32::max);
            // a frame dropped at once leaves about 0.03; the drift was corrected, or the reserve would be gone
            assert!(click < 0.002, "drift {extra}: {click}");
            assert!((reader.ring.len() / CHANNELS).abs_diff(TARGET) <= SLACK + 480, "drift {extra}");
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
    fn buffer_fits_the_delay_in_steps() {
        let seconds = |delay| capacity_for(delay) / (RATE * CHANNELS);
        assert_eq!([0.0, 0.5, 1.0, 1.5, 3.0, 7.5, 600.0].map(seconds), [1, 2, 2, 3, 4, 12, 768]);
    }

    /// A steady stream through a link, packets of 10 ms; returns the frames of silence
    fn run(producer: &Producer, reader: &mut LinkReader, packets: usize) -> usize {
        let mut out = vec![0.0f32; 480 * CHANNELS];
        let mut silent = 0;
        for _ in 0..packets {
            producer.push(&vec![0.5f32; 480 * CHANNELS]);
            out.fill(0.0);
            reader.mix_into(&mut out);
            silent += out.iter().filter(|&&s| s == 0.0).count() / CHANNELS;
        }
        silent
    }

    #[test]
    fn delay_longer_than_the_first_buffer_gathers() {
        let (mut producer, mut reader) = link();
        run(&producer, &mut reader, 100);
        let reserve = reader.ring.len() / CHANNELS;
        // 3 s don't fit the buffer of a link without a delay
        reader.resize(&mut producer, ring(capacity_for(3.0)), 3.0);
        assert_eq!(reader.ring.len() / CHANNELS, reserve);
        let silent = run(&producer, &mut reader, 500);
        assert!(silent.abs_diff(ms(3000)) <= 480, "{silent}");
        assert_eq!(reader.ring.len() / CHANNELS, reserve + ms(3000));
    }

    #[test]
    fn smaller_buffer_keeps_the_new_delay() {
        let (mut producer, mut reader) = link();
        reader.resize(&mut producer, ring(capacity_for(6.0)), 6.0);
        run(&producer, &mut reader, 700);
        let reserve = reader.ring.len() / CHANNELS;
        assert_eq!(reserve + 480, TARGET + ms(6000));
        // down to half a second: the sound goes on with that reserve, with no silence at all
        reader.resize(&mut producer, ring(capacity_for(0.5)), 0.5);
        assert_eq!(reader.capacity(), capacity_for(0.5));
        assert_eq!(reader.ring.len() / CHANNELS + ms(5500), reserve);
        assert_eq!(run(&producer, &mut reader, 100), 0);
    }
}
