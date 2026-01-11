//! DSV (CSV/TSV) generators for benchmarking and testing.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[derive(Debug, Clone, Copy)]
pub enum DsvPattern {
    /// Standard tabular data with mixed types
    Tabular,
    /// User/person records (realistic structure)
    Users,
    /// Numeric-heavy data (financial, scientific)
    Numeric,
    /// String-heavy data with various lengths
    Strings,
    /// Data with quoted fields containing delimiters
    Quoted,
    /// Data with quoted fields containing newlines
    Multiline,
    /// Wide tables (many columns)
    Wide,
    /// Narrow but long tables (few columns, many rows)
    Long,
    /// Mixed data types per row
    Mixed,
    /// Worst case: every field is quoted with embedded delimiters
    Pathological,
}

/// Generate DSV of approximately target_size bytes
pub fn generate_dsv(
    target_size: usize,
    pattern: DsvPattern,
    seed: Option<u64>,
    delimiter: char,
    include_header: bool,
) -> String {
    match pattern {
        DsvPattern::Tabular => generate_tabular(target_size, seed, delimiter, include_header),
        DsvPattern::Users => generate_users(target_size, seed, delimiter, include_header),
        DsvPattern::Numeric => generate_numeric(target_size, seed, delimiter, include_header),
        DsvPattern::Strings => generate_strings(target_size, seed, delimiter, include_header),
        DsvPattern::Quoted => generate_quoted(target_size, seed, delimiter, include_header),
        DsvPattern::Multiline => generate_multiline(target_size, seed, delimiter, include_header),
        DsvPattern::Wide => generate_wide(target_size, seed, delimiter, include_header),
        DsvPattern::Long => generate_long(target_size, seed, delimiter, include_header),
        DsvPattern::Mixed => generate_mixed(target_size, seed, delimiter, include_header),
        DsvPattern::Pathological => {
            generate_pathological(target_size, seed, delimiter, include_header)
        }
    }
}

/// Standard tabular data with mixed types
fn generate_tabular(
    target_size: usize,
    seed: Option<u64>,
    delimiter: char,
    include_header: bool,
) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut csv = String::with_capacity(target_size);

    if include_header {
        csv.push_str(&format!(
            "id{}name{}email{}age{}score{}active{}created\n",
            delimiter, delimiter, delimiter, delimiter, delimiter, delimiter
        ));
    }

    let mut row_id = 1;
    while csv.len() < target_size {
        let age = rng.as_mut().map(|r| r.gen_range(18..80)).unwrap_or(25);
        let score = rng
            .as_mut()
            .map(|r| r.gen_range(0..10000))
            .unwrap_or(row_id * 10);
        let active = rng
            .as_mut()
            .map(|r| r.r#gen::<bool>())
            .unwrap_or(row_id % 2 == 0);
        let day = (row_id % 28) + 1;
        let month = (row_id % 12) + 1;

        csv.push_str(&format!(
            "{}{}User{}{}user{}@example.com{}{}{}{}{}{}{}2024-{:02}-{:02}\n",
            row_id,
            delimiter,
            row_id,
            delimiter,
            row_id,
            delimiter,
            age,
            delimiter,
            score,
            delimiter,
            active,
            delimiter,
            month,
            day
        ));

        row_id += 1;
    }

    csv
}

/// User/person records
fn generate_users(
    target_size: usize,
    seed: Option<u64>,
    delimiter: char,
    include_header: bool,
) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut csv = String::with_capacity(target_size);

    let first_names = [
        "Alice", "Bob", "Charlie", "Diana", "Eve", "Frank", "Grace", "Henry", "Ivy", "Jack",
    ];
    let last_names = [
        "Smith", "Johnson", "Williams", "Brown", "Jones", "Garcia", "Miller", "Davis", "Wilson",
        "Moore",
    ];
    let cities = [
        "New York",
        "Los Angeles",
        "Chicago",
        "Houston",
        "Phoenix",
        "Philadelphia",
        "San Antonio",
        "San Diego",
        "Dallas",
        "Austin",
    ];
    let countries = ["USA", "Canada", "UK", "Germany", "France", "Australia"];

    if include_header {
        csv.push_str(&format!(
            "id{}first_name{}last_name{}email{}phone{}city{}country{}age{}salary\n",
            delimiter, delimiter, delimiter, delimiter, delimiter, delimiter, delimiter, delimiter
        ));
    }

    let mut row_id = 1;
    while csv.len() < target_size {
        let first = first_names[row_id % first_names.len()];
        let last = last_names[(row_id / 10) % last_names.len()];
        let city = cities[row_id % cities.len()];
        let country = countries[row_id % countries.len()];
        let age = rng.as_mut().map(|r| r.gen_range(22..65)).unwrap_or(30);
        let salary = rng
            .as_mut()
            .map(|r| r.gen_range(30000..200000))
            .unwrap_or(50000);
        let phone_suffix = rng
            .as_mut()
            .map(|r| r.gen_range(1000..9999))
            .unwrap_or(1234);

        csv.push_str(&format!(
            "{}{}{}{}{}{}{}.{}@example.com{}+1-555-{:04}{}{}{}{}{}{}{}{}",
            row_id,
            delimiter,
            first,
            delimiter,
            last,
            delimiter,
            first.to_lowercase(),
            last.to_lowercase(),
            delimiter,
            phone_suffix,
            delimiter,
            city,
            delimiter,
            country,
            delimiter,
            age,
            delimiter,
            salary
        ));
        csv.push('\n');

        row_id += 1;
    }

    csv
}

