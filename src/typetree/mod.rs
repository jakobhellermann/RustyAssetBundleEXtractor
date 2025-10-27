//! The serialization format used by unity.

mod provider;

/// Caching implementations of [`TypeTreeProvider`]
pub mod typetree_cache;

pub use provider::TypeTreeProvider;

use crate::commonstring::COMMONSTRING;
use crate::read_ext::ReadUrexExt;
use crate::write_ext::WriteExt;
use bitflags::bitflags;
use byteorder::{ByteOrder, ReadBytesExt, WriteBytesExt};
use std::collections::HashMap;
use std::io::{Read, Seek, Write};

bitflags! {
    struct TransferMetaFlags: i32 {
        const NO_TRANSFER_FLAGS = 0;
        /// Putting this mask in a transfer will make the variable be hidden in the property editor
        const HIDE_IN_EDITOR_MASK = 1 << 0;

        /// Makes a variable not editable in the property editor
        const NOT_EDITABLE_MASK = 1 << 4;

        /// There are 3 types of PPtrs: kStrongPPtrMask, default (weak pointer)
        /// a Strong PPtr forces the referenced object to be cloned.
        /// A Weak PPtr doesnt clone the referenced object, but if the referenced object is being cloned anyway (eg. If another (strong) pptr references this object)
        /// this PPtr will be remapped to the cloned object
        /// If an  object  referenced by a WeakPPtr is not cloned, it will stay the same when duplicating and cloning, but be NULLed when templating
        const STRONG_PPTR_MASK = 1 << 6;
        // unused  = 1 << 7,

        /// kEditorDisplaysCheckBoxMask makes an integer variable appear as a checkbox in the editor
        const EDITOR_DISPLAYS_CHECK_BOX_MASK = 1 << 8;

        // unused = 1 << 9,
        // unused = 1 << 10,

        /// Show in simplified editor
        const SIMPLE_EDITOR_MASK = 1 << 11;

        /// When the options of a serializer tells you to serialize debug properties kSerializeDebugProperties
        /// All debug properties have to be marked kDebugPropertyMask
        /// Debug properties are shown in expert mode in the inspector but are not serialized normally
        const DEBUG_PROPERTY_MASK = 1 << 12;

        const ALIGN_BYTES_FLAG = 1 << 14;
        const ANY_CHILD_USES_ALIGN_BYTES_FLAG = 1 << 15;
        const IGNORE_WITH_INSPECTOR_UNDO_MASK = 1 << 16;

        // unused = 1 << 18,

        // Ignore this property when reading or writing .meta files
        const IGNORE_IN_META_FILES = 1 << 19;

        // When reading meta files and this property is not present, read array entry name instead (for backwards compatibility).
        const TRANSFER_AS_ARRAY_ENTRY_NAME_IN_META_FILES = 1 << 20;

        // When writing YAML Files, uses the flow mapping style (all properties in one line, with "{}").
        const TRANSFER_USING_FLOW_MAPPING_STYLE = 1 << 21;

        // Tells SerializedProperty to generate bitwise difference information for this field.
        const GENERATE_BITWISE_DIFFERENCES = 1 << 22;

        const DONT_ANIMATE = 1 << 23;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TypeTreeNode {
    pub m_Version: i32,
    pub m_Level: u8,
    pub m_TypeFlags: i32,
    pub m_ByteSize: i32,
    pub m_Index: Option<i32>,
    pub m_MetaFlag: Option<i32>,
    pub m_Type: String,
    pub m_Name: String,
    //unsigned short children_count,
    //struct TypeTreeNodeObject **children,
    // UnityFS
    // unsigned int m_TypeStrOffset,
    // unsigned int m_NameStrOffset,
    // UnityFS - version >= 19
    pub m_RefTypeHash: Option<u64>,
    // UnityRaw - versin = 2
    pub m_VariableCount: Option<i32>,
    // helper fields
    //typehash: u32,
    pub children: Vec<TypeTreeNode>,
}
impl TypeTreeNode {
    pub fn from_reader<R: std::io::Read + std::io::Seek, B: ByteOrder>(
        reader: &mut R,
        version: u32,
    ) -> Result<TypeTreeNode, std::io::Error> {
        fn read_node_base<R: std::io::Read + std::io::Seek, B: ByteOrder>(
            reader: &mut R,
            version: u32,
            level: u8,
        ) -> Result<TypeTreeNode, std::io::Error> {
            let mut node = TypeTreeNode {
                m_Level: level,
                m_Type: reader.read_cstr()?,
                m_Name: reader.read_cstr()?,
                m_ByteSize: reader.read_i32::<B>()?,
                m_VariableCount: if version == 2 {
                    Some(reader.read_i32::<B>()?)
                } else {
                    None
                },
                m_Index: if version != 3 {
                    Some(reader.read_i32::<B>()?)
                } else {
                    None
                },
                // in version 4, m_TypeFlags are m_IsArray
                m_TypeFlags: reader.read_i32::<B>()?,
                m_Version: reader.read_i32::<B>()?,
                m_MetaFlag: if version != 3 {
                    Some(reader.read_i32::<B>()?)
                } else {
                    None
                },
                m_RefTypeHash: None,
                children: Vec::new(),
            };
            let children_count = reader.read_i32::<B>()?;
            node.children = (0..children_count)
                .map(|_| read_node_base::<R, B>(reader, version, node.m_Level + 1))
                .collect::<Result<_, _>>()?;
            Ok(node)
        }
        read_node_base::<R, B>(reader, version, 0)
    }

    pub fn blob_from_reader<R: std::io::Read + std::io::Seek, B: ByteOrder>(
        reader: &mut R,
        version: u32,
    ) -> Result<TypeTreeNode, std::io::Error> {
        // originally a list with level slicing
        // reordered here to fit the newer tree structure
        let node_size = if version >= 19 { 32 } else { 24 };
        let node_count = reader.read_i32::<B>()?;
        let string_buffer_size = reader.read_i32::<B>()?;

        let mut node_reader = std::io::Cursor::new(
            reader.read_bytes_sized((node_size as usize).saturating_mul(node_count as usize))?,
        );
        let mut string_buffer_reader =
            std::io::Cursor::new(reader.read_bytes_sized(string_buffer_size as usize)?);

        fn read_string<R: std::io::Read + std::io::Seek>(
            string_buffer_reader: &mut R,
            value: u32,
        ) -> Result<String, std::io::Error> {
            // TODO - cache strings
            let isOffset = (value & 0x80000000) == 0;
            if isOffset {
                string_buffer_reader.seek(std::io::SeekFrom::Start(value as u64))?;
                return string_buffer_reader.read_cstr();
            }
            let offset = value & 0x7FFFFFFF;

            let ret = COMMONSTRING.get(&offset);

            if let Some(ret) = ret {
                Ok(ret.to_string())
            } else {
                Ok(offset.to_string())
            }
        }

        let nodes: Vec<TypeTreeNode> = (0..node_count)
            .map(|_| {
                std::io::Result::Ok(TypeTreeNode {
                    m_Version: node_reader.read_u16::<B>()? as i32,
                    m_Level: node_reader.read_u8()?,
                    m_TypeFlags: node_reader.read_u8()? as i32,
                    m_Type: read_string::<std::io::Cursor<Vec<u8>>>(
                        &mut string_buffer_reader,
                        node_reader.read_u32::<B>()?,
                    )?,
                    m_Name: read_string::<std::io::Cursor<Vec<u8>>>(
                        &mut string_buffer_reader,
                        node_reader.read_u32::<B>()?,
                    )?,
                    m_ByteSize: node_reader.read_i32::<B>()?,
                    m_Index: Some(node_reader.read_i32::<B>()?),
                    m_MetaFlag: Some(node_reader.read_i32::<B>()?),
                    m_RefTypeHash: if version >= 19 {
                        Some(node_reader.read_u64::<B>()?)
                    } else {
                        None
                    },
                    children: Vec::new(),
                    m_VariableCount: None,
                })
            })
            .collect::<Result<_, _>>()?;

        fn add_children(parent: &mut TypeTreeNode, nodes: &[TypeTreeNode], offset: usize) -> i32 {
            let mut added: i32 = 0;
            for i in (offset + 1)..nodes.len() {
                let mut node = nodes[i].clone();
                if node.m_Level == parent.m_Level + 1 {
                    added += add_children(&mut node, nodes, i) + 1;
                    parent.children.push(node.clone());
                } else if node.m_Level <= parent.m_Level {
                    break;
                }
            }
            added
        }

        let mut root_node = nodes
            .first()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "typetree"))?
            .clone();
        let added = add_children(&mut root_node, &nodes, 0);
        if added != node_count - 1 {
            println!("Warning: not all nodes were added to the tree");
        }
        Ok(root_node)
    }

    pub fn write_blob<W: Write, B: ByteOrder>(
        &self,
        mut writer: W,
        version: u32,
        offset_map: &HashMap<&str, u32>,
    ) -> Result<(), std::io::Error> {
        let mut count = 0;

        let mut node_out = Vec::new();
        let mut string_out = Vec::new();

        let mut cache: HashMap<&str, u32> = HashMap::new();
        fn write_string<'a>(
            cache: &mut HashMap<&'a str, u32>,
            string_out: &mut Vec<u8>,
            str: &'a str,
            common_offset_map: &HashMap<&str, u32>,
        ) -> u32 {
            *cache
                .entry(str)
                .or_insert_with(|| match common_offset_map.get(str) {
                    Some(common_offset) => *common_offset | 0x80000000,
                    None => {
                        let offset = string_out.len();
                        let _ = string_out.write_cstr(str);
                        offset as u32
                    }
                })
        }

        fn write_node<'a, B: ByteOrder>(
            version: u32,
            node_out: &mut Vec<u8>,
            cache: &mut HashMap<&'a str, u32>,
            string_out: &mut Vec<u8>,
            common_offset_map: &HashMap<&str, u32>,
            node: &'a TypeTreeNode,
        ) -> Result<(), std::io::Error> {
            node_out.write_i16::<B>(node.m_Version as i16)?;
            node_out.write_u8(node.m_Level)?;
            node_out.write_u8(node.m_TypeFlags as u8)?;
            node_out.write_u32::<B>(write_string(
                cache,
                string_out,
                &node.m_Type,
                common_offset_map,
            ))?;
            node_out.write_u32::<B>(write_string(
                cache,
                string_out,
                &node.m_Name,
                common_offset_map,
            ))?;
            node_out.write_i32::<B>(node.m_ByteSize)?;
            node_out.write_i32::<B>(node.m_Index.unwrap())?;
            node_out.write_i32::<B>(node.m_MetaFlag.unwrap())?;
            if version >= 19 {
                node_out.write_u64::<B>(node.m_RefTypeHash.unwrap_or(0))?;
            }

            Ok(())
        }

        let mut stack = vec![self];
        while let Some(node) = stack.pop() {
            count += 1;

            write_node::<B>(
                version,
                &mut node_out,
                &mut cache,
                &mut string_out,
                offset_map,
                node,
            )?;

            stack.extend(node.children.iter().rev());
        }

        writer.write_i32::<B>(count)?;
        writer.write_i32::<B>(string_out.len() as i32)?;
        writer.write_all(&node_out)?;
        writer.write_all(&string_out)?;

        Ok(())
    }

    pub fn requires_align(&self) -> bool {
        (self.m_MetaFlag.unwrap_or(0) & TransferMetaFlags::ALIGN_BYTES_FLAG.bits()) != 0
    }

    pub fn read<'de, T: serde::Deserialize<'de>, R: Read + Seek, B: ByteOrder>(
        &self,
        reader: &mut R,
    ) -> Result<T, crate::serde_typetree::Error> {
        crate::serde_typetree::from_reader::<_, B>(reader, self)
    }

    pub fn dump(&self) -> String {
        use std::fmt::Write;
        pub fn dump_inner(tt: &TypeTreeNode, out: &mut String, indent: usize) {
            for _ in 0..indent {
                out.push_str("  ");
            }
            let _ = writeln!(out, "{} {}", tt.m_Type, tt.m_Name);

            for child in &tt.children {
                dump_inner(child, out, indent + 1);
            }
        }

        let mut out = String::new();
        dump_inner(self, &mut out, 0);
        out
    }

    pub fn dump_pretty(&self) -> String {
        use std::fmt::Write;
        pub fn dump_inner(tt: &TypeTreeNode, out: &mut String, indent: usize) {
            for _ in 0..indent {
                out.push_str("  ");
            }

            if let [child] = tt.children.as_slice()
                && child.m_Type == "Array"
                && let [_, data] = child.children.as_slice()
            {
                let _ = writeln!(
                    out,
                    "{}<{}> {}",
                    tt.m_Type.trim_end_matches("`1"),
                    data.m_Type,
                    tt.m_Name
                );
                if !data.children.is_empty() {
                    dump_inner(data, out, indent + 1);
                }
                return;
            } else {
                let _ = writeln!(out, "{} {}", tt.m_Type, tt.m_Name);
            }

            if ["string"].contains(&tt.m_Type.as_str()) || tt.m_Type.starts_with("PPtr<") {
                return;
            }

            for child in &tt.children {
                dump_inner(child, out, indent + 1);
            }
        }

        let mut out = String::new();
        dump_inner(self, &mut out, 0);
        out
    }
}
