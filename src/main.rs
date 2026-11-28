//! The `succinctly` binary: the CLI lives in the library as `succinctly::cli` (#7384).

/// The system allocator, counting the heap a run of the jq call machine holds, which its byte
/// budget bounds (#8064). Outside such a run it reads one atomic per allocation and counts
/// nothing.
#[global_allocator]
static ALLOCATOR: succinctly::jq::path_mode::MeteredAlloc = succinctly::jq::path_mode::MeteredAlloc;

fn main() -> anyhow::Result<()> {
    succinctly::cli::main()
}
