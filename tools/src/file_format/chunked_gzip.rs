//! Rendered source files (`file/PATH.gz`) as gzips whose lines can be read
//! without decompressing the whole file, so that `/query/`'s results (see
//! `cmd_augment_results`) only read and decompress the parts of the files
//! with their lines (bug 1794177).
//!
//! They're ordinary gzips (nginx serves them as is, and anything can
//! decompress them), but the deflate data has a full flush, which resets the
//! compressor's history, before the row (see `ROW_START`) of every
//! `LINES_PER_CHUNK`th line, so decompression can start there.  The offsets
//! of those points are in an "SF" subfield of the header's extra field (RFC
//! 1952's FEXTRA), which decompressors skip:
//! - the version (a byte, 1) and the lines per chunk (u16, little-endian);
//! - the offset (u32) in the deflate data of each chunk: chunk 0 is everything
//!   before the row of line `LINES + 1` (including the page's header), chunk K
//!   is the rows of lines `K * LINES + 1` to `(K + 1) * LINES`, and the last
//!   chunk also has the page's footer.
//!
//! The full flushes cost ~18% of the files' size (for firefox's; less for big
//! files), since each chunk starts without the history of the rows before it,
//! for a 15-60x speedup of big queries (ex: 16s to 0.3s for "nsIPrincipal" on
//! firefox, whose results are in 1477 files with 7.8 GB of HTML).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::{Compress, Compression, Crc, Decompress, FlushCompress, FlushDecompress, Status};

/// How many lines each chunk has, unless a file has so many lines that their
/// offsets wouldn't fit in the extra field, in which case it's doubled until
/// they do.
pub const LINES_PER_CHUNK: u32 = 32;

/// The start of the row of a line of a rendered file (before the line number
/// and a quote), and its end (see `format::format_file_data`).
pub const ROW_START: &str = "<div role=\"row\" id=\"line-";
pub const ROW_END: &str = "</code>\n</div>\n";

const SUBFIELD_ID: &[u8; 2] = b"SF";
const VERSION: u8 = 1;
/// The most offsets that fit in an extra field (of at most 65535 bytes, with
/// the subfield's ID, length, version, and lines per chunk).
const MAX_CHUNKS: usize = (65535 - 4 - 3) / 4;

/// The rows of the lines `lines` (1-based) of `html`, by line, found by
/// searching for each from the end of the one before.
pub fn extract_rows(html: &str, lines: &BTreeSet<u32>) -> HashMap<u32, String> {
    let mut rows = HashMap::new();
    let mut pos = 0;
    for &line in lines {
        let start_needle = format!("{}{}\"", ROW_START, line);
        let Some(start) = html[pos..].find(&start_needle).map(|i| pos + i) else {
            continue;
        };
        let Some(end) = html[start..]
            .find(ROW_END)
            .map(|i| start + i + ROW_END.len())
        else {
            continue;
        };
        rows.insert(line, html[start..end].to_string());
        pos = end;
    }
    rows
}

/// The offsets of the rows of `html` with their line numbers, in order.
fn row_offsets(html: &str) -> Vec<(usize, u32)> {
    html.match_indices(ROW_START)
        .filter_map(|(offset, _)| {
            let digits = &html[offset + ROW_START.len()..];
            let end = digits.find(|c: char| !c.is_ascii_digit())?;
            Some((offset, digits[..end].parse().ok()?))
        })
        .collect()
}

/// Write `contents` (a rendered file, or anything else) as a gzip with
/// chunks of lines; see the module docs.
pub fn write_line_chunked_gzip<W: Write>(contents: &[u8], out: &mut W) -> io::Result<()> {
    let rows = std::str::from_utf8(contents).map_or_else(|_| vec![], row_offsets);
    // The offsets of the rows starting chunks after the first, with as many
    // lines per chunk as it takes to fit their offsets in the extra field.
    let mut lines_per_chunk = LINES_PER_CHUNK;
    let boundaries = loop {
        let mut boundaries = vec![];
        let mut next = lines_per_chunk + 1;
        for &(offset, line) in &rows {
            if line >= next {
                boundaries.push(offset);
                next = line - (line - 1) % lines_per_chunk + lines_per_chunk;
            }
        }
        if boundaries.len() < MAX_CHUNKS {
            break boundaries;
        }
        lines_per_chunk *= 2;
    };

    let mut compress = Compress::new(Compression::default(), false);
    let mut deflated = Vec::with_capacity(contents.len() / 8 + 1024);
    let mut offsets = vec![0u32];
    let mut start = 0;
    for (k, end) in boundaries
        .iter()
        .copied()
        .chain(std::iter::once(contents.len()))
        .enumerate()
    {
        let last = k == boundaries.len();
        let flush = if last {
            FlushCompress::Finish
        } else {
            FlushCompress::Full
        };
        let mut input = &contents[start..end];
        loop {
            if deflated.capacity() - deflated.len() < 64 * 1024 {
                deflated.reserve(256 * 1024);
            }
            let before = compress.total_in();
            let status = compress
                .compress_vec(input, &mut deflated, flush)
                .map_err(io::Error::other)?;
            input = &input[(compress.total_in() - before) as usize..];
            // A flush is done once it's left room in the output.
            let done = if last {
                status == Status::StreamEnd
            } else {
                input.is_empty() && deflated.len() < deflated.capacity()
            };
            if done {
                break;
            }
        }
        if !last {
            offsets.push(u32::try_from(deflated.len()).map_err(io::Error::other)?);
        }
        start = end;
    }

    let mut subfield = vec![VERSION];
    subfield.extend_from_slice(&(lines_per_chunk as u16).to_le_bytes());
    for offset in &offsets {
        subfield.extend_from_slice(&offset.to_le_bytes());
    }
    let mut extra = SUBFIELD_ID.to_vec();
    extra.extend_from_slice(&(subfield.len() as u16).to_le_bytes());
    extra.extend_from_slice(&subfield);
    // ID1, ID2, CM (deflate), FLG (FEXTRA), MTIME, XFL, OS (unknown).
    out.write_all(&[0x1f, 0x8b, 8, 4, 0, 0, 0, 0, 0, 255])?;
    out.write_all(&(extra.len() as u16).to_le_bytes())?;
    out.write_all(&extra)?;
    out.write_all(&deflated)?;
    let mut crc = Crc::new();
    crc.update(contents);
    out.write_all(&crc.sum().to_le_bytes())?;
    out.write_all(&(contents.len() as u32).to_le_bytes())
}

