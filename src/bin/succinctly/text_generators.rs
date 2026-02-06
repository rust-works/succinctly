//! UTF-8 text generators for benchmarking and testing.
//!
//! Generates various types of UTF-8 content for validating and benchmarking
//! UTF-8 validation algorithms.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Pattern types for UTF-8 text generation.
#[derive(Debug, Clone, Copy)]
pub enum Utf8Pattern {
    /// Pure ASCII (7-bit, single-byte sequences)
    Ascii,
    /// Latin Extended characters (2-byte sequences: accents, diacritics)
    Latin,
    /// Greek and Cyrillic (2-byte sequences)
    GreekCyrillic,
    /// Chinese/Japanese/Korean (3-byte sequences)
    Cjk,
    /// Emoji and symbols (4-byte sequences)
    Emoji,
    /// Mixed realistic content (prose with occasional non-ASCII)
    Mixed,
    /// Uniform mix of all sequence lengths (1-4 bytes)
    AllLengths,
    /// Log file style (mostly ASCII with timestamps and occasional unicode)
    LogFile,
    /// Source code style (ASCII with unicode in strings/comments)
    SourceCode,
    /// JSON-like structure (tests escape sequences context)
    JsonLike,
    /// Pathological: maximum multi-byte density
    Pathological,
}

/// Generate UTF-8 text of approximately target_size bytes.
pub fn generate_utf8(target_size: usize, pattern: Utf8Pattern, seed: Option<u64>) -> Vec<u8> {
    match pattern {
        Utf8Pattern::Ascii => generate_ascii(target_size, seed),
        Utf8Pattern::Latin => generate_latin(target_size, seed),
        Utf8Pattern::GreekCyrillic => generate_greek_cyrillic(target_size, seed),
        Utf8Pattern::Cjk => generate_cjk(target_size, seed),
        Utf8Pattern::Emoji => generate_emoji(target_size, seed),
        Utf8Pattern::Mixed => generate_mixed(target_size, seed),
        Utf8Pattern::AllLengths => generate_all_lengths(target_size, seed),
        Utf8Pattern::LogFile => generate_log_file(target_size, seed),
        Utf8Pattern::SourceCode => generate_source_code(target_size, seed),
        Utf8Pattern::JsonLike => generate_json_like(target_size, seed),
        Utf8Pattern::Pathological => generate_pathological(target_size, seed),
    }
}

/// Pure ASCII content (English prose).
fn generate_ascii(target_size: usize, seed: Option<u64>) -> Vec<u8> {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut result = Vec::with_capacity(target_size);

    let sentences = [
        "The quick brown fox jumps over the lazy dog.",
        "Pack my box with five dozen liquor jugs.",
        "How vexingly quick daft zebras jump!",
        "The five boxing wizards jump quickly.",
        "Sphinx of black quartz, judge my vow.",
        "Two driven jocks help fax my big quiz.",
        "The jay, pig, fox, zebra and my wolves quack!",
        "Sympathizing would fix Quaker objectives.",
        "A wizard's job is to vex chumps quickly in fog.",
        "Watch Jeopardy!, Alex Trebek's fun TV quiz game.",
    ];

    let mut line_len = 0;
    while result.len() < target_size {
        let idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..sentences.len()))
            .unwrap_or(result.len() % sentences.len());
        let sentence = sentences[idx];

        if line_len > 0 && line_len + sentence.len() + 1 > 80 {
            result.push(b'\n');
            line_len = 0;
        } else if line_len > 0 {
            result.push(b' ');
            line_len += 1;
        }

        let remaining = target_size.saturating_sub(result.len());
        let to_add = sentence.len().min(remaining);
        result.extend_from_slice(&sentence.as_bytes()[..to_add]);
        line_len += to_add;
    }

    result.truncate(target_size);
    result
}

