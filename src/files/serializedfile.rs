use std::borrow::Cow;
use std::io::{Read, Seek};

use super::UnityFile;
use crate::objects::{ClassId, ClassIdType, PPtr};
use crate::serde_typetree;
use crate::tpk::TpkTypeTreeBlob;
use crate::typetree::TypeTreeNode;
use crate::unity_version::UnityVersion;
use crate::{
    config::ExtractionConfig,
    read_ext::{ReadSeekUrexExt, ReadUrexExt},
};
use bitflags::bitflags;
use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt};

use num_enum::TryFromPrimitive;
use rustc_hash::FxHashMap;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, TryFromPrimitive, PartialEq, Eq)]
#[repr(u8)]
pub enum Endianness {
    Little = 0,
    Big = 1,
}

#[derive(Debug, Copy, Clone)]
pub struct SerializedFileHeader {
    m_MetadataSize: u32,
    m_FileSize: i64,
    m_Version: u32,
    m_DataOffset: i64,
    m_Endianess: Endianness,
    m_Reserved: [u8; 3],
    unknown: i64,
}
impl SerializedFileHeader {
    fn from_reader<T: std::io::Read + std::io::Seek>(
        reader: &mut T,
    ) -> Result<SerializedFileHeader, std::io::Error> {
        let mut header = SerializedFileHeader {
            m_MetadataSize: reader.read_u32::<BigEndian>()?,
            m_FileSize: reader.read_u32::<BigEndian>()? as i64,
            m_Version: reader.read_u32::<BigEndian>()?,
            m_DataOffset: reader.read_u32::<BigEndian>()? as i64,
            m_Endianess: Endianness::Little,
            m_Reserved: [0, 0, 0],
            unknown: 0,
        };

        let endianness = if header.m_Version >= 9 {
            let endianness = reader.read_u8()?;
            header.m_Reserved = reader.read_bytes_array::<3>()?;
            endianness
        } else {
            reader.seek(std::io::SeekFrom::Current(
                header.m_FileSize - header.m_MetadataSize as i64,
            ))?;
            reader.read_u8()?
        };
        header.m_Endianess = endianness.try_into().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid Endianness")
        })?;

        if header.m_Version >= 22 {
            header.m_MetadataSize = reader.read_u32::<BigEndian>()?;
            header.m_FileSize = reader.read_i64::<BigEndian>()?;
            header.m_DataOffset = reader.read_i64::<BigEndian>()?;
            header.unknown = reader.read_i64::<BigEndian>()?; // unknown
        };
        Ok(header)
    }
}

