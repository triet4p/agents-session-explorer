use ase_core::{err, Encoding, FileId, RawStore, Result, SpanRef};
use ase_ingest::read_source_bytes;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use crate::reader::Index;

/// Reads raw bytes in source coordinates. zstd files are decompressed once
/// and cached by file id; offsets always address the decompressed stream.
pub struct FsRawStore {
    index: Arc<Index>,
    cache: Mutex<Vec<(FileId, Arc<[u8]>)>>,
    capacity: usize,
}

impl FsRawStore {
    pub fn new(index: Arc<Index>) -> Self {
        Self { index, cache: Mutex::new(Vec::new()), capacity: 4 }
    }

    /// Read a span from per-file decoded bytes. Bulk verification uses this
    /// path so one file read serves all of its spans.
    pub fn span_cached(&self, span: SpanRef) -> Result<Vec<u8>> {
        let bytes = self.bytes(span.file)?;
        let start = span.off as usize;
        let end = start
            .checked_add(span.len as usize)
            .ok_or_else(|| err("span length overflow"))?;
        if end > bytes.len() {
            return Err(err(format!(
                "span out of range: off={} len={} stream_len={}",
                span.off, span.len, bytes.len()
            )));
        }
        Ok(bytes[start..end].to_vec())
    }
}

impl RawStore for FsRawStore {
    fn bytes(&self, file: FileId) -> Result<Arc<[u8]>> {
        {
            let mut cache = self.cache.lock().map_err(|_| err("raw cache poisoned"))?;
            if let Some(pos) = cache.iter().position(|(id, _)| *id == file) {
                let hit = cache[pos].1.clone();
                if pos > 0 {
                    let entry = cache.remove(pos);
                    cache.insert(0, entry);
                }
                return Ok(hit);
            }
        }
        let view = self.index.file(file)?;
        let bytes = read_source_bytes(view.path(), view.encoding())?;
        let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        let mut cache = self.cache.lock().map_err(|_| err("raw cache poisoned"))?;
        cache.insert(0, (file, arc.clone()));
        while cache.len() > self.capacity {
            cache.pop();
        }
        Ok(arc)
    }

    /// Single-span JSONL reads seek directly to the source offset instead of
    /// loading the whole file. zstd spans address the decompressed stream and
    /// therefore must use the cached decoded bytes.
    fn span(&self, span: SpanRef) -> Result<Vec<u8>> {
        let view = self.index.file(span.file)?;
        if view.encoding() != Encoding::Jsonl {
            return self.span_cached(span);
        }
        let mut file =
            File::open(view.path()).map_err(|e| err(format!("open {}: {e}", view.path())))?;
        file.seek(SeekFrom::Start(span.off))
            .map_err(|e| err(format!("seek {}: {e}", view.path())))?;
        let mut buf = vec![0u8; span.len as usize];
        file.read_exact(&mut buf)
            .map_err(|e| err(format!("read span {}:{}+{}: {e}", span.file, span.off, span.len)))?;
        Ok(buf)
    }
}