/// A writer of a gzip with chunks of lines (see the module docs), which it
/// writes once it's finished (or dropped), since it needs all of the
/// contents.
pub struct LineChunkedGzipWriter<W: Write> {
    out: Option<W>,
    contents: Vec<u8>,
}

impl<W: Write> LineChunkedGzipWriter<W> {
    pub fn new(out: W) -> Self {
        LineChunkedGzipWriter {
            out: Some(out),
            contents: vec![],
        }
    }

    pub fn finish(mut self) -> io::Result<W> {
        let mut out = self.out.take().unwrap();
        write_line_chunked_gzip(&self.contents, &mut out)?;
        out.flush()?;
        Ok(out)
    }
}

impl<W: Write> Write for LineChunkedGzipWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.contents.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<W: Write> Drop for LineChunkedGzipWriter<W> {
    fn drop(&mut self) {
        if let Some(mut out) = self.out.take() {
            let _ = write_line_chunked_gzip(&self.contents, &mut out).and_then(|_| out.flush());
        }
    }
}

/// The chunks of a gzip with chunks of lines: the lines per chunk, and each
/// chunk's byte range in the file.
type Chunks = (u32, Vec<(u64, u64)>);

fn read_chunks(file: &mut File) -> io::Result<Option<Chunks>> {
    let mut header = [0u8; 12];
    if file.read_exact(&mut header).is_err() {
        return Ok(None);
    }
    // Only our headers: FEXTRA and no other optional fields.
    if header[..4] != [0x1f, 0x8b, 8, 4] {
        return Ok(None);
    }
    let xlen = u16::from_le_bytes([header[10], header[11]]) as usize;
    let mut extra = vec![0u8; xlen];
    file.read_exact(&mut extra)?;
    let data_start = 12 + xlen as u64;
    let data_end = file.metadata()?.len().saturating_sub(8);
    let mut rest = &extra[..];
    while rest.len() >= 4 {
        let len = u16::from_le_bytes([rest[2], rest[3]]) as usize;
        let subfield = rest.get(4..4 + len).unwrap_or_default();
        if &rest[..2] == SUBFIELD_ID && subfield.len() >= 3 && subfield[0] == VERSION {
            let lines_per_chunk = u16::from_le_bytes([subfield[1], subfield[2]]) as u32;
            let offsets: Vec<u64> = subfield[3..]
                .chunks_exact(4)
                .map(|o| data_start + u32::from_le_bytes([o[0], o[1], o[2], o[3]]) as u64)
                .collect();
            if lines_per_chunk == 0 || offsets.is_empty() {
                return Ok(None);
            }
            let ends = offsets
                .iter()
                .skip(1)
                .copied()
                .chain(std::iter::once(data_end));
            return Ok(Some((
                lines_per_chunk,
                offsets.iter().copied().zip(ends).collect(),
            )));
        }
        rest = &rest[(4 + len).min(rest.len())..];
    }
    Ok(None)
}