#[derive(Debug, Clone)]
pub struct SerializedType {
    pub m_ClassID: ClassId,
    pub m_IsStrippedType: bool,
    pub m_ScriptTypeIndex: i16,
    pub m_ScriptID: [u8; 16],
    pub m_OldTypeHash: [u8; 16],
    pub m_Type: Option<TypeTreeNode>,
    // for reftypes
    pub m_ClassName: Option<String>,
    pub m_NameSpace: Option<String>,
    pub m_AsmName: Option<String>,
    // for non ref-types
    pub m_TypeDependencies: Vec<i32>,
}
impl SerializedType {
    pub fn from_reader<T: std::io::Read + std::io::Seek, B: ByteOrder>(
        reader: &mut T,
        header: &SerializedFileHeader,
        m_EnableTypeTree: bool,
        isRefType: bool,
    ) -> Result<SerializedType, std::io::Error> {
        let mut typ = SerializedType {
            m_ClassID: ClassId::UnknownType,
            m_IsStrippedType: false,
            m_ScriptTypeIndex: -1,
            m_ScriptID: [0; 16],
            m_OldTypeHash: [0; 16],
            m_Type: None,
            m_ClassName: None,
            m_NameSpace: None,
            m_AsmName: None,
            m_TypeDependencies: Vec::new(),
        };
        typ.m_ClassID = ClassId(reader.read_i32::<B>()?);

        if header.m_Version >= SerializedFileFormatVersion::REFACTORED_CLASS_ID.bits() {
            typ.m_IsStrippedType = reader.read_bool()?;
        }

        if header.m_Version >= SerializedFileFormatVersion::REFACTOR_TYPE_DATA.bits() {
            typ.m_ScriptTypeIndex = reader.read_i16::<B>()?;
        }

        if header.m_Version >= SerializedFileFormatVersion::HAS_TYPE_TREE_HASHES.bits() {
            if (isRefType && typ.m_ScriptTypeIndex >= 0)
                || ((header.m_Version < SerializedFileFormatVersion::REFACTORED_CLASS_ID.bits()
                    && typ.m_ClassID.0 < 0)
                    || (header.m_Version
                        >= SerializedFileFormatVersion::REFACTORED_CLASS_ID.bits()
                        && typ.m_ClassID == ClassId::MonoBehaviour))
            {
                typ.m_ScriptID = reader.read_bytes_array::<16>()?;
            }
            typ.m_OldTypeHash = reader.read_bytes_array::<16>()?;
        }

        if m_EnableTypeTree {
            if header.m_Version >= SerializedFileFormatVersion::UNKNOWN_12.bits()
                || header.m_Version == SerializedFileFormatVersion::UNKNOWN_10.bits()
            {
                typ.m_Type = Some(TypeTreeNode::blob_from_reader::<T, B>(
                    reader,
                    header.m_Version,
                )?);
            } else {
                typ.m_Type = Some(TypeTreeNode::from_reader::<T, B>(reader, header.m_Version)?);
            }
            if header.m_Version >= SerializedFileFormatVersion::STORES_TYPE_DEPENDENCIES.bits() {
                if isRefType {
                    typ.m_ClassName = Some(reader.read_cstr()?);
                    typ.m_NameSpace = Some(reader.read_cstr()?);
                    typ.m_AsmName = Some(reader.read_cstr()?);
                } else {
                    typ.m_TypeDependencies = reader.read_i32_array::<B>(None)?;
                }
            }
        }

        Ok(typ)
    }
}

#[derive(Debug, Clone)]
pub struct LocalSerializedObjectIdentifier {
    m_LocalSerializedFileIndex: i32,
    m_LocalIdentifierInFile: i64,
}

impl LocalSerializedObjectIdentifier {
    pub fn from_reader<T: std::io::Read + std::io::Seek, B: ByteOrder>(
        reader: &mut T,
        header: &SerializedFileHeader,
    ) -> Result<LocalSerializedObjectIdentifier, std::io::Error> {
        Ok(LocalSerializedObjectIdentifier {
            m_LocalSerializedFileIndex: reader.read_i32::<B>()?,
            m_LocalIdentifierInFile: if header.m_Version
                < SerializedFileFormatVersion::UNKNOWN_14.bits()
            {
                reader.read_i32::<B>()? as i64
            } else {
                reader.read_i64::<B>()?
            },
        })
    }
}

#[derive(Debug, Clone)]
pub struct ObjectInfo {
    pub m_PathID: i64,
    pub m_Offset: i64,
    pub m_Size: u32,
    pub m_TypeID: i32,
    pub m_ClassID: ClassId,
    pub m_IsDestroyed: Option<u16>,
    pub m_ScriptTypeIndex: Option<i16>,
    pub m_Stripped: Option<u8>,
}
impl ObjectInfo {
    pub fn as_local_pptr(&self) -> PPtr {
        PPtr {
            m_FileID: 0,
            m_PathID: self.m_PathID,
        }
    }

