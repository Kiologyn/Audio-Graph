//! Cables: pairs of system devices shared by all graphs.
//! Stored in one file, cables.json; graph nodes refer to them by id.

use crate::engine::Engine;
use crate::graphs::DeviceRef;
use crate::storage::{check_id, clean_name, err, now_ms, unique_name, write_atomic, CmdResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::State;

const FORMAT_VERSION: u32 = 1;

fn format_version() -> u32 {
    FORMAT_VERSION
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Cable {
    pub id: String,
    pub name: String,
    /// The output device: apps play into it, it is the cable's input
    #[serde(default)]
    pub render: Option<DeviceRef>,
    /// The input device: sound is taken from it, it is the cable's output
    #[serde(default)]
    pub capture: Option<DeviceRef>,
    pub created_at: u64,
}

#[derive(Serialize, Deserialize)]
struct CablesFile {
    #[serde(default = "format_version")]
    version: u32,
    #[serde(default)]
    cables: Vec<Cable>,
}

pub struct CableStore {
    path: PathBuf,
    lock: Mutex<()>,
}

/// A cable has specific devices only: "default" makes no sense for it
fn check_device(device: &Option<DeviceRef>) -> CmdResult<()> {
    match device {
        Some(DeviceRef::Default) => Err(tr!("error_cable_default_device")),
        _ => Ok(()),
    }
}

impl CableStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("cables.json"),
            lock: Mutex::new(()),
        }
    }

    /// All cables for the engine
    pub fn all(&self) -> CmdResult<Vec<Cable>> {
        let _guard = self.lock.lock().map_err(err)?;
        self.read()
    }

    fn read(&self) -> CmdResult<Vec<Cable>> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(serde_json::from_str::<CablesFile>(&text).map_err(err)?.cables),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(err(e)),
        }
    }

    /// Writes the list and tells the engine: a running graph depends on cables
    fn write(&self, cables: Vec<Cable>, engine: &Engine) -> CmdResult<()> {
        let file = CablesFile {
            version: FORMAT_VERSION,
            cables,
        };
        let json = serde_json::to_vec_pretty(&file).map_err(err)?;
        write_atomic(&self.path, &json).map_err(err)?;
        engine.cables_changed(file.cables);
        Ok(())
    }

    /// Cables of an imported graph join the list (see merge). Returns the ids to put into the graph's nodes
    pub fn adopt(&self, incoming: &[Cable], engine: &Engine) -> CmdResult<HashMap<String, String>> {
        let _guard = self.lock.lock().map_err(err)?;
        let mut cables = self.read()?;
        let count = cables.len();
        let ids = merge(&mut cables, incoming)?;
        if cables.len() > count {
            self.write(cables, engine)?;
        }
        Ok(ids)
    }
}

/// Each incoming cable becomes a cable of the list: an existing one with the same id or the same devices,
/// a new one otherwise. Returns incoming id → id in the list
fn merge(cables: &mut Vec<Cable>, incoming: &[Cable]) -> CmdResult<HashMap<String, String>> {
    let device = |d: &Option<DeviceRef>| match d {
        Some(DeviceRef::Device { id, .. }) => Some(id.clone()),
        _ => None,
    };
    let mut ids = HashMap::new();
    for cable in incoming {
        let sides = (device(&cable.render), device(&cable.capture));
        let has_device = sides.0.is_some() || sides.1.is_some();
        let existing = cables
            .iter()
            .find(|c| c.id == cable.id)
            .or_else(|| cables.iter().find(|c| has_device && (device(&c.render), device(&c.capture)) == sides));
        let id = match existing {
            Some(c) => c.id.clone(),
            None => {
                let name = unique_name(&clean_name(&cable.name)?, cables.iter().map(|c| c.name.as_str()));
                // "default" makes no sense for a cable (check_device): such a side stays empty
                let side = |d: &Option<DeviceRef>| d.clone().filter(|d| *d != DeviceRef::Default);
                let new = Cable {
                    id: uuid::Uuid::new_v4().to_string(),
                    name,
                    render: side(&cable.render),
                    capture: side(&cable.capture),
                    created_at: now_ms(),
                };
                let id = new.id.clone();
                cables.push(new);
                id
            }
        };
        ids.insert(cable.id.clone(), id);
    }
    Ok(ids)
}

#[tauri::command]
pub fn cables_list(store: State<CableStore>) -> CmdResult<Vec<Cable>> {
    let _guard = store.lock.lock().map_err(err)?;
    store.read()
}

#[tauri::command]
pub fn cable_create(store: State<CableStore>, engine: State<Engine>, name: String) -> CmdResult<Cable> {
    let _guard = store.lock.lock().map_err(err)?;
    let mut cables = store.read()?;
    let cable = Cable {
        id: uuid::Uuid::new_v4().to_string(),
        name: clean_name(&name)?,
        render: None,
        capture: None,
        created_at: now_ms(),
    };
    cables.push(cable.clone());
    store.write(cables, &engine)?;
    Ok(cable)
}

