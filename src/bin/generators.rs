//! JSON generators for comprehensive benchmarking.

use super::{ChaCha8Rng, Pattern, Rng, SeedableRng};

/// Generate JSON of approximately target_size bytes
pub fn generate_json(
    target_size: usize,
    pattern: Pattern,
    seed: Option<u64>,
    depth: usize,
    escape_density: f64,
) -> String {
    match pattern {
        Pattern::Comprehensive => {
            generate_comprehensive_json(target_size, seed, depth, escape_density)
        }
        Pattern::Users => generate_users_json(target_size, seed),
        Pattern::Nested => generate_nested_json(target_size, seed, depth),
        Pattern::Arrays => generate_arrays_json(target_size, seed),
        Pattern::Mixed => generate_mixed_json(target_size, seed),
        Pattern::Strings => generate_strings_json(target_size, seed, escape_density),
        Pattern::Numbers => generate_numbers_json(target_size, seed),
        Pattern::Literals => generate_literals_json(target_size, seed),
        Pattern::Unicode => generate_unicode_json(target_size, seed),
        Pattern::Pathological => generate_pathological_json(target_size, seed),
    }
}

/// Generate JSON optimized for comprehensive benchmarking
pub fn generate_comprehensive_json(
    target_size: usize,
    seed: Option<u64>,
    depth: usize,
    escape_density: f64,
) -> String {
    let mut rng = seed.map(|s| ChaCha8Rng::seed_from_u64(s));
    let mut json = String::with_capacity(target_size);

    json.push_str(r#"{"metadata":{"version":"1.0","generated":true,"features":["strings","numbers","booleans","nulls","arrays","objects","nesting","escapes","unicode"]},"#);

    // Distribute target size across different features
    let feature_size = (target_size - 200) / 10; // Reserve 200 for metadata/wrapper

    // 1. Simple values (10%)
    json.push_str(r#""simple_values":{"#);
    add_simple_values(&mut json, feature_size, &mut rng);
    json.push_str("},");

    // 2. String variations with escapes (15%)
    json.push_str(r#""strings":["#);
    add_string_variations(&mut json, feature_size * 3 / 2, escape_density, &mut rng);
    json.push_str("],");

    // 3. Number variations (10%)
    json.push_str(r#""numbers":["#);
    add_number_variations(&mut json, feature_size, &mut rng);
    json.push_str("],");

    // 4. Arrays (nested and flat) (15%)
    json.push_str(r#""arrays":{"#);
    add_array_variations(&mut json, feature_size * 3 / 2, depth, &mut rng);
    json.push_str("},");

    // 5. Nested objects (15%)
    json.push_str(r#""nested":{"#);
    add_nested_objects(&mut json, feature_size * 3 / 2, depth, &mut rng);
    json.push_str("},");

    // 6. Mixed realistic data (20%)
    json.push_str(r#""data":["#);
    add_realistic_records(&mut json, feature_size * 2, &mut rng, escape_density);
    json.push_str("],");

    // 7. Unicode strings (10%)
    json.push_str(r#""unicode":["#);
    add_unicode_strings(&mut json, feature_size, &mut rng);
    json.push_str("],");

    // 8. Edge cases (5%)
    json.push_str(r#""edge_cases":{"#);
    add_edge_cases(&mut json, feature_size / 2);
    json.push_str("}");

    json.push('}');
    json
}

/// Add simple boolean, null, and basic values
fn add_simple_values(json: &mut String, target_size: usize, rng: &mut Option<ChaCha8Rng>) {
    let start_len = json.len();
    let mut count = 0;

    while json.len() - start_len < target_size {
        if count > 0 {
            json.push(',');
        }

        let key = format!("field_{}", count);
        let val = match rng.as_mut().map(|r| r.gen_range(0..4)).unwrap_or(count % 4) {
            0 => "true".to_string(),
            1 => "false".to_string(),
            2 => "null".to_string(),
            _ => count.to_string(),
        };

        json.push_str(&format!(r#""{}":"#, key));
        json.push_str(&val);
        count += 1;
    }
}

/// Add strings with various escape sequences
fn add_string_variations(
    json: &mut String,
    target_size: usize,
    escape_density: f64,
    rng: &mut Option<ChaCha8Rng>,
) {
    let start_len = json.len();
    let mut count = 0;

    let escape_patterns = [
        r#"Line with \"quoted\" text"#,
        r#"Path: C:\\Users\\test\\file.txt"#,
        "Line with\nnewline",
        "Tab\tseparated\tvalues",
        r#"JSON: {\"key\":\"value\"}"#,
        "Backspace:\x08 Formfeed:\x0C",
        "Mixed: \"quotes\"\tand\nnewlines",
    ];

    let normal_strings = [
        "simple string without escapes",
        "email@example.com",
        "https://example.com/path/to/resource",
        "Lorem ipsum dolor sit amet",
        "User Name",
        "2024-01-03T12:34:56Z",
    ];

    while json.len() - start_len < target_size {
        if count > 0 {
            json.push(',');
        }

        json.push('"');

        // Mix escaped and normal strings based on density
        let use_escape = rng
            .as_mut()
            .map(|r| r.r#gen::<f64>() < escape_density)
            .unwrap_or(count % 10 < (escape_density * 10.0) as usize);

        if use_escape {
            let pattern = &escape_patterns[count % escape_patterns.len()];
            json.push_str(pattern);
        } else {
            let pattern = &normal_strings[count % normal_strings.len()];
            json.push_str(pattern);
        }

        json.push('"');
        count += 1;
    }
}

/// Add various number formats
fn add_number_variations(json: &mut String, target_size: usize, rng: &mut Option<ChaCha8Rng>) {
    let start_len = json.len();
    let mut count = 0;

    while json.len() - start_len < target_size {
        if count > 0 {
            json.push(',');
        }

        let num = match count % 8 {
            0 => count.to_string(),                              // Simple integer
            1 => format!("-{}", count),                          // Negative integer
            2 => format!("0.{}", count),                         // Decimal < 1
            3 => format!("{}.{}", count, count),                 // Decimal
            4 => format!("{}e{}", count, count % 10),            // Scientific notation
            5 => format!("-{}.{}e-{}", count, count, count % 5), // Negative scientific
            6 => "0".to_string(),                                // Zero
            _ => {
                let val = rng
                    .as_mut()
                    .map(|r| r.r#gen::<f64>() * 1000000.0 - 500000.0)
                    .unwrap_or(count as f64);
                format!("{:.6}", val)
            }
        };

        json.push_str(&num);
        count += 1;
    }
}

/// Add various array structures
fn add_array_variations(
    json: &mut String,
    target_size: usize,
    depth: usize,
    rng: &mut Option<ChaCha8Rng>,
) {
    let start_len = json.len();

    // Empty array
    json.push_str(r#""empty":[],"#);

    // Flat arrays
    json.push_str(r#""flat_numbers":["#);
    for i in 0..20 {
        if i > 0 {
            json.push(',');
        }
        json.push_str(&i.to_string());
    }
    json.push_str(r#"],"flat_strings":["#);
    for i in 0..15 {
        if i > 0 {
            json.push(',');
        }
        json.push_str(&format!(r#""item_{}""#, i));
    }
    json.push_str(r#"],"#);

    // Nested arrays
    json.push_str(r#""nested":["#);
    let mut level = 0;
    while json.len() - start_len < target_size && level < depth {
        if level > 0 {
            json.push(',');
        }
        json.push('[');
        for i in 0..3 {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!("{}", level * 10 + i));
        }
        json.push(']');
        level += 1;
    }
    json.push_str(r#"],"#);

    // Mixed type arrays
    json.push_str(r#""mixed":[1,"two",true,null,{"key":"value"},[1,2,3]]"#);

    // Fill remaining space with random arrays
    while json.len() - start_len < target_size {
        json.push_str(r#","random":["#);
        let array_len = rng.as_mut().map(|r| r.gen_range(3..10)).unwrap_or(5);
        for i in 0..array_len {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&i.to_string());
            if json.len() - start_len >= target_size {
                break;
            }
        }
        json.push(']');
    }
}

/// Add nested object structures
fn add_nested_objects(
    json: &mut String,
    target_size: usize,
    depth: usize,
    rng: &mut Option<ChaCha8Rng>,
) {
    let start_len = json.len();

    json.push_str(r#""shallow":{"a":1,"b":2,"c":3},"#);

    json.push_str(r#""deep":"#);
    for level in 0..depth.min(20) {
        json.push_str(&format!(r#"{{"level_{}":"#, level));
    }
    json.push_str(r#""bottom""#);
    for _ in 0..depth.min(20) {
        json.push('}');
    }
    json.push(',');

    json.push_str(r#""tree":{"#);
    add_tree_structure(
        json,
        (target_size - (json.len() - start_len)) / 2,
        0,
        depth.min(10),
        rng,
    );
    json.push('}');
}

/// Add tree-like nested structure
fn add_tree_structure(
    json: &mut String,
    remaining: usize,
    current_depth: usize,
    max_depth: usize,
    rng: &mut Option<ChaCha8Rng>,
) {
    if current_depth >= max_depth || remaining < 50 {
        json.push_str(r#""leaf":true"#);
        return;
    }

    let num_children = rng.as_mut().map(|r| r.gen_range(2..5)).unwrap_or(3);

    for i in 0..num_children {
        if i > 0 {
            json.push(',');
        }
        json.push_str(&format!(r#""child_{}":{{"#, i));
        add_tree_structure(
            json,
            remaining / num_children,
            current_depth + 1,
            max_depth,
            rng,
        );
        json.push('}');
    }
}

/// Add realistic record structures
fn add_realistic_records(
    json: &mut String,
    target_size: usize,
    rng: &mut Option<ChaCha8Rng>,
    escape_density: f64,
) {
    let start_len = json.len();
    let mut count = 0;

    while json.len() - start_len < target_size {
        if count > 0 {
            json.push(',');
        }

        let age = rng.as_mut().map(|r| r.gen_range(18..80)).unwrap_or(25);
        let score = rng
            .as_mut()
            .map(|r| r.gen_range(0..1000))
            .unwrap_or(count * 10);
        let active = rng
            .as_mut()
            .map(|r| r.r#gen::<bool>())
            .unwrap_or(count % 2 == 0);
        let salary = rng
            .as_mut()
            .map(|r| r.r#gen::<f64>() * 200000.0)
            .unwrap_or(50000.0);

        // Add escaped characters to some fields
        let name = if rng
            .as_mut()
            .map(|r| r.r#gen::<f64>() < escape_density)
            .unwrap_or(false)
        {
            format!(r#"User \"{}\" Smith"#, count)
        } else {
            format!("User{}", count)
        };

        json.push_str(&format!(
            r#"{{"id":{},"name":"{}","email":"user{}@example.com","age":{},"active":{},"score":{},"salary":{:.2},"tags":["tag{}","tag{}"],"metadata":{{"created":"2024-01-0{}","role":"user"}}}}"#,
            count, name, count, age, active, score, salary, count % 10, (count + 1) % 10, (count % 9) + 1
        ));

        count += 1;
    }
}

/// Add Unicode strings (various scripts and emoji)
fn add_unicode_strings(json: &mut String, target_size: usize, _rng: &mut Option<ChaCha8Rng>) {
    let start_len = json.len();

    let unicode_samples = [
        "Hello 世界",        // Chinese
        "Привет мир",        // Russian
        "مرحبا بالعالم",     // Arabic
        "Hello 🌍🌎🌏",      // Emoji
        "Café ☕",           // Latin + emoji
        "日本語テキスト",    // Japanese
        "한글 텍스트",       // Korean
        "Γεια σου κόσμε",    // Greek
        "שלום עולם",         // Hebrew
        "नमस्ते दुनिया",        // Hindi
        "Emoji: 😀🎉🚀💻🔥", // Multiple emoji
        "Math: ∑∫∂∇√∞",      // Math symbols
        "Arrows: ←↑→↓↔↕",    // Arrows
    ];

    let mut count = 0;
    while json.len() - start_len < target_size {
        if count > 0 {
            json.push(',');
        }

        json.push('"');
        json.push_str(unicode_samples[count % unicode_samples.len()]);
        json.push('"');
        count += 1;
    }
}

/// Add edge cases that stress parsers
fn add_edge_cases(json: &mut String, _target_size: usize) {
    json.push_str(r#""empty_string":"","#);
    json.push_str(r#""empty_object":{},"#);
    json.push_str(r#""empty_array":[],"#);
    json.push_str(r#""zero":0,"#);
    json.push_str(r#""negative_zero":-0,"#);
    json.push_str(r#""very_long_string":""#);
    for _ in 0..100 {
        json.push('x');
    }
    json.push_str(r#"","#);
    json.push_str(r#""all_escapes":"\"\\\/\b\f\n\r\t","#);
    json.push_str(r#""consecutive_commas":[1,2,3,4,5,6,7,8,9,10],"#);
    json.push_str(r#""whitespace_heavy":"  lots   of   spaces  ","#);
    json.push_str(r#""numbers":{"int":42,"neg":-42,"decimal":3.14,"exp":1e10,"neg_exp":1e-10}"#);
}

/// Generate user objects pattern
pub fn generate_users_json(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(|s| ChaCha8Rng::seed_from_u64(s));
    let mut json = String::with_capacity(target_size);
    json.push_str(r#"{"users":["#);

    // Each user ~150 bytes
    let num_users = target_size / 150;

    for i in 0..num_users {
        if i > 0 {
            json.push(',');
        }

        let age = rng.as_mut().map(|r| r.gen_range(18..80)).unwrap_or(25);
        let score = rng.as_mut().map(|r| r.gen_range(0..1000)).unwrap_or(i * 10);

        json.push_str(&format!(
            r#"{{"id":{},"name":"User{}","email":"user{}@example.com","age":{},"active":true,"score":{}}}"#,
            i, i, i, age, score
        ));

        // Stop if we've exceeded target size
        if json.len() >= target_size - 10 {
            break;
        }
    }

    json.push_str("]}");
    json
}

/// Generate deeply nested objects
pub fn generate_nested_json(target_size: usize, _seed: Option<u64>, depth: usize) -> String {
    let mut json = String::with_capacity(target_size);
    let actual_depth = depth.min((target_size as f64).log2() as usize);

    for i in 0..actual_depth {
        json.push_str(r#"{"level":"#);
        json.push_str(&i.to_string());
        json.push_str(r#"","data":"#);
    }

    // Fill remaining space
    while json.len() < target_size - 100 {
        json.push_str("padding");
    }

    for _ in 0..actual_depth {
        json.push_str("\"}");
    }

    json
}

/// Generate arrays of arrays
pub fn generate_arrays_json(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(|s| ChaCha8Rng::seed_from_u64(s));
    let mut json = String::with_capacity(target_size);
    json.push('[');

    let num_elements = target_size / 20;
    for i in 0..num_elements {
        if i > 0 {
            json.push(',');
        }

        json.push('[');
        for j in 0..5 {
            if j > 0 {
                json.push(',');
            }
            let val = rng.as_mut().map(|r| r.gen_range(0..1000)).unwrap_or(j);
            json.push_str(&val.to_string());
        }
        json.push(']');

        if json.len() >= target_size - 10 {
            break;
        }
    }

    json.push(']');
    json
}

/// Generate mixed types
pub fn generate_mixed_json(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(|s| ChaCha8Rng::seed_from_u64(s));
    let mut json = String::with_capacity(target_size);
    json.push_str(r#"{"data":["#);

    let patterns = ["string", "number", "bool", "null", "array", "object"];
    let num_items = target_size / 100;

    for i in 0..num_items {
        if i > 0 {
            json.push(',');
        }

        let pattern_idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..patterns.len()))
            .unwrap_or(i % patterns.len());

        match patterns[pattern_idx] {
            "string" => json.push_str(&format!(r#""value_{}""#, i)),
            "number" => json.push_str(&i.to_string()),
            "bool" => json.push_str(if i % 2 == 0 { "true" } else { "false" }),
            "null" => json.push_str("null"),
            "array" => json.push_str(&format!("[{},{},{}]", i, i + 1, i + 2)),
            "object" => json.push_str(&format!(r#"{{"key":"val_{}"}}"#, i)),
            _ => {}
        }

        if json.len() >= target_size - 10 {
            break;
        }
    }

    json.push_str("]}");
    json
}

/// Generate string-heavy JSON
pub fn generate_strings_json(
    target_size: usize,
    _seed: Option<u64>,
    _escape_density: f64,
) -> String {
    let mut json = String::with_capacity(target_size);
    json.push_str(r#"{"strings":["#);

    let lorem = "Lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua";
    let num_strings = target_size / (lorem.len() + 20);

    for i in 0..num_strings {
        if i > 0 {
            json.push(',');
        }
        json.push('"');
        json.push_str(lorem);
        json.push('"');

        if json.len() >= target_size - 10 {
            break;
        }
    }

    json.push_str("]}");
    json
}

/// Generate number-heavy JSON
pub fn generate_numbers_json(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(|s| ChaCha8Rng::seed_from_u64(s));
    let mut json = String::with_capacity(target_size);
    json.push_str(r#"{"numbers":["#);

    let num_count = target_size / 15; // ~15 bytes per number

    for i in 0..num_count {
        if i > 0 {
            json.push(',');
        }

        let num = rng
            .as_mut()
            .map(|r| r.r#gen::<f64>() * 1000000.0)
            .unwrap_or(i as f64 * 3.14159);

        json.push_str(&format!("{:.6}", num));

        if json.len() >= target_size - 10 {
            break;
        }
    }

    json.push_str("]}");
    json
}

/// Generate literal-heavy JSON (booleans and nulls)
pub fn generate_literals_json(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(|s| ChaCha8Rng::seed_from_u64(s));
    let mut json = String::with_capacity(target_size);
    json.push_str(r#"{"literals":["#);

    let mut count = 0;
    while json.len() < target_size - 10 {
        if count > 0 {
            json.push(',');
        }

        let val = match rng.as_mut().map(|r| r.gen_range(0..3)).unwrap_or(count % 3) {
            0 => "true",
            1 => "false",
            _ => "null",
        };

        json.push_str(val);
        count += 1;
    }

    json.push_str("]}");
    json
}

/// Generate Unicode-heavy JSON
pub fn generate_unicode_json(target_size: usize, seed: Option<u64>) -> String {
    let mut json = String::with_capacity(target_size);
    add_unicode_strings(
        &mut json,
        target_size,
        &mut seed.map(|s| ChaCha8Rng::seed_from_u64(s)),
    );
    format!(r#"{{"unicode":[{}]}}"#, json)
}

/// Generate pathological JSON (worst-case for parsing)
pub fn generate_pathological_json(target_size: usize, _seed: Option<u64>) -> String {
    let mut json = String::with_capacity(target_size);

    // Maximum structural character density
    // Pattern: {"a":{"b":{"c":[[[1]]],"d":true},"e":null},"f":""}
    let pattern = r#"{"a":{"b":{"c":[[[1]]],"d":true},"e":null},"f":""}"#;
    let pattern_len = pattern.len();

    json.push('[');
    let repetitions = target_size / pattern_len;
    for i in 0..repetitions {
        if i > 0 {
            json.push(',');
        }
        json.push_str(pattern);

        if json.len() >= target_size - 10 {
            break;
        }
    }
    json.push(']');
    json
}