/// Numeric-heavy data
fn generate_numeric(
    target_size: usize,
    seed: Option<u64>,
    delimiter: char,
    include_header: bool,
) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut csv = String::with_capacity(target_size);

    if include_header {
        csv.push_str(&format!(
            "id{}value1{}value2{}value3{}value4{}value5{}total{}average\n",
            delimiter, delimiter, delimiter, delimiter, delimiter, delimiter, delimiter
        ));
    }

    let mut row_id = 1;
    while csv.len() < target_size {
        let v1: f64 = rng
            .as_mut()
            .map(|r| r.r#gen::<f64>() * 1000.0)
            .unwrap_or(row_id as f64);
        let v2: f64 = rng
            .as_mut()
            .map(|r| r.r#gen::<f64>() * 1000.0)
            .unwrap_or(row_id as f64 * 1.5);
        let v3: f64 = rng
            .as_mut()
            .map(|r| r.r#gen::<f64>() * 1000.0)
            .unwrap_or(row_id as f64 * 2.0);
        let v4: f64 = rng
            .as_mut()
            .map(|r| r.r#gen::<f64>() * 1000.0)
            .unwrap_or(row_id as f64 * 2.5);
        let v5: f64 = rng
            .as_mut()
            .map(|r| r.r#gen::<f64>() * 1000.0)
            .unwrap_or(row_id as f64 * 3.0);
        let total = v1 + v2 + v3 + v4 + v5;
        let avg = total / 5.0;

        csv.push_str(&format!(
            "{}{}{:.4}{}{:.4}{}{:.4}{}{:.4}{}{:.4}{}{:.4}{}{:.4}\n",
            row_id,
            delimiter,
            v1,
            delimiter,
            v2,
            delimiter,
            v3,
            delimiter,
            v4,
            delimiter,
            v5,
            delimiter,
            total,
            delimiter,
            avg
        ));

        row_id += 1;
    }

    csv
}

/// String-heavy data
fn generate_strings(
    target_size: usize,
    seed: Option<u64>,
    delimiter: char,
    include_header: bool,
) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut csv = String::with_capacity(target_size);

    let lorem_words = [
        "lorem",
        "ipsum",
        "dolor",
        "sit",
        "amet",
        "consectetur",
        "adipiscing",
        "elit",
        "sed",
        "do",
        "eiusmod",
        "tempor",
        "incididunt",
        "labore",
        "dolore",
        "magna",
        "aliqua",
    ];

    if include_header {
        csv.push_str(&format!(
            "id{}title{}description{}notes\n",
            delimiter, delimiter, delimiter
        ));
    }

    let mut row_id = 1;
    while csv.len() < target_size {
        // Generate title (3-5 words)
        let title_len = rng.as_mut().map(|r| r.gen_range(3..6)).unwrap_or(4);
        let title: String = (0..title_len)
            .map(|i| lorem_words[(row_id + i) % lorem_words.len()])
            .collect::<Vec<_>>()
            .join(" ");

        // Generate description (10-20 words)
        let desc_len = rng.as_mut().map(|r| r.gen_range(10..21)).unwrap_or(15);
        let description: String = (0..desc_len)
            .map(|i| lorem_words[(row_id * 2 + i) % lorem_words.len()])
            .collect::<Vec<_>>()
            .join(" ");

        // Generate notes (5-10 words)
        let notes_len = rng.as_mut().map(|r| r.gen_range(5..11)).unwrap_or(7);
        let notes: String = (0..notes_len)
            .map(|i| lorem_words[(row_id * 3 + i) % lorem_words.len()])
            .collect::<Vec<_>>()
            .join(" ");

        csv.push_str(&format!(
            "{}{}{}{}{}{}{}",
            row_id, delimiter, title, delimiter, description, delimiter, notes
        ));
        csv.push('\n');

        row_id += 1;
    }

    csv
}

/// Data with quoted fields containing delimiters
fn generate_quoted(
    target_size: usize,
    seed: Option<u64>,
    delimiter: char,
    include_header: bool,
) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut csv = String::with_capacity(target_size);

    if include_header {
        csv.push_str(&format!(
            "id{}name{}address{}notes\n",
            delimiter, delimiter, delimiter
        ));
    }

    let street_names = [
        "Main St", "Oak Ave", "Maple Dr", "Cedar Ln", "Pine Rd", "Elm Blvd",
    ];
    let cities = ["New York", "Los Angeles", "Chicago", "Houston", "Seattle"];

    let mut row_id = 1;
    while csv.len() < target_size {
        let street_num = rng.as_mut().map(|r| r.gen_range(1..9999)).unwrap_or(123);
        let street = street_names[row_id % street_names.len()];
        let city = cities[row_id % cities.len()];
        let zip = 10000 + (row_id % 90000);

        // Address contains delimiter, must be quoted
        let address = format!(
            "{} {}{} {}{} {}",
            street_num, street, delimiter, city, delimiter, zip
        );

        // Notes may contain delimiter
        let notes = if row_id % 3 == 0 {
            format!("Note with{} delimiter", delimiter)
        } else {
            format!("Simple note {}", row_id)
        };

        // Quote fields that contain delimiter
        csv.push_str(&format!(
            "{}{}\"User{} Name\"{}\"{}\"{}\"{}\"",
            row_id, delimiter, row_id, delimiter, address, delimiter, notes
        ));
        csv.push('\n');

        row_id += 1;
    }

    csv
}

/// Data with quoted fields containing newlines
fn generate_multiline(
    target_size: usize,
    seed: Option<u64>,
    delimiter: char,
    include_header: bool,
) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut csv = String::with_capacity(target_size);

    if include_header {
        csv.push_str(&format!(
            "id{}title{}body{}author\n",
            delimiter, delimiter, delimiter
        ));
    }

    let mut row_id = 1;
    while csv.len() < target_size {
        let num_lines = rng.as_mut().map(|r| r.gen_range(2..5)).unwrap_or(3);

        // Body contains newlines, must be quoted
        let body_lines: Vec<String> = (0..num_lines)
            .map(|i| format!("Line {} of entry {}", i + 1, row_id))
            .collect();
        let body = body_lines.join("\n");

        csv.push_str(&format!(
            "{}{}Title {}{}\"{}\"{}Author{}",
            row_id, delimiter, row_id, delimiter, body, delimiter, row_id
        ));
        csv.push('\n');

        row_id += 1;
    }

    csv
}

/// Wide tables (many columns)
fn generate_wide(
    target_size: usize,
    seed: Option<u64>,
    delimiter: char,
    include_header: bool,
) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut csv = String::with_capacity(target_size);
    let num_columns = 50;

    if include_header {
        let headers: Vec<String> = (0..num_columns).map(|i| format!("col{}", i)).collect();
        csv.push_str(&headers.join(&delimiter.to_string()));
        csv.push('\n');
    }

    let mut row_id = 1;
    while csv.len() < target_size {
        let values: Vec<String> = (0..num_columns)
            .map(|col| {
                let val = rng
                    .as_mut()
                    .map(|r| r.gen_range(0..1000))
                    .unwrap_or(row_id * col);
                val.to_string()
            })
            .collect();

        csv.push_str(&values.join(&delimiter.to_string()));
        csv.push('\n');

        row_id += 1;
    }

    csv
}