    pub fn from_reader<T: std::io::Read + std::io::Seek, B: ByteOrder>(
        reader: &mut T,
        header: &SerializedFileHeader,
        bigIDEnabled: Option<i32>,
        types: &[SerializedType],
    ) -> Result<ObjectInfo, std::io::Error> {
        let mut objectInfo = ObjectInfo {
            m_PathID: 0,
            m_Offset: 0,
            m_Size: 0,
            m_TypeID: 0,
            m_ClassID: ClassId(0),
            m_IsDestroyed: None,
            m_ScriptTypeIndex: None,
            m_Stripped: None,
        };
        if bigIDEnabled.is_some_and(|enabled| enabled > 0) {
            objectInfo.m_PathID = reader.read_i64::<B>()?;
        } else if header.m_Version < 14 {
            objectInfo.m_PathID = reader.read_i32::<B>()? as i64;
        } else {
            reader.align(4)?;
            objectInfo.m_PathID = reader.read_i64::<B>()?;
        }

        if header.m_Version >= SerializedFileFormatVersion::LARGE_FILES_SUPPORT.bits() {
            objectInfo.m_Offset = reader.read_i64::<B>()?;
        } else {
            objectInfo.m_Offset = reader.read_u32::<B>()? as i64;
        }
        objectInfo.m_Offset += header.m_DataOffset;
        objectInfo.m_Size = reader.read_u32::<B>()?;
        objectInfo.m_TypeID = reader.read_i32::<B>()?;
        if header.m_Version < SerializedFileFormatVersion::REFACTORED_CLASS_ID.bits() {
            objectInfo.m_ClassID = ClassId(reader.read_u16::<B>()? as i32);
        } else {
            objectInfo.m_ClassID = types
                .get(objectInfo.m_TypeID as usize)
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "reference to undefined type",
                    )
                })?
                .m_ClassID;
        }
        if header.m_Version < SerializedFileFormatVersion::HAS_SCRIPT_TYPE_INDEX.bits() {
            objectInfo.m_IsDestroyed = Some(reader.read_u16::<B>()?);
        }
        if header.m_Version >= SerializedFileFormatVersion::HAS_SCRIPT_TYPE_INDEX.bits()
            && header.m_Version < SerializedFileFormatVersion::REFACTOR_TYPE_DATA.bits()
        {
            objectInfo.m_ScriptTypeIndex = Some(reader.read_i16::<B>()?);
            // if objectInfo.serializedType != null
            //     objectInfo.serializedType.m_ScriptTypeIndex = m_ScriptTypeIndex;
        }
        if header.m_Version == SerializedFileFormatVersion::SUPPORTS_STRIPPED_OBJECT.bits()
            || header.m_Version == SerializedFileFormatVersion::REFACTORED_CLASS_ID.bits()
        {
            objectInfo.m_Stripped = Some(reader.read_u8()?);
        }

        Ok(objectInfo)
    }
}

#[derive(Debug, Clone)]
pub struct ScriptType {
    localSerializedFileIndex: i32,
    localIdentifierInFile: i64,
}

impl ScriptType {
    pub fn from_reader<T: std::io::Read + std::io::Seek, B: ByteOrder>(
        reader: &mut T,
        header: &SerializedFileHeader,
    ) -> Result<ScriptType, std::io::Error> {
        Ok(ScriptType {
            localSerializedFileIndex: reader.read_i32::<B>()?,
            localIdentifierInFile: if header.m_Version
                < SerializedFileFormatVersion::UNKNOWN_14.bits()
            {
                reader.read_i32::<B>()? as i64
            } else {
                reader.read_i64::<B>()?
            },
        })
    }
}

#[derive(Debug, Clone)]
pub struct FileIdentifier {
    pub tempEmpty: Option<String>,
    pub guid: Option<Guid>,
    pub typeId: Option<i32>,
    pub pathName: String,
}

#[derive(Clone)]
pub struct Guid(pub [u8; 16]);

impl Guid {
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|&x| x == 0)
    }
}

impl std::fmt::Debug for Guid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Guid(")?;
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        f.write_str(")")
    }
}

impl FileIdentifier {
    pub fn from_reader<T: std::io::Read + std::io::Seek, B: ByteOrder>(
        reader: &mut T,
        header: &SerializedFileHeader,
    ) -> Result<FileIdentifier, std::io::Error> {
        Ok(FileIdentifier {
            tempEmpty: if header.m_Version >= SerializedFileFormatVersion::UNKNOWN_6.bits() {
                Some(reader.read_cstr()?)
            } else {
                None
            },
            guid: if header.m_Version >= SerializedFileFormatVersion::UNKNOWN_5.bits() {
                Some(Guid(reader.read_bytes_array::<16>()?))
            } else {
                None
            },
            typeId: if header.m_Version >= SerializedFileFormatVersion::UNKNOWN_5.bits() {
                Some(reader.read_i32::<B>()?)
            } else {
                None
            },
            pathName: reader.read_cstr()?,
        })
    }
}

