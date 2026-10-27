use itertools::Itertools;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

extern crate regex;
use serde_json::to_value;
use serde_json::{Value, from_value, json, to_string};

use ustr::Ustr;

use super::analysis::AnalysisUnion;
use super::analysis::{
    AnalysisLine, AnalysisSource, AnalysisStructured, ExpansionInfo, Location, WithLocation,
    filter_analysis_value, for_each_analysis_line, group_by_location, parse_analysis_line,
};

#[derive(Debug)]
pub struct HashedStructured {
    pub platforms: Vec<usize>,
    pub loc: Location,
    pub data: AnalysisStructured,
}

/// Given a list of files and a matching parallel list of platform identifiers,
/// merge the records and write them to the provided writer.
///
/// This logic was extracted out from `merge-analyses.rs` for the purpose of
/// being able to test its logic through the introduction of `cmd_merge_analyses`.
///
/// The logic could almost certainly be further unified into the `cmd_pipeline`
/// data model, with callers potentially altered to use searchfox-tool and
/// eliminate the standalone merge-analyses.rs binary.  But there's no urgency.
/// A record of a file, as `merge_files` uses it.
enum Record {
    /// Not a record it uses (or not a record).
    Ignored,
    /// A target record, as it's written out (once, however many files have
    /// it).
    Target(String),
    /// A source record, with the platform it's from left to fill in.
    Source(Location, AnalysisSource<Ustr>),
    /// A structured record, with its JSON's hash.
    Structured(Location, u64, AnalysisStructured),
}