/// Narrow but long tables (few columns, many rows)
fn generate_long(
    target_size: usize,
    seed: Option<u64>,
    delimiter: char,
    include_header: bool,
) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut csv = String::with_capacity(target_size);

    if include_header {
        csv.push_str(&format!("id{}value\n", delimiter));
    }

    let mut row_id = 1;
    while csv.len() < target_size {
        let value = rng
            .as_mut()
            .map(|r| r.gen_range(0..1000000))
            .unwrap_or(row_id);
        csv.push_str(&format!("{}{}{}\n", row_id, delimiter, value));
        row_id += 1;
    }

    csv
}

/// Mixed data types per row
fn generate_mixed(
    target_size: usize,
    seed: Option<u64>,
    delimiter: char,
    include_header: bool,
) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut csv = String::with_capacity(target_size);

    if include_header {
        csv.push_str(&format!(
            "id{}int_val{}float_val{}bool_val{}string_val{}date_val\n",
            delimiter, delimiter, delimiter, delimiter, delimiter
        ));
    }

    let mut row_id = 1;
    while csv.len() < target_size {
        let int_val = rng
            .as_mut()
            .map(|r| r.gen_range(-1000..1000))
            .unwrap_or(row_id);
        let float_val: f64 = rng
            .as_mut()
            .map(|r| r.r#gen::<f64>() * 1000.0 - 500.0)
            .unwrap_or(row_id as f64 * 1.5);
        let bool_val = rng
            .as_mut()
            .map(|r| r.r#gen::<bool>())
            .unwrap_or(row_id % 2 == 0);
        let day = (row_id % 28) + 1;
        let month = (row_id % 12) + 1;

        csv.push_str(&format!(
            "{}{}{}{}{}{}{}{}value_{}{}2024-{:02}-{:02}\n",
            row_id,
            delimiter,
            int_val,
            delimiter,
            float_val,
            delimiter,
            bool_val,
            delimiter,
            row_id,
            delimiter,
            month,
            day
        ));

        row_id += 1;
    }

    csv
}

/// Worst case: every field is quoted with embedded delimiters and quotes
fn generate_pathological(
    target_size: usize,
    seed: Option<u64>,
    delimiter: char,
    include_header: bool,
) -> String {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut csv = String::with_capacity(target_size);

    if include_header {
        csv.push_str(&format!(
            "\"id\"{}\"field1\"{}\"field2\"{}\"field3\"\n",
            delimiter, delimiter, delimiter
        ));
    }

    let mut row_id = 1;
    while csv.len() < target_size {
        // Every field contains delimiter, quotes, or both
        let extra = rng.as_mut().map(|r| r.gen_range(0..100)).unwrap_or(row_id);

        // Field with embedded delimiter
        let field1 = format!("value{}with{}delimiter", delimiter, extra);

        // Field with embedded quotes (doubled for CSV escaping)
        let field2 = format!("say \"\"hello\"\" {}", row_id);

        // Field with both delimiter and quotes
        let field3 = format!("complex{}\"\"data\"\"", delimiter);

        csv.push_str(&format!(
            "{}{}\"{}\"{}\"{}\"{}\"{}\"",
            row_id, delimiter, field1, delimiter, field2, delimiter, field3
        ));
        csv.push('\n');

        row_id += 1;
    }

    csv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_tabular() {
        let csv = generate_dsv(1024, DsvPattern::Tabular, Some(42), ',', true);
        assert!(csv.len() >= 1024);
        assert!(csv.starts_with("id,name,email,"));
        assert!(csv.contains('\n'));
    }

    #[test]
    fn test_generate_users() {
        let csv = generate_dsv(1024, DsvPattern::Users, Some(42), ',', true);
        assert!(csv.len() >= 1024);
        assert!(csv.contains("first_name"));
    }

    #[test]
    fn test_generate_quoted() {
        let csv = generate_dsv(1024, DsvPattern::Quoted, Some(42), ',', true);
        assert!(csv.len() >= 1024);
        // Should contain quoted fields
        assert!(csv.contains('"'));
    }

    #[test]
    fn test_generate_multiline() {
        let csv = generate_dsv(1024, DsvPattern::Multiline, Some(42), ',', true);
        assert!(csv.len() >= 1024);
        // Should contain quoted fields with newlines
        assert!(csv.contains("\"Line 1"));
    }

    #[test]
    fn test_tsv_generation() {
        let tsv = generate_dsv(1024, DsvPattern::Tabular, Some(42), '\t', true);
        assert!(tsv.contains('\t'));
        assert!(!tsv.contains(','));
    }

    #[test]
    fn test_deterministic_generation() {
        let csv1 = generate_dsv(1024, DsvPattern::Tabular, Some(42), ',', true);
        let csv2 = generate_dsv(1024, DsvPattern::Tabular, Some(42), ',', true);
        assert_eq!(csv1, csv2);
    }
}