#[derive(Debug)]
pub struct ObjectHandler<'a, R: std::io::Read + std::io::Seek> {
    pub info: &'a ObjectInfo,
    pub typ: Option<&'a SerializedType>,
    pub file: &'a SerializedFile,
    pub reader: &'a mut R,
}

impl<'a, R: std::io::Read + std::io::Seek> ObjectHandler<'a, R> {
    pub fn new(
        info: &'a ObjectInfo,
        typ: Option<&'a SerializedType>,
        file: &'a SerializedFile,
        reader: &'a mut R,
    ) -> Self {
        ObjectHandler {
            info,
            typ,
            file,
            reader,
        }
    }

    pub fn get_raw_data(&mut self) -> Result<Vec<u8>, std::io::Error> {
        self.reader
            .seek(std::io::SeekFrom::Start(self.info.m_Offset as u64))?;
        self.reader.read_bytes_sized(self.info.m_Size as usize)
    }

    fn get_typetree(&self) -> Option<&TypeTreeNode> {
        match self.typ {
            Some(typ) => typ.m_Type.as_ref(),
            _ => None,
        }
    }

    pub fn peak_name(&mut self) -> Result<String, std::io::Error> {
        self.reader
            .seek(std::io::SeekFrom::Start(self.info.m_Offset as u64))?;

        // todo - check against typeid
        match self.file.m_Header.m_Endianess {
            Endianness::Little => self.reader.read_string::<LittleEndian>(),
            Endianness::Big => self.reader.read_string::<BigEndian>(),
        }
    }

    pub fn parse<'de, T: serde::de::Deserialize<'de>>(
        &mut self,
    ) -> Result<T, serde_typetree::Error> {
        match self.get_typetree().cloned() {
            Some(node) => {
                self.reader
                    .seek(std::io::SeekFrom::Start(self.info.m_Offset as u64))?;
                match self.file.m_Header.m_Endianess {
                    Endianness::Little => node.read::<T, R, LittleEndian>(self.reader),
                    Endianness::Big => node.read::<T, R, BigEndian>(self.reader),
                }
            }
            _ => Err(serde_typetree::Error::custom("Couldn't find typetree")),
        }
    }

    #[cfg(feature = "formats")]
    pub fn parse_as_json(&mut self) -> Result<serde_json::Value, serde_typetree::Error> {
        self.parse::<serde_json::Value>()
    }

    #[cfg(feature = "formats")]
    pub fn parse_as_yaml(&mut self) -> Result<serde_yaml::Value, serde_typetree::Error> {
        self.parse::<serde_yaml::Value>()
    }

