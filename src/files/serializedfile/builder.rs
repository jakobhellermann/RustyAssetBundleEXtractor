use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Seek, Write};

use super::Error;
use crate::files::serializedfile::{
    self, CommonOffsetMap, FileIdentifier, ObjectInfo, SerializedFile, SerializedFileHeader,
    SerializedType, TypeTreeProvider,
};
use crate::objects::pptr::{FileId, PathId};
use crate::objects::{ClassId, ClassIdType};
use crate::serde_typetree;
use crate::unity_version::UnityVersion;
use byteorder::LittleEndian;
use serde::Serialize;

use super::Endianness;

/// Builder for creating a new [`SerializedFile`]
pub struct SerializedFileBuilder<'a, P> {
    unity_version: UnityVersion,
    common_offset_map: &'a CommonOffsetMap<'a>,
    pub typetree_provider: &'a P,
    pub next_path_id: i64,
    pub objects: BTreeMap<PathId, (ObjectInfo, Cow<'a, [u8]>)>,

    pub serialized: SerializedFile,

    types_cache: HashMap<ClassId, i32>,
}

impl<'a, P: TypeTreeProvider> SerializedFileBuilder<'a, P> {
    /// Create an empty `SerializeFileBuilder`
    pub fn new(
        version: UnityVersion,
        typetree_provider: &'a P,
        common_offset_map: &'a CommonOffsetMap<'a>,
        enable_type_tree: bool,
    ) -> Self {
        Self {
            unity_version: version,
            typetree_provider,
            common_offset_map,
            next_path_id: 0,
            objects: BTreeMap::default(),
            types_cache: HashMap::default(),

            serialized: SerializedFile {
                m_Header: SerializedFileHeader {
                    m_MetadataSize: 0,
                    m_FileSize: 0,
                    m_Version: 22,
                    m_DataOffset: 0,
                    m_Endianess: Endianness::Little,
                    m_Reserved: [0, 0, 0],
                    unknown: 0,
                },
                m_UnityVersion: Some(version),
                m_TargetPlatform: Some(24), // TODO
                m_EnableTypeTree: enable_type_tree,
                m_bigIDEnabled: None,
                m_Types: Vec::new(),
                m_ScriptTypes: Some(Vec::new()),
                m_Externals: Vec::new(),
                m_RefTypes: Some(Vec::new()),
                m_UserInformation: Some("".into()),
                m_Objects: Default::default(),        // unused
                m_Objects_lookup: Default::default(), // unused
            },
        }
    }

    /// Create a `SerializeFileBuilder` containing all data from the given file.
    pub fn from_serialized(
        unity_version: UnityVersion,
        file: &SerializedFile,
        data: &'a [u8],
        typetree_provider: &'a P,
        common_offset_map: &'a HashMap<&'a str, u32>,
    ) -> Self {
        let mut builder = SerializedFileBuilder::from_serialized_meta(
            unity_version,
            file,
            typetree_provider,
            common_offset_map,
        );
        builder.serialized.m_Types = file.m_Types.clone();
        builder.serialized.m_Externals = file.m_Externals.clone();
        builder.serialized.m_ScriptTypes = file.m_ScriptTypes.clone();
        builder.serialized.m_RefTypes = file.m_RefTypes.clone();

        for obj in file.objects() {
            let data = &data[obj.m_Offset as usize..obj.m_Offset as usize + obj.m_Size as usize];
            let res = builder.add_object_inner(obj.clone(), Cow::Borrowed(data));
            res.unwrap(); // the path id must be available
        }

        builder
    }

    /// Copies unity version, and metadata like enabling type trees or the target platform.
    /// Not copied:
    /// - object data
    /// - types
    /// - external file references
    /// - script types
    /// - ref types
    pub fn from_serialized_meta(
        unity_version: UnityVersion,
        file: &SerializedFile,
        typetree_provider: &'a P,
        common_offset_map: &'a HashMap<&'a str, u32>,
    ) -> Self {
        Self {
            unity_version: unity_version,
            typetree_provider,
            common_offset_map,
            next_path_id: 0,
            objects: BTreeMap::default(),
            types_cache: HashMap::default(),

            serialized: SerializedFile {
                m_Header: SerializedFileHeader {
                    m_MetadataSize: 0,
                    m_FileSize: 0,
                    m_Version: file.m_Header.m_Version,
                    m_DataOffset: 0,
                    m_Endianess: file.m_Header.m_Endianess,
                    m_Reserved: [0, 0, 0],
                    unknown: 0,
                },
                m_UnityVersion: file.m_UnityVersion,
                m_TargetPlatform: file.m_TargetPlatform,
                m_EnableTypeTree: file.m_EnableTypeTree,
                m_bigIDEnabled: file.m_bigIDEnabled,
                m_Types: Vec::new(),
                m_Objects: Default::default(),        // unused
                m_Objects_lookup: Default::default(), // unused
                m_ScriptTypes: Some(Vec::new()),
                m_Externals: Vec::new(),
                m_RefTypes: Some(Vec::new()),
                m_UserInformation: file.m_UserInformation.clone(),
            },
        }
    }