/// Replaces a cable's name and devices; the id and creation date stay
#[tauri::command]
pub fn cable_update(store: State<CableStore>, engine: State<Engine>, cable: Cable) -> CmdResult<Cable> {
    let _guard = store.lock.lock().map_err(err)?;
    check_id(&cable.id)?;
    check_device(&cable.render)?;
    check_device(&cable.capture)?;
    let mut cables = store.read()?;
    let target = cables
        .iter_mut()
        .find(|c| c.id == cable.id)
        .ok_or_else(|| tr!("error_cable_not_found"))?;
    target.name = clean_name(&cable.name)?;
    target.render = cable.render;
    target.capture = cable.capture;
    let updated = target.clone();
    store.write(cables, &engine)?;
    Ok(updated)
}

/// Puts the cables in the order of the ids; cables missing from the list keep their order at the end
#[tauri::command]
pub fn cables_reorder(store: State<CableStore>, engine: State<Engine>, ids: Vec<String>) -> CmdResult<()> {
    let _guard = store.lock.lock().map_err(err)?;
    let mut cables = store.read()?;
    reorder(&mut cables, &ids);
    store.write(cables, &engine)
}

/// The cables in the order of the ids; the sort is stable, so cables missing from the ids keep their order
fn reorder(cables: &mut [Cable], ids: &[String]) {
    let place = |id: &str| ids.iter().position(|o| o == id).unwrap_or(usize::MAX);
    cables.sort_by_key(|c| place(&c.id));
}

#[tauri::command]
pub fn cable_delete(store: State<CableStore>, engine: State<Engine>, id: String) -> CmdResult<()> {
    let _guard = store.lock.lock().map_err(err)?;
    let mut cables = store.read()?;
    cables.retain(|c| c.id != id);
    store.write(cables, &engine)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cable(id: &str, name: &str, render: Option<&str>) -> Cable {
        Cable {
            id: id.into(),
            name: name.into(),
            render: render.map(|r| DeviceRef::Device { id: r.into(), name: r.into() }),
            capture: None,
            created_at: 0,
        }
    }

    #[test]
    fn imported_cables_join_the_list() {
        let mut list = vec![cable("1", "Line 1", Some("dev-1")), cable("2", "Calls", None)];
        let incoming = [
            // the same cable exported from this computer
            cable("1", "Line 1", Some("dev-1")),
            // another computer's cable of the same device
            cable("x", "VAC", Some("dev-1")),
            // a cable this computer doesn't have; its name is taken
            cable("y", "Calls", Some("dev-9")),
        ];
        let ids = merge(&mut list, &incoming).unwrap();
        assert_eq!(ids["1"], "1");
        assert_eq!(ids["x"], "1");
        assert_eq!(list.len(), 3);
        assert_eq!(ids["y"], list[2].id);
        assert_eq!(list[2].name, "Calls 2");
    }

    #[test]
    fn imported_cable_is_found_by_id_before_devices() {
        let mut list = vec![cable("1", "Line 1", Some("dev-1")), cable("2", "Line 2", Some("dev-2"))];
        // this computer's cable 2, its device changed since the export
        let ids = merge(&mut list, &[cable("2", "Line 2", Some("dev-1"))]).unwrap();
        assert_eq!((ids["2"].as_str(), list.len()), ("2", 2));
    }

    #[test]
    fn cables_without_devices_are_never_the_same_cable() {
        let mut list = vec![cable("1", "Calls", None)];
        let ids = merge(&mut list, &[cable("x", "Calls", None)]).unwrap();
        assert_ne!(ids["x"], "1");
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].name, "Calls 2");
    }

    #[test]
    fn imported_default_device_is_left_out() {
        let mut list = Vec::new();
        let mut incoming = cable("x", "Odd", Some("dev-1"));
        incoming.capture = Some(DeviceRef::Default);
        merge(&mut list, &[incoming]).unwrap();
        assert!(list[0].render.is_some());
        assert_eq!(list[0].capture, None);
        assert!(check_device(&list[0].capture).is_ok() && check_device(&Some(DeviceRef::Default)).is_err());
    }

    #[test]
    fn imported_cable_with_an_empty_name_fails() {
        assert!(merge(&mut Vec::new(), &[cable("x", "  ", Some("dev-1"))]).is_err());
    }

    #[test]
    fn reorder_puts_unknown_cables_last_in_their_order() {
        let mut list = vec![cable("a", "A", None), cable("b", "B", None), cable("c", "C", None), cable("d", "D", None)];
        reorder(&mut list, &["c".into(), "gone".into(), "a".into()]);
        let order: Vec<&str> = list.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(order, ["c", "a", "b", "d"]);
    }

    #[test]
    fn missing_file_is_an_empty_list_and_a_damaged_one_an_error() {
        let dir = crate::storage::test_dir("cables-file");
        let store = CableStore::new(&dir);
        assert!(store.all().unwrap().is_empty());
        // a damaged list must not silently become empty: the next write would lose the cables
        fs::write(dir.join("cables.json"), "{").unwrap();
        assert!(store.all().is_err());
        fs::write(dir.join("cables.json"), r#"{"cables":[{"id":"1","name":"Line 1","createdAt":1}]}"#).unwrap();
        assert_eq!(store.all().unwrap()[0].name, "Line 1");
    }
}
