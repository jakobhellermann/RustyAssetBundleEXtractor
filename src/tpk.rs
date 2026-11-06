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
pub struct TpkFile {
    pub compression_type: TpkCompressionType,
    pub data_type: TpkDataType,
    pub compressed_size: i32,
    pub uncompressed_size: i32,
    pub compressed_bytes: Vec<u8>,
}

impl TpkFile {
    pub fn from_reader<T: std::io::Read>(reader: &mut T) -> Result<TpkFile> {
        let magic = reader.read_u32::<LittleEndian>()?;
        if u32::to_le_bytes(magic) != *b"TPK*" {
            return Err(Error::InvalidMagic);
        }

        let version_number = reader.read_u8()?;
        if version_number != 1 {
            return Err(Error::UnsupportedVersion(version_number));
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
                let type_tree = TpkTypeTreeBlob::from_reader(&mut data.as_ref())?;

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
pub struct TpkTypeTreeBlob {
    pub creation_time: i64,
    pub versions: Vec<TpkVersion>,
    pub class_information: FxHashMap<ClassId, Vec<(TpkVersion, Option<UnityClass>)>>,
    pub common_string: TpkCommonString,
    pub node_buffer: Vec<TpkUnityNode>,
    pub string_buffer: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TpkClassInformation {
    pub id: i32,
    pub classes: Vec<(TpkVersion, Option<UnityClass>)>,
}
impl TpkClassInformation {
    pub fn read<R: Read>(reader: &mut R) -> Result<TpkClassInformation> {
        let id = reader.read_i32::<LittleEndian>()?;
        let count = reader.read_i32::<LittleEndian>()?;
        let mut classes = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let version = TpkVersion::read(reader)?;
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

#[derive(Debug, Clone)]
pub struct TpkCommonString {
    pub version_information: Vec<(TpkVersion, u8)>,
    pub string_buffer_indices: Vec<u16>,
}
impl TpkCommonString {
    pub fn read<R: Read>(reader: &mut R) -> Result<TpkCommonString> {
        let version_count = reader.read_i32::<LittleEndian>()?;
        let mut version_information = Vec::with_capacity(version_count as usize);
        for _ in 0..version_count {
            let version = TpkVersion::read(reader)?;
            let string_count = reader.read_u8()?;
            version_information.push((version, string_count));
        }
        let indices_count = reader.read_i32::<LittleEndian>()?;
        let mut string_buffer_indices = Vec::new();
        for _ in 0..indices_count {
            string_buffer_indices.push(reader.read_u16::<LittleEndian>()?);
        }

        Ok(TpkCommonString {
            version_information,
            string_buffer_indices,
        })
    }

    pub fn get_count(&self, target_version: &UnityVersion) -> Option<u8> {
        let target_version = &TpkVersion::from(target_version);
        let mut ret = None;
        for (version, item) in &self.version_information {
            if target_version >= version {
                ret = Some(*item)
            } else {
                break;
            }
        }

        let iter = self
            .version_information
            .iter()
            .rev()
            .find_map(|(version, item)| (target_version >= version).then_some(*item));
        assert_eq!(iter, ret);

        ret
    }
}

#[derive(Debug, Clone)]
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

/// A unity version as stored in a TPK file:
/// `major << 48 | minor << 32 | build << 16 | type << 8 | build_number`.
///
/// Versions compare as these integers, which for the types in [`UnityVersionType`] is the
/// order of [`UnityVersion`]. Other type bytes are kept as is, and only fail
/// [`TpkVersion::unity_version`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TpkVersion(pub u64);

impl TpkVersion {
    fn read<R: Read>(reader: &mut R) -> Result<TpkVersion> {
        Ok(TpkVersion(reader.read_u64::<LittleEndian>()?))
    }

    /// Converts to a [`UnityVersion`], failing if the type byte is not a [`UnityVersionType`].
    pub fn unity_version(self) -> Result<UnityVersion> {
        let version = self.0;
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

/// Prints the [`UnityVersion`], or the raw integer if the type byte is not a [`UnityVersionType`].
impl std::fmt::Debug for TpkVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.unity_version() {
            Ok(version) => std::fmt::Debug::fmt(&version, f),
            Err(_) => f.debug_tuple("TpkVersion").field(&self.0).finish(),
        }
    }
}

/// Packs a [`UnityVersion`] as a TPK stores it. The
/// [`trailing_data`](UnityVersion::trailing_data), which TPK versions don't have, is dropped.
impl From<&UnityVersion> for TpkVersion {
    fn from(version: &UnityVersion) -> Self {
        TpkVersion(
            (version.major as u64) << 48
                | (version.minor as u64) << 32
                | (version.build as u64) << 16
                | (version.typ as u64) << 8
                | version.build_number as u64,
        )
    }
}

impl TpkTypeTreeBlob {
    pub fn from_reader<R: Read>(reader: &mut R) -> Result<TpkTypeTreeBlob> {
        let creation_time = reader.read_i64::<LittleEndian>()?;

        let version_count = reader.read_i32::<LittleEndian>()? as usize;
        let mut versions = Vec::with_capacity(version_count);
        for _ in 0..version_count {
            versions.push(TpkVersion::read(reader)?);
        }

        let class_count = reader.read_i32::<LittleEndian>()? as usize;
        let mut class_information =
            FxHashMap::with_capacity_and_hasher(class_count, Default::default());
        for _ in 0..class_count {
            let class = TpkClassInformation::read(reader)?;
            class_information.insert(ClassId(class.id), class.classes);
        }

        let common_string = TpkCommonString::read(reader)?;

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
        let target_version = &TpkVersion::from(target_version);

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

#[cfg(all(test, feature = "embed-tpk"))]
mod commonstring_tests {
    use super::TpkTypeTreeBlob;
    use crate::commonstring::COMMONSTRING;
    use std::collections::BTreeMap;

    /// `src/commonstring.rs` is generated from the TPK via `just update-commonstring`. This
    /// guards against it drifting — a hand edit, or a TPK update without regenerating. The
    /// hardcoded read table must equal the full TPK-derived common string table.
    #[test]
    fn commonstring_matches_tpk() {
        let tpk = TpkTypeTreeBlob::embedded();
        let mut tpk_map = BTreeMap::new();
        let mut offset = 0u32;
        for &i in &tpk.common_string.string_buffer_indices {
            let s = tpk.string_buffer[i as usize].as_str();
            tpk_map.insert(offset, s);
            offset += s.len() as u32 + 1;
        }

        assert_eq!(COMMONSTRING.len(), tpk_map.len());
        for (off, s) in COMMONSTRING.iter() {
            assert_eq!(tpk_map.get(off).copied(), Some(*s), "offset {off}");
        }
    }
}

#[cfg(test)]
mod version_tests {
    use super::{TpkTypeTreeBlob, TpkVersion};
    use crate::objects::ClassId;
    use crate::{UnityVersion, UnityVersionType};

    fn version(major: u64, minor: u64, build: u64, typ: u64, build_number: u64) -> u64 {
        major << 48 | minor << 32 | build << 16 | typ << 8 | build_number
    }

    /// A type tree blob listing `versions`, with class 1 at `class_versions` and the common
    /// string table at `common_versions`. The `i`th class entry's release root is a node named
    /// `"{i}"`, and the `i`th common string count is `i`.
    fn blob(versions: &[u64], class_versions: &[u64], common_versions: &[u64]) -> TpkTypeTreeBlob {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0_i64.to_le_bytes()); // creation time

        bytes.extend_from_slice(&(versions.len() as i32).to_le_bytes());
        for v in versions {
            bytes.extend_from_slice(&v.to_le_bytes());
        }

        bytes.extend_from_slice(&1_i32.to_le_bytes()); // class count
        bytes.extend_from_slice(&1_i32.to_le_bytes()); // class id
        bytes.extend_from_slice(&(class_versions.len() as i32).to_le_bytes());
        for (i, v) in class_versions.iter().enumerate() {
            bytes.extend_from_slice(&v.to_le_bytes());
            bytes.push(1); // has class data
            bytes.extend_from_slice(&0_u16.to_le_bytes()); // name
            bytes.extend_from_slice(&0_u16.to_le_bytes()); // base
            bytes.push(64 | 128); // editor and release root
            bytes.extend_from_slice(&(i as u16).to_le_bytes());
            bytes.extend_from_slice(&(i as u16).to_le_bytes());
        }

        bytes.extend_from_slice(&(common_versions.len() as i32).to_le_bytes());
        for (i, v) in common_versions.iter().enumerate() {
            bytes.extend_from_slice(&v.to_le_bytes());
            bytes.push(i as u8);
        }
        bytes.extend_from_slice(&0_i32.to_le_bytes()); // string buffer indices

        bytes.extend_from_slice(&(class_versions.len() as i32).to_le_bytes());
        for i in 0..class_versions.len() {
            bytes.extend_from_slice(&(i as u16).to_le_bytes()); // type name
            bytes.extend_from_slice(&(i as u16).to_le_bytes()); // name
            bytes.extend_from_slice(&(-1_i32).to_le_bytes()); // byte size
            bytes.extend_from_slice(&1_i16.to_le_bytes()); // version
            bytes.push(0); // type flags
            bytes.extend_from_slice(&0_u32.to_le_bytes()); // meta flag
            bytes.extend_from_slice(&0_u16.to_le_bytes()); // sub nodes
        }

        bytes.extend_from_slice(&(class_versions.len() as i32).to_le_bytes());
        for i in 0..class_versions.len() {
            let string = i.to_string();
            bytes.push(string.len() as u8);
            bytes.extend_from_slice(string.as_bytes());
        }

        TpkTypeTreeBlob::from_reader(&mut bytes.as_slice()).unwrap()
    }

    #[test]
    fn unknown_version_type_is_kept() {
        // 2.0.0f1, and 2.0.0 with the type bytes 6 and 255
        let unknown = [
            version(2, 0, 0, 3, 1),
            version(2, 0, 0, 6, 1),
            version(2, 0, 0, 255, 1),
        ];
        let known = [version(1, 0, 0, 3, 1)];

        let tpk = blob(&unknown, &known, &known);
        assert_eq!(tpk.versions.len(), 3);

        let tpk = blob(&known, &unknown, &known);
        assert_eq!(tpk.class_information[&ClassId(1)].len(), 3);

        let tpk = blob(&known, &known, &unknown);
        assert_eq!(tpk.common_string.version_information.len(), 3);
    }

    #[test]
    fn lookup_compares_tpk_versions() {
        // 2.0.0a1, 2.0.0f1, 2.0.0 with the type byte 6, 3.0.0f1
        let versions = [
            version(2, 0, 0, 0, 1),
            version(2, 0, 0, 3, 1),
            version(2, 0, 0, 6, 1),
            version(3, 0, 0, 3, 1),
        ];
        let tpk = blob(&versions, &versions, &versions);

        let experimental = UnityVersion {
            typ: UnityVersionType::Experimental,
            ..UnityVersion::new(2, 0, 0)
        };
        let requests = [
            ("1.0.0f1".parse().unwrap(), None),
            ("2.0.0b1".parse().unwrap(), Some(0)),
            ("2.0.0f1-a".parse().unwrap(), Some(1)),
            ("2.0.0p3".parse().unwrap(), Some(1)),
            (experimental, Some(1)),
            ("2.1.0f1".parse().unwrap(), Some(2)),
            ("3.0.0f1".parse().unwrap(), Some(3)),
        ];
        for (request, index) in requests {
            let node = tpk.get_typetree_node(ClassId(1), &request);
            let name = node.map(|node| node.m_Name);
            assert_eq!(name, index.map(|i: u8| i.to_string()), "{request}");
            let count = tpk.common_string.get_count(&request);
            assert_eq!(count, index, "{request}");
        }
    }

    #[test]
    fn tpk_version_matches_unity_version() {
        let versions = [
            "5.6.7f1",
            "2018.4.2a3",
            "2018.4.2b1",
            "2018.4.2f1",
            "2018.4.2p2",
            "2018.4.3f1",
            "2022.1.0f1",
            "6000.0.50f1",
        ]
        .map(|v| v.parse::<UnityVersion>().unwrap());

        for a in &versions {
            let tpk_version = TpkVersion::from(a);
            assert_eq!(&tpk_version.unity_version().unwrap(), a);
            assert_eq!(format!("{tpk_version:?}"), format!("{a:?}"));
            for b in &versions {
                let order = tpk_version.partial_cmp(&TpkVersion::from(b));
                assert_eq!(order, a.partial_cmp(b), "{a} {b}");
            }
        }

        let unknown = TpkVersion(version(2, 0, 0, 6, 1));
        assert!(unknown.unity_version().is_err());
        assert_eq!(format!("{unknown:?}"), "TpkVersion(562949953422849)");
    }
}
