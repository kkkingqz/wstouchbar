//! Copy coverage for shared buffers that can lag behind the displayed frame.
use crate::DamageRect;
use std::collections::VecDeque;
use touchbar_protocol::damage::{Damage, merge_damage};
const HISTORY: usize = 64;
pub(crate) struct History {
    size: (u32, u32),
    current: u64,
    buffers: Vec<Option<u64>>,
    records: VecDeque<(u64, Vec<DamageRect>)>,
}
impl History {
    pub fn new(size: (u32, u32), count: usize) -> Self {
        Self {
            size,
            current: 0,
            buffers: vec![None; count],
            records: VecDeque::new(),
        }
    }
    pub fn record(&mut self, damage: &[DamageRect]) {
        if damage.is_empty() {
            return;
        }
        self.current = match self.current.checked_add(1) {
            Some(n) => n,
            None => {
                self.buffers.fill(None);
                self.records.clear();
                1
            }
        };
        self.records.push_back((self.current, damage.to_vec()));
        if self.records.len() > HISTORY {
            self.records.pop_front();
        }
    }
    fn full(&self) -> Vec<DamageRect> {
        vec![DamageRect {
            x: 0,
            y: 0,
            width: self.size.0,
            height: self.size.1,
        }]
    }
    pub fn coverage(&self, index: usize) -> Vec<DamageRect> {
        let Some(Some(version)) = self.buffers.get(index) else {
            return self.full();
        };
        if *version == self.current {
            return Vec::new();
        }
        if self
            .records
            .front()
            .is_none_or(|(n, _)| *n > version.saturating_add(1))
        {
            return self.full();
        }
        let rects = self
            .records
            .iter()
            .filter(|(n, _)| n > version)
            .flat_map(|(_, r)| r)
            .map(|r| {
                Some(touchbar_protocol::DamageRect {
                    x: u16::try_from(r.x).ok()?,
                    y: u16::try_from(r.y).ok()?,
                    width: u16::try_from(r.width).ok()?,
                    height: u16::try_from(r.height).ok()?,
                })
            })
            .collect::<Option<Vec<_>>>();
        match rects.and_then(|r| merge_damage(&r, self.size.0, self.size.1).ok()) {
            Some(Damage::Rects(r)) => r
                .into_iter()
                .map(|r| DamageRect {
                    x: u32::from(r.x),
                    y: u32::from(r.y),
                    width: u32::from(r.width),
                    height: u32::from(r.height),
                })
                .collect(),
            _ => self.full(),
        }
    }
    pub fn copied(&mut self, index: usize) {
        if let Some(buffer) = self.buffers.get_mut(index) {
            *buffer = Some(self.current);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::DamageRect;
    fn copy(rects: &[DamageRect], source: &[u8], destination: &mut [u8], width: u32) {
        for r in rects {
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    let i = (y * width + x) as usize;
                    destination[i] = source[i];
                }
            }
        }
    }
    #[test]
    fn revisiting_stale_buffers_reproduces_the_whole_current_image() {
        let mut history = History::new((16, 2), 3);
        let mut reference = vec![0u8; 32];
        let mut buffers = [vec![0u8; 32], vec![0u8; 32], vec![0u8; 32]];
        for (n, index) in [0, 1, 2, 0, 2, 1, 0].into_iter().enumerate() {
            let x = u32::try_from(n * 2).unwrap();
            let value = u8::try_from(n + 1).unwrap();
            reference[x as usize] = value;
            history.record(&[DamageRect {
                x,
                y: 0,
                width: 1,
                height: 1,
            }]);
            copy(
                &history.coverage(index),
                &reference,
                &mut buffers[index],
                16,
            );
            history.copied(index);
            assert_eq!(
                buffers[index], reference,
                "buffer {index} missed changes from intervening frames"
            );
        }
    }
    #[test]
    fn current_buffer_copies_only_the_new_dirty_rows() {
        let mut h = History::new((4, 20), 3);
        h.record(&[DamageRect {
            x: 0,
            y: 0,
            width: 4,
            height: 20,
        }]);
        h.copied(0);
        let dirty = DamageRect {
            x: 0,
            y: 7,
            width: 4,
            height: 2,
        };
        h.record(&[dirty]);
        assert_eq!(h.coverage(0), vec![dirty]);
    }
    #[test]
    fn lost_history_or_more_than_eight_disjoint_regions_requires_full_copy() {
        let mut h = History::new((40, 2), 3);
        let full = DamageRect {
            x: 0,
            y: 0,
            width: 40,
            height: 2,
        };
        h.record(&[full]);
        h.copied(0);
        for n in 0..9 {
            h.record(&[DamageRect {
                x: n * 3,
                y: 0,
                width: 1,
                height: 1,
            }]);
        }
        assert_eq!(h.coverage(0), vec![full]);
        h.copied(1);
        for _ in 0..65 {
            h.record(&[DamageRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }]);
        }
        assert_eq!(h.coverage(1), vec![full]);
    }
}
