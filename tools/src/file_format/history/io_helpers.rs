use itertools::Itertools;
use serde::{Deserialize, Serialize};
use serde_json::to_string;

pub fn read_record_file_contents<
    Header: for<'a> Deserialize<'a>,
    Record: for<'a> Deserialize<'a>,
>(
    contents: &[u8],
) -> Option<(Header, Vec<Record>)> {
    let contents_as_str = std::str::from_utf8(contents).unwrap();
    let mut lines = contents_as_str.lines();
    let header: Header = match lines.next() {
        Some(line) => serde_json::from_str(line).unwrap(),
        None => return None,
    };
    let mut records: Vec<Record> = vec![];
    for line in lines {
        records.push(serde_json::from_str(line).unwrap());
    }
    Some((header, records))
}

pub fn record_file_contents_to_string<
    Header: Serialize,
    Record: Serialize,
>(
    header: &Header,
    records: &Vec<Record>,
) -> String {
    let header_iter = std::iter::once(to_string(&header).unwrap());
    let records_iter = records.iter().map(|rec| to_string(rec).unwrap());
    header_iter.chain(records_iter).join("\n")
}
