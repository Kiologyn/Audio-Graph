//! Finds the two devices of one virtual cable by the properties the driver gives them,
//! without playing anything. Windows doesn't say which output device a recording device
//! repeats: this lives inside the driver, so the pair is guessed from what drivers share:
//! - both sides belong to one software driver device (a real sound card is never a cable);
//! - the driver filter names differ only by the side: Virtual Audio Cable has
//!   "wave1_r_rt" (output) and "wave1_c_rt" (recording) for Line 1;
//! - the names match without the side words: "Line 1" and "Line 1" (VAC),
//!   "CABLE Input" and "CABLE Output" (VB-Cable), and less so without channel counts:
//!   VB-Cable's second input "CABLE In 16ch" plays into the same "CABLE Output";
//! - a driver named a cable with a single recording device: all its output devices play into it,
//!   whatever the user renamed them to.
//!
//! Mixers aren't cables: Voicemeeter and VB-Audio Matrix number their inputs and outputs
//! in pairs ("Voicemeeter Input" and "Voicemeeter Out B1"), but sound goes from one to
//! the other only the way the mixer routes it, and only while the mixer runs.
//!
//! Every device gets the other side its driver points to, when one candidate is better than
//! all others. A cable's main sides point to each other; a cable can also have spare sides
//! that point to the main side but not back: VB-Cable's 16-channel input, the Mic and S/PDIF
//! recording devices of a VAC line next to its Line one.

use std::collections::HashMap;

/// What the driver tells about a device
#[derive(Clone, Debug, Default)]
pub struct Endpoint {
    pub id: String,
    pub render: bool,
    /// The device's own name without the driver name: "Line 1"
    pub name: String,
    /// The driver device it belongs to
    pub driver: String,
    /// The driver device's name: "Virtual Audio Cable", "VB-Audio Voicemeeter VAIO"
    pub driver_name: String,
    /// The hardware id the driver was installed for: "VBVoicemeeterVAIO"
    pub hardware: String,
    /// The bus the driver device is on: "ROOT" or "SWD" for software devices
    pub enumerator: String,
    /// The driver filter the device is connected to: "wave1_r_rt"
    pub filter: String,
}

/// The other side of a device's virtual cable: the recording device an output device plays into,
/// or the output device whose sound a recording device gives
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub device: String,
    pub other: String,
    /// The cable's name both sides share: "Line 1", "CABLE"
    pub name: String,
}

/// Words naming a side; a cable's input is an output device, so either word can be on either side
const SIDE_WORDS: &[&str] = &["input", "output", "in", "out", "playback", "recording", "record"];

/// Filter name parts naming a side: (output device, recording device)
const SIDE_TOKENS: &[(&str, &str)] = &[("r", "c"), ("render", "capture"), ("play", "rec"), ("out", "in")];

/// Source line types a VAC line can show as recording devices, all with the same sound
const LINE_WORDS: &[&str] = &["line", "mic", "microphone", "s/pdif", "spdif"];

fn is_software(e: &Endpoint) -> bool {
    matches!(e.enumerator.to_ascii_uppercase().as_str(), "ROOT" | "SWD")
}

fn has_word(text: &str, word: &str) -> bool {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|w| w == word)
}

/// Voicemeeter and VB-Audio Matrix (see above)
fn is_mixer(e: &Endpoint) -> bool {
    let (name, hardware) = (e.driver_name.to_lowercase(), e.hardware.to_lowercase());
    ["voicemeeter", "vbmatrix", "matrix vaio"]
        .iter()
        .any(|m| name.contains(m) || hardware.contains(m))
}

/// "Virtual Audio Cable", "VB-Audio Virtual Cable", "VB-Audio Hi-Fi Cable", "VB-Audio Cable A"
fn is_cable_driver(e: &Endpoint) -> bool {
    has_word(&e.driver_name, "cable")
}

