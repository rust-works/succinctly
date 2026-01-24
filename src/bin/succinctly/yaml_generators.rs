//! YAML generators for benchmarking and testing.
//!
//! Generates YAML-lite compatible content (block style only, no flow style).

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[derive(Debug, Clone, Copy)]
pub enum YamlPattern {
    /// Comprehensive pattern testing various YAML features
    Comprehensive,
    /// Array of user/person records (realistic structure)
    Users,
    /// Deeply nested mappings and sequences
    Nested,
    /// Sequences at various levels
    Sequences,
    /// Mixed mappings and sequences
    Mixed,
    /// String-heavy with quoted strings
    Strings,
    /// Numeric values (integers and floats)
    Numbers,
    /// Configuration file style (realistic config)
    Config,
    /// Unicode strings in various scripts
    Unicode,
    /// Worst case for parsing (maximum depth and density)
    Pathological,
    /// Top-level array designed for M2 navigation benchmarks
    /// (supports .[0], .[], .[0].name queries)
    Navigation,
}

/// Generate YAML of approximately target_size bytes
pub fn generate_yaml(target_size: usize, pattern: YamlPattern, seed: Option<u64>) -> String {
    match pattern {
        YamlPattern::Comprehensive => generate_comprehensive(target_size, seed),
        YamlPattern::Users => generate_users(target_size, seed),
        YamlPattern::Nested => generate_nested(target_size, seed),
        YamlPattern::Sequences => generate_sequences(target_size, seed),
        YamlPattern::Mixed => generate_mixed(target_size, seed),
        YamlPattern::Strings => generate_strings(target_size, seed),
        YamlPattern::Numbers => generate_numbers(target_size, seed),
        YamlPattern::Config => generate_config(target_size, seed),
        YamlPattern::Unicode => generate_unicode(target_size, seed),
        YamlPattern::Pathological => generate_pathological(target_size, seed),
        YamlPattern::Navigation => generate_navigation(target_size, seed),
    }
}

/// Comprehensive YAML with various features
fn generate_comprehensive(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut yaml = String::with_capacity(target_size);

    // Metadata section
    yaml.push_str("# Comprehensive YAML test file\n");
    yaml.push_str("metadata:\n");
    yaml.push_str("  version: \"1.0\"\n");
    yaml.push_str("  generated: true\n");
    yaml.push_str("  features:\n");
    yaml.push_str("    - mappings\n");
    yaml.push_str("    - sequences\n");
    yaml.push_str("    - strings\n");
    yaml.push_str("    - numbers\n");
    yaml.push_str("    - nested\n");
    yaml.push('\n');

    // Calculate feature sizes
    let remaining = target_size.saturating_sub(yaml.len());
    let feature_size = remaining / 6;

    // Simple values section
    yaml.push_str("simple_values:\n");
    add_simple_values(&mut yaml, feature_size, &mut rng, 2);
    yaml.push('\n');

    // Strings section
    yaml.push_str("strings:\n");
    add_string_values(&mut yaml, feature_size, &mut rng, 2);
    yaml.push('\n');

    // Numbers section
    yaml.push_str("numbers:\n");
    add_number_values(&mut yaml, feature_size, &mut rng, 2);
    yaml.push('\n');

    // Nested section (limit depth to avoid BP tree overflow for larger documents)
    yaml.push_str("nested:\n");
    add_nested_mapping(&mut yaml, feature_size, &mut rng, 2, 3);
    yaml.push('\n');

    // Data records
    yaml.push_str("data:\n");
    add_user_records(&mut yaml, feature_size, &mut rng, 2);
    yaml.push('\n');

    // Sequences
    yaml.push_str("sequences:\n");
    add_sequence_values(&mut yaml, feature_size, &mut rng, 2);

    yaml
}

/// Generate user/person records
fn generate_users(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut yaml = String::with_capacity(target_size);

    yaml.push_str("# User records\n");
    yaml.push_str("users:\n");

    add_user_records(&mut yaml, target_size.saturating_sub(20), &mut rng, 2);

    yaml
}

/// Generate deeply nested structures
fn generate_nested(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut yaml = String::with_capacity(target_size);

    yaml.push_str("# Deeply nested structure\n");
    yaml.push_str("root:\n");

    // Limit depth to avoid BP tree i16 overflow (max ~6 levels to stay safe)
    let max_depth = 6;
    add_nested_mapping(
        &mut yaml,
        target_size.saturating_sub(30),
        &mut rng,
        2,
        max_depth,
    );

    yaml
}

