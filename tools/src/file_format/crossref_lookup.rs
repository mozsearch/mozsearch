extern crate memmap;

use self::memmap::{Mmap, Protection};
use std::collections::HashMap;
use std::str;

use serde_json::{from_slice, Value};

use crate::{config, abstract_server::Result, abstract_server::{ServerError, ErrorDetails, ErrorLayer}};

#[derive(Debug)]
pub struct CrossrefLookupMap {
    inline_mm: Mmap,
    extra_mm: Mmap,
}

const SPACE: u8 = ' ' as u8;
const NEWLINE: u8 = '\n' as u8;
const ID_START: u8 = '!' as u8;
const INLINE_STORED: u8 = ':' as u8;
const EXTERNALLY_STORED: u8 = '@' as u8;

fn make_crossref_data_error(sym: &str) -> ServerError {
    ServerError::StickyProblem(ErrorDetails {
        layer: ErrorLayer::DataLayer,
        message: format!("bad crossref data for symbol: {}", sym),
    })
}

// This implementation is a port of `crossrefs.py` (which was adapted from
// `identifiers.py`) and informed by `identifiers.rs` (which presumably was
// adapted from `identifiers.py` as well).
impl CrossrefLookupMap {
    pub fn new(inline_path: &str, extra_path: &str) -> Option<CrossrefLookupMap> {
        let inline_mm = match Mmap::open_path(inline_path, Protection::Read) {
            Ok(mmap) => mmap,
            Err(_) => {
                return None
            }
        };
        let extra_mm = match Mmap::open_path(extra_path, Protection::Read) {
          Ok(mmap) => mmap,
          Err(_) => {
              return None
          }
      };
        Some(CrossrefLookupMap { inline_mm, extra_mm })
    }

    pub fn load(config: &config::Config) -> HashMap<String, Option<CrossrefLookupMap>> {
        let mut result = HashMap::new();
        for (tree_name, tree_config) in &config.trees {
            println!("Loading crossref {}", tree_name);
            let inline_path = format!("{}/crossref", tree_config.paths.index_path);
            let extra_path = format!("{}/crossref-extra", tree_config.paths.index_path);
            let map = CrossrefLookupMap::new(&inline_path, &extra_path);
            result.insert(tree_name.clone(), map);
        }
        result
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
        let bytes: &[u8] = unsafe { self.inline_mm.as_slice() };
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
        (&bytes[start+1..end], start, end)
    }

    // Bisect the mmap to look for an exact symbol match `sym`, and returning the
    // payload line which may be either inline JSON or external offsets to be
    // retrieved from another map.
    fn bisect_for_payload(&self, search_sym: &[u8]) -> &[u8] {
        let mut first = 0;
        let mmap_end = self.inline_mm.len();
        let bytes: &[u8] = unsafe { self.inline_mm.as_slice() };
        let mut count = mmap_end;

        while count > 0 {
            let step = count / 2;
            let pos = first + step;

            let (line_sym, line_start, line_end) = self.get_id_line(pos);

            if line_sym == search_sym {
                // Exact Match!  Extract the payload line.
                let payload_start = line_end + 1;
                let mut payload_end = payload_start + 1;
                while payload_end < mmap_end && bytes[payload_end] != NEWLINE {
                    payload_end += 1;
                }
                return &bytes[payload_start..payload_end];
            } else if line_sym < search_sym {
                // ## Bisect latter half
                // We might as well exclude the payload line we're skipping as well.
                // Because payload lines are intentionally limited during the
                // creation of `crossref`, we know this should fault an acceptable
                // number of pages which may have already been pre-fetched.
                while first < mmap_end && bytes[first] != NEWLINE {
                    first += 1;
                }
                // move past the newline
                first += 1;

                // Halve count and also subtract off the parts of the identifier line
                // and payload line we're skipping.
                count -= step + (pos - first)
            } else {
                // ## Bisect first half
                // Halve count and subtract off the part of the identifier line that
                // we can eliminate from consideration.
                count = step - (pos - line_start)
            }
        }

        &[]
    }

    pub fn lookup(
        &self,
        sym: &str,
    ) -> Result<Value> {
        let payload = self.bisect_for_payload(sym.as_bytes());
        let payload_len = payload.len();
        // Finding nothing (a miss!) is not an error and so is an in-band null.
        if payload_len == 0 {
            return Ok(Value::Null);
        }
        // Let's also rule out results that are too short and therefore must be
        // an error.
        if payload_len < 3 {
            return Err(make_crossref_data_error(sym));
        }

        let marker_char = payload[0];

        if marker_char == INLINE_STORED {
            return from_slice(&payload[1..]).or(Ok(Value::Null));
        } else if marker_char != EXTERNALLY_STORED {
            // Fail if we're seeing something other than an external ref.
            return Err(make_crossref_data_error(sym));
        }

        let mut space_pos = 2;
        while space_pos < payload_len && payload[space_pos] != SPACE {
            space_pos += 1;
        }

        let brace_offset = unsafe { usize::from_str_radix(str::from_utf8_unchecked(&payload[1..space_pos]), 16).map_err(|_| make_crossref_data_error(sym))? };
        let length_with_newline = unsafe { usize::from_str_radix(str::from_utf8_unchecked(&payload[space_pos+1..]), 16).map_err(|_| make_crossref_data_error(sym))? };

        let extra_bytes: &[u8] = unsafe { self.extra_mm.as_slice() };
        return Ok(from_slice(&extra_bytes[brace_offset..brace_offset + length_with_newline - 1])?);
    }
}
