//! Кольцевой буфер звука между потоком записи и потоком воспроизведения:
//! один пишет, один читает, без блокировок.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Ring {
    data: Box<[UnsafeCell<f32>]>,
    /// Сколько всего записано и прочитано; позиция в буфере — остаток от деления
    written: AtomicUsize,
    read: AtomicUsize,
}

// пишет только Producer, читает только Consumer, а счётчики разделяют их области
unsafe impl Sync for Ring {}

pub fn ring(capacity: usize) -> (Producer, Consumer) {
    let ring = Arc::new(Ring {
        data: (0..capacity).map(|_| UnsafeCell::new(0.0)).collect(),
        written: AtomicUsize::new(0),
        read: AtomicUsize::new(0),
    });
    (Producer(ring.clone()), Consumer(ring))
}

pub struct Producer(Arc<Ring>);
pub struct Consumer(Arc<Ring>);

impl Producer {
    /// Дописывает сколько влезет; лишнее отбрасывается (читатель отстал или остановлен).
    /// Возвращает, сколько записано
    pub fn push(&self, samples: &[f32]) -> usize {
        let r = &*self.0;
        let written = r.written.load(Ordering::Relaxed);
        let free = r.data.len() - written.wrapping_sub(r.read.load(Ordering::Acquire));
        let n = samples.len().min(free);
        for (i, &s) in samples[..n].iter().enumerate() {
            unsafe { *r.data[(written + i) % r.data.len()].get() = s };
        }
        r.written.store(written.wrapping_add(n), Ordering::Release);
        n
    }
}

impl Consumer {
    pub fn len(&self) -> usize {
        let r = &*self.0;
        r.written.load(Ordering::Acquire).wrapping_sub(r.read.load(Ordering::Relaxed))
    }

    /// Прибавляет прочитанное к out (смешивание); возвращает, сколько прочитано
    pub fn mix_into(&self, out: &mut [f32]) -> usize {
        let r = &*self.0;
        let read = r.read.load(Ordering::Relaxed);
        let n = out.len().min(self.len());
        for (i, o) in out[..n].iter_mut().enumerate() {
            *o += unsafe { *r.data[(read + i) % r.data.len()].get() };
        }
        r.read.store(read.wrapping_add(n), Ordering::Release);
        n
    }

    /// Как mix_into, но с громкостью, плавно идущей от from к to: скачок громкости щёлкал бы
    pub fn mix_into_scaled(&self, out: &mut [f32], from: f32, to: f32) -> usize {
        if from == 1.0 && to == 1.0 {
            return self.mix_into(out);
        }
        let r = &*self.0;
        let read = r.read.load(Ordering::Relaxed);
        let n = out.len().min(self.len());
        let step = if n > 1 { (to - from) / (n - 1) as f32 } else { 0.0 };
        for (i, o) in out[..n].iter_mut().enumerate() {
            *o += unsafe { *r.data[(read + i) % r.data.len()].get() } * (from + step * i as f32);
        }
        r.read.store(read.wrapping_add(n), Ordering::Release);
        n
    }

    /// Выбрасывает n значений (не больше, чем есть)
    pub fn skip(&self, n: usize) {
        let r = &*self.0;
        let n = n.min(self.len());
        r.read.store(r.read.load(Ordering::Relaxed).wrapping_add(n), Ordering::Release);
    }

    /// Значение, которое будет прочитано через offset значений
    pub fn peek(&self, offset: usize) -> f32 {
        let r = &*self.0;
        let read = r.read.load(Ordering::Relaxed);
        unsafe { *r.data[(read + offset) % r.data.len()].get() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_drops_overflow() {
        let (p, c) = ring(4);
        p.push(&[1.0, 2.0, 3.0]);
        let mut out = [0.0; 2];
        assert_eq!(c.mix_into(&mut out), 2);
        assert_eq!(out, [1.0, 2.0]);
        // переход через конец буфера; пятое значение не влезает
        p.push(&[4.0, 5.0, 6.0, 7.0]);
        assert_eq!(c.len(), 4);
        let mut out = [10.0; 5];
        assert_eq!(c.mix_into(&mut out), 4);
        assert_eq!(out, [13.0, 14.0, 15.0, 16.0, 10.0]);
    }

    #[test]
    fn threads() {
        let (p, c) = ring(64);
        let writer = std::thread::spawn(move || {
            for i in 0..100_000 {
                while p.push(&[i as f32]) == 0 {}
            }
        });
        // всё приходит по порядку и без пропусков
        let mut expected = 0.0;
        while expected < 100_000.0 {
            let mut v = [0.0];
            if c.mix_into(&mut v) == 1 {
                assert_eq!(v[0], expected);
                expected += 1.0;
            }
        }
        writer.join().unwrap();
    }
}
