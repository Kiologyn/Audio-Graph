//! Потоки движка на WASAPI: на каждое устройство записи и каждое устройство вывода
//! по своему потоку. Запись раздаёт пакеты во все связи, вывод смешивает свои связи.

use super::super::link::{link, LinkReader, CHANNELS, RATE};
use super::super::plan::Stream;
use super::super::ring::Producer;
use super::super::{DeviceError, StreamStatus};
use crate::com::{wide, ComGuard};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::{
    IAudioCaptureClient, IAudioClient, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    DEVICE_STATE_ACTIVE, WAVEFORMATEX,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, WaitForSingleObject,
};

/// Упавший поток (устройство пропало, занято) перезапускается не чаще этого
const RETRY: Duration = Duration::from_secs(2);
/// Буфер устройства, в сотнях наносекунд
const BUFFER_HNS: i64 = 200_000;

/// Формат движка; AUTOCONVERTPCM просит Windows перевести его в формат устройства
const FORMAT: WAVEFORMATEX = WAVEFORMATEX {
    wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
    nChannels: CHANNELS as u16,
    nSamplesPerSec: RATE as u32,
    nAvgBytesPerSec: (RATE * CHANNELS * 4) as u32,
    nBlockAlign: (CHANNELS * 4) as u16,
    wBitsPerSample: 32,
    cbSize: 0,
};
const FLAGS: u32 =
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // паника в одном потоке звука не должна ронять остальные
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn describe(e: windows::core::Error) -> String {
    match e.code() {
        AUDCLNT_E_DEVICE_INVALIDATED => tr!("устройство отключено"),
        AUDCLNT_E_DEVICE_IN_USE => tr!("устройство занято другим приложением в монопольном режиме"),
        _ => e.message(),
    }
}

/// Список связей устройства; меняется на ходу, поток читает его на каждом пакете
type Links<T> = Arc<Mutex<Vec<(String, T)>>>;
type Body<T> = fn(&str, &Mutex<Vec<(String, T)>>, &AtomicBool, &AtomicBool) -> Result<(), String>;

/// Поток одного устройства
struct Worker<T> {
    links: Links<T>,
    stop: Arc<AtomicBool>,
    /// Устройство открыто и звук идёт
    working: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), String>>>,
    started: Option<Instant>,
    error: Option<String>,
}

impl<T: Send + 'static> Worker<T> {
    fn new() -> Self {
        Self {
            links: Arc::new(Mutex::new(Vec::new())),
            stop: Arc::new(AtomicBool::new(false)),
            working: Arc::new(AtomicBool::new(false)),
            thread: None,
            started: None,
            error: None,
        }
    }

    /// Запускает поток, если он не идёт; упавший — не чаще раза в RETRY
    fn supervise(&mut self, device: &str, body: Body<T>) {
        if self.thread.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        if let Some(thread) = self.thread.take() {
            self.error = match thread.join() {
                Ok(result) => result.err(),
                Err(_) => Some(tr!("сбой потока звука")),
            };
        }
        if self.started.is_some_and(|t| t.elapsed() < RETRY) {
            return;
        }
        let (device, links, stop, working) = (device.to_string(), self.links.clone(), self.stop.clone(), self.working.clone());
        self.started = Some(Instant::now());
        self.working.store(false, Ordering::Relaxed);
        self.thread = std::thread::Builder::new()
            .name(format!("audio {device}"))
            .spawn(move || body(&device, &links, &stop, &working))
            .ok();
    }

    fn error(&self) -> Option<String> {
        if self.working.load(Ordering::Relaxed) {
            None
        } else {
            self.error.clone()
        }
    }

    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn device_error<T: Send + 'static>(device: &str, worker: Option<&Worker<T>>) -> Option<DeviceError> {
    worker.and_then(|w| w.error()).map(|message| DeviceError {
        device: device.to_string(),
        message,
    })
}

#[derive(Default)]
pub struct Streams {
    captures: HashMap<String, Worker<Producer>>,
    renders: HashMap<String, Worker<LinkReader>>,
}

