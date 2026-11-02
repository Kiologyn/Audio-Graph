//! Device changes: a device plugged in, unplugged, turned on or off, the default changed.
//! The engine reconnects at once instead of waiting for its retry, and the window rereads the list.

/// Calls on_change after device changes, from another thread. Watches as long as the app runs
#[cfg(windows)]
pub fn watch(on_change: impl Fn() + Send + Sync + 'static) {
    let _ = std::thread::Builder::new().name("device watch".into()).spawn(move || {
        let _com = crate::com::ComGuard::init();
        // the enumerator and the callback live as long as the thread, that is, as long as the app
        match win::register(Box::new(on_change)) {
            Ok(_registration) => loop {
                std::thread::park();
            },
            Err(e) => crate::log::write(format!("device changes aren't watched: {e}")),
        }
    });
}

/// The audio server has no change callback in the synchronous wrapper: the list is compared every second
#[cfg(not(windows))]
pub fn watch(on_change: impl Fn() + Send + Sync + 'static) {
    let _ = std::thread::Builder::new().name("device watch".into()).spawn(move || {
        let fingerprint = || {
            super::list().ok().map(|list| {
                list.into_iter().map(|d| (d.id, d.name, d.state == super::DeviceState::Active, d.is_default)).collect::<Vec<_>>()
            })
        };
        let mut last = fingerprint();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let now = fingerprint();
            if now.is_some() && now != last {
                on_change();
            }
            last = now;
        }
    });
}

#[cfg(windows)]
mod win {
    use windows::core::{implement, Result, PCWSTR};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::{
        EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
        DEVICE_STATE,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    type Callback = Box<dyn Fn() + Send + Sync>;

    #[implement(IMMNotificationClient)]
    struct Client(Callback);

    impl IMMNotificationClient_Impl for Client_Impl {
        fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
            (self.0)();
            Ok(())
        }
        fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
            (self.0)();
            Ok(())
        }
        fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
            (self.0)();
            Ok(())
        }
        fn OnDefaultDeviceChanged(&self, _: EDataFlow, _: ERole, _: &PCWSTR) -> Result<()> {
            (self.0)();
            Ok(())
        }
        // properties change often (volume, format): they don't change the device list
        fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
            Ok(())
        }
    }

    /// The registration: notifications come while it is alive
    pub struct Registration {
        _enumerator: IMMDeviceEnumerator,
        _client: IMMNotificationClient,
    }

    pub fn register(callback: Callback) -> Result<Registration> {
        unsafe {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let client: IMMNotificationClient = Client(callback).into();
            enumerator.RegisterEndpointNotificationCallback(&client)?;
            Ok(Registration {
                _enumerator: enumerator,
                _client: client,
            })
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::super::{list, set_enabled, DeviceState, Flow};
    use std::sync::mpsc;
    use std::time::Duration;

    /// Turns the VAC Line 1 recording device off and on: both changes must be reported.
    /// Changes the system state for a moment:
    /// cargo test reports_device_changes -- --ignored --nocapture
    #[test]
    #[ignore]
    fn reports_device_changes() {
        let _defaults = crate::devices::win::KeepDefaults::new();
        let (tx, rx) = mpsc::channel();
        super::watch(move || {
            let _ = tx.send(());
        });
        std::thread::sleep(Duration::from_millis(300));
        let id = list()
            .unwrap()
            .into_iter()
            .find(|d| d.name.starts_with("Line 1 (Virtual Audio Cable") && d.flow == Flow::Capture && d.state == DeviceState::Active)
            .expect("VAC Line 1 must be on")
            .id;
        set_enabled(&[id.clone()], false).unwrap();
        let off = rx.recv_timeout(Duration::from_secs(3));
        while rx.try_recv().is_ok() {}
        set_enabled(&[id], true).unwrap();
        let on = rx.recv_timeout(Duration::from_secs(3));
        println!("off: {off:?}, on: {on:?}");
        assert!(off.is_ok() && on.is_ok());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    /// A device appears and disappears on the audio server (PULSE_SERVER of a test server):
    /// cargo test reports_device_changes -- --ignored --nocapture
    #[test]
    #[ignore]
    fn reports_device_changes() {
        let (tx, rx) = mpsc::channel();
        super::watch(move || {
            let _ = tx.send(());
        });
        std::thread::sleep(Duration::from_millis(1500));
        let pactl = |args: &[&str]| String::from_utf8(Command::new("pactl").args(args).output().unwrap().stdout).unwrap();
        let module = pactl(&["load-module", "module-null-sink", "sink_name=agtest.watch"]);
        let added = rx.recv_timeout(Duration::from_secs(3));
        while rx.try_recv().is_ok() {}
        pactl(&["unload-module", module.trim()]);
        let removed = rx.recv_timeout(Duration::from_secs(3));
        println!("added: {added:?}, removed: {removed:?}");
        assert!(added.is_ok() && removed.is_ok());
    }
}
