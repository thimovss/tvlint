//! The class hierarchy of the whole project, for `forbid_catch`.

use std::collections::{BTreeMap, BTreeSet};

use rayon::prelude::*;

use crate::adapters;
use crate::source::SourceFile;
use super::visit;

/// Class name -> names of its direct bases, across every file in the project.
/// Resolution is by simple name, not by import path.
#[derive(Default)]
pub struct ClassIndex {
    pub(super) bases: BTreeMap<String, BTreeSet<String>>,
}

impl ClassIndex {
    pub fn build(files: &[SourceFile]) -> ClassIndex {
        let per_file: Vec<Vec<(String, Vec<String>)>> = files
            .par_iter()
            .map(|file| {
                let adapter = adapters::for_language(file.language);
                let mut classes = Vec::new();
                if adapter.indexes_classes() {
                    visit(file.tree.root_node(), &mut |node| {
                        if let Some(class) = adapter.class_bases(node, &file.text) {
                            classes.push(class);
                        }
                    });
                }
                classes
            })
            .collect();
        let mut index = ClassIndex::default();
        for (name, bases) in per_file.into_iter().flatten() {
            index.bases.entry(name).or_default().extend(bases);
        }
        index
    }

    pub fn defines(&self, class: &str) -> bool {
        self.bases.contains_key(class)
    }

    /// `root` and every class that transitively inherits from it.
    pub fn subclasses_of(&self, root: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::from([root.to_string()]);
        loop {
            let before = found.len();
            for (class, bases) in &self.bases {
                if bases.iter().any(|b| found.contains(b)) {
                    found.insert(class.clone());
                }
            }
            if found.len() == before {
                return found;
            }
        }
    }
}
