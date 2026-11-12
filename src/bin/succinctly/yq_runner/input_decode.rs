//! yq's input formats that the run reads by transcoding them first (#6925).
//!
//! `-p base64` and `-p uri` read a whole input as one string, which yq makes a plain `!!str` node
//! (`createStringScalarNode`). The run reads that string as the JSON string holding its bytes,
//! that is, as `-p json`: a JSON string and yq's decoded node evaluate and print alike (tag, style,
//! the quoting the YAML encoder picks), and every route after the read (streaming, the DOM,
//! `--eval-all`, `-i`) is the JSON route's. Decoded bytes that are not UTF-8 stay as they are in the
//! JSON text, so they meet the `-p json` handling of #1616.
//!
//! `-p props` reads a Java properties file as the YAML text yq prints for it, comments included
//! ([`super::input_props`]), and `-p csv` / `-p tsv` a table as the sequence of mappings yq makes of
//! its records ([`super::input_csv`]), so those runs are `-p yaml` runs.

use crate::InputFormat;

/// An input format the run transcodes before reading it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TranscodedInput {
    /// `decoder_base64.go`: Go's streaming base64 decoder over the trimmed, padded input.
    Base64,
    /// `decoder_uri.go`: `url.QueryUnescape` of the whole input.
    Uri,
    /// `decoder_properties.go`: magiconair/properties, each key a path into one mapping.
    Props,
    /// `decoder_csv_object.go`: Go's `encoding/csv`, a record to a mapping of the header's names.
    Csv,
    /// The same decoder with a tab for its separator and `--tsv-auto-parse` for its cells.
    Tsv,
    /// `decoder_ini.go`: go-ini, a mapping of the `DEFAULT` section's keys and one per section.
    Ini,
}

/// The flags a transcoder reads besides the input.
#[derive(Clone, Copy, Debug)]
pub(super) struct TranscodeOptions {
    /// `--properties-array-brackets`, which yq's properties decoder reads as well as its encoder.
    pub array_brackets: bool,
    /// `--csv-separator`, the field separator of `-p csv`.
    pub csv_separator: char,
    /// `--csv-auto-parse`: whether a `-p csv` cell is read as a snippet of YAML.
    pub csv_auto_parse: bool,
    /// `--tsv-auto-parse`, the same for `-p tsv`.
    pub tsv_auto_parse: bool,
    /// `--ini-preserve-quotes`: a value keeps the quotes that surround it.
    pub ini_preserve_quotes: bool,
}

impl TranscodedInput {
    /// The run's transcoded format: an explicit `-p base64` / `-p uri` / `-p props`, or under
    /// `-p auto` the first file's extension, which yq lowercases and matches against the format
    /// names and their aliases (`a.base64`, `a.URI`, `a.properties`, `a.p`). Every file of a run is
    /// read with that one format (#4394).
    pub(super) fn for_run(format: InputFormat, files: &[String]) -> Option<Self> {
        match format {
            InputFormat::Base64 => Some(Self::Base64),
            InputFormat::Uri => Some(Self::Uri),
            InputFormat::Props => Some(Self::Props),
            InputFormat::Csv => Some(Self::Csv),
            InputFormat::Tsv => Some(Self::Tsv),
            InputFormat::Ini => Some(Self::Ini),
            InputFormat::Auto => match super::lowercase_extension(files.first()?).as_deref() {
                Some("base64") => Some(Self::Base64),
                Some("uri") => Some(Self::Uri),
                Some("props" | "p" | "properties") => Some(Self::Props),
                Some("csv" | "c") => Some(Self::Csv),
                Some("tsv" | "t") => Some(Self::Tsv),
                Some("ini" | "i") => Some(Self::Ini),
                _ => None,
            },
            InputFormat::Yaml | InputFormat::Json => None,
        }
    }

    /// The format the transcoded bytes are read as.
    pub(super) fn reads_as(self) -> InputFormat {
        match self {
            Self::Base64 | Self::Uri => InputFormat::Json,
            Self::Props | Self::Csv | Self::Tsv | Self::Ini => InputFormat::Yaml,
        }
    }