    /// Parses the object as msgpack
    #[cfg(feature = "formats")]
    pub fn parse_as_msgpack(&mut self) -> Result<Vec<u8>, serde_typetree::Error> {
        match self.get_typetree().cloned() {
            Some(node) => {
                self.reader
                    .seek(std::io::SeekFrom::Start(self.info.m_Offset as u64))?;
                match self.file.m_Header.m_Endianess {
                    Endianness::Little => node.read_as_msgpack::<R, LittleEndian>(self.reader),
                    Endianness::Big => node.read_as_msgpack::<R, BigEndian>(self.reader),
                }
            }
            _ => Err(serde_typetree::Error::custom("No Typetree foun")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SerializedFile {
    pub m_Header: SerializedFileHeader,
    pub m_UnityVersion: Option<UnityVersion>,
    pub m_TargetPlatform: Option<i32>,
    pub m_bigIDEnabled: Option<i32>,
    pub m_Types: Vec<SerializedType>,
    m_Objects: Vec<ObjectInfo>,
    // PERF: 20% faster in end-to-end benchmark compared to BTreeMap<i64, ObjectInfo>
    pub m_Objects_lookup: FxHashMap<i64, usize>,
    pub m_ScriptTypes: Option<Vec<ScriptType>>,
    pub m_Externals: Vec<FileIdentifier>,
    pub m_RefTypes: Option<Vec<SerializedType>>,
    pub m_UserInformation: Option<String>,
}

impl SerializedFile {
    pub fn objects(&self) -> impl ExactSizeIterator<Item = &ObjectInfo> {
        self.m_Objects.iter()
    }

    pub fn modify_objects(&mut self, f: impl FnOnce(&mut Vec<ObjectInfo>)) {
        f(&mut self.m_Objects);
        self.recompute_lookup();
    }

    pub fn take_objects(&mut self) -> Vec<ObjectInfo> {
        self.m_Objects_lookup.clear();
        std::mem::take(&mut self.m_Objects)
    }

    fn recompute_lookup(&mut self) {
        self.m_Objects_lookup = self
            .m_Objects
            .iter()
            .enumerate()
            .map(|(i, obj)| (obj.m_PathID, i))
            .collect();
    }

    pub fn get_object(&self, path_id: i64) -> Option<&ObjectInfo> {
        self.m_Objects_lookup
            .get(&path_id)
            .map(|&i| &self.m_Objects[i])
    }

    pub fn from_reader<T: std::io::Read + std::io::Seek>(
        reader: &mut T,
    ) -> Result<SerializedFile, std::io::Error> {
        let header = SerializedFileHeader::from_reader::<T>(reader)?;

        match header.m_Endianess {
            Endianness::Little => {
                SerializedFile::from_reader_endianed::<T, LittleEndian>(reader, header)
            }
            Endianness::Big => SerializedFile::from_reader_endianed::<T, BigEndian>(reader, header),
        }
    }

    fn from_reader_endianed<T, B>(
        reader: &mut T,
        header: SerializedFileHeader,
    ) -> Result<SerializedFile, std::io::Error>
    where
        T: std::io::Read + std::io::Seek,
        B: ByteOrder,
    {
        // Read Metadata
        let mut m_UnityVersion = None;
        if header.m_Version >= 9 {
            m_UnityVersion = Some(reader.read_cstr()?.parse().unwrap());
            // SetVersion(unity_version);
        }

        let mut m_TargetPlatform = None;
        if header.m_Version >= 10 {
            m_TargetPlatform = Some(reader.read_i32::<B>()?);
            // if !Enum.IsDefined(typeof(BuildTarget, m_TargetPlatform))
            // {
            //     m_TargetPlatform = BuildTarget.UnknownPlatform;
            // }
        }

        let mut m_EnabledTypeTree = false;
        if header.m_Version >= 11 {
            m_EnabledTypeTree = reader.read_bool()?;
        }

        // Read Types
        let typeCount = reader.read_i32::<B>()?;
        let m_Types: Vec<SerializedType> = (0..typeCount)
            .map(|_| SerializedType::from_reader::<T, B>(reader, &header, m_EnabledTypeTree, false))
            .collect::<Result<Vec<_>, _>>()?;

        let m_bigIDEnabled = None;
        if header.m_Version >= SerializedFileFormatVersion::UNKNOWN_7.bits()
            && header.m_Version < SerializedFileFormatVersion::UNKNOWN_14.bits()
        {
            let _bigIDEnabled = Some(reader.read_i32::<B>()?);
        }

        // Read Objects
        let objectCount = reader.read_i32::<B>()?;
        let m_Objects: Vec<ObjectInfo> = (0..objectCount)
            .map(|_| ObjectInfo::from_reader::<T, B>(reader, &header, m_bigIDEnabled, &m_Types))
            .collect::<Result<_, _>>()?;

        let m_ScriptTypes = None;
        if header.m_Version >= SerializedFileFormatVersion::HAS_SCRIPT_TYPE_INDEX.bits() {
            let scriptCount = reader.read_i32::<B>()?;
            let _m_ScriptTypes: Option<Vec<LocalSerializedObjectIdentifier>> = Some(
                (0..scriptCount)
                    .map(|_| LocalSerializedObjectIdentifier::from_reader::<T, B>(reader, &header))
                    .collect::<Result<_, _>>()?,
            );
        }

        let externalsCount = reader.read_i32::<B>()?;
        let m_Externals = (0..externalsCount)
            .map(|_| FileIdentifier::from_reader::<T, B>(reader, &header))
            .collect::<Result<_, _>>()?;

        let m_RefTypes = None;
        if header.m_Version >= SerializedFileFormatVersion::SUPPORTS_REF_OBJECT.bits() {
            let refTypesCount = reader.read_i32::<B>()?;
            let _m_RefTypes: Option<Vec<SerializedType>> = Some(
                (0..refTypesCount)
                    .map(|_| {
                        SerializedType::from_reader::<T, B>(
                            reader,
                            &header,
                            m_EnabledTypeTree,
                            true,
                        )
                    })
                    .collect::<Result<_, _>>()?,
            );
        }

        let m_UserInformation = None;
        if header.m_Version >= SerializedFileFormatVersion::UNKNOWN_5.bits() {
            let _m_UserInformation = Some(reader.read_cstr()?);
        }

        //reader.AlignStream(16);
        let mut file = SerializedFile {
            m_Header: header,
            m_UnityVersion,
            m_TargetPlatform,
            m_bigIDEnabled,
            m_Types,
            m_Objects_lookup: FxHashMap::default(),
            m_Objects,
            m_ScriptTypes,
            m_Externals,
            m_RefTypes,
            m_UserInformation,
        };
        file.recompute_lookup();
        Ok(file)
    }

    pub fn get_object_handler<'a, R: std::io::Read + std::io::Seek>(
        &'a self,
        objectinfo: &'a ObjectInfo,
        reader: &'a mut R,
    ) -> ObjectHandler<'a, R> {
        let mut typ = None;
        if self.m_Header.m_Version >= SerializedFileFormatVersion::REFACTORED_CLASS_ID.bits() {
            typ = Some(&self.m_Types[objectinfo.m_TypeID as usize]);
        }

        ObjectHandler::new(objectinfo, typ, self, reader)
    }

    pub fn read_object_of_class_id<'de, T: ClassIdType + Deserialize<'de>>(
        &self,
        tpk: impl TypeTreeProvider,
        reader: &mut (impl Read + Seek),
    ) -> Result<Option<T>, Error> {
        self.object_of_class_id(T::CLASS_ID)
            .map(|obj| self.read::<T>(obj, tpk, reader))
            .transpose()
    }

    pub fn object_of_class_id(&self, class_id: ClassId) -> Option<&ObjectInfo> {
        self.m_Objects
            .iter()
            .find(move |object| object.m_ClassID == class_id)
    }
    pub fn objects_of_class_id(&self, class_id: ClassId) -> impl Iterator<Item = &ObjectInfo> {
        self.m_Objects
            .iter()
            .filter(move |object| object.m_ClassID == class_id)
    }

    pub fn get_typetree_for<'tt>(
        &self,
        object: &ObjectInfo,
        tpk: &'tt impl TypeTreeProvider,
    ) -> Result<Cow<'tt, TypeTreeNode>, Error> {
        tpk.get_typetree_node(object.m_ClassID, self.m_UnityVersion.unwrap())
            .ok_or(Error::NoTypetree(object.m_ClassID))
    }