/// Generate sequence-heavy content
fn generate_sequences(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut yaml = String::with_capacity(target_size);

    yaml.push_str("# Sequence-heavy content\n");
    yaml.push_str("sequences:\n");

    add_sequence_values(&mut yaml, target_size.saturating_sub(30), &mut rng, 2);

    yaml
}

/// Generate mixed mappings and sequences
fn generate_mixed(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut yaml = String::with_capacity(target_size);

    yaml.push_str("# Mixed mappings and sequences\n");

    let section_size = target_size.saturating_sub(30) / 3;

    yaml.push_str("mapping_section:\n");
    add_simple_values(&mut yaml, section_size, &mut rng, 2);
    yaml.push('\n');

    yaml.push_str("sequence_section:\n");
    add_sequence_values(&mut yaml, section_size, &mut rng, 2);
    yaml.push('\n');

    yaml.push_str("nested_section:\n");
    add_mixed_nested(&mut yaml, section_size, &mut rng, 2, 4);

    yaml
}

/// Generate string-heavy content
fn generate_strings(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut yaml = String::with_capacity(target_size);

    yaml.push_str("# String-heavy content\n");
    yaml.push_str("strings:\n");

    add_string_values(&mut yaml, target_size.saturating_sub(30), &mut rng, 2);

    yaml
}

/// Generate number-heavy content
fn generate_numbers(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut yaml = String::with_capacity(target_size);

    yaml.push_str("# Number-heavy content\n");
    yaml.push_str("numbers:\n");

    add_number_values(&mut yaml, target_size.saturating_sub(30), &mut rng, 2);

    yaml
}

/// Generate config file style content
fn generate_config(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut yaml = String::with_capacity(target_size);

    yaml.push_str("# Application configuration\n\n");

    // App section
    yaml.push_str("app:\n");
    yaml.push_str("  name: my-application\n");
    yaml.push_str("  version: \"1.0.0\"\n");
    yaml.push_str("  debug: false\n");
    yaml.push('\n');

    // Server section
    yaml.push_str("server:\n");
    yaml.push_str("  host: \"0.0.0.0\"\n");
    yaml.push_str("  port: 8080\n");
    yaml.push_str("  workers: 4\n");
    yaml.push_str("  timeout: 30\n");
    yaml.push('\n');

    // Database section
    yaml.push_str("database:\n");
    yaml.push_str("  driver: postgresql\n");
    yaml.push_str("  host: localhost\n");
    yaml.push_str("  port: 5432\n");
    yaml.push_str("  name: myapp_db\n");
    yaml.push_str("  pool:\n");
    yaml.push_str("    min_connections: 5\n");
    yaml.push_str("    max_connections: 20\n");
    yaml.push('\n');

    // Logging section
    yaml.push_str("logging:\n");
    yaml.push_str("  level: info\n");
    yaml.push_str("  format: json\n");
    yaml.push_str("  outputs:\n");
    yaml.push_str("    - stdout\n");
    yaml.push_str("    - file\n");
    yaml.push('\n');

    // Fill remaining with feature flags or extra config
    let remaining = target_size.saturating_sub(yaml.len());
    if remaining > 50 {
        yaml.push_str("features:\n");
        add_config_features(&mut yaml, remaining, &mut rng, 2);
    }

    yaml
}

/// Generate Unicode content
fn generate_unicode(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut yaml = String::with_capacity(target_size);

    yaml.push_str("# Unicode content in various scripts\n");
    yaml.push_str("unicode:\n");

    add_unicode_values(&mut yaml, target_size.saturating_sub(50), &mut rng, 2);

    yaml
}

/// Generate pathological content (worst case for parsing)
fn generate_pathological(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut yaml = String::with_capacity(target_size);

    yaml.push_str("# Pathological test case\n");
    yaml.push_str("root:\n");

    // Limit depth to avoid BP tree i16 overflow
    let max_depth = 5;
    add_pathological_nested(
        &mut yaml,
        target_size.saturating_sub(30),
        &mut rng,
        2,
        max_depth,
    );

    yaml
}

