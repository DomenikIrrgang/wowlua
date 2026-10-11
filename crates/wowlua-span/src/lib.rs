//! Source files, file ids and byte spans.
//!
//! Every diagnostic and every syntax node points into a [`SourceDb`] through a
//! [`Span`]: a file id plus a half-open byte range.

use std::ops::Range;
use std::path::{Path, PathBuf};

/// Identifies a file inside a [`SourceDb`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(u32);

impl FileId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A half-open byte range `start..end` inside one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, range: Range<usize>) -> Self {
        Span {
            file,
            start: to_u32(range.start),
            end: to_u32(range.end),
        }
    }

    /// An empty span at the start of a file, for diagnostics about a whole file.
    pub fn file_start(file: FileId) -> Self {
        Span {
            file,
            start: 0,
            end: 0,
        }
    }

    pub fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

/// One-based line and column of a byte offset. Columns count bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub column: u32,
}

/// A source file: its display path, its text and a line index.
#[derive(Debug)]
pub struct SourceFile {
    path: PathBuf,
    text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    fn new(path: PathBuf, text: String) -> Self {
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        SourceFile {
            path,
            text,
            line_starts,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Byte offsets at which each line starts.
    pub fn line_starts(&self) -> &[usize] {
        &self.line_starts
    }

    /// Zero-based index of the line containing `offset`.
    pub fn line_index(&self, offset: usize) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next - 1,
        }
    }

    pub fn line_col(&self, offset: usize) -> LineCol {
        let line = self.line_index(offset);
        LineCol {
            line: to_u32(line + 1),
            column: to_u32(offset - self.line_starts[line] + 1),
        }
    }

    /// Byte offset of a one-based line and column, clamped to the file.
    pub fn offset(&self, line: usize, column: usize) -> usize {
        let line_start = self
            .line_starts
            .get(line.saturating_sub(1))
            .copied()
            .unwrap_or(self.text.len());
        (line_start + column.saturating_sub(1)).min(self.text.len())
    }

    /// Byte range of a one-based line, without its line terminator.
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let start = self.offset(line, 1);
        let end = self.text[start..]
            .find('\n')
            .map_or(self.text.len(), |i| start + i);
        let end = if self.text[start..end].ends_with('\r') {
            end - 1
        } else {
            end
        };
        start..end
    }
}

/// All source files known to one compiler run.
#[derive(Debug, Default)]
pub struct SourceDb {
    files: Vec<SourceFile>,
}

impl SourceDb {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, path: impl Into<PathBuf>, text: impl Into<String>) -> FileId {
        let id = FileId(to_u32(self.files.len()));
        self.files.push(SourceFile::new(path.into(), text.into()));
        id
    }

    pub fn get(&self, id: FileId) -> &SourceFile {
        &self.files[id.index()]
    }

    pub fn files(&self) -> impl Iterator<Item = (FileId, &SourceFile)> {
        self.files
            .iter()
            .enumerate()
            .map(|(i, f)| (FileId(to_u32(i)), f))
    }

    /// A span covering one whole line (one-based) of a file.
    pub fn line_span(&self, file: FileId, line: usize) -> Span {
        Span::new(file, self.get(file).line_range(line))
    }
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("source files larger than 4 GiB are not supported")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_and_offsets_round_trip() {
        let mut db = SourceDb::new();
        let id = db.add("a.lua", "local a = 1\r\nprint(a)\n");
        let file = db.get(id);
        assert_eq!(file.line_col(0), LineCol { line: 1, column: 1 });
        assert_eq!(file.line_col(13), LineCol { line: 2, column: 1 });
        assert_eq!(file.offset(2, 3), 15);
        assert_eq!(&file.text()[file.line_range(1)], "local a = 1");
        assert_eq!(&file.text()[file.line_range(2)], "print(a)");
    }

    #[test]
    fn offsets_past_the_end_are_clamped() {
        let mut db = SourceDb::new();
        let id = db.add("a.lua", "x");
        assert_eq!(db.get(id).offset(10, 10), 1);
    }
}
