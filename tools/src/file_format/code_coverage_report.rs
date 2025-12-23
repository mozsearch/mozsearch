// SPDX-FileCopyrightText: 2025 Mozilla
// SPDX-FileContributor: Nicolas Qiu Guichard <nicolas.guichard@kdab.com>
//
// SPDX-License-Identifier: MPL-2.0

use core::fmt;

use std::{
    collections::HashMap,
    convert::TryInto,
    io::{self, Read, Write},
    path::Path,
};

use serde::{de, Deserialize, Deserializer, Serialize};
use time::{format_description::well_known::Rfc2822, OffsetDateTime};

/// End marker for use when sending fast-import data.
/// NOTE: make sure the data doesn't actually contain this marker!
/// (This shouldn't be the case, we only output numbers and a couple of static strings.)
const END: &str = "END";

#[derive(Debug)]
pub struct Report {
    root: Directory,
    metadata: ReportMetadata,
}

#[derive(Debug)]
pub struct ReportMetadata {
    pub commit: String,
    pub branch: String,
    pub date: OffsetDateTime,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct NodeMetadata {
    coverage_percent: f32,
    lines_covered: u32,
    lines_missed: u32,
    lines_total: u32,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum Node {
    Directory(Directory),
    File(File),
}

#[derive(Deserialize, Debug)]
struct Directory {
    #[serde(flatten)]
    pub metadata: NodeMetadata,
    pub children: HashMap<String, Node>,
}

#[derive(Deserialize, Debug)]
struct File {
    #[serde(flatten)]
    pub metadata: NodeMetadata,
    #[serde(deserialize_with = "vec_int_as_vec_opt_u32")]
    pub coverage: Vec<Option<u32>>,
}

fn vec_int_as_vec_opt_u32<'de, D>(d: D) -> Result<Vec<Option<u32>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct Visitor;

    impl<'de> de::Visitor<'de> for Visitor {
        type Value = Vec<Option<u32>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("sequence of integers")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: de::SeqAccess<'de>,
        {
            let mut vec = Vec::new();
            if let Some(size_hint) = seq.size_hint() {
                vec.reserve(size_hint);
            }
            while let Some(value) = seq.next_element::<i64>()? {
                vec.push(value.try_into().ok());
            }
            Ok(vec)
        }
    }

    d.deserialize_seq(Visitor)
}

impl Report {
    pub fn read(json: impl Read, metadata: ReportMetadata) -> io::Result<Report> {
        let root = serde_json::from_reader(json)?;
        let report = Report { root, metadata };
        Ok(report)
    }

    pub fn write_to_git(
        &self,
        fast_import: &mut impl Write,
        incremental: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let branch = &self.metadata.branch;

        writeln!(fast_import, "feature date-format=rfc2822")?;
        writeln!(fast_import, "commit refs/heads/{branch}")?;
        writeln!(fast_import, "mark :1")?;
        write!(fast_import, "committer <searchfox> ")?;
        self.metadata.date.format_into(fast_import, &Rfc2822)?;
        writeln!(fast_import)?;
        writeln!(fast_import, "data {}", self.metadata.commit.len() + 1)?;
        writeln!(fast_import, "{}", self.metadata.commit)?;
        if incremental {
            writeln!(fast_import, "from {branch}^0")?;
            writeln!(fast_import, "deleteall")?;
        }
        self.root.write_to_git(fast_import, "")?;

        writeln!(fast_import, "tag reverse/{branch}/{}", self.metadata.commit)?;
        writeln!(fast_import, "from :1")?;
        write!(fast_import, "tagger <searchfox> ")?;
        self.metadata.date.format_into(fast_import, &Rfc2822)?;
        writeln!(fast_import)?;
        writeln!(fast_import, "data 0")?;

        Ok(())
    }
}

impl Node {
    fn write_to_git<P: AsRef<Path>>(
        &self,
        fast_import: &mut impl Write,
        path: P,
    ) -> io::Result<()> {
        use Node::*;
        match self {
            Directory(directory) => directory.write_to_git(fast_import, path),
            File(file) => file.write_to_git(fast_import, path),
        }
    }
}

impl Directory {
    fn write_to_git<P: AsRef<Path>>(
        &self,
        fast_import: &mut impl Write,
        path: P,
    ) -> io::Result<()> {
        for (name, child) in &self.children {
            let path = path.as_ref().join(name);
            child.write_to_git(fast_import, &path)?;
        }

        {
            let path = path.as_ref().join("index.json");
            let path = path.as_os_str().to_string_lossy();
            writeln!(fast_import, "M 644 inline {path}")?;
            writeln!(fast_import, "data <<{END}")?;
            serde_json::to_writer(&mut *fast_import, &self.metadata)?;
            writeln!(fast_import)?;
            writeln!(fast_import, "{END}")?;
        }

        Ok(())
    }
}

impl File {
    fn write_to_git<P: AsRef<Path>>(
        &self,
        fast_import: &mut impl Write,
        path: P,
    ) -> io::Result<()> {
        let path = path.as_ref().as_os_str().to_string_lossy();
        {
            writeln!(fast_import, "M 644 inline {path}")?;
            writeln!(fast_import, "data <<{END}")?;
            for line in &self.coverage {
                if let Some(count) = line {
                    writeln!(fast_import, "{count}")?;
                } else {
                    writeln!(fast_import)?;
                }
            }
            writeln!(fast_import, "{END}")?;
        }

        {
            writeln!(fast_import, "M 644 inline {path}.summary.json")?;
            writeln!(fast_import, "data <<{END}")?;
            serde_json::to_writer(&mut *fast_import, &self.metadata)?;
            writeln!(fast_import)?;
            writeln!(fast_import, "{END}")?;
        }

        Ok(())
    }
}