/// Latin Extended characters (2-byte UTF-8).
fn generate_latin(target_size: usize, seed: Option<u64>) -> Vec<u8> {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut result = Vec::with_capacity(target_size);

    let words = [
        "café",
        "résumé",
        "naïve",
        "über",
        "fiancée",
        "cliché",
        "décor",
        "élite",
        "entrée",
        "façade",
        "jalapeño",
        "piñata",
        "señor",
        "mañana",
        "niño",
        "Ångström",
        "smörgåsbord",
        "Müller",
        "Größe",
        "Füße",
        "Köln",
        "Zürich",
        "Ærø",
        "Malmö",
        "Göteborg",
        "Øresund",
        "Łódź",
        "Kraków",
        "Wrocław",
    ];

    let mut line_len = 0;
    while result.len() < target_size {
        let idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..words.len()))
            .unwrap_or(result.len() % words.len());
        let word = words[idx];
        let word_bytes = word.as_bytes();

        if line_len > 0 && line_len + word_bytes.len() + 1 > 80 {
            result.push(b'\n');
            line_len = 0;
        } else if line_len > 0 {
            result.push(b' ');
            line_len += 1;
        }

        if result.len() + word_bytes.len() <= target_size {
            result.extend_from_slice(word_bytes);
            line_len += word_bytes.len();
        } else {
            break;
        }
    }

    // Pad with ASCII if needed
    while result.len() < target_size {
        result.push(b' ');
    }
    result.truncate(target_size);
    result
}

/// Greek and Cyrillic text (2-byte UTF-8).
fn generate_greek_cyrillic(target_size: usize, seed: Option<u64>) -> Vec<u8> {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut result = Vec::with_capacity(target_size);

    let greek = [
        "αλφα",
        "βητα",
        "γαμμα",
        "δελτα",
        "επσιλον",
        "ζητα",
        "ητα",
        "θητα",
        "ιωτα",
        "καππα",
        "λαμδα",
        "μυ",
        "νυ",
        "ξι",
        "ομικρον",
        "πι",
        "ρω",
        "σιγμα",
        "ταυ",
        "υψιλον",
        "φι",
        "χι",
        "ψι",
        "ωμεγα",
    ];

    let cyrillic = [
        "Москва",
        "Санкт",
        "Киев",
        "Минск",
        "Алматы",
        "Ташкент",
        "Баку",
        "Тбилиси",
        "Ереван",
        "Кишинёв",
        "Душанбе",
        "Бишкек",
        "Астана",
        "привет",
        "мир",
        "добро",
        "пожаловать",
        "спасибо",
        "здравствуйте",
    ];

    let mut line_len = 0;
    let mut use_greek = true;
    while result.len() < target_size {
        let words = if use_greek { &greek[..] } else { &cyrillic[..] };
        let idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..words.len()))
            .unwrap_or(result.len() % words.len());
        let word = words[idx];
        let word_bytes = word.as_bytes();

        if line_len > 0 && line_len + word_bytes.len() + 1 > 80 {
            result.push(b'\n');
            line_len = 0;
            use_greek = !use_greek;
        } else if line_len > 0 {
            result.push(b' ');
            line_len += 1;
        }

        if result.len() + word_bytes.len() <= target_size {
            result.extend_from_slice(word_bytes);
            line_len += word_bytes.len();
        } else {
            break;
        }
    }

    while result.len() < target_size {
        result.push(b' ');
    }
    result.truncate(target_size);
    result
}

/// CJK characters (3-byte UTF-8).
fn generate_cjk(target_size: usize, seed: Option<u64>) -> Vec<u8> {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut result = Vec::with_capacity(target_size);

    let phrases = [
        "日本語",
        "中国語",
        "韓国語",
        "漢字",
        "仮名",
        "平仮名",
        "片仮名",
        "東京",
        "北京",
        "上海",
        "香港",
        "台北",
        "首爾",
        "大阪",
        "京都",
        "你好",
        "再见",
        "谢谢",
        "对不起",
        "没关系",
        "欢迎",
        "祝福",
        "안녕하세요",
        "감사합니다",
        "죄송합니다",
        "반갑습니다",
        "こんにちは",
        "ありがとう",
        "すみません",
        "おはよう",
        "さようなら",
    ];

    let mut line_len = 0;
    while result.len() < target_size {
        let idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..phrases.len()))
            .unwrap_or(result.len() % phrases.len());
        let phrase = phrases[idx];
        let phrase_bytes = phrase.as_bytes();

        if line_len > 0 && line_len + phrase_bytes.len() > 60 {
            result.push(b'\n');
            line_len = 0;
        } else if line_len > 0 {
            // CJK typically doesn't use spaces, but add occasionally
            if rng.as_mut().map(|r| r.gen_bool(0.3)).unwrap_or(false) {
                result.push(b' ');
                line_len += 1;
            }
        }

        if result.len() + phrase_bytes.len() <= target_size {
            result.extend_from_slice(phrase_bytes);
            line_len += phrase_bytes.len();
        } else {
            break;
        }
    }

    while result.len() < target_size {
        result.push(b' ');
    }
    result.truncate(target_size);
    result
}

