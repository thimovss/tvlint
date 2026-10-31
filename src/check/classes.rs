//! The class hierarchy of the whole project, for `forbid_catch` and `in_class`.

use std::collections::{BTreeMap, BTreeSet};

use rayon::prelude::*;

use crate::adapters;
use crate::source::SourceFile;
use super::visit;

/// A class and the names of its direct bases.
type Class = (String, Vec<String>);

/// Class name -> names of its direct bases, across every file in the project.
/// Resolution is by simple name, not by import path. Languages that cannot extend each other's
/// classes (Python and TypeScript) keep separate hierarchies, so equal names never meet.
#[derive(Default)]
pub struct ClassIndex {
    bases: BTreeMap<&'static str, BTreeMap<String, BTreeSet<String>>>,
}

impl ClassIndex {
    pub fn build(files: &[SourceFile]) -> ClassIndex {
        let per_file: Vec<(Option<&'static str>, Vec<Class>)> = files
            .par_iter()
            .map(|file| {
                let adapter = adapters::for_language(file.language);
                let family = adapter.class_family();
                let mut classes = Vec::new();
                if family.is_some() {
                    visit(file.tree.root_node(), &mut |node| {
                        if let Some(class) = adapter.class_bases(node, &file.text) {
                            classes.push(class);
                        }
                    });
                }
                (family, classes)
            })
            .collect();
        let mut index = ClassIndex::default();
        for (family, classes) in per_file {
            let Some(family) = family else { continue };
            for (name, bases) in classes {
                index.bases.entry(family).or_default().entry(name).or_default().extend(bases);
            }
        }
        index
    }

    pub fn defines(&self, family: &str, class: &str) -> bool {
        self.bases.get(family).is_some_and(|classes| classes.contains_key(class))
    }

    /// `root` and every class of the family that transitively inherits from it.
    pub fn subclasses_of(&self, family: &str, root: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::from([root.to_string()]);
        let Some(classes) = self.bases.get(family) else { return found };
        loop {
            let before = found.len();
            for (class, bases) in classes {
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