impl Streams {
    /// Приводит потоки к плану: новые связи добавляются, лишние убираются, на ходу
    pub fn apply(&mut self, plan: &[Stream]) -> Vec<StreamStatus> {
        // связь → громкость
        let wanted: BTreeMap<(&str, &str), f32> = plan
            .iter()
            .flat_map(|s| s.sources.iter().map(|src| ((src.device.as_str(), s.sink.as_str()), src.gain)))
            .collect();

        for (device, w) in &self.captures {
            lock(&w.links).retain(|(sink, _)| wanted.contains_key(&(device.as_str(), sink.as_str())));
        }
        for (device, w) in &self.renders {
            let mut links = lock(&w.links);
            links.retain(|(source, _)| wanted.contains_key(&(source.as_str(), device.as_str())));
            // громкость меняется на ходу, без пересоздания связи
            for (source, reader) in links.iter() {
                reader.set_gain(wanted[&(source.as_str(), device.as_str())]);
            }
        }
        for (&(source, sink), &gain) in &wanted {
            let capture = self.captures.entry(source.to_string()).or_insert_with(Worker::new);
            if lock(&capture.links).iter().any(|(s, _)| s == sink) {
                continue;
            }
            let (producer, reader) = link();
            reader.set_gain(gain);
            lock(&capture.links).push((sink.to_string(), producer));
            let render = self.renders.entry(sink.to_string()).or_insert_with(Worker::new);
            lock(&render.links).push((source.to_string(), reader));
        }

        // устройства без связей закрываются
        let unused: Vec<String> = self.captures.iter().filter(|(_, w)| lock(&w.links).is_empty()).map(|(d, _)| d.clone()).collect();
        for device in unused {
            self.captures.remove(&device).unwrap().shutdown();
        }
        let unused: Vec<String> = self.renders.iter().filter(|(_, w)| lock(&w.links).is_empty()).map(|(d, _)| d.clone()).collect();
        for device in unused {
            self.renders.remove(&device).unwrap().shutdown();
        }

        for (device, w) in &mut self.captures {
            w.supervise(device, capture_thread);
        }
        for (device, w) in &mut self.renders {
            w.supervise(device, render_thread);
        }

        plan.iter()
            .map(|s| StreamStatus {
                sink: s.sink.clone(),
                sources: s.sources.iter().map(|src| src.device.clone()).collect(),
                errors: device_error(&s.sink, self.renders.get(&s.sink))
                    .into_iter()
                    .chain(s.sources.iter().filter_map(|src| device_error(&src.device, self.captures.get(&src.device))))
                    .collect(),
            })
            .collect()
    }

    pub fn shutdown(&mut self) {
        for (_, w) in self.captures.drain() {
            w.shutdown();
        }
        for (_, w) in self.renders.drain() {
            w.shutdown();
        }
    }
}

/// Событие, которым WASAPI будит поток, когда пора забрать или отдать пакет
struct Event(HANDLE);

impl Drop for Event {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Поток звука получает приоритет планировщика мультимедиа, как у звуковых программ
struct ProAudio(Option<HANDLE>);

impl ProAudio {
    fn enter() -> Self {
        let mut index = 0;
        Self(unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut index) }.ok())
    }
}

impl Drop for ProAudio {
    fn drop(&mut self) {
        if let Some(h) = self.0 {
            let _ = unsafe { AvRevertMmThreadCharacteristics(h) };
        }
    }
}

/// Открывает устройство и настраивает поток в формате движка
fn open(device: &str) -> Result<(IAudioClient, Event), String> {
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(describe)?;
        let id = wide(device);
        let mm = enumerator.GetDevice(PCWSTR(id.as_ptr())).map_err(|_| tr!("устройство не найдено"))?;
        if mm.GetState().map_err(describe)? != DEVICE_STATE_ACTIVE {
            return Err(tr!("устройство выключено или отсоединено"));
        }
        let client: IAudioClient = mm.Activate(CLSCTX_ALL, None).map_err(describe)?;
        let event = Event(CreateEventW(None, false, false, None).map_err(describe)?);
        client
            .Initialize(AUDCLNT_SHAREMODE_SHARED, FLAGS, BUFFER_HNS, 0, &FORMAT, None)
            .map_err(describe)?;
        client.SetEventHandle(event.0).map_err(describe)?;
        Ok((client, event))
    }
}

fn capture_thread(device: &str, links: &Mutex<Vec<(String, Producer)>>, stop: &AtomicBool, working: &AtomicBool) -> Result<(), String> {
    let _com = ComGuard::init();
    let _priority = ProAudio::enter();
    let (client, event) = open(device)?;
    let result = unsafe { capture_loop(&client, &event, links, stop, working) };
    let _ = unsafe { client.Stop() };
    result.map_err(describe)
}

unsafe fn capture_loop(
    client: &IAudioClient,
    event: &Event,
    links: &Mutex<Vec<(String, Producer)>>,
    stop: &AtomicBool,
    working: &AtomicBool,
) -> windows::core::Result<()> {
    let capture: IAudioCaptureClient = client.GetService()?;
    client.Start()?;
    working.store(true, Ordering::Relaxed);
    let mut silence: Vec<f32> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        WaitForSingleObject(event.0, 100);
        while capture.GetNextPacketSize()? > 0 {
            let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
            capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
            let n = frames as usize * CHANNELS;
            let samples: &[f32] = if data.is_null() || flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                silence.resize(n.max(silence.len()), 0.0);
                &silence[..n]
            } else {
                std::slice::from_raw_parts(data as *const f32, n)
            };
            for (_, producer) in lock(links).iter() {
                producer.push(samples);
            }
            capture.ReleaseBuffer(frames)?;
        }
    }
    Ok(())
}

