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
