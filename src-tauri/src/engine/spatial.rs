//! A source in space around the listener, for headphones: what a stage plays becomes one point, and
//! each ear hears it through the filter a head puts on sound from that direction (see hrtf/README.md).
//! In front it plays as loud as the sound itself, so the volume set is what is heard; elsewhere the head
//! shades it a little, and no ear gets more. How far it is only makes it quieter, which is a plain volume:
//! the stage's gain carries it.

use super::link::CHANNELS;

const FILE: &[u8] = include_bytes!("../../hrtf/hrtf.bin");
const HEADER: usize = 12;

/// The length of a filter
const TAPS: usize = 128;
/// How much sound a filter still needs from before a packet
const HISTORY: usize = TAPS - 1;

/// Up to this distance in metres a source plays at its full volume: nothing gets louder than the sound itself
const NEAR: f32 = 1.0;

fn number(at: usize) -> i16 {
    i16::from_le_bytes([FILE[at], FILE[at + 1]])
}

fn directions() -> usize {
    u16::from_le_bytes([FILE[4], FILE[5]]) as usize
}

/// The direction in the file nearest to a place
fn nearest(place: [f32; 3]) -> usize {
    let [x, y, z] = place;
    (0..directions())
        .map(|i| {
            let at = HEADER + i * 6;
            (i, number(at) as f32 * x + number(at + 2) as f32 * y + number(at + 4) as f32 * z)
        })
        .fold((0, f32::MIN), |best, this| if this.1 > best.1 { this } else { best })
        .0
}

type Ears = [[f32; TAPS]; CHANNELS];

/// The two ears' filters of a direction
fn read(direction: usize, ears: &mut Ears) {
    let step = f32::from_le_bytes([FILE[8], FILE[9], FILE[10], FILE[11]]);
    let start = HEADER + directions() * 6 + direction * CHANNELS * TAPS * 2;
    for (ear, taps) in ears.iter_mut().enumerate() {
        for (i, tap) in taps.iter_mut().enumerate() {
            *tap = number(start + (ear * TAPS + i) * 2) as f32 * step;
        }
    }
}

/// How a place's distance changes the volume: half as loud for twice as far
pub fn distance_gain(place: [f32; 3]) -> f32 {
    let distance = place.iter().map(|v| v * v).sum::<f32>().sqrt();
    NEAR / distance.max(NEAR)
}

/// Writes a filter's work on heard into out. Heard starts HISTORY frames before the first frame of out
fn convolve(heard: &[f32], taps: &[f32; TAPS], out: &mut [f32]) {
    out.fill(0.0);
    for (back, &tap) in taps.iter().enumerate() {
        for (out, sound) in out.iter_mut().zip(&heard[HISTORY - back..]) {
            *out += tap * sound;
        }
    }
}

/// What a stage does to its sound to put it at a place. A change takes one packet: the sound
/// fades from the old direction into the new one, a jump between filters would click
pub struct Spatial {
    /// The direction asked for; none plays the sound as it came
    wanted: Option<usize>,
    /// The direction the sound is at now, and its filters
    now: Option<usize>,
    filter: Box<Ears>,
    /// The filters of the direction it is going to
    next: Box<Ears>,
    /// The sound as one point, from HISTORY frames back
    heard: Vec<f32>,
    /// A packet through the old and the new filters, ear by ear
    old: [Vec<f32>; CHANNELS],
    new: [Vec<f32>; CHANNELS],
}

impl Default for Spatial {
    fn default() -> Self {
        Self {
            wanted: None,
            now: None,
            filter: Box::new([[0.0; TAPS]; CHANNELS]),
            next: Box::new([[0.0; TAPS]; CHANNELS]),
            heard: vec![0.0; HISTORY],
            old: Default::default(),
            new: Default::default(),
        }
    }
}

impl Spatial {
    /// In metres from the listener: x to the right, y to the front, z up
    pub fn set(&mut self, place: Option<[f32; 3]>) {
        self.wanted = place.map(nearest);
        if self.wanted != self.now {
            if let Some(direction) = self.wanted {
                read(direction, &mut self.next);
            }
        }
    }

