//! Rebuildable sidecar wire format: magic/version, XXH3 checksum, little-endian payload.
//! Lengths are bounded by remaining input before allocating; trailing bytes are rejected.
use super::{EntryPos, FileStamp, OffsetIndex};
use std::sync::OnceLock;

pub(super) const MAGIC: &[u8; 8] = b"OINOIDX\x01";

pub(super) fn encode(index: &OffsetIndex) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&[0; 8]);
    macro_rules! number {
        ($value:expr) => {
            out.extend_from_slice(&($value as u64).to_le_bytes())
        };
    }
    number!(u64::from(index.version));
    for value in [
        index.size,
        index.mtime_ns,
        index.identity.len,
        index.identity.mtime,
        index.identity.inode,
        index.identity.device,
        index.identity.changed.0.cast_unsigned(),
        index.identity.changed.1.cast_unsigned(),
        index.identity.tail_digest,
        index.generation,
        index.end,
    ] {
        number!(value);
    }
    number!(index.fingerprint.len());
    for value in &index.fingerprint {
        number!(*value);
    }
    number!(index.entries.len());
    for entry in &index.entries {
        number!(entry.offset);
        number!(u64::from(entry.len));
        string(&mut out, &entry.id);
        optional_number(&mut out, entry.parent);
        out.push(entry.flags);
        number!(entry.depth);
        number!(entry.pending.len());
        for id in &entry.pending {
            string(&mut out, id);
        }
        optional_number(&mut out, entry.checkpoint);
        optional_string(&mut out, entry.custom_type.as_deref());
        optional_string(
            &mut out,
            entry
                .compaction_summary_id
                .as_ref()
                .map(crate::entry::EntryId::as_str),
        );
        optional_number(&mut out, entry.branch_compaction);
    }
    optional_number(&mut out, index.last_compaction);
    optional_string(&mut out, index.title.as_deref());
    optional_string(&mut out, index.moved_cwd.as_deref());
    let checksum = xxhash_rust::xxh3::xxh3_64(&out[16..]);
    out[8..16].copy_from_slice(&checksum.to_le_bytes());
    out
}

fn string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u64).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn optional_number(out: &mut Vec<u8>, value: Option<u32>) {
    out.extend_from_slice(&value.map_or(u64::MAX, u64::from).to_le_bytes());
}

fn optional_string(out: &mut Vec<u8>, value: Option<&str>) {
    if let Some(value) = value {
        string(out, value);
    } else {
        out.extend_from_slice(&u64::MAX.to_le_bytes());
    }
}

struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take(&mut self, len: usize) -> Option<&[u8]> {
        let (head, tail) = self.0.split_at_checked(len)?;
        self.0 = tail;
        Some(head)
    }
    fn number(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn count(&mut self) -> Option<usize> {
        let count = usize::try_from(self.number()?).ok()?;
        (count <= self.0.len()).then_some(count)
    }
    fn text(&mut self, len: usize) -> Option<String> {
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
    fn string(&mut self) -> Option<String> {
        let len = self.count()?;
        self.text(len)
    }
    #[allow(
        clippy::option_option,
        reason = "Outer None is malformed input; inner None is the encoded absent value."
    )]
    fn optional_number(&mut self) -> Option<Option<u32>> {
        match self.number()? {
            u64::MAX => Some(None),
            n => Some(Some(u32::try_from(n).ok()?)),
        }
    }
    #[allow(
        clippy::option_option,
        reason = "Outer None is malformed input; inner None is the encoded absent value."
    )]
    fn optional_string(&mut self) -> Option<Option<String>> {
        match self.number()? {
            u64::MAX => Some(None),
            n => Some(Some(self.text(usize::try_from(n).ok()?)?)),
        }
    }
}

pub(super) fn decode(bytes: &[u8]) -> Option<OffsetIndex> {
    if bytes.get(..8)? != MAGIC {
        return None;
    }
    let checksum = u64::from_le_bytes(bytes.get(8..16)?.try_into().ok()?);
    if xxhash_rust::xxh3::xxh3_64(bytes.get(16..)?) != checksum {
        return None;
    }
    let mut r = Reader(&bytes[16..]);
    let version = u32::try_from(r.number()?).ok()?;
    let size = r.number()?;
    let mtime_ns = r.number()?;
    let identity = FileStamp {
        len: r.number()?,
        mtime: r.number()?,
        inode: r.number()?,
        device: r.number()?,
        changed: (r.number()?.cast_signed(), r.number()?.cast_signed()),
        tail_digest: r.number()?,
    };
    let generation = r.number()?;
    let end = r.number()?;
    let count = r.count()?;
    let mut fingerprint = Vec::with_capacity(count.min(r.0.len() / 8));
    for _ in 0..count {
        fingerprint.push(r.number()?);
    }
    let count = r.count()?;
    let mut entries = Vec::with_capacity(count.min(r.0.len() / 81));
    for _ in 0..count {
        let offset = r.number()?;
        let len = u32::try_from(r.number()?).ok()?;
        let id = r.string()?;
        let parent = r.optional_number()?;
        let flags = *r.take(1)?.first()?;
        let depth = usize::try_from(r.number()?).ok()?;
        let count = r.count()?;
        let mut pending = Vec::with_capacity(count.min(r.0.len() / 8));
        for _ in 0..count {
            pending.push(r.string()?);
        }
        entries.push(EntryPos {
            offset,
            len,
            id,
            parent,
            flags,
            depth,
            pending,
            checkpoint: r.optional_number()?,
            custom_type: r.optional_string()?,
            compaction_summary_id: r.optional_string()?.map(Into::into),
            branch_compaction: r.optional_number()?,
        });
    }
    let index = OffsetIndex {
        version,
        size,
        mtime_ns,
        identity,
        generation,
        end,
        fingerprint,
        entries,
        last_compaction: r.optional_number()?,
        title: r.optional_string()?,
        moved_cwd: r.optional_string()?,
        by_id: OnceLock::default(),
    };
    r.0.is_empty().then_some(index)
}

/// Only the fixed metadata and block hashes, so validation can overlap entry decoding.
pub(super) fn prefix(bytes: &[u8]) -> Option<OffsetIndex> {
    if bytes.get(..8)? != MAGIC {
        return None;
    }
    let mut r = Reader(bytes.get(16..)?);
    let version = u32::try_from(r.number()?).ok()?;
    let size = r.number()?;
    let mtime_ns = r.number()?;
    let identity = FileStamp {
        len: r.number()?,
        mtime: r.number()?,
        inode: r.number()?,
        device: r.number()?,
        changed: (r.number()?.cast_signed(), r.number()?.cast_signed()),
        tail_digest: r.number()?,
    };
    let generation = r.number()?;
    let end = r.number()?;
    let count = r.count()?;
    let mut fingerprint = Vec::with_capacity(count.min(r.0.len() / 8));
    for _ in 0..count {
        fingerprint.push(r.number()?);
    }
    Some(OffsetIndex {
        version,
        size,
        mtime_ns,
        identity,
        generation,
        end,
        fingerprint,
        entries: Vec::new(),
        last_compaction: None,
        title: None,
        moved_cwd: None,
        by_id: OnceLock::default(),
    })
}
