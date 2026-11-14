//! Part of `extension.ts`: [`ChangedRange`]. The view plugins, the update
//! object and the view's facets come with the view.

/// A changed range, in positions of the old (`a`) and the new (`b`)
/// document (`ChangedRange`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangedRange {
    /// Start in the old document.
    pub from_a: usize,
    /// End in the old document.
    pub to_a: usize,
    /// Start in the new document.
    pub from_b: usize,
    /// End in the new document.
    pub to_b: usize,
}

impl ChangedRange {
    /// A changed range.
    pub fn new(from_a: usize, to_a: usize, from_b: usize, to_b: usize) -> ChangedRange {
        ChangedRange {
            from_a,
            to_a,
            from_b,
            to_b,
        }
    }

    /// The smallest range covering both (`join`).
    pub fn join(&self, other: &ChangedRange) -> ChangedRange {
        ChangedRange::new(
            self.from_a.min(other.from_a),
            self.to_a.max(other.to_a),
            self.from_b.min(other.from_b),
            self.to_b.max(other.to_b),
        )
    }

    /// Add this range to a set sorted by position, joining it with the
    /// ranges it touches (`addToSet`).
    pub fn add_to_set(&self, set: &mut Vec<ChangedRange>) {
        let mut i = set.len();
        let mut me = *self;
        while i > 0 {
            let range = set[i - 1];
            if range.from_a > me.to_a {
                i -= 1;
                continue;
            }
            if range.to_a < me.from_a {
                break;
            }
            me = me.join(&range);
            set.remove(i - 1);
            i -= 1;
        }
        set.insert(i, me);
    }

    /// Extend `diff` to cover the content in `ranges`, a flat list of
    /// `from_b`, `to_b` pairs in unchanged content (`extendWithRanges`).
    pub fn extend_with_ranges(diff: &[ChangedRange], ranges: &[usize]) -> Vec<ChangedRange> {
        if ranges.is_empty() {
            return diff.to_vec();
        }
        const FAR: i64 = 1_000_000_000;
        let mut result = Vec::new();
        let (mut d_i, mut r_i, mut off) = (0usize, 0usize, 0i64);
        loop {
            let next_d = diff.get(d_i).map_or(FAR, |d| d.from_b as i64);
            let next_r = ranges.get(r_i).map_or(FAR, |&r| r as i64);
            let from_b = next_d.min(next_r);
            if from_b == FAR {
                break;
            }
            let from_a = from_b + off;
            let (mut to_b, mut to_a) = (from_b, from_a);
            loop {
                if r_i < ranges.len() && ranges[r_i] as i64 <= to_b {
                    let end = ranges[r_i + 1] as i64;
                    r_i += 2;
                    to_b = to_b.max(end);
                    let mut i = d_i;
                    while i < diff.len() && diff[i].from_b as i64 <= to_b {
                        off = diff[i].to_a as i64 - diff[i].to_b as i64;
                        i += 1;
                    }
                    to_a = to_a.max(end + off);
                } else if d_i < diff.len() && diff[d_i].from_b as i64 <= to_b {
                    let next = diff[d_i];
                    d_i += 1;
                    to_b = to_b.max(next.to_b as i64);
                    to_a = to_a.max(next.to_a as i64);
                    off = next.to_a as i64 - next.to_b as i64;
                } else {
                    break;
                }
            }
            result.push(ChangedRange::new(
                from_a as usize,
                to_a as usize,
                from_b as usize,
                to_b as usize,
            ));
        }
        result
    }
}
