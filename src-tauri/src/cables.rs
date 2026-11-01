//! Cables: pairs of system devices shared by all graphs.
//! Stored in one file, cables.json; graph nodes refer to them by id.

use crate::engine::Engine;
use crate::graphs::DeviceRef;
use crate::storage::{check_id, clean_name, err, now_ms, write_atomic, CmdResult};
use serde::{Deserialize, Serialize};
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
    let place = |id: &str| ids.iter().position(|o| o == id).unwrap_or(usize::MAX);
    // the sort is stable: cables not in the list stay in their order
    cables.sort_by_key(|c| place(&c.id));
    store.write(cables, &engine)
}

#[tauri::command]
pub fn cable_delete(store: State<CableStore>, engine: State<Engine>, id: String) -> CmdResult<()> {
    let _guard = store.lock.lock().map_err(err)?;
    let mut cables = store.read()?;
    cables.retain(|c| c.id != id);
    store.write(cables, &engine)
}
