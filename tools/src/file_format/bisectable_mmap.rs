extern crate memmap;

use self::memmap::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::str;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::{cmp::Ordering, marker::PhantomData};

use serde::Deserialize;
use serde_json::from_slice;

use crate::abstract_server::{ErrorDetails, ErrorLayer, Result, ServerError};

#[derive(Clone, Debug)]
pub struct BisectableMmap<T> {
    inline_mm: Arc<Mmap>,
    extra_mm: Arc<Mmap>,
    _data_type: PhantomData<T>,
}

const SPACE: u8 = b' ';
const NEWLINE: u8 = b'\n';
const ID_START: u8 = b'!';
const INLINE_STORED: u8 = b':';
const EXTERNALLY_STORED: u8 = b'@';

/// The files mapped in the process, by device, inode and length, so that a
/// file several trees look up is mapped once: a linked tree's crossref and
/// jumpref, which each tree linking to it maps too (see
/// `TreeConfigPaths::linked_trees`; ex: firefox-disco's, ~37 GB, for
/// wubspecs).  Each mapping takes its file's size of the process's address
/// space, which the pipeline-server's is limited to by its files' sizes, once
/// each (see `infrastructure/web-server-run.sh`): mapping firefox-disco's
/// twice left it ~0.5 GB of room on config8, and queries failed allocating.
/// (A replaced or grown file is mapped again.)
static MAPPED: LazyLock<Mutex<HashMap<(u64, u64, u64), Weak<Mmap>>>> =
    LazyLock::new(Default::default);

/// A file's mapping: the process's, if it's mapped already (see `MAPPED`).
fn map_shared(file: &File) -> std::io::Result<Arc<Mmap>> {
    let key = file.metadata().map(|m| (m.dev(), m.ino(), m.len()))?;
    let mut mapped = MAPPED.lock().unwrap();
    if let Some(mmap) = mapped.get(&key).and_then(Weak::upgrade) {
        return Ok(mmap);
    }
    let mmap = Arc::new(unsafe { Mmap::map(file) }?);
    mapped.retain(|_, weak| weak.strong_count() > 0);
    mapped.insert(key, Arc::downgrade(&mmap));
    Ok(mmap)
}

fn make_data_error(sym: &str) -> ServerError {
    ServerError::StickyProblem(ErrorDetails {
        layer: ErrorLayer::DataLayer,
        message: format!("bad bisectable mmap data for symbol: {}", sym),
    })
}

// This implementation is a port of `crossrefs.py` (which was adapted from
// `identifiers.py`) and informed by `identifiers.rs` (which presumably was
// adapted from `identifiers.py` as well).
impl<'de, T: Deserialize<'de>> BisectableMmap<T> {
    pub fn new(inline_path: &str, extra_path: &str) -> Option<Self> {
        // (An empty file can't be mapped, which callers take as no data, but
        // other failures are problems, ex: a process's address space limit,
        // which would otherwise make it serve without the data.)
        let map = |path: &str| {
            let file = File::open(path).unwrap();
            match map_shared(&file) {
                Ok(mmap) => Some(mmap),
                Err(err) => {
                    if file.metadata().is_ok_and(|m| m.len() > 0) {
                        eprintln!("Couldn't map {}: {}", path, err);
                    }
                    None
                }
            }
        };
        let inline_mm = map(inline_path)?;
        let extra_mm = map(extra_path)?;
        Some(Self {
            inline_mm,
            extra_mm,
            _data_type: PhantomData,
        })
    }

