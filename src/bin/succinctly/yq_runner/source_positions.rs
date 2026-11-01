//! The source position `--eval-all` answers for `line` and `column` (#4646).
//!
//! `--eval-all` evaluates each document through a reindexed copy of its value, so the cursor a
//! node stands on indexes the regenerated text (`[1,14]` for the `b` on the third line). The
//! scope [`enter`] installs is asked for the node by its path instead, and answers from the
//! source's own index: the cursor, and so the `line`/`column`, the streaming route reads.
//!
//! The index of a source is built when the first position is asked of it, so a run that never
//! reads one pays nothing beyond keeping a copy of the bytes. A path is followed one step at a
//! time from the cursor of its parent, which is remembered, and the elements of a sequence asked
//! for in order resume where the last one stopped: reading the position of every node costs one
//! step each, not a walk from the root.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use succinctly::jq::document::{DocumentCursor, DocumentElements, DocumentFields, DocumentValue};
use succinctly::jq::eval_generic::{SourcePath, SourcePathStep};
use succinctly::jq::{self, Expr};
use succinctly::yaml::{YamlCursor, YamlIndex, YamlValue};

use super::{gathered_file_indices, resolve_run_input_format, GatheredSource};
use crate::InputFormat;

type Cursor<'a> = YamlCursor<'a, Vec<u64>>;

/// Installs the resolver for a run whose filter reads `line` or `column`, until the returned
/// guard is dropped. The sources are the run's own, in the order `eval_all_documents` reads them.
pub(super) fn enter(
    expr: &Expr,
    input_sources: &[GatheredSource],
    input_files: &[String],
    input_format: InputFormat,
) -> Option<jq::SourcePositionsScope> {
    if !jq::walk::uses_position_builtins(expr) {
        return None;
    }
    let file_indices = gathered_file_indices(
        input_files,
        resolve_run_input_format(input_format, input_files),
    );
    let sources: Vec<Source> = input_sources
        .iter()
        .enumerate()
        .filter(|(_, (_, format, _))| *format != InputFormat::Json)
        .map(|(source_idx, (bytes, ..))| Source {
            file: file_indices.get(source_idx).copied().unwrap_or(source_idx),
            bytes: bytes.clone(),
            loaded: OnceCell::new(),
        })
        .collect();
    Some(jq::enter_source_positions_scope(Rc::new(
        move |file, document, path, at_key| {
            sources
                .iter()
                .find(|source| source.file == file)?
                .position(document, path, at_key)
        },
    )))
}

/// One YAML input file: its bytes and, once a position is asked of it, its index.
struct Source {
    file: usize,
    bytes: Vec<u8>,
    loaded: OnceCell<Option<Loaded>>,
}

/// A source's index and what has been learned about it. Everything remembered is a node id, so
/// nothing here borrows the index.
struct Loaded {
    index: YamlIndex,
    /// The node id of each document's root.
    documents: Vec<usize>,
    /// (document, address of a path link) -> the node id that path leads to.
    nodes: RefCell<HashMap<(usize, usize), usize>>,
    /// (document, address of a sequence's path link) -> how its elements are reached.
    sequences: RefCell<HashMap<(usize, usize), Sequence>>,
    /// (document, address of a mapping's path link) -> what is known of its keys.
    members: RefCell<HashMap<(usize, usize), Members>>,
}

/// How the elements of one sequence are reached.
enum Sequence {
    /// The elements have been asked for in order so far: the index of the one after the element
    /// asked for last, and the list standing there.
    Resume(usize, usize),
    /// An element was asked for out of order, so the sequence is listed once: the node id of
    /// each element.
    Listed(Vec<usize>),
}

/// What is known of one mapping's keys.
enum Members {
    /// A member has been looked up once, by the walk every lookup used to make.
    Seen,
    /// The mapping has been looked up before, so its keys are indexed: key -> value node id.
    Indexed(HashMap<String, usize>),
    /// The mapping lists a merge-resolved view, which only yq's own traversal answers.
    Walked,
}

impl Source {
    fn loaded(&self) -> Option<&Loaded> {
        self.loaded
            .get_or_init(|| {
                let index = YamlIndex::build(&self.bytes).ok()?;
                let mut documents = Vec::new();
                let YamlValue::Sequence(mut rest) = index.root(&self.bytes).value() else {
                    return None;
                };
                while let Some((document, next)) = DocumentElements::uncons_cursor(&rest) {
                    documents.push(document.node_id());
                    rest = next;
                }
                Some(Loaded {
                    index,
                    documents,
                    nodes: RefCell::default(),
                    sequences: RefCell::default(),
                    members: RefCell::default(),
                })
            })
            .as_ref()
    }

    /// `(line, column)` of the node `path` leads to in document `document` (of its key, when
    /// `at_key`), or `None` where the source has no such node.
    fn position(&self, document: usize, path: &SourcePath, at_key: bool) -> Option<(usize, usize)> {
        let loaded = self.loaded()?;
        let root = loaded.index.root(&self.bytes);
        let node = loaded.resolve(root, document, path)?;
        // The key of a member is the sibling before its value, in both indexed formats.
        let node = if at_key { node.prev_sibling()? } else { node };
        Some((node.line(), node.column()))
    }
}