fn render_thread(device: &str, links: &Mutex<Vec<(String, LinkReader)>>, stop: &AtomicBool, working: &AtomicBool) -> Result<(), String> {
    let _com = ComGuard::init();
    let _priority = ProAudio::enter();
    let (client, event) = open(device)?;
    let result = unsafe { render_loop(&client, &event, links, stop, working) };
    let _ = unsafe { client.Stop() };
    result.map_err(describe)
}

unsafe fn render_loop(
    client: &IAudioClient,
    event: &Event,
    links: &Mutex<Vec<(String, LinkReader)>>,
    stop: &AtomicBool,
    working: &AtomicBool,
) -> windows::core::Result<()> {
    let render: IAudioRenderClient = client.GetService()?;
    let size = client.GetBufferSize()?;
    // до старта буфер устройства заполняется тишиной, чтобы начало не щёлкнуло
    render.GetBuffer(size)?;
    render.ReleaseBuffer(size, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
    client.Start()?;
    working.store(true, Ordering::Relaxed);
    let mut mix = vec![0.0f32; size as usize * CHANNELS];
    while !stop.load(Ordering::Relaxed) {
        WaitForSingleObject(event.0, 100);
        let frames = size - client.GetCurrentPadding()?;
        if frames == 0 {
            continue;
        }
        let out = &mut mix[..frames as usize * CHANNELS];
        out.fill(0.0);
        for (_, reader) in lock(links).iter_mut() {
            reader.mix_into(out);
        }
        // сумма нескольких источников может выйти за пределы
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        let data = render.GetBuffer(frames)?;
        std::ptr::copy_nonoverlapping(out.as_ptr(), data as *mut f32, out.len());
        render.ReleaseBuffer(frames, 0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::ring::ring;
    use super::*;
    use crate::devices::{list, Flow};

    /// Играет тон во вход VAC (Line 1) и слушает его выход: проверяет запись, воспроизведение
    /// и полную задержку. Слышно только тому, кто слушает этот кабель:
    /// cargo test vac_loopback -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vac_loopback() {
        let devices = list().unwrap();
        let vac = |flow| {
            devices
                .iter()
                .find(|d| d.name.contains("Virtual Audio Cable") && d.flow == flow)
                .expect("нет Virtual Audio Cable")
                .id
                .clone()
        };
        let (render_id, capture_id) = (vac(Flow::Render), vac(Flow::Capture));

        let mut capture = Worker::<Producer>::new();
        let (heard_in, heard) = ring(RATE * CHANNELS);
        lock(&capture.links).push(("test".into(), heard_in));
        capture.supervise(&capture_id, capture_thread);

        let mut render = Worker::<LinkReader>::new();
        let (tone, reader) = link();
        lock(&render.links).push(("test".into(), reader));
        render.supervise(&render_id, render_thread);
        std::thread::sleep(Duration::from_millis(300));
        assert!(capture.error().is_none() && render.error().is_none(), "{:?} {:?}", capture.error(), render.error());

        // тон 440 Гц пакетами по 10 мс; выход кабеля слушаем всё это время
        let started = Instant::now();
        let (mut phase, mut onset, mut energy, mut count) = (0usize, None, 0.0f64, 0usize);
        let mut buf = vec![0.0f32; RATE * CHANNELS];
        while started.elapsed() < Duration::from_millis(1500) {
            let packet: Vec<f32> = (0..480)
                .flat_map(|i| {
                    let s = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * (phase + i) as f32 / RATE as f32).sin();
                    [s, s]
                })
                .collect();
            phase += 480;
            tone.push(&packet);
            std::thread::sleep(Duration::from_millis(10));
            buf.fill(0.0);
            let n = heard.mix_into(&mut buf);
            if onset.is_none() && buf[..n].iter().any(|s| s.abs() > 0.05) {
                onset = Some(started.elapsed());
            }
            if started.elapsed() > Duration::from_millis(700) {
                energy += buf[..n].iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>();
                count += n;
            }
        }
        let rms = (energy / count.max(1) as f64).sqrt();
        println!("звук на выходе кабеля через {onset:?}, громкость (RMS) {rms:.3}, ожидалось ~0.354");
        render.shutdown();
        capture.shutdown();
        assert!(onset.is_some() && rms > 0.1, "тон не дошёл");
    }
}
