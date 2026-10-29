//! Support for parsing compressed type tree dumps from [`AssetRipper/Tpk`](https://github.com/AssetRipper/Tpk)
//!
//! ```rust,no_run
//! use std::fs::File;
//! use rabex::tpk::TpkFile;
//! use rabex::objects::ClassId;
//! # use anyhow::Result;
//!
//! fn main() -> Result<()> {
//!     let mut tpk_file = File::open("lz4.tpk")?;
//!     let tpk_file = TpkFile::from_reader(&mut tpk_file)?;
//!     let tpk = tpk_file.as_type_tree()?.unwrap();
//!
//!     let version = "2023.2.18f1".parse().unwrap();
//!
//!     let ty = tpk.get_typetree_node(ClassId::GameObject, &version).unwrap();
//!     println!("{}", ty.dump());
//!
//!     Ok(())
//! }
//!
//! ```
//!
//! [`TpkTypeTreeBlob`] implements the [`TypeTreeProvider`](crate::typetree::TypeTreeProvider) crate, so with
//! the `embed-tpk` feature, using TPK to look up types is as easy as
//! ```rust
//! # use rabex::tpk::TpkTypeTreeBlob;
//! # use rabex::typetree::typetree_cache::TypeTreeCache;
//! # use rabex::typetree::TypeTreeProvider;
//! # use rabex::objects::ClassId;
//! # let unity_version = "2020.2.2f1".parse().unwrap();
//! let tpk = TypeTreeCache::new(TpkTypeTreeBlob::embedded());
//! let node = tpk.get_typetree_node(ClassId::Transform, &unity_version);
//! ```

