//! #7264: the first chunk of the lazy scan is small and each later one doubles to the cap, so a
//! program that stops after one document over a file scans little more than it reads. The answers
//! must not depend on where the chunks fall: a stray `}` placed before, at and past each boundary of
//! the ramp is met by the chunk that holds it, and jq resumes after it as it did when the first
//! chunk was 64 KiB. The expected text of the fault rows was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// Records of about 40 bytes, so that the file is past the lone-source prefix the lazy start reads
/// (256 KiB) and the scan has to ramp through every chunk size from the first to the cap.
const RECORDS: usize = 12_000;

fn record(id: usize) -> String {
    format!(
        "{{\"id\":{id},\"name\":\"user-{id:06}\",\"s\":{}}}\n",
        id % 97
    )
}

fn stream(fault_at: Option<usize>) -> String {
    (0..RECORDS)
        .map(|id| {
            if fault_at == Some(id) {
                "}\n".to_string()
            } else {
                record(id)
            }
        })
        .collect()
}

#[test]
fn first_of_inputs_over_a_file_past_the_prefix_is_the_first_record_7264() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("stream.ndjson");
    let file = path.to_string_lossy().into_owned();
    std::fs::write(&path, stream(None))?;
    assert!(std::fs::metadata(&path)?.len() > 256 * 1024);
    for (program, want) in [
        ("first(inputs)|.id", "0\n"),
        ("[limit(3; inputs)|.id]", "[0,1,2]\n"),
        ("[limit(300; inputs)|.id]|[length, .[-1]]", "[300,299]\n"),
        ("[limit(5000; inputs)|.id]|[length, .[-1]]", "[5000,4999]\n"),
        ("reduce inputs as $r (0; . + 1)", "12000\n"),
        ("last(inputs)|.id", "11999\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program, &file], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, "", 0),
            "{program}"
        );
    }
    Ok(())
}

/// What jq 1.7.1 prints for every read of the stream with one `}` in it: the count of documents
/// read, and each error met (the `}`, then the reads past the end, which jq 1.7.1 words as `break`).
fn after_fault(fault_line: usize) -> String {
    format!(
        "{{\"n\":{},\"e\":[\"Unmatched '}}' at line {fault_line}, column 1\",\"break\",\"break\"]}}\n",
        RECORDS - 1
    )
}

#[test]
fn a_stray_brace_at_any_chunk_boundary_is_met_as_jq_meets_it_7264() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("fault.ndjson");
    let file = path.to_string_lossy().into_owned();
    let program = format!(
        "reduce range(0; {}) as $i ({{n:0,e:[]}}; (try (input|1) catch .) as $r | if $r == 1 then .n += 1 else .e += [$r] end)",
        RECORDS + 2
    );
    // Lines near the byte offsets 1 KiB .. 512 KiB, where a ramp from 1-4 KiB doubles, and in the
    // first and last records.
    for fault_at in [
        0, 1, 24, 25, 26, 49, 50, 51, 100, 101, 102, 205, 410, 411, 820, 1640, 3280, 6000, 6560,
        9000, 11_998, 11_999,
    ] {
        std::fs::write(&path, stream(Some(fault_at)))?;
        let (stdout, stderr, code) = run_jq_full(&["-nc", &program, &file], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (after_fault(fault_at + 1).as_str(), "", 0),
            "fault at record {fault_at}"
        );
    }
    Ok(())
}

/// Documents bigger than the 16 KiB a long file is first read on (and than the 4 KiB first chunk) do
/// not fit in that start: it reads on to the 256 KiB front, and a document bigger than that too is
/// read by the route that reads the file whole. Each answers as it did.
#[test]
fn documents_bigger_than_the_start_are_read_as_before_7264() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("big.ndjson");
    let file = path.to_string_lossy().into_owned();
    for (size, count) in [(20_000usize, 40usize), (100_000, 8), (300_000, 3)] {
        let stream: String = (0..count)
            .map(|id| format!("{{\"id\":{id},\"pad\":\"{}\"}}\n", "x".repeat(size)))
            .collect();
        std::fs::write(&path, stream)?;
        for (program, want) in [
            ("first(inputs)|.id", "0\n".to_string()),
            ("[inputs|.id]|length", format!("{count}\n")),
            ("last(inputs)|.id", format!("{}\n", count - 1)),
        ] {
            let (stdout, stderr, code) = run_jq_full(&["-n", program, &file], None)?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                (want.as_str(), "", 0),
                "{program} over {count} documents of {size} bytes"
            );
        }
    }
    Ok(())
}
