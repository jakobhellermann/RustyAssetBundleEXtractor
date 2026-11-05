use crate::archive_storage_manager::ArchiveStorageDecryptor;
use crate::files::bundlefile::{
    BundleFileHeader, BundleSignature, CompressionType, StorageBlock, StorageBlockFlags,
    read_unityfs_info,
};
use crate::files::unityfile::FileEntry;
use std::borrow::Cow;
use std::io::{Cursor, Read, Seek, SeekFrom};

use super::ExtractionConfig;

// The chunks of the bundle file are compressed in blocks, but those blocks don't necessarily
// align with the stored `FileEntry`s inside.
pub struct BundleFileReader<R> {
    header: BundleFileHeader,
    decryptor: Option<ArchiveStorageDecryptor>,
    pub reader: R,
    /// The position in the reader after the header where the file data starts
    data_offset: u64,

    blocks: Vec<StorageBlock>,
    block_starts: Vec<BlockStart>,
    files: Vec<FileEntry>,
    block_index: usize,
    file_index: isize,
    /// Amount of bytes in `scratch` that were returned by `read`, and need to be clear before the next file
    scratch_pending_clear: usize,
    /// Scratch buffer which `read` returns a slice to
    scratch: Vec<u8>,
    /// The current (uncompressed) position for which the scratch buffer stores data
    /// Mainly used when files aren't continuosly next to each other, but have some
    /// space in between them.
    scratch_offset: usize,
}

impl<R: std::fmt::Debug> std::fmt::Debug for BundleFileReader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BundleFileReader")
            .field("header", &self.header)
            .field("blocks", &self.blocks)
            .field("files", &self.files)
            .finish_non_exhaustive()
    }
}

impl<R: Read + Seek> BundleFileReader<R> {
    /// Read the bundlefile from an I/O stream.
    pub fn from_reader(
        mut reader: R,
        config: &ExtractionConfig,
    ) -> Result<BundleFileReader<R>, std::io::Error> {
        let mut header = BundleFileHeader::from_reader(&mut reader)?;
        match header.signature {
            BundleSignature::UnityFS => {}
            _ => unimplemented!(),
        };
        let (blocks, files, decryptor) = read_unityfs_info(&mut header, reader.by_ref(), config)?;

        let data_offset = reader.stream_position()?;
        let block_starts = block_starts(&blocks);
        Ok(BundleFileReader {
            header,
            decryptor,
            reader,
            data_offset,
            blocks,
            block_starts,
            files,
            block_index: 0,
            file_index: -1,
            scratch_pending_clear: 0,
            scratch: Vec::new(),
            scratch_offset: 0,
        })
    }

    pub fn compression(&self) -> CompressionType {
        self.blocks
            .iter()
            .flat_map(|block| {
                CompressionType::try_from(
                    block.flags & StorageBlockFlags::COMPRESSION_TYPE_MASK.bits(),
                )
            })
            .find(|flags| *flags != CompressionType::None)
            .unwrap_or(CompressionType::None)
    }

    pub fn header(&self) -> &BundleFileHeader {
        &self.header
    }

    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    pub fn serialized_files(&self) -> impl Iterator<Item = &FileEntry> {
        self.files.iter().filter(|file| file.is_serialized())
    }

    pub fn main_serializedfile(&self) -> Option<&FileEntry> {
        self.serialized_files()
            .find(|file| !file.path.ends_with(".sharedAssets"))
    }

    pub fn blocks(&self) -> &[StorageBlock] {
        &self.blocks
    }

    /// Advance the reader to the next file entry. Call [`BundleFileRef::read`] to get the actual data.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<BundleFileRef<'_, R>> {
        self.file_index += 1;
        let file = self.files.get(self.file_index as usize)?.clone();