#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    InvalidMagic,
    UnsupportedVersion(u8),
    IO(std::io::Error),
    UTF8(std::string::FromUtf8Error),
    UnsupportedCompression(&'static str),
    Decompression(Box<dyn std::error::Error + Send + Sync + 'static>),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidMagic => write!(f, "Invalid TPK magic value"),
            Error::UnsupportedVersion(ver) => write!(f, "Unsupported version: {ver}"),
            Error::IO(e) => write!(f, "IO error: {e}"),
            Error::UnsupportedCompression(method) => {
                write!(f, "rabex feature for {method} support is not enabled")
            }
            Error::UTF8(e) => write!(f, "invalid utf8: {e}"),
            Error::Decompression(e) => write!(f, "failed to decompress: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::IO(e) => Some(e),
            Error::UTF8(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::IO(err)
    }
}
impl<T: TryFromPrimitive> From<TryFromPrimitiveError<T>> for Error {
    fn from(err: TryFromPrimitiveError<T>) -> Self {
        Error::IO(std::io::Error::other(format!(
            "No discriminant in enum `{name}` matches the value `{input:?}`",
            name = T::NAME,
            input = err.number,
        )))
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

use bitflags::bitflags;
use num_enum::{TryFromPrimitive, TryFromPrimitiveError};
use rustc_hash::FxHashMap;

use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::Read;

use byteorder::{LittleEndian, ReadBytesExt};

use crate::objects::ClassId;
use crate::typetree::TypeTreeNode;
use crate::unity_version::{UnityVersion, UnityVersionType};

/// The TPK container type, contained (possibly compressed) data for the inner format.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TpkFile {
    pub version: u8,
    pub compression_type: TpkCompressionType,
    pub data_type: TpkDataType,
    pub compressed_size: i32,
    pub uncompressed_size: i32,
    pub compressed_bytes: Vec<u8>,
}

impl Default for TpkFile {
    fn default() -> Self {
        Self {
            version: 1,
            compression_type: TpkCompressionType::None,
            data_type: TpkDataType::TypeTreeInformation,
            compressed_size: 0,
            uncompressed_size: 0,
            compressed_bytes: Vec::new(),
        }
    }
}

impl TpkFile {
    pub fn from_reader<T: std::io::Read>(reader: &mut T) -> Result<TpkFile> {
        let magic = reader.read_u32::<LittleEndian>()?;
        if u32::to_le_bytes(magic) != *b"TPK*" {
            return Err(Error::InvalidMagic);
        }

        let version = reader.read_u8()?;
        if !matches!(version, 1 | 2) {
            return Err(Error::UnsupportedVersion(version));
        }

        let compression_type = TpkCompressionType::try_from(reader.read_u8()?)?;
        let data_type = TpkDataType::try_from(reader.read_u8()?)?;
        reader.read_u8()?;
        reader.read_u32::<LittleEndian>()?;
        let compressed_size = reader.read_i32::<LittleEndian>()?;
        let decompressed_size = reader.read_i32::<LittleEndian>()?;
        let mut compressed_bytes = vec![0; compressed_size as usize]; // TODO resilience
        reader.read_exact(&mut compressed_bytes)?;

        Ok(TpkFile {
            version,
            compression_type,
            data_type,
            compressed_size,
            uncompressed_size: decompressed_size,
            compressed_bytes,
        })
    }

    pub fn decompress(&self) -> Result<Cow<'_, [u8]>> {
        Ok(match self.compression_type {
            TpkCompressionType::None => Cow::Borrowed(&self.compressed_bytes),

            #[cfg(not(feature = "compression-lz4"))]
            TpkCompressionType::Lz4 => return Err(Error::UnsupportedCompression("lz4")),
            #[cfg(feature = "compression-lz4")]
            TpkCompressionType::Lz4 => {
                lz4_flex::block::decompress(&self.compressed_bytes, self.uncompressed_size as usize)
                    .map_err(|e| Error::Decompression(Box::new(e)))?
                    .into()
            }
            #[cfg(not(feature = "compression-lzma"))]
            TpkCompressionType::Lzma => return Err(Error::UnsupportedCompression("lzma")),
            #[cfg(feature = "compression-lzma")]
            TpkCompressionType::Lzma => {
                let mut uncompressed = vec![0; self.uncompressed_size as usize];
                lzma_rs::lzma_decompress_with_options(
                    &mut self.compressed_bytes.as_slice(),
                    &mut uncompressed,
                    &lzma_rs::decompress::Options {
                        unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(
                            self.uncompressed_size as u64,
                        )),
                        memlimit: None,
                        allow_incomplete: false,
                    },
                )
                .map_err(|e| Error::Decompression(Box::new(e)))?;
                Cow::Owned(uncompressed)
            }
            #[cfg(not(feature = "tpk-compression-brotli"))]
            TpkCompressionType::Brotli => return Err(Error::UnsupportedCompression("brotli")),
            #[cfg(feature = "tpk-compression-brotli")]
            TpkCompressionType::Brotli => {
                let mut uncompressed = vec![0; self.uncompressed_size as usize];
                brotli::BrotliDecompress(&mut self.compressed_bytes.as_slice(), &mut uncompressed)?;
                Cow::Owned(uncompressed)
            }
        })
    }

    /// Reads the inner data in case of [`TpkDataType::TypeTreeInformation`]
    pub fn as_type_tree(&self) -> Result<Option<TpkTypeTreeBlob>> {
        match self.data_type {
            TpkDataType::TypeTreeInformation => {
                let data = self.decompress()?;
                let type_tree = TpkTypeTreeBlob::from_reader(&mut data.as_ref(), self.version)?;

                Ok(Some(type_tree))
            }
            _ => Ok(None),
        }
    }
}

impl TpkTypeTreeBlob {
    /// A type tree blob bundled with the crate. Might not always contain the newest version,
    /// pleas open an issue if you need a more modern release.
    #[cfg(feature = "embed-tpk")]
    pub fn embedded() -> Self {
        let bytes = include_bytes!("../resources/lz4.tpk");
        let tpk_file = TpkFile::from_reader(&mut bytes.as_slice()).unwrap();
        tpk_file.as_type_tree().unwrap().unwrap()
    }
}

#[repr(u8)]
#[derive(Debug, Clone, TryFromPrimitive)]
#[non_exhaustive]
pub enum TpkCompressionType {
    None,
    Lz4,
    Lzma,
    Brotli,
}

#[repr(u8)]
#[derive(Debug, Clone, TryFromPrimitive)]
#[non_exhaustive]
pub enum TpkDataType {
    /// Information about the structure of Unity assets
    TypeTreeInformation = 0,
    /// A collection of blobs
    Collection = 1,
    /// A file system archive
    FileSystem = 2,
    /// Custom json data
    Json = 3,
    /// Lists of reference assemblies in the editor
    ReferenceAssemblies = 4,
    /// Lists of default Unity assets and their export ids
    EngineAssets = 5,
}

/// Size-Optimized store of unity type trees across multiple unity versions.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TpkTypeTreeBlob {
    pub creation_time: i64,
    pub versions: Vec<UnityVersion>,
    pub class_information: FxHashMap<ClassId, Vec<(UnityVersion, Option<UnityClass>)>>,
    pub common_string: TpkCommonString,
    pub node_buffer: Vec<TpkUnityNode>,
    pub string_buffer: Vec<String>,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TpkClassInformation {
    pub id: i32,
    pub classes: Vec<(UnityVersion, Option<UnityClass>)>,
}
impl TpkClassInformation {
    pub fn read<R: Read>(reader: &mut R) -> Result<TpkClassInformation> {
        let id = reader.read_i32::<LittleEndian>()?;
        let count = reader.read_i32::<LittleEndian>()?;
        let mut classes = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let version = UnityVersion::read_tpk_encoding(reader)?;
            let has_class_data = reader.read_u8()? != 0;
            let class = has_class_data
                .then(|| UnityClass::read(reader))
                .transpose()?;
            classes.push((version, class))
        }
        Ok(TpkClassInformation { id, classes })
    }
}

