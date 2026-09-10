//! Peak-RSS attribution harness for #1588's `KeyHashes` growth path.
//!
//! One process per regime; peak RSS is read from the outside
//! (`/usr/bin/time -l` on macOS, `-v` on Linux), so the process must do
//! nothing else. Distinguishes two candidate mechanisms for the streaming
//! duplicate-key detector's memory overhead:
//!
//! - *last transient only* -- `grow()` holds the old and new tables at the
//!   same moment, so peak is 1.5x the final table however many doublings
//!   preceded it. Predicts `half` ~= `grow`.
//! - *dead intermediates* -- every intermediate table's pages stay resident
//!   after it is freed, so peak is ~2x the final table. Predicts
//!   `grow` > `half` > `exact`, each step about half a table apart.
//!
//! `vec` is the batch shape the wide-object path would switch to: push into
//! a `Vec<u64>` and sort. Measured here because the fix's predicted landing
//! depends on whether `Vec`'s own doubling retains dead buffers the way
//! `KeyHashes::grow`'s fresh `vec![0; n]` does.
//!
//! Usage: `keyhashes_rss <exact|half|quarter|grow|vec> [keys]`
use succinctly::jq::document::KeyHashes;

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "grow".into());
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(629_881);

    if mode == "vec" {
        let mut hashes: Vec<u64> = Vec::new();
        for z in Splitmix::default().take(n) {
            hashes.push(z);
        }
        hashes.sort_unstable();
        let repeats = hashes.windows(2).filter(|p| p[0] == p[1]).count();
        println!(
            "mode={mode} keys={n} distinct={} collisions={repeats}",
            hashes.len()
        );
        return;
    }

    let mut table = match mode.as_str() {
        "exact" => KeyHashes::with_capacity(n),
        "half" => KeyHashes::with_capacity(n / 2),
        "quarter" => KeyHashes::with_capacity(n / 4),
        "grow" => KeyHashes::new(),
        other => panic!("unknown mode {other}"),
    };

    let mut collisions = 0usize;
    for z in Splitmix::default().take(n) {
        if table.insert(z) {
            collisions += 1;
        }
    }
    println!(
        "mode={mode} keys={n} distinct={} collisions={collisions}",
        table.len()
    );
}

/// splitmix64: distinct, well-spread hashes without pulling in a dependency.
#[derive(Default)]
struct Splitmix(u64);

impl Iterator for Splitmix {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Some(z ^ (z >> 31))
    }
}