        Some(BundleFileRef {
            iter: self,
            path: file.path,
            flags: file.flags,
            offset: file.offset,
            size: file.size as usize,
        })
    }

    /// Advance to the next serialized file, skipping non-serialized entries such as `.resS`
    /// resources. Call [`BundleFileRef::read`] to get the actual data.
    pub fn next_serialized(&mut self) -> Option<BundleFileRef<'_, R>> {
        let (index, file) = self
            .files
            .iter()
            .enumerate()
            .skip((self.file_index + 1) as usize)
            .find(|(_, file)| file.is_serialized())
            .map(|(index, file)| (index, file.clone()))?;

        self.file_index = index as isize;
        Some(BundleFileRef {
            iter: self,
            path: file.path,
            flags: file.flags,
            offset: file.offset,
            size: file.size as usize,
        })
    }

    /// Go back to the start of the file, so that [`BundleFileReader::next`] will yield the first file.
    pub fn reset(&mut self) -> Result<(), std::io::Error> {
        self.block_index = 0;
        self.file_index = -1;
        self.scratch_pending_clear = 0;
        self.scratch.clear();
        self.scratch_offset = 0;
        self.reader.seek(SeekFrom::Start(self.data_offset))?;
        Ok(())
    }

    /// The first block whose data reaches past `offset` into the uncompressed stream,
    /// `blocks.len()` if none does.
    fn first_block(&self, offset: u64) -> usize {
        self.block_starts[1..].partition_point(|end| end.uncompressed <= offset)
    }

    pub fn file(&self, file_path: &str) -> Option<&FileEntry> {
        self.files().iter().find(|file| file.path == file_path)
    }

    /// Get a [`BundleFileRef`] to an arbitrary file in the bundle by path.
    pub fn seek_file(
        &mut self,
        file_path: &str,
    ) -> Option<Result<BundleFileRef<'_, R>, std::io::Error>> {
        self.files()
            .iter()
            .enumerate()
            .find_map(|(file_index, file)| {
                (file.path == file_path).then(|| (file_index, file.clone()))
            })
            .map(|(file_index, file_entry)| self.seek_file_inner(file_index, file_entry.clone()))
    }

    fn seek_file_inner(
        &mut self,
        file_index: usize,
        file: FileEntry,
    ) -> Result<BundleFileRef<'_, R>, std::io::Error> {
        let block_index = self.first_block(file.offset as u64);
        let start = self.block_starts[block_index];

        self.block_index = block_index;
        self.file_index = file_index as isize;
        self.scratch_pending_clear = 0;
        self.scratch.clear();
        self.scratch_offset = start.uncompressed as usize;
        self.reader
            .seek(SeekFrom::Start(self.data_offset + start.stored))?;

        Ok(BundleFileRef {
            path: file.path,
            flags: file.flags,
            size: file.size as usize,
            offset: file.offset,
            iter: self,
        })
    }
}

impl<T: AsRef<[u8]>> BundleFileReader<Cursor<T>> {
    pub fn read_at(&self, path: &str) -> Result<Option<Vec<u8>>, std::io::Error> {
        self.file(path)
            .map(|entry| self.read_entry(entry).map(Cow::into_owned))
            .transpose()
    }

    #[deprecated(note = "use `read_entry`, which borrows where it can")]
    pub fn read_at_entry(&self, entry: &FileEntry) -> Result<Vec<u8>, std::io::Error> {
        self.read_entry(entry).map(Cow::into_owned)
    }

    pub fn read_entry(&self, entry: &FileEntry) -> Result<Cow<'_, [u8]>, std::io::Error> {
        if entry.size == 0 {
            return Ok(Cow::Borrowed(&[]));
        }
        let stream_end = self.block_starts[self.blocks.len()].uncompressed;
        if entry.end() as u64 > stream_end {
            return Err(std::io::Error::other("entry past the end of the blocks"));
        }

        let data = &self.reader.get_ref().as_ref()[self.data_offset as usize..];
        let (offset, size) = (entry.offset as u64, entry.size as usize);
        let blocks = self.first_block(offset)..=self.first_block(entry.end() as u64 - 1);
        let start = self.block_starts[*blocks.start()];
        let skip = (offset - start.uncompressed) as usize;

        if self.blocks[blocks.clone()].iter().all(stored_as_is) {
            // consecutive blocks stored as they are hold their uncompressed bytes back to back
            let at = start.stored as usize + skip;
            return data
                .get(at..at + size)
                .map(Cow::Borrowed)
                .ok_or_else(|| std::io::Error::other("the archive ends inside an entry"));
        }

        let end = self.block_starts[*blocks.end() + 1];
        let mut decoded = Vec::with_capacity((end.uncompressed - start.uncompressed) as usize);
        let mut reader = Cursor::new(&data[start.stored as usize..]);
        for index in blocks {
            super::decompress_block(
                &mut reader,
                &mut decoded,
                &self.blocks[index],
                index,
                self.decryptor.as_ref(),
            )?;
        }

