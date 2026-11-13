//! What an output device plays: its links go into stages as the plan says, each stage sums what
//! comes to it, its noise gate listens to that sum, and it passes the sound on with its volume
//! from its place in space.
//! Everything runs in the device's stream, so nothing waits for other threads. A node on the way
//! to two devices has a stage in each: both hear the same sound and decide the same.

use super::levels::{Level, Levels};
use super::link::{LinkReader, CHANNELS, RATE};
use super::plan::{Gain, Stage, NO_GATE};
use super::spatial::Spatial;
use super::streams::Peer;
use std::sync::Arc;

/// A noise gate stays open this long after the sound fell below its level: pauses between words stay
const GATE_HOLD: usize = RATE / 4;
/// and then closes in this time, a sudden cut would click
const GATE_RELEASE: usize = RATE / 10;

/// A noise gate. It decides by a whole packet before the packet is passed on, so the start of a word isn't lost
struct Gate {
    /// As a sample value; 0 lets everything through
    level: f32,
    /// How open it is, from 0 to 1, and how many frames it stays so without sound
    open: f32,
    hold: usize,
}

impl Gate {
    fn set(&mut self, db: f32) {
        self.level = if db <= NO_GATE { 0.0 } else { 10f32.powf(db / 20.0) };
    }

    /// How open it is at the end of a packet of this many frames with this peak
    fn after(&mut self, peak: f32, frames: usize) -> f32 {
        if self.level == 0.0 {
            self.hold = 0;
            self.open = 1.0;
        } else if peak >= self.level {
            self.hold = GATE_HOLD;
            self.open = 1.0;
        } else if self.hold > 0 {
            self.hold = self.hold.saturating_sub(frames);
        } else {
            self.open = (self.open - frames as f32 / GATE_RELEASE as f32).max(0.0);
        }
        self.open
    }
}

struct Running {
    id: String,
    parent: usize,
    /// What came to the stage in this packet
    sound: Vec<f32>,
    gain: Gain,
    /// The volume at the end of the last packet, the gate included: the next one goes on from it
    played: Gain,
    gate: Gate,
    /// Only while the stage has a place in space, or is still leaving it
    spatial: Option<Box<Spatial>>,
    place: Option<[f32; 3]>,
    level: Option<Arc<Level>>,
}

impl Running {
    fn new(id: &str, gain: Gain) -> Self {
        Self {
            id: id.to_string(),
            parent: 0,
            sound: Vec::new(),
            gain,
            played: gain,
            gate: Gate { level: 0.0, open: 1.0, hold: 0 },
            spatial: None,
            place: None,
            level: None,
        }
    }

    fn set_place(&mut self, place: Option<[f32; 3]>) {
        if place == self.place {
            return;
        }
        self.place = place;
        if place.is_some() || self.spatial.is_some() {
            self.spatial.get_or_insert_with(Default::default).set(place);
        }
    }

    /// Adds the packet's sound to out, the volume going smoothly from the last packet's: a jump would click
    fn pass_into(&mut self, out: &mut [f32]) {
        let peak = self.sound.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        if let Some(level) = &self.level {
            level.heard(peak);
        }
        let frames = self.sound.len() / CHANNELS;
        let open = self.gate.after(peak, frames);
        if let Some(spatial) = &mut self.spatial {
            spatial.process(&mut self.sound);
            if spatial.idle() {
                self.spatial = None;
            }
        }
        let (from, to) = (self.played, self.gain.map(|g| g * open));
        self.played = to;
        if from == to {
            for (frame, sound) in out.chunks_exact_mut(CHANNELS).zip(self.sound.chunks_exact(CHANNELS)) {
                for ch in 0..CHANNELS {
                    frame[ch] += sound[ch] * to[ch];
                }
            }
            return;
        }
        let step: Gain = std::array::from_fn(|ch| if frames > 1 { (to[ch] - from[ch]) / (frames - 1) as f32 } else { 0.0 });
        for (i, (frame, sound)) in out.chunks_exact_mut(CHANNELS).zip(self.sound.chunks_exact(CHANNELS)).enumerate() {
            for ch in 0..CHANNELS {
                frame[ch] += sound[ch] * (from[ch] + step[ch] * i as f32);
            }
        }
    }
}

pub struct Mixer {
    pub links: Vec<(Peer, LinkReader)>,
    /// A stage comes after the one it goes into: the first goes into the device
    stages: Vec<Running>,
}

impl Default for Mixer {
    fn default() -> Self {
        Self { links: Vec::new(), stages: vec![Running::new("", [1.0; CHANNELS])] }
    }
}

