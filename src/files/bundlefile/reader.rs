use crate::archive_storage_manager::ArchiveStorageDecryptor;
use crate::files::bundlefile::{
    BundleFileHeader, BundleSignature, StorageBlock, read_unityfs_info,
};
use crate::files::unityfile::FileEntry;
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
        Ok(BundleFileReader {
            header,
            decryptor,
            reader,
            data_offset,
            blocks,
            files,
            block_index: 0,
            file_index: -1,
            scratch_pending_clear: 0,
            scratch: Vec::new(),
            scratch_offset: 0,
        })
    }

    pub fn header(&self) -> &BundleFileHeader {
        &self.header
    }

    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    pub fn serialized_files(&self) -> impl Iterator<Item = &FileEntry> {
        self.files.iter().filter(|file| (file.flags & 4) != 0)
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
        let (block_index, block_offset_compressed, block_offset_uncompressed) =
            scan_file_start(&self.blocks, &file);

        self.block_index = block_index;
        self.file_index = file_index as isize;
        self.scratch_pending_clear = 0;
        self.scratch.clear();
        self.scratch_offset = block_offset_uncompressed as usize;
        self.reader
            .seek(SeekFrom::Start(self.data_offset + block_offset_compressed))?;

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
        self.files()
            .iter()
            .find(|entry| entry.path == path)
            .map(|entry| self.read_at_entry(entry))
            .transpose()
    }

    pub fn read_at_entry(&self, entry: &FileEntry) -> Result<Vec<u8>, std::io::Error> {
        let data = &self.reader.get_ref().as_ref()[self.data_offset as usize..];
        read_single_file(&self.blocks, entry, data)
    }
}

fn scan_file_start(blocks: &[StorageBlock], file: &FileEntry) -> (usize, u64, u64) {
    let mut offset_uncompressed = 0;
    let mut offset_compressed = 0;
    let mut block_index = 0;

    // PERF: this linear scan on every seek could be avoided if the file block offsets
    // were computed and saved once at creation
    for block in blocks {
        if offset_uncompressed + block.uncompressed_size as u64 > file.offset as u64 {
            break;
        }

        offset_uncompressed += block.uncompressed_size as u64;
        offset_compressed += block.compressed_size as u64;
        block_index += 1;
    }

    (block_index, offset_compressed, offset_uncompressed)
}

/// Reads a single file to memory
fn read_single_file(
    blocks: &[StorageBlock],
    file: &FileEntry,
    data: &[u8],
) -> Result<Vec<u8>, std::io::Error> {
    let (mut block_index, block_offset_compressed, block_offset_uncompressed) =
        scan_file_start(blocks, file);

    let data = &data[block_offset_compressed as usize..];
    let mut reader = Cursor::new(data);

    let mut skip_read = file.offset as usize - block_offset_uncompressed as usize;

    let mut data = Vec::new();
    let mut block_offset = block_offset_uncompressed as usize;

    while let Some(block) = blocks.get(block_index) {
        super::decompress_block(&mut reader, &mut data, block, block_index, None)?;
        block_index += 1;
        block_offset += block.uncompressed_size as usize;

        discard_front(&mut data, std::mem::take(&mut skip_read));

        if block_offset > file.end() as usize {
            break;
        }
    }
    data.truncate(file.size as usize);

    Ok(data)
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