pub fn merge_files<W: std::io::Write>(filenames: &[String], platforms: &[String], mut writer: W) {
    let mut unique_targets = HashSet::new();
    // Maps from symbol name to a BTreeMap<u64 hash, HashedStructured>
    let mut structured_syms: BTreeMap<Ustr, BTreeMap<u64, HashedStructured>> = BTreeMap::new();
    let mut sources: Vec<WithLocation<AnalysisSource<Ustr>>> = vec![];

    let record_of = |obj: Value, loc: &Location| -> Record {
        let Ok(unified) = from_value::<AnalysisUnion<Ustr>>(obj) else {
            return Record::Ignored;
        };
        match unified {
            AnalysisUnion::Source(src) => Record::Source(*loc, src),
            // for target objects, just print them back out (deduplicated)
            AnalysisUnion::Target(tgt) => Record::Target(
                to_string(&WithLocation {
                    data: tgt,
                    loc: *loc,
                })
                .unwrap(),
            ),
            // Structured objects may have different data for different platforms.  We
            // detect this by building a map for each symbol from the hash of the string
            // representation of their JSON encoding to the AnalysisStructured
            // representation.  If, after processing the files we find there was a single
            // hash, then we emit that record as we originally found it.  However, if there
            // were multiple hashes, we pick the last.
            //
            // We used to have AnalysisStructured be hashable, but the `extra` Map was
            // not currently hashable due to https://github.com/serde-rs/json/issues/747
            // and in reality we just want to hash the JSON string, but it's already
            // been parsed into a Value, which is why we're not using the string.
            //
            // (By hash, in order, so that the output doesn't depend on a HashMap's
            // random order when variants tie: ex: a local variable's and a
            // parameter's records with the same location-derived symbol, from one
            // platform.)
            AnalysisUnion::Structured(structured) => {
                let json_str = to_string(&structured).unwrap();
                let mut hasher = DefaultHasher::new();
                json_str.hash(&mut hasher);
                Record::Structured(*loc, hasher.finish(), structured)
            }
        }
    };
    // A record of file `i_file`'s.
    let mut use_record = |record: &Record, i_file: usize| match record {
        Record::Ignored => {}
        Record::Target(target_str) => {
            if !unique_targets.contains(target_str) {
                writeln!(writer, "{}", target_str).unwrap();
                unique_targets.insert(target_str.clone());
            }
        }
        Record::Source(loc, src) => {
            let mut src = src.clone();
            // populate the platform key of expansions and expanded symbols
            use ExpansionInfo::*;
            match src.expansion_info {
                Some(ExpandsTo(ref mut expansions)) => {
                    for expansions in expansions.values_mut() {
                        if let Some(unnamed_expansions) = expansions.remove("") {
                            expansions.insert(platforms[i_file].clone(), unnamed_expansions);
                        }
                    }
                }
                Some(InExpansionAt(ref mut offsets)) => {
                    for offsets in offsets.values_mut() {
                        if let Some(unnamed_offsets) = offsets.remove("") {
                            offsets.insert(platforms[i_file].clone(), unnamed_offsets);
                        }
                    }
                }
                None => {}
            }
            sources.push(WithLocation {
                data: src,
                loc: *loc,
            });
        }
        Record::Structured(loc, hash_key, structured) => {
            let variants = structured_syms.entry(structured.sym).or_default();
            let hs = variants
                .entry(*hash_key)
                .or_insert_with(|| HashedStructured {
                    platforms: vec![],
                    loc: *loc,
                    data: structured.clone(),
                });
            hs.platforms.push(i_file);
        }
    };

    // The files' records, in order (as `read_analyses` reads them), each line
    // parsed once: a file's platforms' analyses mostly have the same lines
    // (firefox's: 18% of the lines without locals' edits are distinct).  (The
    // records locals' edits make are used as they come.)
    let mut records_of_lines: HashMap<String, Record> = HashMap::new();
    for (i_file, filename) in filenames.iter().enumerate() {
        let used_filename =
            if filename.ends_with(".gz") && !std::fs::exists(filename).unwrap_or(false) {
                &filename[..filename.len() - 3]
            } else {
                filename.as_str()
            };
        // (`filter_analysis_value` takes the location out, as `read_analyses`
        // does.)
        let mut record_of_value = |data: Value| -> Record {
            let mut out = vec![];
            filter_analysis_value(
                data,
                i_file,
                &mut |obj, loc, _| Some(record_of(obj, loc)),
                &mut out,
            );
            out.pop().map_or(Record::Ignored, |with_loc| with_loc.data)
        };
        for_each_analysis_line(used_filename, &mut |line| match line {
            AnalysisLine::Unedited { lineno, line } => {
                // (A target record is written out the first time: then it's
                // as good as ignored.)
                if let Some(record) = records_of_lines.get_mut(line) {
                    use_record(record, i_file);
                    if let Record::Target(_) = record {
                        *record = Record::Ignored;
                    }
                    return;
                }
                let mut record = parse_analysis_line(used_filename, lineno, line)
                    .map_or(Record::Ignored, &mut record_of_value);
                use_record(&record, i_file);
                if let Record::Target(_) = record {
                    record = Record::Ignored;
                }
                records_of_lines.insert(line.to_string(), record);
            }
            AnalysisLine::Edited(data) => use_record(&record_of_value(data), i_file),
        });
    }
    drop(records_of_lines);
    let src_data = group_by_location(sources);

    // For each bucket of source data at a given location, sort the source data by
    // the `pretty` field. This allows us to walk through the bucket and operate
    // with the assumption that entries with the same (location, pretty) tuple are
    // adjacent. If we do run into such entries we merge them to union the tokens
    // in the `syntax` and `sym` fields.  (All at once, in their order: see
    // `AnalysisSource::merge_all`.)
    //
    // But records of what's in a macro's expansion (`InExpansionAt`, whose
    // location is the macro's use) are merged only with those of the same
    // symbols: a record's symbols are those at each of its offsets in the
    // expansion, so merging records of different symbols gave each offset all
    // of their symbols.  (Ex: js/src/jit/LIR.cpp's `LIR_OPCODE_LIST` use,
    // whose expansion has a `std::is_polymorphic_v<L...>` for each of ~1,000
    // LIR instructions, each a record of its own instantiation's symbol on
    // each platform: merged, every one of them in the expansion had all 2,495
    // symbols, and the page was 1.6 GB.)
    for mut loc_data in src_data {
        loc_data.data.sort_by(|s1, s2| s1.pretty.cmp(&s2.pretty));
        let mut entries = std::mem::take(&mut loc_data.data).into_iter().peekable();
        while let Some(first) = entries.next() {
            let mut same_pretty: Vec<AnalysisSource> = vec![first];
            while let Some(entry) = entries.next_if(|e| e.pretty == same_pretty[0].pretty) {
                same_pretty.push(entry);
            }
            // (Groups to merge, in their first records' order: the records not
            // in an expansion, and those in one by their symbols.)
            let mut groups: Vec<Vec<AnalysisSource>> = vec![];
            let mut group_of_syms: HashMap<Option<Vec<Ustr>>, usize> = HashMap::new();
            for entry in same_pretty {
                let key = matches!(entry.expansion_info, Some(ExpansionInfo::InExpansionAt(_)))
                    .then(|| entry.sym.clone());
                match group_of_syms.get(&key) {
                    Some(&i) => groups[i].push(entry),
                    None => {
                        group_of_syms.insert(key, groups.len());
                        groups.push(vec![entry]);
                    }
                }
            }
            for group in groups {
                let mut group = group.into_iter();
                let mut first = group.next().unwrap();
                let rest: Vec<AnalysisSource> = group.collect();
                if !rest.is_empty() {
                    first.merge_all(rest);
                }
                loc_data.data.push(first);
            }
        }
        // We can't convert WithLocation<Vec<T>> directly to JSON; we need to
        // spread the loc to each individual piece of data.
        let loc = loc_data.loc;
        for datum in loc_data.data {
            writeln!(
                writer,
                "{}",
                to_string(&WithLocation { loc, data: datum }).unwrap()
            )
            .unwrap();
        }
    }

    for (_id, mut hmap) in structured_syms {
        if hmap.len() == 1 {
            // There was only one variant of the structured info, so we can just use it as-is.
            let (_hash, hs) = hmap.pop_first().unwrap();
            writeln!(
                writer,
                "{}",
                to_string(&WithLocation {
                    loc: hs.loc,
                    data: hs.data
                })
                .unwrap()
            )
            .unwrap();
        } else {
            // There are multiple variants, so we want to:
            // 1. Pick one of the variants as the canonical variant.  For now our heuristic is to
            //    pick the highest platform index.  This is because the platform list is currently
            //    accomplished via wildcard that puts "android-armv7" first and that's a 32-bit
            //    platform, and we'd rather our defaults be 64-bit.
            // 2. Using the `extras` field, populate a `platforms` value in
            //    the canonical variant as well a `variants` field.  This should
            //    allow round-tripping while also avoiding us actually doing
            //    anything with this surplus-ish info which we expect to only be
            //    consumed by front-end JS UI at this time for the purposes of
            //    showing differing memory layouts across platforms.
            //
            // Prior to the conversion to serde_json, the `extras` field was a
            // JSON-string `payload` field and we just did a lot of sketchy
            // gluing together of raw JSON string fragments.

            // Do a pass to pick the best hash.
            let mut best_hash = 0;
            let mut best_plat = 0;
            for (hash, hs) in hmap.iter() {
                let local_plat = hs.platforms.iter().max().unwrap();
                if local_plat >= &best_plat {
                    best_plat = *local_plat;
                    best_hash = *hash;
                }
            }

            let mut hs = hmap.remove(&best_hash).unwrap();
            hs.data.extra.insert(
                "platforms".to_string(),
                json!(
                    hs.platforms
                        .iter()
                        .map(|x| platforms[*x].clone())
                        .collect::<Vec<String>>()
                ),
            );
            let mut values: Vec<HashedStructured> = hmap.into_values().collect();
            values.sort_by(|a, b| {
                let pa = a.platforms.iter().map(|x| platforms[*x].clone()).join(",");
                let pb = b.platforms.iter().map(|x| platforms[*x].clone()).join(",");
                pa.cmp(&pb)
            });
            hs.data.extra.insert(
                "variants".to_string(),
                values
                    .into_iter()
                    .map(|mut variant| {
                        variant.data.extra.insert(
                            "platforms".to_string(),
                            json!(
                                variant
                                    .platforms
                                    .iter()
                                    .map(|x| platforms[*x].clone())
                                    .collect::<Vec<String>>()
                            ),
                        );
                        to_value(&variant.data).unwrap()
                    })
                    .collect(),
            );

            writeln!(
                writer,
                "{}",
                to_string(&WithLocation {
                    loc: hs.loc,
                    data: hs.data
                })
                .unwrap()
            )
            .unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The source records of `merge_files`'s output at a location with a
    /// `pretty`, as (symbols, expansion offsets by platform).
    fn sources(out: &str, loc: &str, pretty: &str) -> Vec<(String, Value)> {
        out.lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|v| v["loc"] == loc && v["pretty"] == pretty && v.get("source").is_some())
            .map(|v| {
                let offsets = v["inExpansionAt"]
                    .as_object()
                    .and_then(|keys| keys.values().next().cloned())
                    .unwrap_or(Value::Null);
                (v["sym"].as_str().unwrap().to_string(), offsets)
            })
            .collect()
    }

    #[test]
    fn test_merge_expansion_records_by_symbols() {
        let dir = std::env::temp_dir().join(format!("merger-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // (A macro use whose expansion has `f` twice, a different
        // instantiation each time, on platform "a", and once on "b"; and a
        // variable used outside the expansion on both.)
        let a = dir.join("a");
        std::fs::write(
            &a,
            [
                r#"{"loc":"00010:0-5","source":1,"syntax":"macro,use","pretty":"macro M","sym":"M_1","expandsTo":{"M_1":{"":"f(1); f(2);"}}}"#,
                r#"{"loc":"00010:0-5","source":1,"syntax":"use,function","pretty":"function f","sym":"F_1","inExpansionAt":{"M_1":{"":[0]}}}"#,
                r#"{"loc":"00010:0-5","source":1,"syntax":"use,function","pretty":"function f","sym":"F_2","inExpansionAt":{"M_1":{"":[6]}}}"#,
                r#"{"loc":"00012:0-3","source":1,"syntax":"use,variable","pretty":"variable v","sym":"V_a"}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let b = dir.join("b");
        std::fs::write(
            &b,
            [
                r#"{"loc":"00010:0-5","source":1,"syntax":"macro,use","pretty":"macro M","sym":"M_1","expandsTo":{"M_1":{"":"f(1);"}}}"#,
                r#"{"loc":"00010:0-5","source":1,"syntax":"use,function","pretty":"function f","sym":"F_1","inExpansionAt":{"M_1":{"":[0]}}}"#,
                r#"{"loc":"00012:0-3","source":1,"syntax":"use,variable","pretty":"variable v","sym":"V_b"}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let mut out = vec![];
        merge_files(
            &[
                a.to_str().unwrap().to_string(),
                b.to_str().unwrap().to_string(),
            ],
            &["pa".to_string(), "pb".to_string()],
            &mut out,
        );
        std::fs::remove_dir_all(&dir).unwrap();
        let out = String::from_utf8(out).unwrap();

        // (Each instantiation's record keeps its own offsets, the same
        // symbol's merged across the platforms.)
        let mut fs = sources(&out, "00010:0-5", "function f");
        fs.sort_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(
            fs,
            vec![
                ("F_1".to_string(), json!({"pa": [0], "pb": [0]})),
                ("F_2".to_string(), json!({"pa": [6]})),
            ]
        );
        // (Records outside expansions merge as before.)
        let vs = sources(&out, "00012:0-3", "variable v");
        assert_eq!(vs.len(), 1);
        let mut syms: Vec<&str> = vs[0].0.split(',').collect();
        syms.sort();
        assert_eq!(syms, vec!["V_a", "V_b"]);
    }
}
