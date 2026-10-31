//! Связь «устройство записи → устройство вывода»: буфер между их потоками и подстройка
//! задержки. Часы у устройств идут немного по-разному, поэтому запас в буфере медленно
//! растёт или тает; его держат около цели, выбрасывая или повторяя по одному кадру.

use super::ring::{ring, Consumer, Producer};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Формат звука внутри движка; Windows сама переводит в формат каждого устройства
pub const CHANNELS: usize = 2;
pub const RATE: usize = 48_000;

const fn ms(ms: usize) -> usize {
    RATE * ms / 1000
}

/// Запас в буфере связи, кадров: перекрывает неравномерность пакетов записи
const TARGET: usize = ms(20);
/// Насколько средний запас может отойти от цели, прежде чем начнётся подстройка
const SLACK: usize = ms(8);
/// Если накопилось больше (например, устройство вывода перезапускалось), лишнее выбрасывается сразу
const MAX: usize = ms(120);

pub fn link() -> (Producer, LinkReader) {
    let (producer, consumer) = ring(RATE * CHANNELS);
    (
        producer,
        LinkReader {
            ring: consumer,
            primed: false,
            average: 0.0,
            gain: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            current_gain: 1.0,
        },
    )
}

pub struct LinkReader {
    ring: Consumer,
    /// Запас набран; после старта и после опустошения сначала копим его заново
    primed: bool,
    /// Средний запас за последние ~полсекунды: мгновенный скачет в такт пакетам
    average: f32,
    /// Громкость связи; меняется из потока движка на ходу, поток вывода подхватывает её плавно
    gain: Arc<AtomicU32>,
    current_gain: f32,
}

impl LinkReader {
    pub fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// Подмешивает звук связи в out (кадры по CHANNELS значений)
    pub fn mix_into(&mut self, out: &mut [f32]) {
        let (from, to) = (self.current_gain, f32::from_bits(self.gain.load(Ordering::Relaxed)));
        self.current_gain = to;
        self.mix_frames(out, from, to);
    }

    fn mix_frames(&mut self, out: &mut [f32], from: f32, to: f32) {
        let need = out.len() / CHANNELS;
        let mut available = self.ring.len() / CHANNELS;
        if !self.primed {
            if available < TARGET {
                return;
            }
            self.primed = true;
            self.average = available as f32;
        }
        if available > MAX {
            self.ring.skip((available - TARGET) * CHANNELS);
            available = TARGET;
            self.average = TARGET as f32;
        }
        self.average += (available as f32 - self.average) * 0.02;

        if available < need {
            // запись не успевает: отдаём, что есть, и копим запас заново
            self.ring.mix_into_scaled(&mut out[..available * CHANNELS], from, to);
            self.primed = false;
            return;
        }
        if self.average > (TARGET + SLACK) as f32 && available > need {
            // запись спешит: один кадр выбрасываем
            self.ring.skip(CHANNELS);
        } else if self.average < (TARGET - SLACK) as f32 && need > 1 {
            // запись отстаёт: последний кадр повторяем
            let last: [f32; CHANNELS] = std::array::from_fn(|ch| self.ring.peek((need - 2) * CHANNELS + ch));
            let n = (need - 1) * CHANNELS;
            self.ring.mix_into_scaled(&mut out[..n], from, to);
            for (o, s) in out[n..].iter_mut().zip(last) {
                *o += s * to;
            }
            return;
        }
        self.ring.mix_into_scaled(out, from, to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Запись и воспроизведение пакетами по 10 мс; каждый десятый пакет записи длиннее
    /// или короче на drift кадров (1 кадр на 4800 — около 200 ppm, обычные часы расходятся меньше)
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
        // за минуту без подстройки разошлось бы на 6000 кадров, а запас держится у цели
        for drift in [1, -1, 5, -5] {
            let (silent, level) = simulate(drift, 60);
            assert_eq!(silent, 0, "расхождение {drift}");
            assert!(level.abs_diff(TARGET) <= SLACK + 50, "расхождение {drift}: запас {level}");
        }
    }

    #[test]
    fn gain_ramps_without_jump() {
        let (producer, mut reader) = link();
        producer.push(&vec![0.5f32; TARGET * 4 * CHANNELS]);
        let mut out = vec![0.0f32; 480 * CHANNELS];
        reader.mix_into(&mut out);
        assert!(out.iter().all(|&s| s == 0.5));
        // громкость 50%: за один пакет плавно от 0.5 до 0.25, дальше ровно 0.25
        reader.set_gain(0.5);
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
