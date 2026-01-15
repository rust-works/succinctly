//! Profile the YAML processing pipeline to identify bottlenecks.
//!
//! Run with:
//! ```bash
//! cargo run --release --example yaml_profile -- data/bench/generated/yaml/comprehensive/100kb.yaml
//! ```

use indexmap::IndexMap;
use std::borrow::Cow;
use std::time::Instant;
use succinctly::jq::OwnedValue;
use succinctly::json::JsonIndex;
use succinctly::yaml::{YamlIndex, YamlValue};

/// Convert a YAML value to an OwnedValue (mirrors yq_runner.rs logic)
fn yaml_to_owned_value<W: AsRef<[u64]>>(value: YamlValue<'_, W>) -> OwnedValue {
    match value {
        YamlValue::String(s) => {
            let str_value: Cow<str> = s.as_str().unwrap_or(Cow::Borrowed(""));

            // Try to parse as special YAML values
            match str_value.as_ref() {
                "null" | "~" | "" => return OwnedValue::Null,
                "true" | "True" | "TRUE" => return OwnedValue::Bool(true),
                "false" | "False" | "FALSE" => return OwnedValue::Bool(false),
                _ => {}
            }

            // Try to parse as number
            if let Ok(n) = str_value.parse::<i64>() {
                return OwnedValue::Int(n);
            }
            if let Ok(f) = str_value.parse::<f64>() {
                if !f.is_nan() {
                    return OwnedValue::Float(f);
                }
            }

            // Check for special float literals
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
                    other => {
                        let value = yaml_to_owned_value(other);
                        value.to_json()
                    }
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

    // Step 1: Time parsing (build YAML index)
    let start = Instant::now();
    let index = YamlIndex::build(&data).unwrap();
    let yaml_parse_time = start.elapsed();

    // Step 2: Time YAML to OwnedValue conversion
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
    let yaml_to_owned_time = start.elapsed();

    // Step 3: Time OwnedValue to JSON string conversion
    let start = Instant::now();
    let json_str = owned.to_json();
    let to_json_time = start.elapsed();

    // Step 4: Time JSON parsing (build JSON index)
    let start = Instant::now();
    let json_index = JsonIndex::build(json_str.as_bytes());
    let json_parse_time = start.elapsed();

    // Step 5: Time jq evaluation (identity filter)
    let start = Instant::now();
    let _cursor = json_index.root(json_str.as_bytes());
    // For identity filter, we just access the root - no real work
    let jq_eval_time = start.elapsed();

    // Step 6: Time final JSON output
    let start = Instant::now();
    let _output = owned.to_json(); // This is what gets printed
    let output_time = start.elapsed();

    let total =
        yaml_parse_time + yaml_to_owned_time + to_json_time + json_parse_time + jq_eval_time;
    let total_with_output = total + output_time;

    eprintln!();
    eprintln!("=== Timing breakdown for {:.1} KiB ===", size_kb);
    eprintln!(
        "1. YAML parse (index):   {:>8.2?} ({:>5.1}%)",
        yaml_parse_time,
        100.0 * yaml_parse_time.as_secs_f64() / total.as_secs_f64()
    );
    eprintln!(
        "2. YAML -> OwnedValue:   {:>8.2?} ({:>5.1}%)",
        yaml_to_owned_time,
        100.0 * yaml_to_owned_time.as_secs_f64() / total.as_secs_f64()
    );
    eprintln!(
        "3. OwnedValue -> JSON:   {:>8.2?} ({:>5.1}%)",
        to_json_time,
        100.0 * to_json_time.as_secs_f64() / total.as_secs_f64()
    );
    eprintln!(
        "4. JSON parse (index):   {:>8.2?} ({:>5.1}%)",
        json_parse_time,
        100.0 * json_parse_time.as_secs_f64() / total.as_secs_f64()
    );
    eprintln!(
        "5. jq eval (identity):   {:>8.2?} ({:>5.1}%)",
        jq_eval_time,
        100.0 * jq_eval_time.as_secs_f64() / total.as_secs_f64()
    );
    eprintln!("─────────────────────────────────────");
    eprintln!("Processing total:        {:>8.2?}", total);
    eprintln!("6. Final JSON output:    {:>8.2?}", output_time);
    eprintln!("─────────────────────────────────────");
    eprintln!("Grand total:             {:>8.2?}", total_with_output);
    eprintln!(
        "Throughput:              {:.1} MiB/s",
        (size_kb / 1024.0) / total_with_output.as_secs_f64()
    );

    eprintln!();
    eprintln!("=== Bottleneck Analysis ===");
    let json_roundtrip = to_json_time + json_parse_time;
    eprintln!(
        "JSON round-trip (3+4):   {:>8.2?} ({:.1}% of processing)",
        json_roundtrip,
        100.0 * json_roundtrip.as_secs_f64() / total.as_secs_f64()
    );
    eprintln!("Could save by direct eval: ~{:.2?}", json_roundtrip);
}