impl Loaded {
    /// The cursor `path` leads to in document `document`.
    fn resolve<'a>(
        &self,
        root: Cursor<'a>,
        document: usize,
        path: &SourcePath,
    ) -> Option<Cursor<'a>> {
        let Some(link) = path else {
            return root.at_node_id(*self.documents.get(document)?);
        };
        let key = (document, Rc::as_ptr(link) as usize);
        let known = self.nodes.borrow().get(&key).copied();
        if let Some(id) = known {
            return root.at_node_id(id);
        }
        let parent = self.resolve(root, document, &link.parent)?;
        let child = match &link.step {
            SourcePathStep::Member(name) => self.member(document, &link.parent, parent, name)?,
            SourcePathStep::Element(index) => {
                self.element(document, &link.parent, parent, *index)?
            }
        };
        self.nodes.borrow_mut().insert(key, child.node_id());
        Some(child)
    }

    /// The value of member `name` of the mapping at `parent`: the last member spelled `name`.
    ///
    /// The first lookup in a mapping walks its members, as a lookup always has. A second one
    /// indexes the keys once (a walk makes the same choices: the last of equal keys wins, a
    /// complex key is spelled `""`, a key that is not a string is never matched), so reading
    /// every member of a wide mapping is not a walk of the mapping per member. A mapping with a
    /// merge key keeps the walk, which is yq's traversal and has no key list to index.
    fn member<'a>(
        &self,
        document: usize,
        path: &SourcePath,
        parent: Cursor<'a>,
        name: &str,
    ) -> Option<Cursor<'a>> {
        let fields = parent.value().as_object()?;
        let slot = (
            document,
            path.as_ref().map_or(0, |link| Rc::as_ptr(link) as usize),
        );
        let mut members = self.members.borrow_mut();
        match members.get(&slot) {
            None => {
                members.insert(slot, Members::Seen);
                drop(members);
                return fields.find_cursor(name);
            }
            Some(Members::Seen) => {
                if fields.head_key_cursor().is_some() {
                    let mut keys = HashMap::new();
                    let mut rest = fields.clone();
                    while let Some((field, next)) = rest.uncons() {
                        rest = next;
                        match field.key() {
                            YamlValue::String(key) => {
                                if let Ok(key) = key.as_str() {
                                    keys.insert(key.to_string(), field.value_cursor().node_id());
                                }
                            }
                            YamlValue::Mapping(_) | YamlValue::Sequence(_) => {
                                keys.insert(String::new(), field.value_cursor().node_id());
                            }
                            _ => {}
                        }
                    }
                    members.insert(slot, Members::Indexed(keys));
                } else {
                    members.insert(slot, Members::Walked);
                }
            }
            Some(_) => {}
        }
        match members.get(&slot) {
            Some(Members::Indexed(keys)) => {
                let id = keys.get(name).copied();
                drop(members);
                id.and_then(|id| parent.at_node_id(id))
                    .or_else(|| fields.find_cursor(name))
            }
            _ => {
                drop(members);
                fields.find_cursor(name)
            }
        }
    }

    /// Element `index` of the sequence at `parent`.
    ///
    /// Elements asked for in order resume after the one read last; the first one asked for out of
    /// order lists the sequence once, so no order costs a walk from the head per element.
    fn element<'a>(
        &self,
        document: usize,
        path: &SourcePath,
        parent: Cursor<'a>,
        index: usize,
    ) -> Option<Cursor<'a>> {
        let elements = parent.value().as_array()?;
        let slot = (
            document,
            path.as_ref().map_or(0, |link| Rc::as_ptr(link) as usize),
        );
        let mut sequences = self.sequences.borrow_mut();
        let rest = match sequences.get(&slot) {
            Some(Sequence::Listed(ids)) => {
                return ids.get(index).and_then(|id| parent.at_node_id(*id));
            }
            Some(Sequence::Resume(next, head)) if *next == index => elements.at_head_id(*head)?,
            Some(Sequence::Resume(..)) => {
                let mut ids = Vec::new();
                let mut rest = elements;
                while let Some((element, after)) = DocumentElements::uncons_cursor(&rest) {
                    ids.push(element.node_id());
                    rest = after;
                }
                let found = ids.get(index).and_then(|id| parent.at_node_id(*id));
                sequences.insert(slot, Sequence::Listed(ids));
                return found;
            }
            None => {
                let mut rest = elements;
                for _ in 0..index {
                    rest = DocumentElements::uncons_cursor(&rest)?.1;
                }
                rest
            }
        };
        let (element, after) = DocumentElements::uncons_cursor(&rest)?;
        match after.head_id() {
            Some((head, _)) => sequences.insert(slot, Sequence::Resume(index + 1, head)),
            None => sequences.remove(&slot),
        };
        Some(element)
    }
}