/// The name without side words: "CABLE Input" → "CABLE"
fn base_name(name: &str) -> String {
    name.split_whitespace()
        .filter(|w| !SIDE_WORDS.contains(&w.to_lowercase().as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_number(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_ascii_digit())
}

/// The base name without channel counts, to compare: "CABLE In 16ch" → "cable"
fn without_channels(base: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for w in base.to_lowercase().split_whitespace() {
        if w.strip_suffix("ch").is_some_and(is_number) {
            continue;
        }
        // "16 ch"
        if w == "ch" && words.last().is_some_and(|p| is_number(p)) {
            words.pop();
            continue;
        }
        words.push(w.to_string());
    }
    words.join(" ")
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

/// How much a recording device looks like the other side of an output device; 0: not at all.
/// recorders: how many recording devices each driver device has
fn score(render: &Endpoint, capture: &Endpoint, recorders: &HashMap<&str, usize>) -> u32 {
    if render.driver.is_empty() || render.driver != capture.driver {
        return 0;
    }
    let mut score = 0;
    if filters_match(&render.filter, &capture.filter) {
        score += 2;
    }
    let (r, c) = (base_name(&render.name).to_lowercase(), base_name(&capture.name).to_lowercase());
    if !r.is_empty() && r == c {
        score += 3;
    } else if !without_channels(&r).is_empty() && without_channels(&r) == without_channels(&c) {
        score += 2;
    }
    if is_cable_driver(render) && recorders.get(render.driver.as_str()) == Some(&1) {
        score += 1;
    }
    score
}

/// Recording devices of one VAC line share its filter and give the same sound: "Line 1",
/// "Mic 1", "S/PDIF 1". Any of them is right, Line is the usual one
fn same_line<'a>(captures: &[&'a Endpoint]) -> Option<&'a Endpoint> {
    let line = |e: &Endpoint| -> String {
        e.name
            .split_whitespace()
            .filter(|w| !LINE_WORDS.contains(&w.to_lowercase().as_str()))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let first = captures.first()?;
    let same = !first.render
        && !first.filter.is_empty()
        && !line(first).is_empty()
        && captures.iter().all(|c| c.filter == first.filter && line(c) == line(first));
    if !same {
        return None;
    }
    captures
        .iter()
        .min_by_key(|c| (!c.name.to_lowercase().starts_with("line"), c.name.to_lowercase()))
        .copied()
}

/// The single best candidate; candidates with equal scores are told apart by `tie`
fn best<'a>(scored: Vec<(u32, &'a Endpoint)>, tie: impl Fn(&[&'a Endpoint]) -> Option<&'a Endpoint>) -> Option<&'a Endpoint> {
    let top = scored.iter().map(|(s, _)| *s).filter(|s| *s > 0).max()?;
    let tops: Vec<&Endpoint> = scored.iter().filter(|(s, _)| *s == top).map(|(_, e)| *e).collect();
    match tops.as_slice() {
        [one] => Some(one),
        many => tie(many),
    }
}

/// The cable's name: the recording device tells it best, as with VAC's output devices
/// that are all called "Speakers" while the recording ones are "Line 1", "Line 2"
fn cable_name(render: &Endpoint, capture: &Endpoint) -> String {
    [base_name(&capture.name), base_name(&render.name), render.name.clone()]
        .into_iter()
        .find(|n| !n.is_empty())
        .unwrap_or_default()
}

