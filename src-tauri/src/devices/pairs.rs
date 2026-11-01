//! Finds the two devices of one virtual cable by the properties the driver gives them,
//! without playing anything. Windows doesn't say which output device a recording device
//! repeats: this lives inside the driver, so the pair is guessed from what drivers share:
//! - both sides belong to one software driver device (a real sound card is never a cable);
//! - the driver filter names differ only by the side: Virtual Audio Cable has
//!   "wave1_r_rt" (output) and "wave1_c_rt" (recording) for Line 1;
//! - the names match without the side words: "Line 1" and "Line 1" (VAC),
//!   "CABLE Input" and "CABLE Output" (VB-Cable).
//!
//! A device gets a pair only when one candidate is better than all others.

/// What the driver tells about a device
#[derive(Clone, Debug, Default)]
pub struct Endpoint {
    pub id: String,
    pub render: bool,
    /// The device's own name without the driver name: "Line 1"
    pub name: String,
    /// The driver device it belongs to
    pub driver: String,
    /// The bus the driver device is on: "ROOT" or "SWD" for software devices
    pub enumerator: String,
    /// The driver filter the device is connected to: "wave1_r_rt"
    pub filter: String,
}

/// A found cable: the output device (its input) and the recording device (its output)
#[derive(Clone, Debug, PartialEq)]
pub struct Pair {
    pub render: String,
    pub capture: String,
    /// The name both sides share: "Line 1", "CABLE"
    pub name: String,
}

/// Words naming a side; a cable's input is an output device, so either word can be on either side
const SIDE_WORDS: &[&str] = &["input", "output", "in", "out", "playback", "recording", "record"];

/// Filter name parts naming a side: (output device, recording device)
const SIDE_TOKENS: &[(&str, &str)] = &[("r", "c"), ("render", "capture"), ("play", "rec"), ("out", "in")];

fn is_software(e: &Endpoint) -> bool {
    matches!(e.enumerator.to_ascii_uppercase().as_str(), "ROOT" | "SWD")
}

/// The name without side words: "CABLE Input" → "CABLE"
fn base_name(name: &str) -> String {
    name.split_whitespace()
        .filter(|w| !SIDE_WORDS.contains(&w.to_lowercase().as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The filters differ only by one side part: "wave1_r_rt" and "wave1_c_rt"
fn filters_match(render: &str, capture: &str) -> bool {
    let split = |s: &str| s.to_lowercase().split(['_', '-', '.']).map(str::to_string).collect::<Vec<_>>();
    let (r, c) = (split(render), split(capture));
    if r.len() != c.len() || render.is_empty() {
        return false;
    }
    let differ: Vec<(&String, &String)> = r.iter().zip(&c).filter(|(a, b)| a != b).collect();
    matches!(differ.as_slice(), [(a, b)] if SIDE_TOKENS.contains(&(a.as_str(), b.as_str())))
}

/// How much a recording device looks like the other side of an output device; 0: not at all
fn score(render: &Endpoint, capture: &Endpoint) -> u32 {
    if render.driver.is_empty() || render.driver != capture.driver {
        return 0;
    }
    let mut score = 0;
    if filters_match(&render.filter, &capture.filter) {
        score += 2;
    }
    let base = base_name(&render.name).to_lowercase();
    if !base.is_empty() && base == base_name(&capture.name).to_lowercase() {
        score += 2;
    }
    score
}

/// Pairs of output and recording devices that are two sides of one virtual cable
pub fn find(endpoints: &[Endpoint]) -> Vec<Pair> {
    let (renders, captures): (Vec<&Endpoint>, Vec<&Endpoint>) =
        endpoints.iter().filter(|e| is_software(e)).partition(|e| e.render);
    // the single best candidate of a device among the other side's devices
    fn best<'a>(of: &Endpoint, among: &[&'a Endpoint], score: impl Fn(&Endpoint, &Endpoint) -> u32) -> Option<&'a Endpoint> {
        let scored: Vec<(u32, &Endpoint)> = among.iter().map(|e| (score(of, e), *e)).filter(|(s, _)| *s > 0).collect();
        let top = scored.iter().map(|(s, _)| *s).max()?;
        match scored.iter().filter(|(s, _)| *s == top).collect::<Vec<_>>().as_slice() {
            [(_, e)] => Some(e),
            _ => None,
        }
    }
    renders
        .iter()
        .filter_map(|r| {
            let c = best(r, &captures, score)?;
            // the choice must be mutual, otherwise two outputs would claim one recording device
            let back = best(c, &renders, |c, r| score(r, c))?;
            (back.id == r.id).then(|| {
                let base = base_name(&r.name);
                Pair {
                    render: r.id.clone(),
                    capture: c.id.clone(),
                    name: if base.is_empty() { r.name.clone() } else { base },
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(id: &str, render: bool, name: &str, driver: &str, filter: &str) -> Endpoint {
        Endpoint {
            id: id.into(),
            render,
            name: name.into(),
            driver: driver.into(),
            enumerator: if driver.starts_with("USB") { "USB" } else { "ROOT" }.into(),
            filter: filter.into(),
        }
    }

    #[test]
    fn virtual_audio_cable_lines() {
        // as on a real machine: two lines on one driver device, Line 1 also has Mic 1 and S/PDIF 1 on its filter
        let vac = "ROOT\\{83ED7F0E}\\0000";
        let found = find(&[
            ep("r1", true, "Line 1", vac, "wave1_r_rt"),
            ep("r2", true, "Line 2", vac, "wave2_r_rt"),
            ep("c1", false, "Line 1", vac, "wave1_c_rt"),
            ep("m1", false, "Mic 1", vac, "wave1_c_rt"),
            ep("s1", false, "S/PDIF 1", vac, "wave1_c_rt"),
            ep("c2", false, "Line 2", vac, "wave2_c_rt"),
        ]);
        assert_eq!(
            found,
            vec![
                Pair { render: "r1".into(), capture: "c1".into(), name: "Line 1".into() },
                Pair { render: "r2".into(), capture: "c2".into(), name: "Line 2".into() },
            ]
        );
    }

    #[test]
    fn vb_cable_by_names() {
        let found = find(&[
            ep("in", true, "CABLE Input", "ROOT\\VBAudioVACWDM\\0000", "vbaudiovacwave"),
            ep("out", false, "CABLE Output", "ROOT\\VBAudioVACWDM\\0000", "vbaudiovacwave"),
        ]);
        assert_eq!(found, vec![Pair { render: "in".into(), capture: "out".into(), name: "CABLE".into() }]);
    }

    #[test]
    fn real_devices_and_strangers_are_not_cables() {
        let usb = "USB\\VID_0D8C&PID_033B&MI_01\\8&25B81DEB&0&0001";
        let found = find(&[
            // a USB headset: speakers and a microphone on one device
            ep("spk", true, "Speakers", usb, "spkout00wave"),
            ep("mic", false, "Microphone", usb, "micin00wave"),
            // software devices of different drivers, though with the same name
            ep("vs", true, "Virtual", "ROOT\\MEDIA\\0001", "audiorelaywave"),
            ep("vm", false, "Virtual", "ROOT\\MEDIA\\0000", "audiorelaywave"),
        ]);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn ambiguous_candidates_give_no_pair() {
        // one filter for everything and no telling names: can't choose
        let d = "ROOT\\X\\0000";
        let found = find(&[
            ep("r", true, "Out A", d, "wave_r"),
            ep("c1", false, "In B", d, "wave_c"),
            ep("c2", false, "In C", d, "wave_c"),
        ]);
        assert!(found.is_empty(), "{found:?}");
    }
}