/// The rows (see `ROW_START`) of the lines `lines` (1-based) of the rendered
/// file whose gzip is at `path`, by line, from just the chunks with them if
/// it has chunks of lines (see the module docs), and otherwise (ex: files of
/// indexes from before them) from all of it.
pub fn read_rows(path: &Path, lines: &BTreeSet<u32>) -> io::Result<HashMap<u32, String>> {
    let mut file = File::open(path)?;
    let Some((lines_per_chunk, chunks)) = read_chunks(&mut file)? else {
        file.seek(SeekFrom::Start(0))?;
        let mut contents = vec![];
        GzDecoder::new(io::BufReader::new(file)).read_to_end(&mut contents)?;
        return Ok(extract_rows(&String::from_utf8_lossy(&contents), lines));
    };

    let chunk_of =
        |line: u32| (((line.max(1) - 1) / lines_per_chunk) as usize).min(chunks.len() - 1);
    let mut chunk_lines: BTreeMap<usize, BTreeSet<u32>> = BTreeMap::new();
    for &line in lines {
        chunk_lines.entry(chunk_of(line)).or_default().insert(line);
    }
    let mut rows = HashMap::new();
    let mut deflated = vec![];
    for (k, lines) in chunk_lines {
        let (start, end) = chunks[k];
        deflated.resize(end.saturating_sub(start) as usize, 0);
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut deflated)?;
        let html = inflate_raw(&deflated)?;
        rows.extend(extract_rows(&String::from_utf8_lossy(&html), &lines));
    }
    Ok(rows)
}

/// Decompress raw deflate data which starts after a full flush.
fn inflate_raw(mut deflated: &[u8]) -> io::Result<Vec<u8>> {
    let mut decompress = Decompress::new(false);
    let mut out = Vec::with_capacity(deflated.len() * 8);
    loop {
        if out.capacity() - out.len() < 64 * 1024 {
            out.reserve(256 * 1024);
        }
        let before = decompress.total_in();
        let status = decompress
            .decompress_vec(deflated, &mut out, FlushDecompress::Sync)
            .map_err(io::Error::other)?;
        deflated = &deflated[(decompress.total_in() - before) as usize..];
        if status == Status::StreamEnd || (deflated.is_empty() && out.len() < out.capacity()) {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(line: u32, text: &str) -> String {
        format!(
            "<div role=\"row\" id=\"line-{}\" class=\"source-line-with-number\">\n  <div role=\"cell\" class=\"line-number\" data-line-number=\"{}\"></div>\n  <code role=\"cell\" class=\"source-line\">{}\n</code>\n</div>\n",
            line, line, text
        )
    }

    fn page(lines: u32) -> String {
        let mut html = String::from("<html><body><div id=\"file\">\n");
        for line in 1..=lines {
            if line % 10 == 0 {
                // (A nesting container, whose rows are rows too.)
                html.push_str("<div class=\"nesting-container\">");
                html.push_str(&row(line, &format!("fn f{}() {{", line)));
                html.push_str("</div>\n");
            } else {
                html.push_str(&row(line, &format!("  let x{} = &lt;{}&gt;;", line, line)));
            }
        }
        html.push_str("</div></body></html>\n");
        html
    }

    fn write_and_read(contents: &[u8], lines: &BTreeSet<u32>) -> HashMap<u32, String> {
        let dir = std::env::temp_dir().join(format!("chunked-gzip-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.gz", lines.len()));
        let mut file = File::create(&path).unwrap();
        write_line_chunked_gzip(contents, &mut file).unwrap();
        drop(file);
        // It's an ordinary gzip.
        let mut decompressed = vec![];
        GzDecoder::new(File::open(&path).unwrap())
            .read_to_end(&mut decompressed)
            .unwrap();
        assert_eq!(decompressed, contents);
        let rows = read_rows(&path, lines).unwrap();
        std::fs::remove_file(&path).unwrap();
        rows
    }

    #[test]
    fn test_chunked_rows() {
        let html = page(200);
        let lines: BTreeSet<u32> = [1, 2, 10, 32, 33, 64, 65, 100, 199, 200, 201]
            .into_iter()
            .collect();
        let rows = write_and_read(html.as_bytes(), &lines);
        // Every line in the page, from whichever chunk, with its whole row.
        for &line in &lines {
            match line {
                201 => assert!(!rows.contains_key(&line)),
                _ => assert_eq!(
                    rows[&line],
                    extract_rows(&html, &[line].into_iter().collect())[&line]
                ),
            }
        }
        assert!(rows[&10].starts_with("<div role=\"row\" id=\"line-10\""));
        assert!(rows[&10].ends_with(ROW_END));
        assert!(rows[&33].contains("x33 = &lt;33&gt;;"));
        assert!(!rows[&1].contains("line-2\""));
    }

    #[test]
    fn test_chunked_others() {
        // Contents without rows, or which aren't text, are one chunk.
        let lines: BTreeSet<u32> = [1].into_iter().collect();
        assert!(write_and_read(b"Symlink to 'elsewhere'", &lines).is_empty());
        assert!(write_and_read(&[0xff, 0xfe, 0, 1, 2, 3], &lines).is_empty());
        assert!(write_and_read(b"", &lines).is_empty());
    }

    #[test]
    fn test_ordinary_gzip() {
        // Gzips from before chunks of lines are read whole.
        let html = page(50);
        let dir = std::env::temp_dir().join(format!("chunked-gzip-plain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plain.gz");
        let mut encoder =
            flate2::write::GzEncoder::new(File::create(&path).unwrap(), Compression::default());
        encoder.write_all(html.as_bytes()).unwrap();
        encoder.finish().unwrap();
        let rows = read_rows(&path, &[7, 40].into_iter().collect()).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[&40].contains("fn f40()"));
    }
}
