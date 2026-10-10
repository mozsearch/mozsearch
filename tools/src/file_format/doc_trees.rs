use serde_json::from_reader;
use std::collections::HashMap;
use std::fs::File;

#[derive(Debug)]
pub struct DocTrees {
    data: HashMap<String, String>,
}

impl DocTrees {
    fn new(data: HashMap<String, String>) -> Self {
        Self { data }
    }

    pub fn new_empty() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    pub fn find(&self, src_path: &str) -> Option<String> {
        for (target_prefix, src_prefix) in &self.data {
            if let Some(inner_path) = src_path.strip_prefix(src_prefix) {
                let no_ext_inner_path = inner_path
                    .strip_suffix(".md")
                    .or_else(|| inner_path.strip_suffix(".rst"))?;
                return Some(target_prefix.to_owned() + no_ext_inner_path + ".html");
            }
        }

        None
    }

    /// The source path (without an extension) of a document by its path in
    /// the staged documentation (without an extension), the reverse of
    /// `find`.
    pub fn find_source(&self, target_path: &str) -> Option<String> {
        self.data
            .iter()
            .filter(|(target_prefix, _)| target_path.starts_with(target_prefix.as_str()))
            .max_by_key(|(target_prefix, _)| target_prefix.len())
            .map(|(target_prefix, src_prefix)| {
                src_prefix.to_owned() + &target_path[target_prefix.len()..]
            })
    }
}

pub fn read_doc_trees(filename: &String) -> DocTrees {
    let file = match File::open(filename) {
        Ok(f) => f,
        Err(_) => {
            info!("Error trying to open doc trees file [{}]", filename);
            return DocTrees::new_empty();
        }
    };

    let data: HashMap<String, String> = match from_reader(file) {
        Ok(result) => result,
        Err(_) => {
            info!("Error trying to read doc trees file [{}]", filename);
            return DocTrees::new_empty();
        }
    };

    DocTrees::new(data)
}