impl Mixer {
    /// Takes the plan's stages; those that stay keep their volume and gate as they are, so
    /// a change doesn't reach the sound with a jump. Links must be told their new stages along with it
    pub fn set_stages(&mut self, plan: &[Stage], levels: &Levels) {
        let mut old = std::mem::take(&mut self.stages);
        self.stages = plan
            .iter()
            .map(|stage| {
                let kept = old.iter().position(|o| o.id == stage.id).map(|i| old.swap_remove(i));
                let mut running = kept.unwrap_or_else(|| Running::new(&stage.id, stage.gain));
                running.parent = stage.parent;
                running.gain = stage.gain;
                running.gate.set(stage.gate);
                running.set_place(stage.place);
                running.level = (!stage.node.is_empty()).then(|| levels.of(&stage.node));
                running
            })
            .collect();
    }

    /// Fills out (frames of CHANNELS values) with the packet to play
    pub fn play(&mut self, out: &mut [f32]) {
        for stage in &mut self.stages {
            stage.sound.clear();
            stage.sound.resize(out.len(), 0.0);
        }
        for (_, reader) in &mut self.links {
            reader.mix_into(&mut self.stages[reader.stage].sound);
        }
        for i in (1..self.stages.len()).rev() {
            let (before, rest) = self.stages.split_at_mut(i);
            let stage = &mut rest[0];
            stage.pass_into(&mut before[stage.parent].sound);
        }
        out.fill(0.0);
        self.stages[0].pass_into(out);
        // the sum of several sources can go out of range
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::link::link;
    use super::super::ring::Producer;
    use super::*;

    const PACKET: usize = 480;

    fn stage(id: &str, node: &str, parent: usize, gain: Gain, gate: f32) -> Stage {
        Stage { id: id.into(), node: node.into(), parent, gain, gate, place: None }
    }

    /// A mixer with these stages after the device's own, and a link into each of the given stages
    fn mixer(stages: &[Stage], into: &[usize]) -> (Mixer, Vec<Producer>, Levels) {
        let levels = Levels::default();
        let mut mixer = Mixer::default();
        let mut plan = vec![stage("", "", 0, [1.0; 2], NO_GATE)];
        plan.extend_from_slice(stages);
        mixer.set_stages(&plan, &levels);
        let mut producers = Vec::new();
        for (i, &stage) in into.iter().enumerate() {
            let (producer, mut reader) = link();
            reader.stage = stage;
            mixer.links.push(((i.to_string(), 0), reader));
            producers.push(producer);
        }
        (mixer, producers, levels)
    }

    /// Packets of these levels, one per link; returns the last packet's loudest value in each channel
    fn run(mixer: &mut Mixer, producers: &[Producer], values: &[f32], packets: usize) -> Gain {
        let mut out = vec![0.0f32; PACKET * CHANNELS];
        for _ in 0..packets {
            for (producer, &value) in producers.iter().zip(values) {
                producer.push(&vec![value; PACKET * CHANNELS]);
            }
            mixer.play(&mut out);
        }
        let channel = |ch: usize| out.iter().skip(ch).step_by(CHANNELS).fold(0.0f32, |a, s| a.max(s.abs()));
        [channel(0), channel(1)]
    }

    fn gated(db: f32) -> Stage {
        stage("/g", "g", 0, [1.0; 2], db)
    }

    #[test]
    fn a_gate_lets_through_only_what_is_louder() {
        let (mut mixer, links, _) = mixer(&[gated(-20.0)], &[1]);
        // 0.2 is about -14 dB, 0.01 is -40 dB
        assert!(run(&mut mixer, &links, &[0.2], 20)[0] > 0.19);
        assert_eq!(run(&mut mixer, &links, &[0.01], 60)[0], 0.0);
        assert!(run(&mut mixer, &links, &[0.2], 20)[0] > 0.19);
    }

    #[test]
    fn a_gate_hears_the_sum_of_what_comes_to_it() {
        // each is quieter than the gate, together they are louder
        let (mut mixer, links, _) = mixer(&[gated(-20.0)], &[1, 1]);
        run(&mut mixer, &links, &[0.06, 0.0], 60);
        assert_eq!(run(&mut mixer, &links, &[0.06, 0.0], 1)[0], 0.0);
        assert!(run(&mut mixer, &links, &[0.06, 0.06], 5)[0] > 0.11);
        // and a loud one opens the gate for a quiet one
        assert!(run(&mut mixer, &links, &[0.5, 0.01], 5)[0] > 0.5);
    }

    #[test]
    fn a_gate_opens_only_for_what_reaches_it() {
        // the volume of the stage before it counts: 0.2 at 10% is 0.02, below -20 dB
        let first = stage("/a", "a", 0, [1.0; 2], -20.0);
        let second = stage("/a/b", "b", 1, [0.1; 2], NO_GATE);
        let (mut mixer, links, _) = mixer(&[first, second], &[2]);
        run(&mut mixer, &links, &[0.2], 60);
        assert_eq!(run(&mut mixer, &links, &[0.2], 1)[0], 0.0);
    }

    #[test]
    fn a_gate_comes_before_its_own_volume() {
        // at 10% the sound comes out quieter than the gate, but it hears what came in
        let (mut mixer, links, _) = mixer(&[stage("/g", "g", 0, [0.1; 2], -20.0)], &[1]);
        let heard = run(&mut mixer, &links, &[0.2], 60)[0];
        assert!((heard - 0.02).abs() < 1e-6, "{heard}");
    }

    #[test]
    fn cables_in_a_row_each_do_their_part() {
        let first = stage("/a", "a", 0, [0.5; 2], NO_GATE);
        let second = stage("/a/b", "b", 1, [0.5; 2], NO_GATE);
        let (mut mixer, links, _) = mixer(&[first, second], &[2]);
        assert_eq!(run(&mut mixer, &links, &[0.8], 20)[0], 0.2);
    }

    #[test]
    fn a_gate_on_one_path_leaves_the_other_alone() {
        let (mut mixer, links, _) = mixer(&[gated(-20.0)], &[1, 0]);
        run(&mut mixer, &links, &[0.01, 0.01], 60);
        assert_eq!(run(&mut mixer, &links, &[0.01, 0.01], 1)[0], 0.01);
    }

    #[test]
    fn paths_add_up() {
        let (mut mixer, links, _) = mixer(&[gated(NO_GATE), stage("/h", "h", 0, [1.0; 2], NO_GATE)], &[1, 2]);
        assert_eq!(run(&mut mixer, &links, &[0.25, 0.25], 20)[0], 0.5);
    }

    #[test]
    fn a_gate_keeps_short_pauses() {
        let (mut mixer, links, _) = mixer(&[gated(-20.0)], &[1]);
        run(&mut mixer, &links, &[0.2], 20);
        // 200 ms of quiet sound is shorter than the hold
        assert!(run(&mut mixer, &links, &[0.01], 20)[0] > 0.009);
    }

    #[test]
    fn a_gate_closes_without_a_jump() {
        let (mut mixer, links, _) = mixer(&[gated(-20.0)], &[1]);
        run(&mut mixer, &links, &[0.2], 20);
        run(&mut mixer, &links, &[0.01], 28);
        // past the hold: every packet is quieter than the last, none drops all at once
        let mut before = 0.01f32;
        for _ in 0..12 {
            let now = run(&mut mixer, &links, &[0.01], 1)[0];
            assert!(now <= before && before - now < 0.0025, "{before} {now}");
            before = now;
        }
        assert_eq!(before, 0.0);
    }

    #[test]
    fn no_gate_lets_silence_and_quiet_sound_through() {
        let (mut mixer, links, levels) = mixer(&[gated(NO_GATE)], &[1]);
        assert!(run(&mut mixer, &links, &[0.00001], 60)[0] > 0.0);
        // turned off while closed, it opens again
        let root = stage("", "", 0, [1.0; 2], NO_GATE);
        mixer.set_stages(&[root.clone(), gated(-20.0)], &levels);
        run(&mut mixer, &links, &[0.00001], 60);
        mixer.set_stages(&[root, gated(NO_GATE)], &levels);
        assert!(run(&mut mixer, &links, &[0.00001], 3)[0] > 0.0);
    }

    #[test]
    fn a_stage_keeps_its_gate_when_the_plan_changes() {
        let (mut mixer, links, levels) = mixer(&[gated(-20.0)], &[1]);
        run(&mut mixer, &links, &[0.01], 60);
        // the same stage after another graph edit: still closed, no burst of what it muted
        mixer.set_stages(&[stage("", "", 0, [1.0; 2], NO_GATE), gated(-20.0)], &levels);
        assert_eq!(run(&mut mixer, &links, &[0.01], 1)[0], 0.0);
    }

    #[test]
    fn the_level_is_what_comes_in_however_it_goes_on() {
        // the gate is closed and the volume is low, the meter still shows what the gate decides by
        let (mut mixer, links, levels) = mixer(&[stage("/g", "g", 0, [0.1; 2], -20.0)], &[1, 1]);
        run(&mut mixer, &links, &[0.02, 0.03], 60);
        assert!((levels.take()["g"] - 0.05).abs() < 1e-6);
    }

    #[test]
    fn balance_quiets_one_channel() {
        let (mut mixer, links, _) = mixer(&[stage("/g", "g", 0, [1.0, 0.25], NO_GATE)], &[1]);
        assert_eq!(run(&mut mixer, &links, &[0.5], 20), [0.5, 0.125]);
    }

    #[test]
    fn gain_ramps_without_jump() {
        let (mut mixer, links, levels) = mixer(&[stage("/g", "g", 0, [1.0; 2], NO_GATE)], &[1]);
        run(&mut mixer, &links, &[0.5], 20);
        // volume 50%: smoothly from 0.5 to 0.25 over one packet, then a steady 0.25
        mixer.set_stages(&[stage("", "", 0, [1.0; 2], NO_GATE), stage("/g", "g", 0, [0.5; 2], NO_GATE)], &levels);
        let mut out = vec![0.0f32; PACKET * CHANNELS];
        links[0].push(&vec![0.5; PACKET * CHANNELS]);
        mixer.play(&mut out);
        assert_eq!(out[0], 0.5);
        assert!((out[out.len() - 1] - 0.25).abs() < 1e-6);
        assert!(out.windows(2).all(|w| w[1] <= w[0]));
        links[0].push(&vec![0.5; PACKET * CHANNELS]);
        mixer.play(&mut out);
        assert!(out.iter().all(|&s| (s - 0.25).abs() < 1e-6));
    }

    /// The two channels' loudest values while a steady sound goes through a stage with this place
    fn placed(place: Option<[f32; 3]>, gain: Gain) -> Gain {
        let (mut mixer, links, _) = mixer(&[Stage { place, ..stage("/g", "g", 0, gain, NO_GATE) }], &[1]);
        run(&mut mixer, &links, &[0.25], 4)
    }

    #[test]
    fn a_place_puts_the_sound_on_its_side() {
        // a head shades only sounds that change: a 2 kHz square wave
        let place = Some([1.0, 0.0, 0.0]);
        let (mut mixer, links, _) = mixer(&[Stage { place, ..stage("/g", "g", 0, [1.0; 2], NO_GATE) }], &[1]);
        let wave: Vec<f32> = (0..PACKET * CHANNELS).map(|i| if (i / CHANNELS / 12) % 2 == 0 { 0.25 } else { -0.25 }).collect();
        let mut out = vec![0.0f32; PACKET * CHANNELS];
        for _ in 0..4 {
            links[0].push(&wave);
            mixer.play(&mut out);
        }
        let energy = |ch: usize| out.iter().skip(ch).step_by(CHANNELS).map(|s| s * s).sum::<f32>();
        assert!(energy(1) > energy(0) * 2.0, "{} {}", energy(0), energy(1));
    }

    #[test]
    fn balance_comes_after_the_place() {
        // the source is on the right, and the balance takes the right channel off the result
        let [left, right] = placed(Some([1.0, 0.0, 0.0]), [1.0, 0.0]);
        assert!(left > 0.0 && right == 0.0, "{left} {right}");
    }

    #[test]
    fn a_gate_hears_the_sound_before_it_goes_to_its_place() {
        let place = Some([1.0, 0.0, 0.0]);
        let (mut mixer, links, levels) = mixer(&[Stage { place, ..stage("/g", "g", 0, [1.0; 2], -20.0) }], &[1]);
        run(&mut mixer, &links, &[0.11], 20);
        assert!((levels.take()["g"] - 0.11).abs() < 1e-6);
        assert!(run(&mut mixer, &links, &[0.11], 1)[1] > 0.0);
    }

    #[test]
    fn a_stage_leaves_its_place_when_told() {
        let root = stage("", "", 0, [1.0; 2], NO_GATE);
        let at = |place| Stage { place, ..stage("/g", "g", 0, [1.0; 2], NO_GATE) };
        let (mut mixer, links, levels) = mixer(&[at(Some([1.0, 0.0, 0.0]))], &[1]);
        run(&mut mixer, &links, &[0.25], 4);
        mixer.set_stages(&[root, at(None)], &levels);
        // one packet of fading, then the sound as it came
        assert_eq!(run(&mut mixer, &links, &[0.25], 2), [0.25, 0.25]);
        assert!(mixer.stages[1].spatial.is_none());
    }

    #[test]
    fn nothing_is_cut_on_the_way() {
        // two loud sounds into a stage at 25%: only what reaches the device has to fit
        let (mut mixer, links, _) = mixer(&[stage("/g", "g", 0, [0.25; 2], NO_GATE)], &[1, 1]);
        assert_eq!(run(&mut mixer, &links, &[1.0, 1.0], 20)[0], 0.5);
    }

    #[test]
    fn the_device_gets_what_it_can_play() {
        let (mut mixer, links, _) = mixer(&[], &[0, 0]);
        assert_eq!(run(&mut mixer, &links, &[1.0, 1.0], 20)[0], 1.0);
    }
}