    /// Copy `m_Externals` from the other file
    pub fn copy_externals(&mut self, file: &SerializedFile) {
        for external in &file.m_Externals {
            self.add_external_uncached(external.clone());
        }
    }

    /// Add an object to the serialized file.
    ///
    /// This will choose the next free path ID.
    pub fn add_object<T: Serialize + ClassIdType>(&mut self, object: &T) -> Result<(), Error> {
        let path_id = self.get_next_path_id();
        self.add_object_at(path_id, object)
    }

    /// Add an object to the serialized file.
    ///
    /// This will choose the next free path ID.
    pub fn add_object_at<T: Serialize + ClassIdType>(
        &mut self,
        path_id: PathId,
        object: &T,
    ) -> Result<(), Error> {
        let type_id = self.get_or_insert_type(T::CLASS_ID);
        self.add_object_with(object, path_id, T::CLASS_ID, type_id)
    }

    /// Add an object to the serialized file with the specified meta data.
    /// You need to ensure that:
    /// - all file IDs in the data match `m_Externals`
    /// - `path_id` is free
    /// - `type_id` corresponds to a `m_Types` index
    pub fn add_object_with<T: Serialize>(
        &mut self,
        object: &T,
        path_id: PathId,
        class_id: ClassId,
        type_id: i32,
    ) -> Result<(), Error> {
        let tt = self
            .typetree_provider
            .get_typetree_node(class_id, self.unity_version)
            .unwrap();

        let data =
            serde_typetree::to_vec::<_, LittleEndian>(&object, &tt).map_err(Error::Serialize)?;

        self.add_object_untyped_with(path_id, class_id, type_id, Cow::Owned(data))
    }

    /// Add an object to the file with the specified meta data.
    /// You need to ensure that:
    /// - all file IDs in the data match `m_Externals`
    /// - `path_id` is free
    /// - `type_id` corresponds to a `m_Types` index
    ///
    /// The method does not set:
    /// - `m_IsDestroyed`
    /// - `m_ScriptTypeIndex`
    /// - `m_Stripped`
    pub fn add_object_untyped_with(
        &mut self,
        path_id: PathId,
        class_id: ClassId,
        type_id: i32,
        data: Cow<'a, [u8]>,
    ) -> Result<(), Error> {
        let info = ObjectInfo {
            m_PathID: path_id,
            m_Offset: 0, // ignored
            m_Size: 0,   // ignored
            m_TypeID: type_id,
            m_ClassID: class_id,
            m_IsDestroyed: None,
            m_ScriptTypeIndex: None,
            m_Stripped: None,
        };
        self.add_object_inner(info, data)
    }

    /// - all file IDs in the data must match `m_Externals`
    /// - if `m_ScriptTypeIndex` is set, it must match `m_ScriptTypes`
    /// - `info.m_TypeID` must correspond to a `m_Types` index
    fn add_object_inner(&mut self, info: ObjectInfo, data: Cow<'a, [u8]>) -> Result<(), Error> {
        let path_id = info.m_PathID;
        let previous = self.objects.insert(path_id, (info, data));
        if let Some((previous, _)) = previous {
            return Err(Error::IO(std::io::Error::other(format!(
                "Can't add object {path_id} to SerializeFileBuilder: a {:?} already exists",
                previous.m_ClassID
            ))));
        }

        Ok(())
    }

    // TODO: reuse existing types if possible
    fn get_or_insert_type(&mut self, class_id: ClassId) -> i32 {
        *self.types_cache.entry(class_id).or_insert_with(|| {
            if class_id == ClassId::MonoBehaviour {
                todo!();
            }

            let ty = self
                .typetree_provider
                .get_typetree_node(class_id, self.unity_version)
                .unwrap();
            let serialized_type =
                SerializedType::new(class_id, ty, self.serialized.m_EnableTypeTree);

            let type_index = self.serialized.m_Types.len();
            self.serialized.m_Types.push(serialized_type);
            type_index as i32
        })
    }

    /// Add an entry to the `m_Types` of the serialized file
    pub fn add_type_uncached(&mut self, ty: SerializedType) -> i32 {
        self.serialized.add_type(ty)
    }

    /// Add an entry to the `m_Externals` of the serialized file
    pub fn add_external_uncached(&mut self, external: FileIdentifier) -> FileId {
        self.serialized.add_external(external)
    }

    pub fn get_next_path_id(&mut self) -> PathId {
        let id = self.next_path_id;
        self.next_path_id += 1;
        id
    }

    /// Serialize the file into the given writer.
    pub fn write<W: Write + Seek>(self, writer: W) -> Result<(), Error> {
        serializedfile::write_serialized_with_objects(
            writer,
            &self.serialized,
            self.common_offset_map,
            self.objects.into_values(),
        )?;

        Ok(())
    }

    /// Serialize the file.
    pub fn write_vec(self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        self.write(Cursor::new(&mut out))?;
        Ok(out)
    }
}
