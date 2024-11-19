use serde::Deserialize;
use serde_json::from_reader;
use std::collections::HashMap;
use std::fs::File;

#[derive(Clone, Deserialize, Debug)]
pub struct URLMapItem {
    pub pretty: String,
    pub sym: String,
}

#[derive(Debug)]
pub struct URLMap {
    data: HashMap<String, Vec<URLMapItem>>,
}

impl URLMap {
    fn new(data: HashMap<String, Vec<URLMapItem>>) -> Self {
        Self { data }
    }

    pub fn new_empty() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    pub fn get(&self, sym: &String) -> Option<Vec<URLMapItem>> {
        self.data.get(sym).cloned()
    }
}

pub fn read_url_map(filename: &String) -> URLMap {
    let file = match File::open(filename) {
        Ok(f) => f,
        Err(_) => {
            info!("Error trying to open URL map file [{}]", filename);
            return URLMap::new_empty();
        }
    };

    let data: HashMap<String, Vec<URLMapItem>> = match from_reader(file) {
        Ok(result) => result,
        Err(_) => {
            info!("Error trying to read URL map file [{}]", filename);
            return URLMap::new_empty();
        }
    };

    URLMap::new(data)
}