    /// The name yq gives the format, for a message.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Base64 => "base64",
            Self::Uri => "uri",
            Self::Props => "props",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Ini => "ini",
        }
    }

    /// Whether an empty input is a document: the empty string for a string format and `null` for
    /// a table, where yq's properties decoder reads no document from an empty input (`io.EOF`).
    pub(super) fn empty_is_document(self) -> bool {
        match self {
            Self::Base64 | Self::Uri | Self::Csv | Self::Tsv | Self::Ini => true,
            Self::Props => false,
        }
    }

    /// The code [`Self::from_code`] reads back, for the run's static.
    pub(super) fn code(format: Option<Self>) -> u8 {
        match format {
            None => 0,
            Some(Self::Base64) => 1,
            Some(Self::Uri) => 2,
            Some(Self::Props) => 3,
            Some(Self::Csv) => 4,
            Some(Self::Tsv) => 5,
            Some(Self::Ini) => 6,
        }
    }

    pub(super) fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Base64),
            2 => Some(Self::Uri),
            3 => Some(Self::Props),
            4 => Some(Self::Csv),
            5 => Some(Self::Tsv),
            6 => Some(Self::Ini),
            _ => None,
        }
    }

    /// `raw` as the run reads it: for a string format the text of the JSON string it decodes to,
    /// for props and the tables the YAML text of their document; `Err` is yq's message for an
    /// input that does not decode, which it prints after `bad file '<name>': `.
    pub(super) fn transcode(
        self,
        raw: &[u8],
        options: &TranscodeOptions,
    ) -> Result<Vec<u8>, String> {
        let decoded = match self {
            Self::Base64 => succinctly::jq::go_base64_decode(raw).map_err(|e| e.to_string())?,
            Self::Uri => decode_uri(raw)?,
            Self::Props => return super::input_props::transcode(raw, options.array_brackets),
            Self::Csv => {
                return super::input_csv::transcode(
                    raw,
                    options.csv_separator,
                    options.csv_auto_parse,
                )
            }
            Self::Tsv => return super::input_csv::transcode(raw, '\t', options.tsv_auto_parse),
            Self::Ini => return super::input_ini::transcode(raw, options.ini_preserve_quotes),
        };
        Ok(json_string(&decoded))
    }
}

/// `bytes` as a JSON string: `"` and `\` escaped, a control character below U+0020 written as an
/// escape, every other byte as it is (a byte that is not UTF-8 included).
fn json_string(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 2);
    out.push(b'"');
    for &byte in bytes {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x00..=0x1F => out.extend_from_slice(format!("\\u{byte:04x}").as_bytes()),
            _ => out.push(byte),
        }
    }
    out.push(b'"');
    out
}

/// `url.QueryUnescape` of the whole input, the definition yq-mode `@urid` uses too (`+` is a
/// space, `invalid URL escape "%zz"`).
fn decode_uri(raw: &[u8]) -> Result<Vec<u8>, String> {
    succinctly::jq::go_query_unescape(raw).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(input: &[u8]) -> Result<Vec<u8>, String> {
        decode_uri(input)
    }

    #[test]
    fn uri_unescapes_like_query_unescape() {
        assert_eq!(uri(b"a+b%41\n"), Ok(b"a bA\n".to_vec()));
        assert_eq!(uri(b"a%FFb"), Ok(b"a\xffb".to_vec()));
        assert_eq!(uri(b"%zz"), Err(r#"invalid URL escape "%zz""#.into()));
        assert_eq!(uri(b"ab%"), Err(r#"invalid URL escape "%""#.into()));
        assert_eq!(uri(b"ab%\n"), Err(r#"invalid URL escape "%\n""#.into()));
        assert_eq!(uri(b"ab%\xc3"), Err(r#"invalid URL escape "%\xc3""#.into()));
        assert_eq!(
            uri("ab%é".as_bytes()),
            Err(r#"invalid URL escape "%é""#.into())
        );
    }

    #[test]
    fn json_string_escapes_only_what_json_needs() {
        assert_eq!(
            json_string(b"a\"b\\c\n\x01\x7f\xff"),
            b"\"a\\\"b\\\\c\\n\\u0001\x7f\xff\""
        );
    }
}