bitflags! {
#[derive(Clone, Debug)]
pub struct TpkUnityClassFlags : u8 {
    /// None of the flags apply to this class
    const NONE = 0;
    /// Is the class abstract?
    const IS_ABSTRACT = 1;
    /// Is the class sealed? Not necessarily accurate.
    const IS_SEALED = 2;
    /// Does the class only appear in the editor?
    const IS_EDITOR_ONLY = 4;
    /// Does the class only appear in game files? Not currently used
    const IS_RELEASE_ONLY = 8;
    /// Is the class stripped?
    const IS_STRIPPED = 16;
    /// Not currently used
    const RESERVED = 32;
    /// Does the class have an editor root node?
    const HAS_EDITOR_ROOT_NODE = 64;
    /// Does the class have a release root node?
    const HAS_RELEASE_ROOT_NODE = 128;
}
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct UnityClass {
    pub name: u16,
    pub base: u16,
    pub flags: TpkUnityClassFlags,
    pub editor_root_node: Option<u16>,
    pub release_root_node: Option<u16>,
}

impl UnityClass {
    pub fn read<R: Read>(reader: &mut R) -> Result<UnityClass> {
        let name = reader.read_u16::<LittleEndian>()?;
        let base = reader.read_u16::<LittleEndian>()?;
        let flags = TpkUnityClassFlags::from_bits(reader.read_u8()?).unwrap();
        let editor_root_node = flags
            .contains(TpkUnityClassFlags::HAS_EDITOR_ROOT_NODE)
            .then(|| reader.read_u16::<LittleEndian>())
            .transpose()?;
        let release_root_node = flags
            .contains(TpkUnityClassFlags::HAS_EDITOR_ROOT_NODE)
            .then(|| reader.read_u16::<LittleEndian>())
            .transpose()?;
        Ok(UnityClass {
            name,
            base,
            flags,
            editor_root_node,
            release_root_node,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TpkCommonStringEntry {
    /// Byte offset of the string in the common string table
    pub offset: u16,
    /// Index into [`TpkTypeTreeBlob::string_buffer`]
    pub string: u16,
}

/// Common string tables, sequential by ascending Unity version.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TpkCommonString {
    pub version_information: Vec<(UnityVersion, Vec<TpkCommonStringEntry>)>,
}
impl TpkCommonString {
    fn read_v2<R: Read>(reader: &mut R) -> Result<TpkCommonString> {
        let version_count = reader.read_i32::<LittleEndian>()?;
        let mut version_information = Vec::with_capacity(version_count as usize);
        for _ in 0..version_count {
            let version = UnityVersion::read_tpk_encoding(reader)?;
            let entry_count = reader.read_i32::<LittleEndian>()?;
            let mut entries = Vec::with_capacity(entry_count as usize);
            for _ in 0..entry_count {
                entries.push(TpkCommonStringEntry {
                    offset: reader.read_u16::<LittleEndian>()?,
                    string: reader.read_u16::<LittleEndian>()?,
                });
            }
            version_information.push((version, entries));
        }
        Ok(TpkCommonString {
            version_information,
        })
    }

    pub fn get_entries(&self, target_version: &UnityVersion) -> Option<&[TpkCommonStringEntry]> {
        self.version_information
            .iter()
            .rev()
            .find_map(|(version, entries)| {
                (target_version >= version).then_some(entries.as_slice())
            })
    }
}

/// The v1 format stores one append-only string list and a per-version prefix length.
struct TpkCommonStringV1 {
    version_information: Vec<(UnityVersion, u8)>,
    string_buffer_indices: Vec<u16>,
}
impl TpkCommonStringV1 {
    fn read<R: Read>(reader: &mut R) -> Result<TpkCommonStringV1> {
        let version_count = reader.read_i32::<LittleEndian>()?;
        let mut version_information = Vec::with_capacity(version_count as usize);
        for _ in 0..version_count {
            let version = UnityVersion::read_tpk_encoding(reader)?;
            let string_count = reader.read_u8()?;
            version_information.push((version, string_count));
        }
        let indices_count = reader.read_i32::<LittleEndian>()?;
        let mut string_buffer_indices = Vec::with_capacity(indices_count as usize);
        for _ in 0..indices_count {
            string_buffer_indices.push(reader.read_u16::<LittleEndian>()?);
        }
        Ok(TpkCommonStringV1 {
            version_information,
            string_buffer_indices,
        })
    }

    fn resolve(self, string_buffer: &[String]) -> TpkCommonString {
        let mut all_entries = Vec::with_capacity(self.string_buffer_indices.len());
        let mut offset = 0usize;
        for string in self.string_buffer_indices {
            all_entries.push(TpkCommonStringEntry {
                offset: u16::try_from(offset).unwrap(),
                string,
            });
            offset += string_buffer[string as usize].len() + 1;
        }

        let version_information = self
            .version_information
            .into_iter()
            .map(|(version, count)| (version, all_entries[..count as usize].to_vec()))
            .collect();
        TpkCommonString {
            version_information,
        }
    }
}

enum CommonStringRaw {
    V1(TpkCommonStringV1),
    V2(TpkCommonString),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TpkUnityNode {
    pub type_name: u16,
    pub name: u16,
    pub byte_size: i32,
    pub version: i16,
    pub type_flags: u8,
    pub meta_flag: u32,
    pub sub_nodes: Vec<u16>,
}
impl TpkUnityNode {
    pub fn read<R: Read>(reader: &mut R) -> Result<TpkUnityNode> {
        let type_name = reader.read_u16::<LittleEndian>()?;
        let name = reader.read_u16::<LittleEndian>()?;
        let byte_size = reader.read_i32::<LittleEndian>()?;
        let version = reader.read_i16::<LittleEndian>()?;
        let type_flags = reader.read_u8()?;
        let meta_flag = reader.read_u32::<LittleEndian>()?;
        let node_count = reader.read_u16::<LittleEndian>()?;
        let mut sub_nodes = Vec::with_capacity(node_count as usize);
        for _ in 0..node_count {
            sub_nodes.push(reader.read_u16::<LittleEndian>()?);
        }

        Ok(TpkUnityNode {
            type_name,
            name,
            byte_size,
            version,
            type_flags,
            meta_flag,
            sub_nodes,
        })
    }
}

impl UnityVersion {
    fn read_tpk_encoding<R: Read>(reader: &mut R) -> Result<UnityVersion> {
        let version = reader.read_u64::<LittleEndian>()?;
        let major = ((version >> 48) & 0xFFFF) as u16;
        let minor = ((version >> 32) & 0xFFFF) as u16;
        let build = ((version >> 16) & 0xFFFF) as u16;
        let typ = UnityVersionType::try_from(((version >> 8) & 0xFF) as u8)?;
        let type_number = (version & 0xFF) as u8;
        Ok(UnityVersion {
            major,
            minor,
            build,
            typ,
            build_number: type_number,
            trailing_data: String::new(),
        })
    }
}

impl TpkTypeTreeBlob {
    /// `version` is the [`TpkFile::version`] of the containing file.
    pub fn from_reader<R: Read>(reader: &mut R, version: u8) -> Result<TpkTypeTreeBlob> {
        let creation_time = reader.read_i64::<LittleEndian>()?;

        let version_count = reader.read_i32::<LittleEndian>()? as usize;
        let mut versions = Vec::with_capacity(version_count);
        for _ in 0..version_count {
            versions.push(UnityVersion::read_tpk_encoding(reader)?);
        }

        let class_count = reader.read_i32::<LittleEndian>()? as usize;
        let mut class_information =
            FxHashMap::with_capacity_and_hasher(class_count, Default::default());
        for _ in 0..class_count {
            let class = TpkClassInformation::read(reader)?;
            class_information.insert(ClassId(class.id), class.classes);
        }

        let common_string = match version {
            1 => CommonStringRaw::V1(TpkCommonStringV1::read(reader)?),
            2 => CommonStringRaw::V2(TpkCommonString::read_v2(reader)?),
            _ => return Err(Error::UnsupportedVersion(version)),
        };

        let node_count = reader.read_i32::<LittleEndian>()?;
        let mut node_buffer = Vec::new();
        for _ in 0..node_count {
            node_buffer.push(TpkUnityNode::read(reader)?);
        }
        let string_count = reader.read_i32::<LittleEndian>()?;
        let mut string_buffer = Vec::new();
        for _ in 0..string_count {
            string_buffer.push(read_7bit_encoded_string(reader)?);
        }
        let common_string = match common_string {
            CommonStringRaw::V1(v1) => v1.resolve(&string_buffer),
            CommonStringRaw::V2(v2) => v2,
        };

        Ok(TpkTypeTreeBlob {
            creation_time,
            versions,
            class_information,
            common_string,
            node_buffer,
            string_buffer,
        })
    }

    /// Look up and construct a [`TypeTreeNode`] for the class and unity version.
    /// If you call this method often, consider using [`TpkTypeTreeCache`](crate::typetree::typetree_cache::TypeTreeCache).
    pub fn get_typetree_node(
        &self,
        class_id: ClassId,
        target_version: &UnityVersion,
    ) -> Option<TypeTreeNode> {
        let version_classes = &self.class_information[&class_id];

        let class = version_classes
            .iter()
            .rev()
            .find_map(|(version, class)| (target_version >= version).then_some(class))?
            .as_ref()?;

        self.get_typetree_node_for_class(class, false)
    }

    pub fn get_typetree_node_for_class(
        &self,
        class: &UnityClass,
        editor: bool,
    ) -> Option<TypeTreeNode> {
        let mut nodes = Vec::new();

        let mut stack = VecDeque::new();

        let root = match editor {
            false => class.release_root_node,
            true => class.editor_root_node,
        }?;

        stack.push_back((root, 0));

        // adapted from https://github.com/K0lb3/UnityPy/blob/62bf31064bb36659661f024a770f6e55f3fbd3d3/UnityPy/helpers/Tpk.py#L55
        while let Some((node_id, level)) = stack.pop_front() {
            let node = &self.node_buffer[node_id as usize];
            let type_name = &self.string_buffer[node.type_name as usize];
            let name = &self.string_buffer[node.name as usize];
            nodes.push((
                level,
                TypeTreeNode {
                    m_Version: node.version as i32,
                    m_TypeFlags: node.type_flags as i32,
                    m_ByteSize: node.byte_size,
                    m_MetaFlag: Some(node.meta_flag as i32),
                    m_Type: type_name.clone(),
                    m_Name: name.clone(),
                    m_RefTypeHash: None,
                    m_VariableCount: None,
                    children: Vec::new(),
                },
            ));
            for &sub_node_id in node.sub_nodes.iter().rev() {
                stack.push_front((sub_node_id, level + 1));
            }
        }

        Some(reconstruct_tree(nodes))
    }
}

fn read_7bit_encoded_int<R: Read>(reader: &mut R) -> Result<usize, std::io::Error> {
    let mut result = 0usize;
    let mut shift = 0;

    loop {
        let byte = reader.read_u8()?;
        result |= ((byte & 0x7F) as usize) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;

        if shift >= 35 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Too many bytes in 7-bit encoded int",
            ));
        }
    }

    Ok(result)
}

/// Reads a 7-bit-length-prefixed UTF-8 string from a stream
fn read_7bit_encoded_string<R: Read>(reader: &mut R) -> Result<String> {
    let len = read_7bit_encoded_int(reader)?;
    let mut buffer = vec![0u8; len];
    reader.read_exact(&mut buffer)?;
    let string = String::from_utf8(buffer).map_err(Error::UTF8)?;
    Ok(string)
}

/// Given a flat list of TypeTreeNodes, reconstruct the tree in memory.
/// Note: This method assumes that the nodes are in the order of the
/// flattened representation of the tree, i.e.
/// level=0 idx=1
/// level=0 idx=2
///   level=1 idx=1
///   level=1 idx=2
///   level=1 idx=3
/// level=0 idx=3
/// level=0 idx=4
fn reconstruct_tree(mut flat_nodes: Vec<(u8, TypeTreeNode)>) -> TypeTreeNode {
    fn access(mut node: &mut TypeTreeNode, mut depth: usize) -> &mut TypeTreeNode {
        while depth > 0 {
            node = node.children.last_mut().unwrap();
            depth -= 1;
        }
        node
    }

    let (_, mut root) = flat_nodes.remove(0);

    for (level, flat_node) in flat_nodes {
        access(&mut root, level as usize - 1)
            .children
            .push(flat_node);
    }

    root
}

#[cfg(test)]
mod format_tests {
    use super::{TpkCommonString, TpkCommonStringEntry, TpkFile};
    use crate::unity_version::UnityVersion;

    fn version(major: u16, minor: u16) -> (u64, UnityVersion) {
        // 'f1' release type
        let encoded = (major as u64) << 48 | (minor as u64) << 32 | 3 << 8 | 1;
        let parsed = format!("{major}.{minor}.0f1").parse().unwrap();
        (encoded, parsed)
    }

    /// Uncompressed TPK with no classes or nodes, the strings `AABB` and `Array`, and the given
    /// encoded common string section.
    fn tpk_bytes(file_version: u8, common_string: &[u8]) -> Vec<u8> {
        let mut blob = Vec::new();
        blob.extend(0i64.to_le_bytes()); // creation time
        blob.extend(0i32.to_le_bytes()); // versions
        blob.extend(0i32.to_le_bytes()); // classes
        blob.extend(common_string);
        blob.extend(0i32.to_le_bytes()); // nodes
        blob.extend(2i32.to_le_bytes()); // strings
        for s in ["AABB", "Array"] {
            blob.push(s.len() as u8);
            blob.extend(s.as_bytes());
        }

        let mut file = b"TPK*".to_vec();
        file.extend([file_version, 0, 0, 0]); // version, no compression, type tree, reserved
        file.extend(0u32.to_le_bytes()); // reserved
        file.extend((blob.len() as i32).to_le_bytes());
        file.extend((blob.len() as i32).to_le_bytes());
        file.extend(blob);
        file
    }

    fn expected() -> TpkCommonString {
        let aabb = TpkCommonStringEntry {
            offset: 0,
            string: 0,
        };
        let array = TpkCommonStringEntry {
            offset: 5,
            string: 1,
        };
        TpkCommonString {
            version_information: vec![
                (version(5, 0).1, vec![aabb.clone()]),
                (version(2019, 1).1, vec![aabb, array]),
            ],
        }
    }

    fn parse(bytes: &[u8]) -> TpkCommonString {
        let file = TpkFile::from_reader(&mut &bytes[..]).unwrap();
        file.as_type_tree().unwrap().unwrap().common_string
    }

    #[test]
    fn common_string_v1() {
        let mut cs = Vec::new();
        cs.extend(2i32.to_le_bytes());
        cs.extend(version(5, 0).0.to_le_bytes());
        cs.push(1);
        cs.extend(version(2019, 1).0.to_le_bytes());
        cs.push(2);
        cs.extend(2i32.to_le_bytes());
        cs.extend(0u16.to_le_bytes());
        cs.extend(1u16.to_le_bytes());

        assert_eq!(parse(&tpk_bytes(1, &cs)), expected());
    }

    #[test]
    fn common_string_v2() {
        let mut cs = Vec::new();
        cs.extend(2i32.to_le_bytes());
        cs.extend(version(5, 0).0.to_le_bytes());
        cs.extend(1i32.to_le_bytes());
        cs.extend([0u16, 0].iter().flat_map(|x| x.to_le_bytes()));
        cs.extend(version(2019, 1).0.to_le_bytes());
        cs.extend(2i32.to_le_bytes());
        cs.extend([0u16, 0, 5, 1].iter().flat_map(|x| x.to_le_bytes()));

        assert_eq!(parse(&tpk_bytes(2, &cs)), expected());
    }

    #[test]
    fn unsupported_version() {
        let err = TpkFile::from_reader(&mut &tpk_bytes(3, &[])[..]).unwrap_err();
        assert!(matches!(err, super::Error::UnsupportedVersion(3)), "{err}");
    }
}

#[cfg(all(test, feature = "embed-tpk"))]
mod commonstring_tests {
    use super::TpkTypeTreeBlob;
    use crate::commonstring::COMMONSTRING;
    use std::collections::BTreeMap;

    /// `src/commonstring.rs` is generated from the TPK via `just update-commonstring`,
    /// the two should agree.
    #[test]
    fn commonstring_matches_tpk() {
        let tpk = TpkTypeTreeBlob::embedded();
        let mut tpk_map = BTreeMap::new();
        for (version, entries) in &tpk.common_string.version_information {
            for entry in entries {
                let s = tpk.string_buffer[entry.string as usize].as_str();
                if let Some(prev) = tpk_map.insert(entry.offset as u32, s) {
                    assert_eq!(prev, s, "offset {} in {version}", entry.offset);
                }
            }
        }

        assert_eq!(*COMMONSTRING, tpk_map);
    }
}
