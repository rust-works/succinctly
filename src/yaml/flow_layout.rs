//! The text of a flow collection with comments inside it, laid out the way go-yaml writes it
//! (#2708, #6334). Shared by the DOM emitter of `succinctly yq` and the cursor route, which stream
//! the same collections through different walkers.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// One element of a flow collection being rendered: its text, and the line comment
/// it carries (#2708).
pub struct FlowItem {
    pub text: String,
    /// The element's own trailing comment, `#` and all, possibly several lines.
    pub comment: Option<String>,
    /// Whether the element is itself a collection (empty or not), which takes its
    /// comment differently from a scalar. An alias to one is not: it is written `*x`.
    pub container: bool,
}

/// The indent a flow collection's elements continue at after a line break (#2708).
///
/// That is go-yaml's `emitter.indent` once it has entered the collection. The collection's
/// caller computed that as `indent`, except at the document root, where it is empty
/// and the first flow level is one step in.
#[must_use]
pub fn flow_continuation(indent: &str, step: &str) -> String {
    if indent.is_empty() { step } else { indent }.to_string()
}

/// A flow collection's text, `[a, b]` or `{k: v}`, with element line comments placed as go-yaml does.
///
/// `continuation` is the indent the elements continue at after a line break (#2708) and `owner`
/// the indent of the container the collection sits in, where a closing `]` goes.
///
/// A `#` comment runs to the end of the line, so a commented element breaks the
/// line. After a scalar (an alias too) the comma comes first (`[1, # c`) and the
/// next element continues on the next line at `continuation`; when it was the last,
/// go-yaml has written a trailing comma and the closing bracket gets a line of its
/// own. After a collection (an empty one too) the comment follows the closing
/// bracket with no comma (`[[1] # c`) and the comma opens the next line at column 0
/// (`, 2]`). A further line of a multi-line comment sits at `continuation`. A `]`
/// closes at `owner`, but a `}` always at column 0, which go-yaml does not do
/// consistently. Without a comment this is the elements joined by `, `.
///
/// Out of line to keep the recursive emitter's frame small (see
/// `emit_yaml_value_panics_past_nesting_depth_limit_1017`).
#[inline(never)]
#[must_use]
pub fn render_flow(
    open: char,
    close: char,
    items: Vec<FlowItem>,
    continuation: &str,
    owner: &str,
) -> String {
    if items.iter().all(|item| item.comment.is_none()) {
        let joined = items
            .into_iter()
            .map(|item| item.text)
            .collect::<Vec<_>>()
            .join(", ");
        return format!("{open}{joined}{close}");
    }
    #[derive(PartialEq)]
    enum Pending {
        Nothing,
        AfterScalar,
        AfterCollection,
    }
    let mut out = String::from(open);
    let mut pending = Pending::Nothing;
    for (i, item) in items.into_iter().enumerate() {
        match pending {
            Pending::Nothing if i > 0 => out.push_str(", "),
            Pending::Nothing => {}
            Pending::AfterScalar => {
                out.push('\n');
                out.push_str(continuation);
            }
            Pending::AfterCollection => out.push_str("\n, "),
        }
        out.push_str(&item.text);
        pending = Pending::Nothing;
        if let Some(comment) = item.comment {
            let mut lines = comment.split('\n');
            out.push_str(if item.container { " " } else { ", " });
            out.push_str(lines.next().unwrap_or_default());
            for line in lines {
                out.push('\n');
                if !line.is_empty() {
                    out.push_str(continuation);
                    out.push_str(line);
                }
            }
            pending = if item.container {
                Pending::AfterCollection
            } else {
                Pending::AfterScalar
            };
        }
    }
    if pending != Pending::Nothing {
        out.push('\n');
        if close == ']' {
            out.push_str(owner);
        }
    }
    out.push(close);
    out
}
