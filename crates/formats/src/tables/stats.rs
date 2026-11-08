//! Numeric helpers with Python `statistics` / builtin semantics (parity with the spike prototype).

/// `statistics.mean`; callers guard the empty case.
pub(crate) fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// `statistics.median`: mean of the two middle values for even length.
pub(crate) fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// `statistics.pstdev` (population form, two-pass).
pub(crate) fn pstdev(v: &[f64]) -> f64 {
    let m = mean(v);
    (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64).sqrt()
}

/// Python `round(x)` (ties to even) as an integer key.
pub(crate) fn py_round(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// Python `round(x, 4)`; the logistic model was fitted on 4-decimal features.
pub(crate) fn round4(x: f64) -> f64 {
    (x * 10_000.0).round_ties_even() / 10_000.0
}

/// `cluster(vals, tol)`: sort, chain values closer than `tol` to the previous one, return cluster means.
pub(crate) fn cluster(vals: &[f64], tol: f64) -> Vec<f64> {
    let mut s = vals.to_vec();
    s.sort_by(f64::total_cmp);
    let mut out: Vec<Vec<f64>> = Vec::new();
    for v in s {
        match out.last_mut() {
            Some(c) if v - c[c.len() - 1] <= tol => c.push(v),
            _ => out.push(vec![v]),
        }
    }
    out.iter()
        .map(|c| c.iter().sum::<f64>() / c.len() as f64)
        .collect()
}

/// Disjoint-set union with path halving (`DSU` in proto.py).
pub(crate) struct Dsu(Vec<usize>);

impl Dsu {
    pub(crate) fn new(n: usize) -> Self {
        Self((0..n).collect())
    }
    pub(crate) fn find(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }
    pub(crate) fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        self.0[ra] = rb;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_semantics() {
        assert_eq!(median(&[3.0, 1.0, 2.0, 10.0]), 2.5);
        assert_eq!(median(&[5.0]), 5.0);
        assert!((pstdev(&[1.0, 2.0, 3.0, 4.0]) - 1.118_033_988_749_895).abs() < 1e-12);
        assert_eq!((py_round(2.5), py_round(3.5), py_round(-0.5)), (2, 4, 0));
        assert_eq!(round4(2.0 / 3.0), 0.6667);
        assert_eq!(cluster(&[10.0, 1.0, 2.5, 4.0, 9.0], 2.0), vec![2.5, 9.5]);
    }

    #[test]
    fn dsu_groups() {
        let mut d = Dsu::new(4);
        d.union(0, 2);
        d.union(2, 3);
        assert_eq!(d.find(0), d.find(3));
        assert_ne!(d.find(1), d.find(0));
    }
}