    pub fn read_as<'de, T: Deserialize<'de>>(
        &self,
        object: &ObjectInfo,
        typetree: &TypeTreeNode,
        reader: &mut (impl Read + Seek),
    ) -> Result<T, Error> {
        reader.seek(std::io::SeekFrom::Start(object.m_Offset as u64))?;
        match self.m_Header.m_Endianess {
            Endianness::Little => serde_typetree::from_reader::<_, byteorder::LE>(reader, typetree),
            Endianness::Big => serde_typetree::from_reader::<_, byteorder::BE>(reader, typetree),
        }
        .map_err(Error::Deserialize)
    }

    pub fn read_single<'de, T: Deserialize<'de>>(
        &self,
        class_id: ClassId,
        tpk: impl TypeTreeProvider,
        reader: &mut (impl Read + Seek),
    ) -> Result<T, Error> {
        let info = self
            .object_of_class_id(class_id)
            .expect("read_single doesn't exist");
        self.read(info, tpk, reader)
    }

    pub fn read<'de, T: Deserialize<'de>>(
        &self,
        object: &ObjectInfo,
        tpk: impl TypeTreeProvider,
        reader: &mut (impl Read + Seek),
    ) -> Result<T, Error> {
        let tt = tpk
            .get_typetree_node(object.m_ClassID, self.m_UnityVersion.unwrap())
            .ok_or(Error::NoTypetree(object.m_ClassID))?;
        reader.seek(std::io::SeekFrom::Start(object.m_Offset as u64))?;
        match self.m_Header.m_Endianess {
            Endianness::Little => serde_typetree::from_reader::<_, byteorder::LE>(reader, &tt),
            Endianness::Big => serde_typetree::from_reader::<_, byteorder::BE>(reader, &tt),
        }
        .map_err(Error::Deserialize)
    }

