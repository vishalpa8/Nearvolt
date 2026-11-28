//! A set of byte ranges, kept sorted and merged. The receiver uses it to know which
//! parts of a file have arrived, in whatever order the streams delivered them.

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct Ranges(Vec<(u64, u64)>);

impl Ranges {
    /// Builds a set from ranges in any order; empty and overlapping ones are merged.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (u64, u64)>) -> Self {
        let mut set = Self::default();
        for (start, end) in pairs {
            set.insert(start, end);
        }
        set
    }

    pub fn insert(&mut self, start: u64, end: u64) {
        if start >= end {
            return;
        }
        let (mut start, mut end) = (start, end);
        let mut merged = Vec::with_capacity(self.0.len() + 1);
        let mut placed = false;
        for &(s, e) in &self.0 {
            if e < start {
                merged.push((s, e));
            } else if s > end {
                if !placed {
                    merged.push((start, end));
                    placed = true;
                }
                merged.push((s, e));
            } else {
                start = start.min(s);
                end = end.max(e);
            }
        }
        if !placed {
            merged.push((start, end));
        }
        self.0 = merged;
    }

    /// Bytes covered.
    pub fn total(&self) -> u64 {
        self.0.iter().map(|(s, e)| e - s).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn to_pairs(&self) -> Vec<(u64, u64)> {
        self.0.clone()
    }

    /// Bytes of `start..end` that are already covered.
    pub fn overlap(&self, start: u64, end: u64) -> u64 {
        self.0.iter().map(|&(s, e)| e.min(end).saturating_sub(s.max(start))).sum()
    }

    pub fn contains(&self, start: u64, end: u64) -> bool {
        start >= end || self.0.iter().any(|&(s, e)| s <= start && end <= e)
    }

    /// What is missing from `0..size`, in order.
    pub fn gaps(&self, size: u64) -> Vec<(u64, u64)> {
        let mut gaps = Vec::new();
        let mut pos = 0;
        for &(s, e) in &self.0 {
            if s >= size {
                break;
            }
            if s > pos {
                gaps.push((pos, s));
            }
            pos = pos.max(e);
        }
        if pos < size {
            gaps.push((pos, size));
        }
        gaps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_merge_whatever_the_order() {
        let r = Ranges::from_pairs([(20, 30), (0, 10), (10, 15), (28, 40), (50, 50)]);
        assert_eq!(r.to_pairs(), vec![(0, 15), (20, 40)]);
        assert_eq!(r.total(), 35);
        assert_eq!(r.gaps(60), vec![(15, 20), (40, 60)]);
        assert_eq!(r.gaps(30), vec![(15, 20)]);
        assert_eq!(r.overlap(10, 25), 10);
        assert!(r.contains(22, 40) && !r.contains(14, 21));
    }

    #[test]
    fn an_empty_set_is_one_gap() {
        assert_eq!(Ranges::default().gaps(5), vec![(0, 5)]);
        assert!(Ranges::default().gaps(0).is_empty());
    }
}