        decoded.drain(..skip);
        decoded.truncate(size);
        Ok(Cow::Owned(decoded))
    }
}

#[derive(Debug, Clone, Copy)]
struct BlockStart {
    stored: u64,
    uncompressed: u64,
}

fn block_starts(blocks: &[StorageBlock]) -> Vec<BlockStart> {
    let mut start = BlockStart {
        stored: 0,
        uncompressed: 0,
    };
    let mut starts = Vec::with_capacity(blocks.len() + 1);
    starts.push(start);
    for block in blocks {
        start.stored += block.compressed_size as u64;
        start.uncompressed += block.uncompressed_size as u64;
        starts.push(start);
    }
    starts
}

fn stored_as_is(block: &StorageBlock) -> bool {
    let transformed = StorageBlockFlags::COMPRESSION_TYPE_MASK | StorageBlockFlags::ENCRYPTED;
    block.flags & transformed.bits() == 0
}

pub struct BundleFileRef<'s, R> {
    pub path: String,
    pub size: usize,
    pub flags: u32,
    iter: &'s mut BundleFileReader<R>,
    offset: i64,
}

impl<'s, R: std::fmt::Debug> std::fmt::Debug for BundleFileRef<'s, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BundleFileRef")
            .field("path", &self.path)
            .field("flags", &self.flags)
            .field("size", &self.size)
            .field("offset", &self.offset)
            .finish()
    }
}

impl<R: Read + Seek> BundleFileRef<'_, R> {
    pub fn read(&mut self) -> Result<&[u8], std::io::Error> {
        // clear part of scratch that was already read
        discard_front(&mut self.iter.scratch, self.iter.scratch_pending_clear);
        self.iter.scratch_offset += self.iter.scratch_pending_clear;
        self.iter.scratch_pending_clear = 0;

        // This is nonzero if you call `BundleIterator::next` without reading the file,
        // OR if two files in the asset bundle aren't actually contiguous and there's a gap.
        let mut skip_read = self.offset as usize - self.iter.scratch_offset;

        if skip_read > 0 {
            if self.iter.scratch.len() > skip_read {
                discard_front(&mut self.iter.scratch, skip_read);
                self.iter.scratch_offset += skip_read;
                skip_read = 0;
            } else if !self.iter.scratch.is_empty() {
                skip_read -= self.iter.scratch.len();
                self.iter.scratch_offset += self.iter.scratch.len();
                self.iter.scratch.clear();
            }
        }

        if self.iter.scratch.len() < self.size {
            while let Some(block) = self.iter.blocks.get(self.iter.block_index) {
                let block_index = self.iter.block_index;
                self.iter.block_index += 1;

                if skip_read >= block.uncompressed_size as usize {
                    skip_read -= block.uncompressed_size as usize;
                    self.iter
                        .reader
                        .seek_relative(block.compressed_size as i64)?;
                    self.iter.scratch_offset += block.uncompressed_size as usize;
                    continue;
                }

                super::decompress_block(
                    self.iter.reader.by_ref(),
                    &mut self.iter.scratch,
                    block,
                    block_index,
                    self.iter.decryptor.as_ref(),
                )?;

                if skip_read > 0 {
                    debug_assert!(skip_read <= self.iter.scratch.len());
                    discard_front(&mut self.iter.scratch, skip_read);
                    self.iter.scratch_offset += skip_read;
                    skip_read = 0;
                }

                if self.iter.scratch.len() >= self.size {
                    break;
                }
            }
        }

        assert_eq!(self.iter.scratch_offset, self.offset as usize);
        self.iter.scratch_pending_clear = self.size;
        Ok(&self.iter.scratch[..self.size])
    }
}

/// Removes the first <begin> bytes of the vector
fn discard_front(vec: &mut Vec<u8>, begin: usize) {
    if begin == 0 {
        return;
    }

    let len = vec.len();
    if begin >= len {
        if len != 0 {
            panic!(
                "Tried to discard_front {begin} bytes, length is only {}",
                vec.len()
            )
        }
    } else {
        let rest_len = len - begin;
        vec.copy_within(begin.., 0);
        vec.truncate(rest_len);
    }
}