    /// Nothing to do, and nothing left of what it did
    pub fn idle(&self) -> bool {
        self.now.is_none() && self.wanted.is_none()
    }

    /// Turns a packet (frames of CHANNELS values) into what the ears get
    pub fn process(&mut self, sound: &mut [f32]) {
        if self.idle() {
            return;
        }
        let frames = sound.len() / CHANNELS;
        self.heard.truncate(HISTORY);
        self.heard.extend(sound.chunks_exact(CHANNELS).map(|frame| frame.iter().sum::<f32>() / CHANNELS as f32));

        let moved = self.wanted != self.now;
        for ear in 0..CHANNELS {
            self.old[ear].resize(frames, 0.0);
            self.new[ear].resize(frames, 0.0);
            if self.now.is_some() {
                convolve(&self.heard, &self.filter[ear], &mut self.old[ear]);
            }
            if moved && self.wanted.is_some() {
                convolve(&self.heard, &self.next[ear], &mut self.new[ear]);
            }
        }

        for (i, frame) in sound.chunks_exact_mut(CHANNELS).enumerate() {
            // how far the fade has come: the last frame of a packet is all new
            let along = if moved { (i + 1) as f32 / frames as f32 } else { 0.0 };
            for ear in 0..CHANNELS {
                let before = if self.now.is_some() { self.old[ear][i] } else { frame[ear] };
                let after = if self.wanted.is_some() { self.new[ear][i] } else { frame[ear] };
                frame[ear] = before + (after - before) * along;
            }
        }

        if moved {
            self.now = self.wanted;
            std::mem::swap(&mut self.filter, &mut self.next);
        }
        if self.now.is_some() {
            self.heard.drain(..frames);
        } else {
            // what it heard before going off must not come out of a filter later
            self.heard.clear();
            self.heard.resize(HISTORY, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48000.0;
    const PACKET: usize = 480;

    fn tone(packet: usize, hz: f32) -> Vec<f32> {
        (0..PACKET)
            .flat_map(|i| {
                let sample = (std::f32::consts::TAU * hz * (packet * PACKET + i) as f32 / RATE).sin() * 0.5;
                [sample; CHANNELS]
            })
            .collect()
    }

    /// A tone through a place; returns the two ears' levels in the last packet
    fn levels(place: Option<[f32; 3]>, hz: f32) -> [f32; 2] {
        let mut spatial = Spatial::default();
        spatial.set(place);
        let mut out = Vec::new();
        for packet in 0..10 {
            out = tone(packet, hz);
            spatial.process(&mut out);
        }
        [0, 1].map(|ear| (out.iter().skip(ear).step_by(CHANNELS).map(|s| s * s).sum::<f32>() / PACKET as f32).sqrt())
    }

    /// When a click reaches each ear, in frames
    fn arrival(place: [f32; 3]) -> [usize; 2] {
        let mut spatial = Spatial::default();
        spatial.set(Some(place));
        spatial.process(&mut vec![0.0; PACKET * CHANNELS]);
        let mut click = vec![0.0; PACKET * CHANNELS];
        click[0] = 1.0;
        click[1] = 1.0;
        spatial.process(&mut click);
        [0, 1].map(|ear| {
            let channel: Vec<f32> = click.iter().skip(ear).step_by(CHANNELS).map(|s| s.abs()).collect();
            let loudest = channel.iter().cloned().fold(0.0, f32::max);
            channel.iter().position(|&s| s >= loudest * 0.3).unwrap()
        })
    }

    #[test]
    fn the_file_is_what_the_code_reads() {
        assert_eq!(&FILE[..4], b"AGH1");
        assert_eq!(u16::from_le_bytes([FILE[6], FILE[7]]) as usize, TAPS);
        assert_eq!(FILE.len(), HEADER + directions() * (6 + CHANNELS * TAPS * 2));
    }

    #[test]
    fn the_nearest_direction_is_within_the_grid_step() {
        for place in [[0.3f32, 0.8, 0.1], [-0.9, -0.2, 0.4], [0.0, 0.0, -1.0], [0.01, 0.0, 1.0], [-0.5, 0.5, -0.7]] {
            let length = place.iter().map(|v| v * v).sum::<f32>().sqrt();
            let at = HEADER + nearest(place) * 6;
            let dot: f32 = (0..3).map(|i| number(at + i * 2) as f32 / 32767.0 * place[i] / length).sum();
            assert!(dot.min(1.0).acos().to_degrees() < 5.0, "{place:?} {dot}");
        }
    }

    #[test]
    fn a_source_on_the_right_is_louder_and_earlier_in_the_right_ear() {
        let [left, right] = levels(Some([1.0, 0.0, 0.0]), 2000.0);
        assert!(right > left * 2.0, "{left} {right}");
        let [left, right] = arrival([1.0, 0.0, 0.0]);
        // a head delays the far ear by more than a third of a millisecond
        assert!(left > right + 16, "{left} {right}");
    }

    #[test]
    fn a_source_on_the_left_is_the_mirror_of_it() {
        let [left, right] = levels(Some([-1.0, 0.0, 0.0]), 2000.0);
        assert!(left > right * 2.0, "{left} {right}");
        let [left, right] = arrival([-1.0, 0.0, 0.0]);
        assert!(right > left + 16, "{left} {right}");
    }

    #[test]
    fn a_source_straight_ahead_is_even_in_both_ears() {
        let [left, right] = levels(Some([0.0, 1.0, 0.0]), 1000.0);
        assert!((left / right - 1.0).abs() < 0.3, "{left} {right}");
    }

    /// An ear's level in dB for sound with as much in every octave as in the next, as music and speech about have
    fn level(taps: &[f32; TAPS]) -> f32 {
        const POINTS: usize = 64;
        let power: f32 = (0..POINTS)
            .map(|k| {
                let hz = 40.0 * (18000.0f32 / 40.0).powf(k as f32 / (POINTS - 1) as f32);
                let step = std::f32::consts::TAU * hz / RATE;
                let (re, im) = taps.iter().enumerate().fold((0.0, 0.0), |(re, im), (n, &tap)| {
                    (re + tap * (step * n as f32).cos(), im - tap * (step * n as f32).sin())
                });
                re * re + im * im
            })
            .sum();
        10.0 * (power / POINTS as f32).log10()
    }

    #[test]
    fn a_source_in_front_plays_as_loud_as_the_sound_itself() {
        let mut ears = [[0.0; TAPS]; CHANNELS];
        read(nearest([0.0, 1.0, 0.0]), &mut ears);
        let [left, right] = [level(&ears[0]), level(&ears[1])];
        assert!(left.max(right).abs() < 0.3, "{left} {right}");
        assert!(left.min(right) > -1.5, "{left} {right}");
    }

    #[test]
    fn no_ear_gets_more_than_the_sound_and_no_direction_is_much_quieter() {
        let mut ears = [[0.0; TAPS]; CHANNELS];
        let (mut most, mut least) = (f32::MIN, f32::MAX);
        for direction in 0..directions() {
            read(direction, &mut ears);
            let louder = level(&ears[0]).max(level(&ears[1]));
            (most, least) = (most.max(louder), least.min(louder));
        }
        assert!(most < 0.3, "{most}");
        assert!(least > -4.0, "{least}");
    }

    /// How much of a device's packet time a stage takes: cargo test how_long -- --ignored --nocapture
    #[test]
    #[ignore]
    fn how_long_a_packet_takes() {
        let mut spatial = Spatial::default();
        spatial.set(Some([1.0, 0.0, 0.0]));
        let mut sound = tone(0, 500.0);
        spatial.process(&mut sound);
        let started = std::time::Instant::now();
        for _ in 0..500 {
            spatial.process(&mut sound);
        }
        let steady = started.elapsed() / 500;
        let started = std::time::Instant::now();
        for i in 0..500 {
            spatial.set(Some(if i % 2 == 0 { [1.0, 0.0, 0.0] } else { [-1.0, 0.0, 0.0] }));
            spatial.process(&mut sound);
        }
        println!("a 10 ms packet takes {steady:?}, while moving {:?}", started.elapsed() / 500);
    }

    #[test]
    fn behind_sounds_different_from_ahead() {
        // the shells of the ears shade high sounds coming from behind
        let [front, _] = levels(Some([0.0, 1.0, 0.0]), 6000.0);
        let [back, _] = levels(Some([0.0, -1.0, 0.0]), 6000.0);
        assert!(back < front * 0.8, "{front} {back}");
    }

    #[test]
    fn above_sounds_different_from_below() {
        let [up, _] = levels(Some([0.0, 0.0, 1.0]), 6000.0);
        let [down, _] = levels(Some([0.0, 0.0, -1.0]), 6000.0);
        assert!((up / down - 1.0).abs() > 0.1, "{up} {down}");
    }

    #[test]
    fn a_stereo_source_becomes_one_point() {
        // the same sound lands at the same place from either channel
        let run = |channel: usize| {
            let mut spatial = Spatial::default();
            spatial.set(Some([1.0, 1.0, 0.0]));
            let mut out = Vec::new();
            for packet in 0..4 {
                out = tone(packet, 700.0).chunks(CHANNELS).flat_map(|f| if channel == 0 { [f[0], 0.0] } else { [0.0, f[0]] }).collect();
                spatial.process(&mut out);
            }
            out
        };
        assert_eq!(run(0), run(1));
    }

    #[test]
    fn nothing_changes_without_a_place() {
        let mut spatial = Spatial::default();
        spatial.set(None);
        let mut packet = tone(0, 500.0);
        let before = packet.clone();
        spatial.process(&mut packet);
        assert_eq!(packet, before);
        assert!(spatial.idle());
    }

    /// The largest step between samples of a steady tone while the place changes
    fn largest_step(change: impl Fn(usize) -> Option<[f32; 3]>) -> f32 {
        let mut spatial = Spatial::default();
        let mut last = [0.0f32; CHANNELS];
        let mut largest = 0.0f32;
        for packet in 0..60 {
            spatial.set(change(packet));
            let mut sound = tone(packet, 200.0);
            spatial.process(&mut sound);
            for frame in sound.chunks_exact(CHANNELS) {
                if packet > 3 {
                    largest = (0..CHANNELS).fold(largest, |a, ch| a.max((frame[ch] - last[ch]).abs()));
                }
                last = [frame[0], frame[1]];
            }
        }
        largest
    }

    #[test]
    fn moving_doesnt_click() {
        let still = largest_step(|_| Some([1.0, 0.0, 0.0]));
        let jumping = largest_step(|packet| Some(if (packet / 5) % 2 == 0 { [1.0, 0.0, 0.0] } else { [-1.0, 0.0, 0.0] }));
        assert!(jumping < still * 1.5, "{still} {jumping}");
    }

    #[test]
    fn turning_on_and_off_doesnt_click() {
        let still = largest_step(|_| None);
        let toggling = largest_step(|packet| ((packet / 5) % 2 == 0).then_some([0.0, 1.0, 0.0]));
        assert!(toggling < still * 2.0, "{still} {toggling}");
    }

    #[test]
    fn nothing_old_comes_back_after_being_off() {
        let mut spatial = Spatial::default();
        spatial.set(Some([1.0, 0.0, 0.0]));
        spatial.process(&mut tone(0, 500.0));
        spatial.set(None);
        spatial.process(&mut tone(1, 500.0));
        assert!(spatial.idle());
        spatial.set(Some([1.0, 0.0, 0.0]));
        let mut silence = vec![0.0; PACKET * CHANNELS];
        spatial.process(&mut silence);
        assert!(silence.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn distance_only_quiets_beyond_a_metre() {
        assert_eq!(distance_gain([0.3, 0.1, 0.0]), 1.0);
        assert_eq!(distance_gain([0.0, 1.0, 0.0]), 1.0);
        assert_eq!(distance_gain([0.0, 2.0, 0.0]), 0.5);
        assert_eq!(distance_gain([0.0, -6.0, 8.0]), 0.1);
    }
}
