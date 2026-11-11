//! yq's `-p base64` and `-p uri` input formats (#6925).
//!
//! Each reads a whole input as one string, which yq makes a plain `!!str` node
//! (`createStringScalarNode`). The run reads that string as the JSON string holding its bytes,
//! that is, as `-p json`: a JSON string and yq's decoded node evaluate and print alike (tag, style,
//! the quoting the YAML encoder picks), and every route after the read (streaming, the DOM,
//! `--eval-all`, `-i`) is the JSON route's. Decoded bytes that are not UTF-8 stay as they are in the
//! JSON text, so they meet the `-p json` handling of #1616.

use crate::InputFormat;

/// An input format yq decodes to one string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StringInput {
    /// `decoder_base64.go`: Go's streaming base64 decoder over the trimmed, padded input.
    Base64,
    /// `decoder_uri.go`: `url.QueryUnescape` of the whole input.
    Uri,
}

impl StringInput {
    /// The run's string format: an explicit `-p base64` / `-p uri`, or under `-p auto` the first
    /// file's extension, which yq lowercases and matches against the format names (`a.base64`,
    /// `a.URI`). Every file of a run is read with that one format (#4394).
    pub(super) fn for_run(format: InputFormat, files: &[String]) -> Option<Self> {
        match format {
            InputFormat::Base64 => Some(Self::Base64),
            InputFormat::Uri => Some(Self::Uri),
            InputFormat::Auto => match super::lowercase_extension(files.first()?).as_deref() {
                Some("base64") => Some(Self::Base64),
                Some("uri") => Some(Self::Uri),
                _ => None,
            },
            InputFormat::Yaml | InputFormat::Json => None,
        }
    }

    /// The code [`Self::from_code`] reads back, for the run's static.
    pub(super) fn code(format: Option<Self>) -> u8 {
        match format {
            None => 0,
            Some(Self::Base64) => 1,
            Some(Self::Uri) => 2,
        }
    }

    pub(super) fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Base64),
            2 => Some(Self::Uri),
            _ => None,
        }
    }

    /// `raw` decoded, as the text of the JSON string holding it; `Err` is yq's message for an
    /// input that does not decode, which it prints after `bad file '<name>': `.
    pub(super) fn transcode(self, raw: &[u8]) -> Result<Vec<u8>, String> {
        let decoded = match self {
            Self::Base64 => succinctly::jq::go_base64_decode(raw).map_err(|e| e.to_string())?,
            Self::Uri => decode_uri(raw)?,
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