/// The other side of every device that is a side of a virtual cable
pub fn find(endpoints: &[Endpoint]) -> Vec<Link> {
    let usable: Vec<&Endpoint> = endpoints.iter().filter(|e| is_software(e) && !is_mixer(e)).collect();
    let mut recorders: HashMap<&str, usize> = HashMap::new();
    for e in usable.iter().filter(|e| !e.render) {
        *recorders.entry(e.driver.as_str()).or_default() += 1;
    }
    let (renders, captures): (Vec<&Endpoint>, Vec<&Endpoint>) = usable.into_iter().partition(|e| e.render);
    let mut plays_into: HashMap<&str, &Endpoint> = HashMap::new();
    let mut links = Vec::new();
    for r in &renders {
        let scored = captures.iter().map(|c| (score(r, c, &recorders), *c)).collect();
        if let Some(c) = best(scored, same_line) {
            plays_into.insert(r.id.as_str(), c);
            links.push(Link { device: r.id.clone(), other: c.id.clone(), name: cable_name(r, c) });
        }
    }
    for c in &captures {
        // inputs of one output mix together: which of them is the main one is unknown
        let scored = renders.iter().map(|r| (score(r, c, &recorders), *r)).collect();
        if let Some(r) = best(scored, |_| None) {
            // a spare recording device is called as the cable it belongs to: Mic 1 is on Line 1
            let name = cable_name(r, plays_into.get(r.id.as_str()).copied().unwrap_or(c));
            links.push(Link { device: c.id.clone(), other: r.id.clone(), name });
        }
    }
    links
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
            ..Default::default()
        }
    }

    fn named(driver_name: &str, e: Endpoint) -> Endpoint {
        Endpoint { driver_name: driver_name.into(), ..e }
    }

    /// The links as (device, other) to compare
    fn links(endpoints: &[Endpoint]) -> Vec<(String, String)> {
        find(endpoints).into_iter().map(|l| (l.device, l.other)).collect()
    }

    fn expect(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn virtual_audio_cable_lines() {
        // as on a real machine: two lines on one driver device, Line 1 also has Mic 1 and S/PDIF 1 on its filter
        let vac = "ROOT\\{83ED7F0E}\\0000";
        let endpoints = [
            ep("r1", true, "Line 1", vac, "wave1_r_rt"),
            ep("r2", true, "Line 2", vac, "wave2_r_rt"),
            ep("c1", false, "Line 1", vac, "wave1_c_rt"),
            ep("m1", false, "Mic 1", vac, "wave1_c_rt"),
            ep("s1", false, "S/PDIF 1", vac, "wave1_c_rt"),
            ep("c2", false, "Line 2", vac, "wave2_c_rt"),
        ];
        // the Mic and S/PDIF devices give Line 1's sound too, but Line 1 plays into its Line device
        assert_eq!(links(&endpoints), expect(&[("r1", "c1"), ("r2", "c2"), ("c1", "r1"), ("m1", "r1"), ("s1", "r1"), ("c2", "r2")]));
        let names: Vec<String> = find(&endpoints).into_iter().map(|l| l.name).collect();
        assert_eq!(names, ["Line 1", "Line 2", "Line 1", "Line 1", "Line 1", "Line 2"]);
    }

    #[test]
    fn virtual_audio_cable_speaker_pins() {
        // with the speaker pin type all output devices of VAC are called "Speakers": the filters tell the lines
        let vac = "ROOT\\{83ED7F0E}\\0000";
        let endpoints = [
            ep("r1", true, "Speakers", vac, "wave1_r_rt"),
            ep("r2", true, "Speakers", vac, "wave2_r_rt"),
            ep("s1", false, "S/PDIF 1", vac, "wave1_c_rt"),
            ep("m1", false, "Mic 1", vac, "wave1_c_rt"),
            ep("c1", false, "Line 1", vac, "wave1_c_rt"),
            ep("c2", false, "Line 2", vac, "wave2_c_rt"),
        ];
        assert_eq!(links(&endpoints), expect(&[("r1", "c1"), ("r2", "c2"), ("s1", "r1"), ("m1", "r1"), ("c1", "r1"), ("c2", "r2")]));
        // the cables are named by their recording devices, not "Speakers" twice
        assert_eq!(find(&endpoints)[..2].iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Line 1", "Line 2"]);
    }

    #[test]
    fn vb_cable_and_its_second_input() {
        // VB-Cable on Windows 10/11 has two inputs into one output: a speaker one and a 16-channel one
        let d = "ROOT\\VBAudioVACWDM\\0000";
        let cable = |id, render, name| named("VB-Audio Virtual Cable", ep(id, render, name, d, "vbaudiovacwave"));
        let endpoints = [cable("in", true, "CABLE Input"), cable("in16", true, "CABLE In 16ch"), cable("out", false, "CABLE Output")];
        // the output names the speaker input: the 16-channel one is a spare that points to the output only
        assert_eq!(links(&endpoints), expect(&[("in", "out"), ("in16", "out"), ("out", "in")]));
        assert_eq!(find(&endpoints)[0].name, "CABLE");
    }

    #[test]
    fn renamed_cable_driver_devices_still_pair() {
        // the user renamed both devices of a VB-Cable in the sound settings
        let d = "ROOT\\VBAudioVACWDM\\0000";
        let cable = |id, render, name| named("VB-Audio Virtual Cable", ep(id, render, name, d, "vbaudiovacwave"));
        assert_eq!(links(&[cable("in", true, "Music"), cable("out", false, "Discord mic")]), expect(&[("in", "out"), ("out", "in")]));
        // the same devices of a driver that doesn't say it is a cable can be anything
        let other = |id, render, name| named("NVIDIA Broadcast", ep(id, render, name, "ROOT\\MEDIA\\0003", "nvbroadcastwave"));
        assert!(links(&[other("spk", true, "Speakers"), other("mic", false, "Microphone")]).is_empty());
    }

    #[test]
    fn vb_audio_cables_by_names() {
        let endpoints = [
            named("VB-Audio Cable A", ep("a-in", true, "CABLE-A Input", "ROOT\\MEDIA\\0004", "w")),
            named("VB-Audio Cable A", ep("a-out", false, "CABLE-A Output", "ROOT\\MEDIA\\0004", "w")),
            named("VB-Audio Cable B", ep("b-in", true, "CABLE-B Input", "ROOT\\MEDIA\\0005", "w")),
            named("VB-Audio Cable B", ep("b-out", false, "CABLE-B Output", "ROOT\\MEDIA\\0005", "w")),
            named("VB-Audio Hi-Fi Cable", ep("hi-in", true, "Hi-Fi Cable Input", "ROOT\\MEDIA\\0006", "hifiwave")),
            named("VB-Audio Hi-Fi Cable", ep("hi-out", false, "Hi-Fi Cable Output", "ROOT\\MEDIA\\0006", "hifiwave")),
        ];
        let found = find(&endpoints);
        assert_eq!(
            found.iter().filter(|l| endpoints.iter().any(|e| e.render && e.id == l.device)).map(|l| (l.device.as_str(), l.other.as_str(), l.name.as_str())).collect::<Vec<_>>(),
            [("a-in", "a-out", "CABLE-A"), ("b-in", "b-out", "CABLE-B"), ("hi-in", "hi-out", "Hi-Fi Cable")]
        );
        assert_eq!(found.len(), 6);
    }

    #[test]
    fn mixers_are_not_cables() {
        // the older Voicemeeter names look like a cable, the newer filters do too: out6 → in6
        let d = "ROOT\\MEDIA\\0002";
        let vm = |id, render, name, filter| Endpoint {
            hardware: "VBVoicemeeterVAIO".into(),
            ..named("VB-Audio Voicemeeter VAIO", ep(id, render, name, d, filter))
        };
        let found = links(&[
            vm("in", true, "Voicemeeter Input", "vbvoicemeetervaio2023_out6_wave"),
            vm("out", false, "Voicemeeter Output", "vbvoicemeetervaio2023_in6_wave"),
            vm("aux-in", true, "Voicemeeter AUX Input", "vbvoicemeetervaio2023_out7_wave"),
            vm("aux-out", false, "Voicemeeter AUX Output", "vbvoicemeetervaio2023_in7_wave"),
        ]);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn real_devices_and_strangers_are_not_cables() {
        let usb = "USB\\VID_0D8C&PID_033B&MI_01\\8&25B81DEB&0&0001";
        let found = links(&[
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
        let found = links(&[
            ep("r", true, "Out A", d, "wave_r"),
            ep("c1", false, "In B", d, "wave_c"),
            ep("c2", false, "In C", d, "wave_c"),
        ]);
        // each recording device points to the only output device, the output device can't choose
        assert_eq!(found, expect(&[("c1", "r"), ("c2", "r")]));
    }

    #[test]
    fn channel_counts_are_left_out_of_names() {
        assert_eq!(without_channels("CABLE 16ch"), "cable");
        assert_eq!(without_channels("CABLE 16 Ch"), "cable");
        assert_eq!(without_channels("Line 16"), "line 16");
        assert_eq!(without_channels("Church"), "church");
    }
}