    // Given a memory map and a position, expand from `pos` to find the identifier
    // line (`!` prefixed) that covers the position.  Returns (the identifier,
    // the offset of the `!` from the start of the identifier line, the offset of
    // the newline ending the identifier line).
    //
    // `pos` is either inside an identifier line or a payload line that follows an
    // identifier line, so we always walk backwards until we find an identifier.
    // We should never need to walk forward (to find the start of the identifier
    // line) because the result of any comparison should always tell the bisection
    // to bisect in the positive direction (because the file is sorted), which
    // should then find the subsequent record (if that's the one we're looking
    // for, etc.).
    fn get_id_line(&self, pos: usize) -> (&[u8], usize, usize) {
        let mut pos = pos;
        let bytes: &[u8] = self.inline_mm.as_ref();
        if bytes[pos] == NEWLINE {
            pos -= 1;
        }

        let mut start = pos;
        let mut end = pos;

        while start > 0 {
            if bytes[start - 1] == NEWLINE {
                if bytes[start] == ID_START {
                    break;
                } else {
                    // We're hitting a ":" and we need to reset end to this newlin
                    end = start - 1
                    // and we want to keep going...
                }
            }
            start -= 1;
        }

        // Start should now be pointing at the `!` of the identifier line.

        let size = self.inline_mm.len();
        while end < size && bytes[end] != NEWLINE {
            end += 1;
        }

        // end should now be pointing at the trailing newline.

        // Skip the leading `!`
        (&bytes[start + 1..end], start, end)
    }

    // Bisect the mmap to look for an exact symbol match `sym`, and returning the
    // payload line which may be either inline JSON or external offsets to be
    // retrieved from another map.
    fn bisect_for_payload(&self, search_sym: &[u8]) -> &[u8] {
        // We are always looking at a byte-range window within the mmap that is
        // a slice with bounds [0, mmap_end).
        let mut first = 0;
        let mmap_end = self.inline_mm.len();
        let bytes: &[u8] = self.inline_mm.as_ref();
        let mut count = mmap_end;

        while count > 0 {
            let step = count / 2;
            let mut pos = first + step;
            // Our range is currently [first, first + count), and we have chosen
            // a sample point `pos` that's the midpoint of the range.
            //
            // Because we take care to always adjust the byte-range window we
            // are looking at to eliminate both the key and value lines, we can
            // be certain that `pos` will not somehow fall within a line of text
            // we have already looked at.

            let (line_sym, line_start, line_end) = self.get_id_line(pos);

            match line_sym.cmp(search_sym) {
                Ordering::Equal => {
                    // Exact Match!  Extract the payload line.
                    let payload_start = line_end + 1;
                    let mut payload_end = payload_start + 1;
                    while payload_end < mmap_end && bytes[payload_end] != NEWLINE {
                        payload_end += 1;
                    }
                    return &bytes[payload_start..payload_end];
                }
                Ordering::Less => {
                    // ## Bisect latter half
                    // The line we found was less than our needle, so the answer is
                    // in the second half of our current range of [first, first + count).
                    // The second half of the range is [pos, first + count), but
                    // this also includes at least some of our payload line, and we
                    // want to skip that.  (Actually need, by our rules.)
                    //
                    // If `pos` was in an id line, then `line_end` will be greater
                    // than `pos` and we should use `line_end + 1` because it's
                    // already pointing at a newline.  If `pos` is greater, then it
                    // must be in the value line.
                    if pos <= line_end {
                        pos = line_end + 1
                    }

                    // Now scan forward until we find the newline ending the value.
                    while pos < mmap_end && bytes[pos] != NEWLINE {
                        pos += 1;
                    }
                    // move past the newline
                    pos += 1;

                    // Our new range now wants to be [pos, first + count).
                    // We want to halve the count, but we also want to compensate
                    // for the extra data to skip.  `pos` is effectively
                    // `first + step + value_length` so subtracting off `first` from
                    // `pos` gets us the step plus the extra length.
                    count -= pos - first;
                    // And now we want to be starting from the `pos`.
                    first = pos;
                }
                Ordering::Greater => {
                    // ## Bisect first half
                    // Halve count and subtract off the part of the identifier line that
                    // we can eliminate from consideration.
                    count = step - (pos - line_start)
                }
            }
        }

        &[]
    }