    pub fn read_raw(
        &self,
        object: &ObjectInfo,
        reader: &mut (impl Read + Seek),
    ) -> Result<Vec<u8>, Error> {
        reader.seek(std::io::SeekFrom::Start(object.m_Offset as u64))?;
        let bytes = reader.read_bytes_sized(object.m_Size as usize)?;
        Ok(bytes)
    }
}

pub trait TypeTreeProvider {
    fn get_typetree_node(
        &self,
        class_id: ClassId,
        target_version: UnityVersion,
    ) -> Option<Cow<'_, TypeTreeNode>>;
}

impl TypeTreeProvider for TpkTypeTreeBlob {
    fn get_typetree_node(
        &self,
        class_id: ClassId,
        target_version: UnityVersion,
    ) -> Option<Cow<'_, TypeTreeNode>> {
        TpkTypeTreeBlob::get_typetree_node(self, class_id, target_version).map(Cow::Owned)
    }
}
impl<T: TypeTreeProvider> TypeTreeProvider for &T {
    fn get_typetree_node(
        &self,
        class_id: ClassId,
        target_version: UnityVersion,
    ) -> Option<Cow<'_, TypeTreeNode>> {
        (*self).get_typetree_node(class_id, target_version)
    }
}

#[derive(Debug)]
pub enum Error {
    NoTypetree(ClassId),
    Deserialize(serde_typetree::Error),
    Serialize(serde_typetree::Error),
    IO(std::io::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoTypetree(class_id) => {
                write!(f, "No typetree found for class {class_id:?}")
            }
            Error::Deserialize(error) => write!(f, "Error trying to deserialize: {error}"),
            Error::Serialize(error) => write!(f, "Error trying to serialize: {error}"),
            Error::IO(error) => write!(f, "IO error: {error}"),
        }
    }
}
impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::IO(error)
    }
}

impl UnityFile for SerializedFile {
    fn from_reader<T: std::io::Read + std::io::Seek>(
        reader: &mut T,
        _: &ExtractionConfig,
    ) -> Result<Self, std::io::Error>
    where
        Self: Sized,
    {
        SerializedFile::from_reader(reader)
    }
}

bitflags! {
    pub struct SerializedFileFormatVersion: u32 {
        const UNSUPPORTED = 1;
        const UNKNOWN_2 = 2;
        const UNKNOWN_3 = 3;
        /// 1.2.0 to 2.0.0
        const UNKNOWN_5 = 5;
        /// 2.1.0 to 2.6.1
        const UNKNOWN_6 = 6;
        /// 3.0.0b
        const UNKNOWN_7 = 7;
        /// 3.0.0 to 3.4.2
        const UNKNOWN_8 = 8;
        /// 3.5.0 to 4.7.2
        const UNKNOWN_9 = 9;
        /// 5.0.0aunk1
        const UNKNOWN_10 = 10;
        /// 5.0.0aunk2
        const HAS_SCRIPT_TYPE_INDEX = 11;
        /// 5.0.0aunk3
        const UNKNOWN_12 = 12;
        /// 5.0.0aunk4
        const HAS_TYPE_TREE_HASHES = 13;
        /// 5.0.0unk
        const UNKNOWN_14 = 14;
        /// 5.0.1 to 5.4.0
        const SUPPORTS_STRIPPED_OBJECT = 15;
        /// 5.5.0a
        const REFACTORED_CLASS_ID = 16;
        /// 5.5.0unk to 2018.4
        const REFACTOR_TYPE_DATA = 17;
        /// 2019.1a
        const REFACTOR_SHAREABLE_TYPE_TREE_DATA = 18;
        /// 2019.1unk
        const TYPE_TREE_NODE_WITH_TYPE_FLAGS = 19;
        /// 2019.2
        const SUPPORTS_REF_OBJECT = 20;
        /// 2019.3 to 2019.4
        const STORES_TYPE_DEPENDENCIES = 21;
        /// 2020.1 to x
        const LARGE_FILES_SUPPORT = 22;
    }
}
