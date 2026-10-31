use tree_sitter::Node;

use super::Adapter;

/// Data files are read through `data::Doc`, not through syntax questions: nothing here applies.
pub(super) struct Data;

impl Adapter for Data {
    fn raised_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    fn call_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }
}