    pub fn lookup(&'de self, sym: &str) -> Result<Option<T>> {
        let payload = self.bisect_for_payload(sym.as_bytes());
        let payload_len = payload.len();
        // Finding nothing (a miss!) is not an error and so is an in-band null.
        if payload_len == 0 {
            return Ok(None);
        }
        // Let's also rule out results that are too short and therefore must be
        // an error.
        if payload_len < 3 {
            return Err(make_data_error(sym));
        }

        let marker_char = payload[0];

        if marker_char == INLINE_STORED {
            return from_slice(&payload[1..]).or(Ok(None));
        } else if marker_char != EXTERNALLY_STORED {
            // Fail if we're seeing something other than an external ref.
            return Err(make_data_error(sym));
        }

        let mut space_pos = 2;
        while space_pos < payload_len && payload[space_pos] != SPACE {
            space_pos += 1;
        }

        let brace_offset = unsafe {
            usize::from_str_radix(str::from_utf8_unchecked(&payload[1..space_pos]), 16)
                .map_err(|_| make_data_error(sym))?
        };
        let length_with_newline = unsafe {
            usize::from_str_radix(str::from_utf8_unchecked(&payload[space_pos + 1..]), 16)
                .map_err(|_| make_data_error(sym))?
        };

        let extra_bytes: &[u8] = self.extra_mm.as_ref();
        Ok(from_slice(
            &extra_bytes[brace_offset..brace_offset + length_with_newline - 1],
        )?)
    }
}

/// The size of the inline line (`:` + JSON + newline) from which crossref
/// stores values in the extra file instead.
const EXTERNAL_STORAGE_THRESHOLD: usize = 1024 * 3;

/// Write `entries`, sorted by id, as the inline and extra files of a
/// bisectable map (see `BisectableMmap::new`), as crossref writes its: an
/// `!ID` line, and then the value as a `:JSON` line, or, for big values, an
/// `@OFFSET LENGTH` line (in hex) pointing at the `:JSON` line in the extra
/// file (after its own `!ID` line).
pub fn write_bisectable<T: serde::Serialize>(
    inline_path: &str,
    extra_path: &str,
    entries: impl IntoIterator<Item = (String, T)>,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut entries: Vec<(String, T)> = entries.into_iter().collect();
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut inline = std::io::BufWriter::new(File::create(inline_path)?);
    let mut extra = std::io::BufWriter::new(File::create(extra_path)?);
    let mut extra_offset = 0;
    for (id, value) in entries {
        let id_line = format!("!{}\n", id);
        let value_line = format!(":{}\n", serde_json::to_string(&value)?);
        inline.write_all(id_line.as_bytes())?;
        if value_line.len() >= EXTERNAL_STORAGE_THRESHOLD {
            extra.write_all(id_line.as_bytes())?;
            extra_offset += id_line.len();
            // (After the ":", with the newline.)
            write!(
                inline,
                "@{:x} {:x}\n",
                extra_offset + 1,
                value_line.len() - 1
            )?;
            extra.write_all(value_line.as_bytes())?;
            extra_offset += value_line.len();
        } else {
            inline.write_all(value_line.as_bytes())?;
        }
    }
    inline.flush()?;
    extra.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_mappings() {
        let dir = std::env::temp_dir().join(format!("bisectable-mmap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (inline, extra) = (dir.join("crossref"), dir.join("crossref-extra"));
        std::fs::write(&inline, "!a\n:{}\n").unwrap();
        std::fs::write(&extra, "{}\n").unwrap();
        let (inline, extra) = (inline.to_str().unwrap(), extra.to_str().unwrap());
        // (A tree's, and another tree's as its linked tree's: one mapping.)
        let own = BisectableMmap::<serde_json::Value>::new(inline, extra).unwrap();
        let linked = BisectableMmap::<serde_json::Value>::new(inline, extra).unwrap();
        assert!(Arc::ptr_eq(&own.inline_mm, &linked.inline_mm));
        assert!(Arc::ptr_eq(&own.extra_mm, &linked.extra_mm));
        // (A file rewritten in place, grown, is mapped again.)
        std::fs::write(dir.join("crossref"), "!a\n:{}\n!b\n:{}\n").unwrap();
        let rewritten = BisectableMmap::<serde_json::Value>::new(inline, extra).unwrap();
        assert!(!Arc::ptr_eq(&own.inline_mm, &rewritten.inline_mm));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