/// Emoji and symbols (4-byte UTF-8).
fn generate_emoji(target_size: usize, seed: Option<u64>) -> Vec<u8> {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut result = Vec::with_capacity(target_size);

    let emojis = [
        "😀", "😃", "😄", "😁", "😆", "😅", "🤣", "😂", "🙂", "🙃", "😉", "😊", "😇", "🥰", "😍",
        "🤩", "😘", "😗", "😚", "😙", "🥲", "😋", "😛", "😜", "🤪", "😝", "🤑", "🤗", "🤭", "🤫",
        "🎉", "🎊", "🎈", "🎁", "🎀", "🎄", "🎃", "🎗️", "🎟️", "🎫", "🚀", "✈️", "🚁", "🚂", "🚃",
        "🚄", "🚅", "🚆", "🚇", "🚈", "🌍", "🌎", "🌏", "🌐", "🗺️", "🧭", "🏔️", "⛰️", "🌋", "🗻",
        "💻", "🖥️", "🖨️", "⌨️", "🖱️", "🖲️", "💽", "💾", "💿", "📀", "🔥", "💧", "🌊", "💨", "⚡",
        "❄️", "☀️", "🌙", "⭐", "🌟",
    ];

    while result.len() < target_size {
        let idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..emojis.len()))
            .unwrap_or(result.len() % emojis.len());
        let emoji = emojis[idx];
        let emoji_bytes = emoji.as_bytes();

        if result.len() + emoji_bytes.len() <= target_size {
            result.extend_from_slice(emoji_bytes);
        } else {
            break;
        }

        // Occasionally add space or newline
        if result.len() < target_size {
            let choice = rng.as_mut().map(|r| r.gen_range(0..10)).unwrap_or(0);
            if choice == 0 {
                result.push(b'\n');
            } else if choice < 3 {
                result.push(b' ');
            }
        }
    }

    while result.len() < target_size {
        result.push(b' ');
    }
    result.truncate(target_size);
    result
}

/// Mixed realistic content.
fn generate_mixed(target_size: usize, seed: Option<u64>) -> Vec<u8> {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut result = Vec::with_capacity(target_size);

    let phrases = [
        // ASCII (most common)
        "The quick brown fox jumps over the lazy dog.",
        "Hello, world! This is a test message.",
        "Lorem ipsum dolor sit amet, consectetur adipiscing elit.",
        "Pack my box with five dozen liquor jugs.",
        // Latin extended
        "Café au lait with crème brûlée is très délicieux.",
        "The naïve résumé was written by the fiancée.",
        "Señor García lives in São Paulo near the Ångström lab.",
        // With emoji
        "Great job! 🎉 Keep up the good work! 💪",
        "Weather forecast: ☀️ sunny with occasional 🌧️ rain.",
        "I love coding 💻 and drinking ☕ coffee!",
        // CJK mixed
        "Meeting at 東京 station at 3pm tomorrow.",
        "The document was translated to 中文 and 日本語.",
        // Currency and symbols
        "Price: €50.00 or £42.00 or ¥6,000 or ₹4,200",
        "Math: α + β = γ, ∑(x²) = n, ∞ > 0",
    ];

    let mut line_len = 0;
    while result.len() < target_size {
        let idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..phrases.len()))
            .unwrap_or(result.len() % phrases.len());
        let phrase = phrases[idx];
        let phrase_bytes = phrase.as_bytes();

        if line_len > 0 && line_len + phrase_bytes.len() + 1 > 100 {
            result.push(b'\n');
            line_len = 0;
        } else if line_len > 0 {
            result.push(b' ');
            line_len += 1;
        }

        if result.len() + phrase_bytes.len() <= target_size {
            result.extend_from_slice(phrase_bytes);
            line_len += phrase_bytes.len();
        } else {
            break;
        }
    }

    while result.len() < target_size {
        result.push(b' ');
    }
    result.truncate(target_size);
    result
}

