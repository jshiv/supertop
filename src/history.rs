use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Samples kept per series: one hour at the configured sampling interval.
static CAPACITY: AtomicUsize = AtomicUsize::new(3600);

pub fn set_capacity(samples: usize) {
    CAPACITY.store(samples.max(1), Ordering::Relaxed);
}

fn capacity() -> usize {
    CAPACITY.load(Ordering::Relaxed)
}

/// A fixed-capacity time series, newest sample at the back.
#[derive(Clone, Default)]
pub struct Series {
    data: VecDeque<f64>,
}

impl Series {
    pub fn push(&mut self, v: f64) {
        while self.data.len() >= capacity() {
            self.data.pop_front();
        }
        self.data.push_back(v);
    }

    /// Pads the front so a series that appears late (a new sensor, a new core)
    /// stays time-aligned with the others.
    pub fn push_aligned(&mut self, v: f64, len: usize) {
        while self.data.len() + 1 < len.min(capacity()) {
            self.data.push_front(f64::NAN);
        }
        self.push(v);
    }

    pub fn last(&self) -> Option<f64> {
        self.data.back().copied().filter(|v| v.is_finite())
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// The most recent `window` samples (fewer if not yet collected).
    pub fn tail(&self, window: usize) -> impl Iterator<Item = f64> + '_ {
        self.data.iter().skip(self.data.len().saturating_sub(window)).copied()
    }

    pub fn max_in(&self, window: usize) -> f64 {
        self.tail(window).filter(|v| v.is_finite()).fold(0.0, f64::max)
    }

    pub fn min_in(&self, window: usize) -> Option<f64> {
        self.tail(window).filter(|v| v.is_finite()).reduce(f64::min)
    }

    pub fn avg_in(&self, window: usize) -> f64 {
        let (sum, n) = self
            .tail(window)
            .filter(|v| v.is_finite())
            .fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
        if n == 0 { 0.0 } else { sum / n as f64 }
    }

    /// Resamples the last `window` samples onto `cols` columns, right-aligned so
    /// "now" is always at the right edge. Columns with no data yet are NaN.
    /// When several samples land in one column we keep the peak so short spikes
    /// stay visible on long windows.
    pub fn resample(&self, window: usize, cols: usize) -> Vec<f64> {
        let mut out = vec![f64::NAN; cols];
        if cols == 0 || window == 0 {
            return out;
        }
        let have = self.data.len().min(window);
        let start = self.data.len() - have;
        // Sample index (0 = oldest in window) → column.
        let offset = window - have;
        for (i, v) in self.data.iter().skip(start).enumerate() {
            if !v.is_finite() {
                continue;
            }
            let pos = offset + i;
            let col = (pos * cols / window).min(cols - 1);
            if out[col].is_nan() || *v > out[col] {
                out[col] = *v;
            }
        }
        // When the window has fewer samples than columns, stretch each sample
        // across the gap so lines look continuous rather than dotted.
        let mut last = f64::NAN;
        let first_col = offset * cols / window;
        for v in out.iter_mut().skip(first_col) {
            if v.is_nan() {
                *v = last;
            } else {
                last = *v;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_right_aligns_partial_history() {
        let mut s = Series::default();
        for v in [1.0, 2.0, 3.0] {
            s.push(v);
        }
        let r = s.resample(6, 6);
        assert!(r[..3].iter().all(|v| v.is_nan()));
        assert_eq!(&r[3..], &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn resample_keeps_peaks_when_compressing() {
        let mut s = Series::default();
        for v in [1.0, 9.0, 1.0, 1.0] {
            s.push(v);
        }
        assert_eq!(s.resample(4, 2), vec![9.0, 1.0]);
    }

    #[test]
    fn resample_stretches_when_expanding() {
        let mut s = Series::default();
        s.push(5.0);
        s.push(7.0);
        assert_eq!(s.resample(2, 4), vec![5.0, 5.0, 7.0, 7.0]);
    }

    #[test]
    fn push_aligned_pads_front() {
        let mut s = Series::default();
        s.push_aligned(4.0, 3);
        assert_eq!(s.len(), 3);
        assert_eq!(s.last(), Some(4.0));
    }
}
