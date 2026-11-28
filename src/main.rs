//! The `succinctly` binary: the CLI lives in the library as `succinctly::cli` (#7384).

fn main() -> anyhow::Result<()> {
    succinctly::cli::main()
}