/// Generate navigation-optimized content (top-level array for M2 streaming benchmarks)
///
/// This pattern generates a top-level array of objects, designed specifically for
/// testing M2 streaming navigation queries like:
/// - `.[0]` - first element access
/// - `.[]` - iteration
/// - `.[0].name` - chained navigation
/// - `length` - builtin that requires OwnedValue
fn generate_navigation(target_size: usize, seed: Option<u64>) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut yaml = String::with_capacity(target_size);

    yaml.push_str("# Navigation benchmark data (top-level array)\n");
    yaml.push_str("# Designed for M2 streaming queries: .[0], .[], .[0].name\n");

    let start_len = yaml.len();
    let mut count = 0;

    let first_names = [
        "Alice", "Bob", "Charlie", "Diana", "Eve", "Frank", "Grace", "Henry", "Ivy", "Jack",
        "Kate", "Leo", "Mia", "Noah", "Olivia", "Paul",
    ];
    let last_names = [
        "Smith",
        "Johnson",
        "Williams",
        "Brown",
        "Jones",
        "Garcia",
        "Miller",
        "Davis",
        "Rodriguez",
        "Martinez",
        "Hernandez",
        "Lopez",
        "Gonzalez",
    ];
    let cities = [
        "New York",
        "Los Angeles",
        "Chicago",
        "Houston",
        "Phoenix",
        "Seattle",
        "Boston",
        "Denver",
        "Miami",
        "Atlanta",
        "Portland",
        "Austin",
    ];
    let departments = [
        "Engineering",
        "Marketing",
        "Sales",
        "Support",
        "Finance",
        "HR",
        "Operations",
        "Research",
        "Design",
        "Legal",
    ];

    while yaml.len().saturating_sub(start_len) < target_size {
        let first = first_names[count % first_names.len()];
        let last = last_names[(count / 16) % last_names.len()];
        let city = cities[count % cities.len()];
        let dept = departments[count % departments.len()];
        let age = rng
            .as_mut()
            .map(|r| r.gen_range(22..65))
            .unwrap_or(25 + count % 40);
        let salary = rng
            .as_mut()
            .map(|r| r.gen_range(50000..200000))
            .unwrap_or(60000 + count * 1000);
        let years = rng
            .as_mut()
            .map(|r| r.gen_range(1..20))
            .unwrap_or(1 + count % 15);

        // Top-level array item (block style)
        yaml.push_str("- \n");
        yaml.push_str(&format!("  id: {}\n", count));
        yaml.push_str(&format!("  name: {} {}\n", first, last));
        yaml.push_str(&format!(
            "  email: {}.{}@example.com\n",
            first.to_lowercase(),
            last.to_lowercase()
        ));
        yaml.push_str(&format!("  age: {}\n", age));
        yaml.push_str(&format!("  city: {}\n", city));
        yaml.push_str(&format!("  department: {}\n", dept));
        yaml.push_str(&format!("  salary: {}\n", salary));
        yaml.push_str(&format!("  years_employed: {}\n", years));
        yaml.push_str(&format!("  active: {}\n", count % 10 != 0));

        // Add nested structure for chained navigation tests
        yaml.push_str("  metadata:\n");
        yaml.push_str(&format!(
            "    created_at: \"2024-01-{:02}T12:00:00Z\"\n",
            (count % 28) + 1
        ));
        yaml.push_str(&format!(
            "    updated_at: \"2024-06-{:02}T15:30:00Z\"\n",
            (count % 28) + 1
        ));
        yaml.push_str("    tags:\n");
        yaml.push_str(&format!("      - tag_{}\n", count % 5));
        yaml.push_str(&format!("      - category_{}\n", count % 3));

        count += 1;
    }

    yaml
}

// ============================================================================
// Helper functions
// ============================================================================

fn indent(n: usize) -> String {
    " ".repeat(n)
}

fn add_simple_values(
    yaml: &mut String,
    target_size: usize,
    rng: &mut Option<ChaCha8Rng>,
    indent_level: usize,
) {
    let start_len = yaml.len();
    let ind = indent(indent_level);
    let mut count = 0;

    while yaml.len().saturating_sub(start_len) < target_size {
        let value = match rng.as_mut().map(|r| r.gen_range(0..4)).unwrap_or(count % 4) {
            0 => "true".to_string(),
            1 => "false".to_string(),
            2 => "null".to_string(),
            _ => count.to_string(),
        };

        yaml.push_str(&format!("{}field_{}: {}\n", ind, count, value));
        count += 1;
    }
}

