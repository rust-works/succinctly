//! A port of Go's `sort.Stable` (`sort/zsortinterface.go`, Go 1.24), for yq's `sort`/`sort_by`
//! (#2799).
//!
//! yq's comparator (`sortableNodeArray.compare`, `pkg/yqlib/operator_sort.go`) is **not a strict
//! weak order**: two integers compare numerically, but an integer against a string compares by
//! the text, so `2 < 3` and `"10" < 2` and `3 < "10"` can all hold at once. A sort over such a
//! relation has no unique answer; what comes out depends on the algorithm. Go's `sort.Stable`
//! (insertion sort on blocks of 20, then `symMerge` of doubling blocks) is the one yq uses, so
//! this is the one that reproduces yq's output for a mixed-type array. Rust's own
//! `slice::sort_by` is a different algorithm and, on a relation like this, may order differently
//! and (since Rust 1.81) may panic on the inconsistency.
//!
//! Only the algorithm is ported, not Go's `sort.Interface`: `less(a, b)` takes the two elements
//! and the slice is permuted with `swap`, which is what the Go code does through `data.Swap`.

use core::cmp::Ordering;

/// Sort `items` stably by `compare`, in the order Go's `sort.Stable` would, with `Less(i, j)`
/// being `compare(&items[i], &items[j]) == Ordering::Less`.
pub(crate) fn stable_sort_by<T>(items: &mut [T], mut compare: impl FnMut(&T, &T) -> Ordering) {
    let n = items.len();
    let mut less = |data: &[T], i: usize, j: usize| compare(&data[i], &data[j]) == Ordering::Less;
    stable(items, n, &mut less);
}

/// `sort.stable`.
fn stable<T>(data: &mut [T], n: usize, less: &mut impl FnMut(&[T], usize, usize) -> bool) {
    let mut block_size = 20; // must be > 0
    let (mut a, mut b) = (0, block_size);
    while b <= n {
        insertion_sort(data, a, b, less);
        a = b;
        b += block_size;
    }
    insertion_sort(data, a, n, less);

    while block_size < n {
        a = 0;
        b = 2 * block_size;
        while b <= n {
            sym_merge(data, a, a + block_size, b, less);
            a = b;
            b += 2 * block_size;
        }
        let m = a + block_size;
        if m < n {
            sym_merge(data, a, m, n, less);
        }
        block_size *= 2;
    }
}

/// `sort.insertionSort`.
fn insertion_sort<T>(
    data: &mut [T],
    a: usize,
    b: usize,
    less: &mut impl FnMut(&[T], usize, usize) -> bool,
) {
    let mut i = a + 1;
    while i < b {
        let mut j = i;
        while j > a && less(data, j, j - 1) {
            data.swap(j, j - 1);
            j -= 1;
        }
        i += 1;
    }
}

/// `sort.symMerge`: merge the sorted runs `data[a..m]` and `data[m..b]` in place.
fn sym_merge<T>(
    data: &mut [T],
    a: usize,
    m: usize,
    b: usize,
    less: &mut impl FnMut(&[T], usize, usize) -> bool,
) {
    // A one-element left run: binary search for its position and rotate it there.
    if m - a == 1 {
        let (mut i, mut j) = (m, b);
        while i < j {
            let h = (i + j) >> 1;
            if less(data, h, a) {
                i = h + 1;
            } else {
                j = h;
            }
        }
        // Swap values until data[a] reaches the position before i.
        let mut k = a;
        while k + 1 < i {
            data.swap(k, k + 1);
            k += 1;
        }
        return;
    }
    // A one-element right run.
    if b - m == 1 {
        let (mut i, mut j) = (a, m);
        while i < j {
            let h = (i + j) >> 1;
            if !less(data, m, h) {
                i = h + 1;
            } else {
                j = h;
            }
        }
        // Swap values until data[m] reaches the position i.
        let mut k = m;
        while k > i {
            data.swap(k, k - 1);
            k -= 1;
        }
        return;
    }

    let mid = (a + b) >> 1;
    let n = mid + m;
    let (mut start, mut r) = if m > mid { (n - b, mid) } else { (a, m) };
    let p = n - 1;

    while start < r {
        let c = (start + r) >> 1;
        if !less(data, p - c, c) {
            start = c + 1;
        } else {
            r = c;
        }
    }

    let end = n - start;
    if start < m && m < end {
        rotate(data, start, m, end);
    }
    if a < start && start < mid {
        sym_merge(data, a, start, mid, less);
    }
    if mid < end && end < b {
        sym_merge(data, mid, end, b, less);
    }
}

/// `sort.rotate`: rotate `data[a..b]` so that `data[m..b]` comes first, by block swaps.
fn rotate<T>(data: &mut [T], a: usize, m: usize, b: usize) {
    let mut i = m - a;
    let mut j = b - m;
    while i != j {
        if i > j {
            swap_range(data, m - i, m, j);
            i -= j;
        } else {
            swap_range(data, m - i, m + j - i, i);
            j -= i;
        }
    }
    // i == j
    swap_range(data, m - i, m, i);
}

/// `sort.swapRange`.
fn swap_range<T>(data: &mut [T], a: usize, b: usize, n: usize) {
    for i in 0..n {
        data.swap(a + i, b + i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// A total order: the result must be the sorted, stable permutation, whatever the length
    /// (this crosses the 20-element block boundary and several `symMerge` levels).
    #[test]
    fn sorts_a_total_order_stably() {
        for len in [0usize, 1, 2, 3, 19, 20, 21, 39, 40, 41, 100, 257, 1000] {
            // (key, original index): a small key range forces many ties.
            let mut items: Vec<(u32, usize)> = (0..len)
                .map(|i| (((i as u32).wrapping_mul(2654435761) >> 7) % 13, i))
                .collect();
            let mut expected = items.clone();
            expected.sort_by_key(|&(key, _)| key); // Rust's sort is stable too
            stable_sort_by(&mut items, |a, b| a.0.cmp(&b.0));
            assert_eq!(items, expected, "len {len}");
        }
    }

    /// yq's comparator is not transitive, and Go's `sort.Stable` still terminates and returns a
    /// permutation. The expected strings were captured from yq v4.53.3 (`sort` on the same
    /// input), which is what pins the port to Go's algorithm rather than to "a" stable sort.
    #[test]
    fn matches_go_on_a_non_transitive_relation() {
        // yq's compare for texts and integers: two integers numerically, otherwise by text.
        #[derive(Clone, Debug, PartialEq)]
        enum V {
            Int(i64),
            Str(&'static str),
        }
        fn text(v: &V) -> alloc::string::String {
            match v {
                V::Int(n) => alloc::format!("{n}"),
                V::Str(s) => (*s).into(),
            }
        }
        fn cmp(a: &V, b: &V) -> Ordering {
            match (a, b) {
                (V::Int(x), V::Int(y)) => x.cmp(y),
                _ => text(a).cmp(&text(b)),
            }
        }
        // `[2, "10", 3] | sort` in yq is `["10",2,3]`.
        let mut v = [V::Int(2), V::Str("10"), V::Int(3)];
        stable_sort_by(&mut v, cmp);
        assert_eq!(v, [V::Str("10"), V::Int(2), V::Int(3)]);
        // A longer mixed array stays a permutation of the input.
        let mut many: Vec<V> = (0..64)
            .map(|i| {
                if i % 3 == 0 {
                    V::Str("7")
                } else {
                    V::Int(i % 11)
                }
            })
            .collect();
        let before = many.len();
        stable_sort_by(&mut many, cmp);
        assert_eq!(many.len(), before);
    }
}