/// Uniform distribution of all sequence lengths.
fn generate_all_lengths(target_size: usize, seed: Option<u64>) -> Vec<u8> {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut result = Vec::with_capacity(target_size);

    // Characters of each byte length
    let one_byte = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let two_byte = [
        "é", "ñ", "ü", "ö", "ä", "ß", "ç", "ø", "å", "æ", "α", "β", "γ", "δ", "ε",
    ];
    let three_byte = [
        "日", "本", "語", "中", "文", "韓", "国", "漢", "字", "你", "好", "世", "界",
    ];
    let four_byte = ["😀", "🎉", "🚀", "💻", "🔥", "⭐", "🌍", "💡", "🎯", "🎨"];

    while result.len() < target_size {
        let choice = rng
            .as_mut()
            .map(|r| r.gen_range(0..4))
            .unwrap_or(result.len() % 4);

        let bytes: &[u8] = match choice {
            0 => {
                let idx = rng
                    .as_mut()
                    .map(|r| r.gen_range(0..one_byte.len()))
                    .unwrap_or(0);
                &one_byte[idx..idx + 1]
            }
            1 => {
                let idx = rng
                    .as_mut()
                    .map(|r| r.gen_range(0..two_byte.len()))
                    .unwrap_or(0);
                two_byte[idx].as_bytes()
            }
            2 => {
                let idx = rng
                    .as_mut()
                    .map(|r| r.gen_range(0..three_byte.len()))
                    .unwrap_or(0);
                three_byte[idx].as_bytes()
            }
            _ => {
                let idx = rng
                    .as_mut()
                    .map(|r| r.gen_range(0..four_byte.len()))
                    .unwrap_or(0);
                four_byte[idx].as_bytes()
            }
        };

        if result.len() + bytes.len() <= target_size {
            result.extend_from_slice(bytes);
        } else {
            break;
        }

        // Occasionally add whitespace
        if result.len() < target_size {
            let add_ws = rng.as_mut().map(|r| r.gen_range(0..8)).unwrap_or(0);
            if add_ws == 0 {
                result.push(b'\n');
            } else if add_ws == 1 {
                result.push(b' ');
            }
        }
    }

    while result.len() < target_size {
        result.push(b' ');
    }
    result.truncate(target_size);
    result
}

/// Log file style (mostly ASCII with timestamps).
fn generate_log_file(target_size: usize, seed: Option<u64>) -> Vec<u8> {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut result = Vec::with_capacity(target_size);

    let levels = ["INFO", "DEBUG", "WARN", "ERROR", "TRACE"];
    let components = [
        "server",
        "database",
        "cache",
        "auth",
        "api",
        "worker",
        "scheduler",
        "queue",
    ];
    let messages = [
        "Request processed successfully",
        "Connection established to endpoint",
        "Cache miss for key",
        "User authentication completed",
        "Query executed in 42ms",
        "Task scheduled for execution",
        "Message published to queue",
        "Configuration reloaded",
        "Health check passed",
        "Retry attempt 1 of 3",
        "Connection closed gracefully",
        "Processing batch of 100 items",
        // Some with unicode
        "User 田中太郎 logged in",
        "Order from São Paulo processed",
        "Message from user@münchen.de",
        "Payment of €50.00 received",
        "Temperature: 23°C",
    ];

    let mut hour = 0u32;
    let mut minute = 0u32;
    let mut second = 0u32;
    let mut ms = 0u32;

    while result.len() < target_size {
        // Timestamp
        let timestamp = format!(
            "2024-01-15T{:02}:{:02}:{:02}.{:03}Z",
            hour, minute, second, ms
        );
        result.extend_from_slice(timestamp.as_bytes());
        result.push(b' ');

        // Level
        let level_idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..levels.len()))
            .unwrap_or(0);
        result.extend_from_slice(levels[level_idx].as_bytes());
        result.push(b' ');

        // Component
        let comp_idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..components.len()))
            .unwrap_or(0);
        result.push(b'[');
        result.extend_from_slice(components[comp_idx].as_bytes());
        result.push(b']');
        result.push(b' ');

        // Message
        let msg_idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..messages.len()))
            .unwrap_or(0);
        result.extend_from_slice(messages[msg_idx].as_bytes());
        result.push(b'\n');

        // Advance time
        ms += rng.as_mut().map(|r| r.gen_range(1..500)).unwrap_or(100);
        if ms >= 1000 {
            ms = 0;
            second += 1;
            if second >= 60 {
                second = 0;
                minute += 1;
                if minute >= 60 {
                    minute = 0;
                    hour = (hour + 1) % 24;
                }
            }
        }
    }

    result.truncate(target_size);
    result
}