fn add_string_values(
    yaml: &mut String,
    target_size: usize,
    rng: &mut Option<ChaCha8Rng>,
    indent_level: usize,
) {
    let start_len = yaml.len();
    let ind = indent(indent_level);
    let mut count = 0;

    let string_patterns = [
        "simple string without special chars",
        "email@example.com",
        "https://example.com/path/to/resource",
        "Lorem ipsum dolor sit amet",
        "User Name",
        "2024-01-03T12:34:56Z",
        "path/to/file.txt",
        "value with spaces",
    ];

    let quoted_patterns = [
        "\"quoted: with colon\"",
        "\"has # hash\"",
        "\"contains 'single quotes'\"",
        "'single quoted string'",
        "\"multi word value\"",
    ];

    while yaml.len().saturating_sub(start_len) < target_size {
        let use_quoted = rng
            .as_mut()
            .map(|r| r.gen_range(0..5) == 0)
            .unwrap_or(count % 5 == 0);

        let value = if use_quoted {
            quoted_patterns[count % quoted_patterns.len()]
        } else {
            string_patterns[count % string_patterns.len()]
        };

        yaml.push_str(&format!("{}string_{}: {}\n", ind, count, value));
        count += 1;
    }
}

fn add_number_values(
    yaml: &mut String,
    target_size: usize,
    rng: &mut Option<ChaCha8Rng>,
    indent_level: usize,
) {
    let start_len = yaml.len();
    let ind = indent(indent_level);
    let mut count = 0;

    while yaml.len().saturating_sub(start_len) < target_size {
        let num = match count % 6 {
            0 => count.to_string(),                             // Simple integer
            1 => format!("-{}", count),                         // Negative integer
            2 => format!("0.{}", count),                        // Decimal < 1
            3 => format!("{}.{}", count, count),                // Decimal
            4 => format!("{}.{}e{}", count, count, count % 10), // Scientific notation
            _ => {
                let val = rng
                    .as_mut()
                    .map(|r| r.r#gen::<f64>() * 1000.0)
                    .unwrap_or(count as f64);
                format!("{:.4}", val)
            }
        };

        yaml.push_str(&format!("{}number_{}: {}\n", ind, count, num));
        count += 1;
    }
}

fn add_sequence_values(
    yaml: &mut String,
    target_size: usize,
    rng: &mut Option<ChaCha8Rng>,
    indent_level: usize,
) {
    let start_len = yaml.len();
    let ind = indent(indent_level);
    let mut count = 0;

    while yaml.len().saturating_sub(start_len) < target_size {
        let item_type = rng.as_mut().map(|r| r.gen_range(0..4)).unwrap_or(count % 4);

        match item_type {
            0 => yaml.push_str(&format!("{}- item_{}\n", ind, count)),
            1 => yaml.push_str(&format!("{}- {}\n", ind, count)),
            2 => yaml.push_str(&format!("{}- true\n", ind)),
            _ => yaml.push_str(&format!("{}- \"value {}\"\n", ind, count)),
        }

        count += 1;
    }
}

fn add_nested_mapping(
    yaml: &mut String,
    target_size: usize,
    rng: &mut Option<ChaCha8Rng>,
    indent_level: usize,
    max_depth: usize,
) {
    let start_len = yaml.len();
    let ind = indent(indent_level);
    let mut count = 0;

    while yaml.len().saturating_sub(start_len) < target_size && count < 50 {
        let used = yaml.len().saturating_sub(start_len);
        if max_depth > 0 && used < target_size / 2 {
            yaml.push_str(&format!("{}level_{}:\n", ind, count));
            let remaining = target_size.saturating_sub(yaml.len().saturating_sub(start_len));
            add_nested_mapping(yaml, remaining / 2, rng, indent_level + 2, max_depth - 1);
        } else {
            let value = rng.as_mut().map(|r| r.gen_range(0..100)).unwrap_or(count);
            yaml.push_str(&format!("{}leaf_{}: {}\n", ind, count, value));
        }
        count += 1;
    }
}

fn add_mixed_nested(
    yaml: &mut String,
    target_size: usize,
    rng: &mut Option<ChaCha8Rng>,
    indent_level: usize,
    max_depth: usize,
) {
    let start_len = yaml.len();
    let ind = indent(indent_level);
    let mut count = 0;

    while yaml.len().saturating_sub(start_len) < target_size && count < 30 {
        let use_sequence = rng
            .as_mut()
            .map(|r| r.r#gen::<bool>())
            .unwrap_or(count % 2 == 0);
        let used = yaml.len().saturating_sub(start_len);

        if max_depth > 0 && used < target_size / 2 {
            if use_sequence {
                yaml.push_str(&format!("{}list_{}:\n", ind, count));
                let items = rng.as_mut().map(|r| r.gen_range(2..5)).unwrap_or(3);
                for i in 0..items {
                    yaml.push_str(&format!("{}- item_{}\n", indent(indent_level + 2), i));
                }
            } else {
                yaml.push_str(&format!("{}map_{}:\n", ind, count));
                let remaining = target_size.saturating_sub(yaml.len().saturating_sub(start_len));
                add_mixed_nested(yaml, remaining / 3, rng, indent_level + 2, max_depth - 1);
            }
        } else {
            yaml.push_str(&format!("{}value_{}: {}\n", ind, count, count));
        }
        count += 1;
    }
}

fn add_user_records(
    yaml: &mut String,
    target_size: usize,
    rng: &mut Option<ChaCha8Rng>,
    indent_level: usize,
) {
    let start_len = yaml.len();
    let ind = indent(indent_level);
    let inner_ind = indent(indent_level + 2);
    let mut count = 0;

    let first_names = [
        "Alice", "Bob", "Charlie", "Diana", "Eve", "Frank", "Grace", "Henry",
    ];
    let last_names = [
        "Smith", "Johnson", "Williams", "Brown", "Jones", "Garcia", "Miller",
    ];
    let cities = [
        "New York",
        "Los Angeles",
        "Chicago",
        "Houston",
        "Phoenix",
        "Seattle",
    ];

    while yaml.len().saturating_sub(start_len) < target_size {
        let first = first_names[count % first_names.len()];
        let last = last_names[(count / 8) % last_names.len()];
        let city = cities[count % cities.len()];
        let age = rng
            .as_mut()
            .map(|r| r.gen_range(18..80))
            .unwrap_or(25 + count % 50);
        let score = rng
            .as_mut()
            .map(|r| r.gen_range(0..1000))
            .unwrap_or(count * 10);

        // Use block style for sequence items with mappings
        // Note: YAML-lite parser requires `- ` (dash-space) not just `-`
        yaml.push_str(&format!("{}- \n", ind));
        yaml.push_str(&format!("{}id: {}\n", inner_ind, count));
        yaml.push_str(&format!("{}name: {} {}\n", inner_ind, first, last));
        yaml.push_str(&format!(
            "{}email: {}.{}@example.com\n",
            inner_ind,
            first.to_lowercase(),
            last.to_lowercase()
        ));
        yaml.push_str(&format!("{}age: {}\n", inner_ind, age));
        yaml.push_str(&format!("{}city: {}\n", inner_ind, city));
        yaml.push_str(&format!("{}score: {}\n", inner_ind, score));
        yaml.push_str(&format!("{}active: true\n", inner_ind));

        count += 1;
    }
}

fn add_config_features(
    yaml: &mut String,
    target_size: usize,
    rng: &mut Option<ChaCha8Rng>,
    indent_level: usize,
) {
    let start_len = yaml.len();
    let ind = indent(indent_level);
    let inner_ind = indent(indent_level + 2);
    let mut count = 0;

    let feature_names = [
        "auth",
        "cache",
        "metrics",
        "logging",
        "ratelimit",
        "compression",
        "cors",
        "csrf",
        "ssl",
        "websocket",
    ];

    while yaml.len().saturating_sub(start_len) < target_size && count < feature_names.len() {
        let enabled = rng
            .as_mut()
            .map(|r| r.r#gen::<bool>())
            .unwrap_or(count % 2 == 0);
        let priority = rng
            .as_mut()
            .map(|r| r.gen_range(1..10))
            .unwrap_or(count + 1);

        yaml.push_str(&format!("{}{}:\n", ind, feature_names[count]));
        yaml.push_str(&format!("{}enabled: {}\n", inner_ind, enabled));
        yaml.push_str(&format!("{}priority: {}\n", inner_ind, priority));

        count += 1;
    }
}

fn add_unicode_values(
    yaml: &mut String,
    target_size: usize,
    _rng: &mut Option<ChaCha8Rng>,
    indent_level: usize,
) {
    let start_len = yaml.len();
    let ind = indent(indent_level);
    let mut count = 0;

    let unicode_samples = [
        ("chinese", "Hello 世界"),
        ("russian", "Привет мир"),
        ("arabic", "مرحبا بالعالم"),
        ("emoji", "Hello 🌍🌎🌏"),
        ("cafe", "Café ☕"),
        ("japanese", "日本語テキスト"),
        ("korean", "한글 텍스트"),
        ("greek", "Γεια σου κόσμε"),
        ("hebrew", "שלום עולם"),
        ("hindi", "नमस्ते दुनिया"),
        ("math", "Math: ∑∫∂∇√∞"),
        ("arrows", "Arrows: ←↑→↓↔↕"),
    ];

    while yaml.len().saturating_sub(start_len) < target_size {
        let (name, value) = unicode_samples[count % unicode_samples.len()];
        yaml.push_str(&format!(
            "{}{}_{}: \"{}\"\n",
            ind,
            name,
            count / unicode_samples.len(),
            value
        ));
        count += 1;
    }
}

fn add_pathological_nested(
    yaml: &mut String,
    target_size: usize,
    rng: &mut Option<ChaCha8Rng>,
    indent_level: usize,
    max_depth: usize,
) {
    let start_len = yaml.len();
    let ind = indent(indent_level);
    let mut count = 0;

    // Generate many siblings at each level with some nesting
    while yaml.len().saturating_sub(start_len) < target_size && count < 100 {
        if max_depth > 0 && count % 3 == 0 {
            yaml.push_str(&format!("{}node_{}:\n", ind, count));
            // Add some immediate values
            yaml.push_str(&format!("{}val: {}\n", indent(indent_level + 2), count));
            // Recurse
            let remaining = target_size.saturating_sub(yaml.len().saturating_sub(start_len));
            add_pathological_nested(yaml, remaining / 4, rng, indent_level + 2, max_depth - 1);
        } else {
            let value = rng.as_mut().map(|r| r.gen_range(0..10000)).unwrap_or(count);
            yaml.push_str(&format!("{}item_{}: {}\n", ind, count, value));
        }
        count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_comprehensive() {
        let yaml = generate_yaml(1024, YamlPattern::Comprehensive, Some(42));
        assert!(yaml.len() >= 512); // May not hit exactly 1024 due to structure
        assert!(yaml.contains("metadata:"));
        assert!(yaml.contains("simple_values:"));
    }

    #[test]
    fn test_generate_users() {
        let yaml = generate_yaml(1024, YamlPattern::Users, Some(42));
        assert!(yaml.contains("users:"));
        assert!(yaml.contains("name:"));
        assert!(yaml.contains("email:"));
    }

    #[test]
    fn test_generate_nested() {
        let yaml = generate_yaml(1024, YamlPattern::Nested, Some(42));
        assert!(yaml.contains("root:"));
        assert!(yaml.contains("level_"));
    }

    #[test]
    fn test_generate_config() {
        let yaml = generate_yaml(1024, YamlPattern::Config, Some(42));
        assert!(yaml.contains("app:"));
        assert!(yaml.contains("server:"));
        assert!(yaml.contains("database:"));
    }

    #[test]
    fn test_deterministic_generation() {
        let yaml1 = generate_yaml(1024, YamlPattern::Users, Some(42));
        let yaml2 = generate_yaml(1024, YamlPattern::Users, Some(42));
        assert_eq!(yaml1, yaml2);
    }

    #[test]
    fn test_different_seeds() {
        let yaml1 = generate_yaml(1024, YamlPattern::Users, Some(42));
        let yaml2 = generate_yaml(1024, YamlPattern::Users, Some(123));
        assert_ne!(yaml1, yaml2);
    }

    #[test]
    fn test_generate_navigation() {
        let yaml = generate_yaml(1024, YamlPattern::Navigation, Some(42));
        // Navigation pattern has top-level array items
        assert!(yaml.contains("- \n"));
        assert!(yaml.contains("  id:"));
        assert!(yaml.contains("  name:"));
        assert!(yaml.contains("  metadata:"));
        assert!(yaml.contains("    tags:"));
    }
}
