//! Profile the YAML processing pipeline to compare old vs new approach.
//!
//! Run with:
//! ```bash
//! cargo run --release --example yaml_profile -- data/bench/generated/yaml/comprehensive/100kb.yaml
//! ```

use indexmap::IndexMap;
use std::borrow::Cow;
use std::time::Instant;
use succinctly::jq::OwnedValue;
use succinctly::yaml::{YamlIndex, YamlValue};

/// OLD approach: Convert a YAML value to an OwnedValue
fn yaml_to_owned_value<W: AsRef<[u64]>>(value: YamlValue<'_, W>) -> OwnedValue {
    match value {
        YamlValue::String(s) => {
            let str_value: Cow<str> = s.as_str().unwrap_or(Cow::Borrowed(""));
            match str_value.as_ref() {
                "null" | "~" | "" => return OwnedValue::Null,
                "true" | "True" | "TRUE" => return OwnedValue::Bool(true),
                "false" | "False" | "FALSE" => return OwnedValue::Bool(false),
                _ => {}
            }
            if let Ok(n) = str_value.parse::<i64>() {
                return OwnedValue::Int(n);
            }
            if let Ok(f) = str_value.parse::<f64>() {
                if !f.is_nan() {
                    return OwnedValue::Float(f);
                }
            }
            match str_value.as_ref() {
                ".inf" | ".Inf" | ".INF" => return OwnedValue::Float(f64::INFINITY),
                "-.inf" | "-.Inf" | "-.INF" => return OwnedValue::Float(f64::NEG_INFINITY),
                ".nan" | ".NaN" | ".NAN" => return OwnedValue::Float(f64::NAN),
                _ => {}
            }
            OwnedValue::String(str_value.into_owned())
        }
        YamlValue::Mapping(fields) => {
            let mut map = IndexMap::new();
            for field in fields {
                let key = match field.key() {
                    YamlValue::String(s) => s.as_str().unwrap_or(Cow::Borrowed("")).into_owned(),
                    other => yaml_to_owned_value(other).to_json(),
                };
                let value = yaml_to_owned_value(field.value());
                map.insert(key, value);
            }
            OwnedValue::Object(map)
        }
        YamlValue::Sequence(elements) => {
            let mut arr = Vec::new();
            for elem in elements {
                arr.push(yaml_to_owned_value(elem));
            }
            OwnedValue::Array(arr)
        }
        YamlValue::Alias { target, .. } => {
            if let Some(target_cursor) = target {
                yaml_to_owned_value(target_cursor.value())
            } else {
                OwnedValue::Null
            }
        }
        YamlValue::Error(_) | YamlValue::Null => OwnedValue::Null,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: cargo run --release --example yaml_profile <yaml_file>");
        std::process::exit(1);
    }

    let data = std::fs::read(&args[1]).unwrap();
    let size_kb = data.len() as f64 / 1024.0;

    // Warm up
    let _ = YamlIndex::build(&data);

    // ========== OLD APPROACH ==========
    let start = Instant::now();
    let index = YamlIndex::build(&data).unwrap();
    let old_parse_time = start.elapsed();

    let start = Instant::now();
    let root = index.root(&data);
    let owned = match root.value() {
        YamlValue::Sequence(docs) => {
            let mut values = Vec::new();
            for doc in docs {
                values.push(yaml_to_owned_value(doc));
            }
            if values.len() == 1 {
                values.pop().unwrap()
            } else {
                OwnedValue::Array(values)
            }
        }
        other => yaml_to_owned_value(other),
    };
    let old_convert_time = start.elapsed();

    let start = Instant::now();
    let old_json = owned.to_json();
    let old_serialize_time = start.elapsed();

    let old_total = old_parse_time + old_convert_time + old_serialize_time;

    // ========== NEW APPROACH ==========
    let start = Instant::now();
    let index = YamlIndex::build(&data).unwrap();
    let new_parse_time = start.elapsed();

    let start = Instant::now();
    let root = index.root(&data);
    let new_json = root.to_json_document();
    let new_convert_time = start.elapsed();

    let new_total = new_parse_time + new_convert_time;

    // ========== RESULTS ==========
    eprintln!();
    eprintln!("=== Comparison for {:.1} KiB ===", size_kb);
    eprintln!();
    eprintln!("OLD (via OwnedValue):");
    eprintln!("  Parse:     {:>8.2?}", old_parse_time);
    eprintln!("  Convert:   {:>8.2?}", old_convert_time);
    eprintln!("  Serialize: {:>8.2?}", old_serialize_time);
    eprintln!(
        "  Total:     {:>8.2?}  ({:.1} MiB/s)",
        old_total,
        (size_kb / 1024.0) / old_total.as_secs_f64()
    );
    eprintln!();
    eprintln!("NEW (direct cursor → JSON):");
    eprintln!("  Parse:     {:>8.2?}", new_parse_time);
    eprintln!("  Convert:   {:>8.2?}", new_convert_time);
    eprintln!(
        "  Total:     {:>8.2?}  ({:.1} MiB/s)",
        new_total,
        (size_kb / 1024.0) / new_total.as_secs_f64()
    );
    eprintln!();
    eprintln!(
        "Speedup: {:.2}x",
        old_total.as_secs_f64() / new_total.as_secs_f64()
    );
    eprintln!();

    // Verify outputs match
    if old_json == new_json {
        eprintln!("✓ Output matches ({} bytes)", old_json.len());
    } else {
        eprintln!("✗ Output MISMATCH!");
        eprintln!("  Old: {} bytes", old_json.len());
        eprintln!("  New: {} bytes", new_json.len());

        // Find first difference
        let old_bytes = old_json.as_bytes();
        let new_bytes = new_json.as_bytes();
        for i in 0..old_bytes.len().min(new_bytes.len()) {
            if old_bytes[i] != new_bytes[i] {
                let start = i.saturating_sub(20);
                let end = (i + 40).min(old_bytes.len()).min(new_bytes.len());
                eprintln!("  First diff at byte {}", i);
                eprintln!(
                    "  Old[{}..{}]: {:?}",
                    start,
                    end,
                    &old_json[start..end.min(old_json.len())]
                );
                eprintln!(
                    "  New[{}..{}]: {:?}",
                    start,
                    end,
                    &new_json[start..end.min(new_json.len())]
                );
                break;
            }
        }
    }
}