/// Source code style content.
fn generate_source_code(target_size: usize, seed: Option<u64>) -> Vec<u8> {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut result = Vec::with_capacity(target_size);

    let code_lines = [
        "fn main() {",
        "    println!(\"Hello, world!\");",
        "}",
        "",
        "// This is a comment",
        "/* Multi-line",
        "   comment */",
        "let x = 42;",
        "let name = \"Alice\";",
        "let greeting = \"Hello, 世界!\";",
        "let emoji = \"🎉🚀💻\";",
        "const PI: f64 = 3.14159;",
        "const GREETING: &str = \"Bonjour!\";",
        "// TODO: Add error handling",
        "// FIXME: This is a workaround",
        "struct User {",
        "    name: String,",
        "    email: String,",
        "}",
        "impl User {",
        "    fn new(name: &str) -> Self {",
        "        Self { name: name.to_string(), email: String::new() }",
        "    }",
        "}",
        "// Unicode in identifiers (not valid Rust, but valid UTF-8)",
        "// let 変数 = \"value\";",
        "// let données = vec![1, 2, 3];",
        "fn calculate(α: f64, β: f64) -> f64 {",
        "    α * β + 2.0 * α",
        "}",
        "#[test]",
        "fn test_basic() {",
        "    assert_eq!(2 + 2, 4);",
        "}",
    ];

    while result.len() < target_size {
        let idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..code_lines.len()))
            .unwrap_or(result.len() % code_lines.len());
        let line = code_lines[idx];
        let line_bytes = line.as_bytes();

        if result.len() + line_bytes.len() < target_size {
            result.extend_from_slice(line_bytes);
            result.push(b'\n');
        } else {
            break;
        }
    }

    while result.len() < target_size {
        result.push(b' ');
    }
    result.truncate(target_size);
    result
}

/// JSON-like structure with unicode strings.
fn generate_json_like(target_size: usize, seed: Option<u64>) -> Vec<u8> {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut result = Vec::with_capacity(target_size);

    let names = [
        "Alice",
        "Bob",
        "Charlie",
        "田中太郎",
        "Müller",
        "García",
        "Αλέξανδρος",
    ];
    let cities = [
        "New York",
        "London",
        "東京",
        "Paris",
        "München",
        "São Paulo",
        "Москва",
    ];
    let descriptions = [
        "Software engineer",
        "Data scientist",
        "プログラマー",
        "Développeur",
        "Ingenieur",
    ];

    result.extend_from_slice(b"[\n");
    let mut first = true;

    while result.len() < target_size.saturating_sub(100) {
        if !first {
            result.extend_from_slice(b",\n");
        }
        first = false;

        let name_idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..names.len()))
            .unwrap_or(0);
        let city_idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..cities.len()))
            .unwrap_or(0);
        let desc_idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..descriptions.len()))
            .unwrap_or(0);
        let age = rng.as_mut().map(|r| r.gen_range(20..70)).unwrap_or(30);

        let entry = format!(
            "  {{\n    \"name\": \"{}\",\n    \"city\": \"{}\",\n    \"description\": \"{}\",\n    \"age\": {}\n  }}",
            names[name_idx], cities[city_idx], descriptions[desc_idx], age
        );

        result.extend_from_slice(entry.as_bytes());
    }

    result.extend_from_slice(b"\n]\n");

    while result.len() < target_size {
        result.push(b' ');
    }
    result.truncate(target_size);
    result
}

/// Pathological case: maximum multi-byte density.
fn generate_pathological(target_size: usize, seed: Option<u64>) -> Vec<u8> {
    let mut rng = seed.map(ChaCha8Rng::seed_from_u64);
    let mut result = Vec::with_capacity(target_size);

    // Use only 4-byte characters (maximum bytes per character)
    let four_byte_chars = [
        "😀", "😃", "😄", "😁", "😆", "😅", "🤣", "😂", "🙂", "🙃", "😉", "😊", "😇", "🥰", "😍",
        "🤩", "😘", "😗", "😚", "😙", "🥲", "😋", "😛", "😜", "🤪", "😝", "🤑", "🤗", "🤭", "🤫",
        "🤔", "🤐", "🤨", "😐", "😑", "😶", "😏", "😒", "🙄", "😬", "🤥", "😌", "😔", "😪", "🤤",
        "😴", "😷", "🤒", "🤕", "🤢", "🤮", "🤧", "🥵", "🥶", "🥴", "😵", "🤯", "🤠", "🥳", "🥸",
    ];

    while result.len() < target_size {
        let idx = rng
            .as_mut()
            .map(|r| r.gen_range(0..four_byte_chars.len()))
            .unwrap_or(result.len() % four_byte_chars.len());
        let char_bytes = four_byte_chars[idx].as_bytes();

        if result.len() + char_bytes.len() <= target_size {
            result.extend_from_slice(char_bytes);
        } else {
            break;
        }
    }

    // Pad with ASCII if needed (shouldn't happen much)
    while result.len() < target_size {
        result.push(b'X');
    }
    result.truncate(target_size);
    result
}
