use std::io::{Cursor, Read, Seek};
use std::marker::PhantomData;

use super::{Error, Result};
use byteorder::{ByteOrder, ReadBytesExt};
use rustc_hash::FxHashSet;
use serde::de::Error as _;
use serde::de::{DeserializeSeed, IgnoredAny, IntoDeserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer as _};

use crate::files::serializedfile::Endianness;
use crate::read_ext::{ReadSeekUrexExt, ReadUrexExt};
use crate::serde_typetree::error::ErrorImpl;
use crate::typetree::TypeTreeNode;
use crate::typetree::TypetreeNodeKind as Kind;

/// A structure that deserializes typetree data into Rust values.
pub struct Deserializer<'cx, R, B> {
    typetree: &'cx TypeTreeNode,
    reader: &'cx mut R,
    marker: PhantomData<B>,
}

/// Deserialize an instance of type `T` from a a slice.
pub fn from_slice<'de, T: Deserialize<'de>, B: ByteOrder>(
    slice: &'de [u8],
    typetree: &'de TypeTreeNode,
) -> Result<T> {
    let mut reader = Cursor::new(slice);
    T::deserialize(&mut Deserializer::<_, B>::from_reader(
        &mut reader,
        typetree,
    ))
}

/// Deserialize an instance of type `T` from an I/O stream.
pub fn from_reader<'de, T: Deserialize<'de>, B: ByteOrder>(
    reader: &mut (impl Read + Seek),
    typetree: &TypeTreeNode,
) -> Result<T> {
    let deserializer = &mut Deserializer::<_, B>::from_reader(reader, typetree);
    #[cfg(feature = "serde_path_to_error")]
    return serde_path_to_error::deserialize::<_, T>(deserializer)
        .map_err(|e| Error::custom(format!("{}", e)));
    #[cfg(not(feature = "serde_path_to_error"))]
    T::deserialize(deserializer)
}

/// Deserialize an instance of type `T` from an I/O stream, with the endianness supplied at runtime.
pub fn from_reader_endianed<'de, T: Deserialize<'de>>(
    reader: &mut (impl Read + Seek),
    typetree: &TypeTreeNode,
    endianness: Endianness,
) -> Result<T> {
    match endianness {
        Endianness::Little => from_reader::<T, byteorder::LE>(reader, typetree),
        Endianness::Big => from_reader::<T, byteorder::BE>(reader, typetree),
    }
}

impl<'a, R: Read + Seek, B: ByteOrder> Deserializer<'a, R, B> {
    pub fn from_reader(reader: &'a mut R, typetree: &'a TypeTreeNode) -> Self {
        Deserializer {
            typetree,
            reader,
            marker: PhantomData,
        }
    }
}

macro_rules! deserialize_by {
    ($name:ident, $pat:pat, $read:path, $visit:ident) => {
        fn $name<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: serde::de::Visitor<'de>,
        {
            let $pat = self.typetree.m_Type.as_str() else {
                return Err(Error::invalid_typetree_type(
                    &self.typetree,
                    &stringify!($name),
                ));
            };
            let value = $read(&mut self.reader)?;
            if self.typetree.requires_align() {
                self.reader.align4()?;
            }

            visitor.$visit(value)
        }
    };
}
macro_rules! deserialize_unsupported {
    ($name:ident, $expected:literal) => {
        fn $name<V>(self, _: V) -> Result<V::Value, Self::Error>
        where
            V: serde::de::Visitor<'de>,
        {
            Err(Error::invalid_typetree_type(&self.typetree, &$expected))
        }
    };
}

impl<'de, R: Read + Seek, B: ByteOrder> serde::Deserializer<'de> for &mut Deserializer<'_, R, B> {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        /*println!(
            "{} {} {}",
            self.reader.stream_position()?,
            self.typetree.m_Type,
            self.typetree.m_Name
        );*/

        match self.typetree.classify() {
            Kind::Bool => self.deserialize_bool(visitor),
            Kind::U8 => self.deserialize_u8(visitor),
            Kind::U16 => self.deserialize_u16(visitor),
            Kind::U32 => self.deserialize_u32(visitor),
            Kind::U64 => self.deserialize_u64(visitor),
            Kind::I8 => self.deserialize_i8(visitor),
            Kind::I16 => self.deserialize_i16(visitor),
            Kind::I32 => self.deserialize_i32(visitor),
            Kind::I64 => self.deserialize_i64(visitor),
            Kind::Float => self.deserialize_f32(visitor),
            Kind::Double => self.deserialize_f64(visitor),
            Kind::Char => self.deserialize_char(visitor),
            Kind::String => self.deserialize_string(visitor),
            Kind::Map => self.deserialize_map(visitor),
            Kind::Untyped => visitor.visit_seq(ByteSeqDeserializer {
                data: self.reader.read_bytes::<B>()?.into_iter(),
            }),
            Kind::Empty => visitor.visit_unit(),
            Kind::ArrayWrapper | Kind::Array => self.deserialize_seq(visitor),
            Kind::Struct => {
                let result = visitor.visit_map(StructDeserializer::new(self));
                if self.typetree.requires_align() {
                    self.reader.align4()?;
                }
                result
            }
            Kind::TodoReferenced => Err(Error::custom(format!(
                "unimplemented: {}",
                self.typetree.m_Type
            ))),
            Kind::TodoType => self.deserialize_unit(visitor),
        }
    }

    deserialize_by!(deserialize_bool, "bool", ReadUrexExt::read_bool, visit_bool);

    deserialize_by!(deserialize_u8, "UInt8", ReadBytesExt::read_u8, visit_u8);
    deserialize_by!(
        deserialize_u16,
        ("UInt16" | "unsigned short"),
        ReadBytesExt::read_u16::<B>,
        visit_u16
    );
    deserialize_by!(
        deserialize_u32,
        ("UInt32" | "unsigned int" | "Type*"),
        ReadBytesExt::read_u32<B>,
        visit_u32
    );
    deserialize_by!(
        deserialize_u64,
        ("UInt64" | "unsigned long long" | "FileSize"),
        ReadBytesExt::read_u64<B>,
        visit_u64
    );

    deserialize_by!(deserialize_i8, "SInt8", ReadBytesExt::read_i8, visit_i8);
    deserialize_by!(
        deserialize_i16,
        ("SInt16" | "short"),
        ReadBytesExt::read_i16::<B>,
        visit_i16
    );
    deserialize_by!(
        deserialize_i32,
        ("SInt32" | "int"),
        ReadBytesExt::read_i32<B>,
        visit_i32
    );
    deserialize_by!(
        deserialize_i64,
        ("SInt64" | "long long"),
        ReadBytesExt::read_i64<B>,
        visit_i64
    );

    deserialize_by!(
        deserialize_f32,
        "float",
        ReadBytesExt::read_f32<B>,
        visit_f32
    );
    deserialize_by!(
        deserialize_f64,
        "double",
        ReadBytesExt::read_f64<B>,
        visit_f64
    );

    deserialize_by!(deserialize_char, "char", ReadUrexExt::read_char, visit_char);

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        self.deserialize_string(visitor)
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        ensure_type(self.typetree, "string")?;
        let len = self.reader.read_array_len::<B>()?;
        let data = self.reader.read_bytes_sized(len)?;
        self.reader.align4()?;
        match String::from_utf8(data) {
            Ok(s) => visitor.visit_string(s),
            Err(e) => visitor.visit_byte_buf(e.into_bytes()),
        }
    }

    deserialize_unsupported!(deserialize_bytes, "bytes");
    deserialize_unsupported!(deserialize_byte_buf, "byte buffer");

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_some(&mut *self)
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V>(self, _: &'static str, _: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        Err(Error::invalid_type(
            serde::de::Unexpected::Other(&self.typetree.m_Type),
            &"unit struct",
        ))
    }

    fn deserialize_newtype_struct<V>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        // vector m_Component
        //   Array Array
        //     int size
        //     ComponentPair data

        if self.typetree.children.len() != 1 || self.typetree.children[0].m_Type != "Array" {
            return Err(Error::invalid_type(
                serde::de::Unexpected::Other(&self.typetree.m_Type),
                &"a list",
            ));
        };
        let array = &self.typetree.children[0];

        let length = self.reader.read_array_len::<B>()?;
        let item_type = &array.children[1];

        let mut count = length;
        let sequence = visitor.visit_seq(SeqDeserializer {
            item_de: &mut Deserializer {
                typetree: item_type,
                reader: self.reader.by_ref(),
                marker: self.marker,
            },
            count: &mut count,
        });
        if count != 0 {
            return Err(Error::custom(format_args!(
                "Deserialize implementation did not exhaust sequence: length is {length} but only read {} size",
                length - count,
                //self.typetree.dump_pretty()
            )));
        }

        if self.typetree.requires_align() || array.requires_align() {
            self.reader.align4()?;
        }

        sequence
    }

    fn deserialize_tuple<V>(self, _: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let result = visitor.visit_seq(StructDeserializer::new(self));
        if self.typetree.requires_align() {
            self.reader.align4()?;
        }
        result
    }

    fn deserialize_tuple_struct<V>(
        self,
        _: &'static str,
        _: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let result = visitor.visit_seq(StructDeserializer::new(self));
        if self.typetree.requires_align() {
            self.reader.align4()?;
        }
        result
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        // map m_Container
        //  Array Array
        //   int size
        //   pair data
        //     TYPE first
        //     TYPE second
        if self.typetree.m_Type != "map" {
            return Err(Error::invalid_type(
                serde::de::Unexpected::Other(&self.typetree.m_Type),
                &"a map",
            ));
        };

        let size = self.reader.read_array_len::<B>()?;
        let pair = &self.typetree.children[0].children[1];
        let key_type = &pair.children[0];
        let value_type = &pair.children[1];

        let map = visitor.visit_map(MapDeserializer {
            reader: self.reader.by_ref(),
            key_type,
            value_type,
            count: size,
            marker: self.marker,
        });

        if self.typetree.requires_align() || pair.requires_align() {
            self.reader.align4()?;
        }

        map
    }

    fn deserialize_struct<V>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let result = visitor.visit_map(FieldStructDeserializer::new(self, fields)?);
        if self.typetree.requires_align() {
            self.reader.align4()?;
        }
        result
    }

    fn deserialize_enum<V>(
        self,
        name: &'static str,
        _: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        Err(Error::new(ErrorImpl::Enum(name)))
    }

    deserialize_unsupported!(deserialize_identifier, "identifier");

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }
}

struct SeqDeserializer<'cx, R, B> {
    item_de: &'cx mut Deserializer<'cx, R, B>,
    count: &'cx mut usize,
}

impl<'de, R: Read + Seek, B: ByteOrder> SeqAccess<'de> for SeqDeserializer<'_, R, B> {
    type Error = Error;

    fn next_element_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        if *self.count == 0 {
            return Ok(None);
        }
        *self.count -= 1;

        seed.deserialize(&mut *self.item_de).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(*self.count)
    }
}

struct FieldStructDeserializer<'cx, R, B> {
    typetree: &'cx TypeTreeNode,
    reader: &'cx mut R,
    next_index: usize,
    field_indices: FxHashSet<usize>,
    deserialized_field_count: usize,
    marker: PhantomData<B>,
}
impl<'cx, R, B> FieldStructDeserializer<'cx, R, B> {
    fn new(de: &'cx mut Deserializer<'_, R, B>, fields: &'static [&'static str]) -> Result<Self> {
        let children = de.typetree.children.as_slice();
        let field_indices = fields
            .iter()
            .filter_map(|&field| children.iter().position(|child| child.m_Name == field))
            .collect();

        Ok(FieldStructDeserializer {
            typetree: de.typetree,
            reader: de.reader,
            next_index: 0,
            field_indices,
            deserialized_field_count: 0,
            marker: de.marker,
        })
    }
}

impl<'de, R: Read + Seek, B: ByteOrder> MapAccess<'de> for FieldStructDeserializer<'_, R, B> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        while self.deserialized_field_count < self.field_indices.len() {
            let index = self.next_index;
            self.next_index += 1;

            let Some(child) = self.typetree.children.get(index) else {
                return Ok(None);
            };

            if self.field_indices.contains(&index) {
                self.deserialized_field_count += 1;
                return seed
                    .deserialize(FixedKeyDeserializer { key: &child.m_Name })
                    .map(Some);
            } else {
                Deserializer {
                    typetree: child,
                    reader: self.reader.by_ref(),
                    marker: self.marker,
                }
                .deserialize_ignored_any(IgnoredAny)?;
                continue;
            }
        }

        Ok(None)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let child = &self.typetree.children[self.next_index - 1];

        seed.deserialize(&mut Deserializer {
            typetree: child,
            reader: self.reader.by_ref(),
            marker: self.marker,
        })
    }
}

/// Requires fields specified in order without gaps (possibly with missing ones at the end)
struct StructDeserializer<'cx, R, B> {
    typetree: &'cx TypeTreeNode,
    reader: &'cx mut R,
    next_index: usize,
    marker: PhantomData<B>,
}
impl<'cx, R, B> StructDeserializer<'cx, R, B> {
    fn new(de: &'cx mut Deserializer<'_, R, B>) -> Self {
        StructDeserializer {
            typetree: de.typetree,
            reader: de.reader,
            next_index: 0,
            marker: de.marker,
        }
    }
}

impl<'de, R: Read + Seek, B: ByteOrder> MapAccess<'de> for StructDeserializer<'_, R, B> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        if self.next_index >= self.typetree.children.len() {
            return Ok(None);
        }

        self.next_index += 1;
        let child = &self.typetree.children[self.next_index - 1];

        seed.deserialize(FixedKeyDeserializer { key: &child.m_Name })
            .map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let child = &self.typetree.children[self.next_index - 1];

        seed.deserialize(&mut Deserializer {
            typetree: child,
            reader: self.reader.by_ref(),
            marker: self.marker,
        })
    }
}
impl<'de, R: Read + Seek, B: ByteOrder> SeqAccess<'de> for StructDeserializer<'_, R, B> {
    type Error = Error;

    fn next_element_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        if self.next_index >= self.typetree.children.len() {
            return Ok(None);
        }

        self.next_index += 1;
        let child = &self.typetree.children[self.next_index - 1];

        seed.deserialize(&mut Deserializer {
            typetree: child,
            reader: self.reader.by_ref(),
            marker: self.marker,
        })
        .map(Some)
    }
}

struct MapDeserializer<'cx, R, B> {
    reader: &'cx mut R,
    key_type: &'cx TypeTreeNode,
    value_type: &'cx TypeTreeNode,
    count: usize,
    marker: PhantomData<B>,
}

impl<'de, R: Read + Seek, B: ByteOrder> MapAccess<'de> for MapDeserializer<'_, R, B> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        if self.count == 0 {
            return Ok(None);
        }
        self.count -= 1;

        /*return seed
        .deserialize(&mut Deserializer {
            typetree: self.key_type,
            reader: self.reader.by_ref(),
            marker: self.marker,
        })
        .map(Some);*/

        seed.deserialize(&mut LenientMapKeyDeserializer {
            typetree: self.key_type,
            reader: self.reader.by_ref(),
            marker: self.marker,
        })
        .map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        seed.deserialize(&mut Deserializer {
            typetree: self.value_type,
            reader: self.reader.by_ref(),
            marker: self.marker,
        })
    }
}

struct FixedKeyDeserializer<'a> {
    key: &'a str,
}

impl<'de> serde::Deserializer<'de> for FixedKeyDeserializer<'_> {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_str(self.key)
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_str(self.key)
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_str(self.key)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum ignored_any
    }
}

struct ByteSeqDeserializer {
    data: std::vec::IntoIter<u8>,
}
impl<'de> serde::de::SeqAccess<'de> for ByteSeqDeserializer {
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        match self.data.next() {
            Some(value) => seed.deserialize(value.into_deserializer()).map(Some),
            None => Ok(None),
        }
    }
}

fn ensure_type(tt: &TypeTreeNode, expected: &'static str) -> Result<(), Error> {
    if tt.m_Type != expected {
        return Err(Error::invalid_typetree_type(tt, expected));
    };

    Ok(())
}

/// A structure that deserializes typetree data into Rust values.
pub struct LenientMapKeyDeserializer<'cx, R, B> {
    typetree: &'cx TypeTreeNode,
    reader: &'cx mut R,
    marker: PhantomData<B>,
}

impl<'cx, R, B> LenientMapKeyDeserializer<'cx, R, B> {
    fn as_deserializer(&mut self) -> Deserializer<'_, R, B> {
        Deserializer {
            typetree: self.typetree,
            reader: self.reader,
            marker: self.marker,
        }
    }
}

impl<'de, R: Read + Seek, B: ByteOrder> serde::Deserializer<'de>
    for &mut LenientMapKeyDeserializer<'_, R, B>
{
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.as_deserializer().deserialize_any(visitor)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.typetree.classify() {
            Kind::U32 => {
                let value = ReadBytesExt::read_u32::<B>(&mut self.reader)?;
                if self.typetree.requires_align() {
                    self.reader.align4()?;
                }

                visitor.visit_string(value.to_string())
            }
            Kind::String => self.as_deserializer().deserialize_string(visitor),
            _ => Err(Error::invalid_typetree_type(
                self.typetree,
                "a string map key",
            )),
        }
    }

    fn deserialize_string<V>(self, visitor: V) -> std::result::Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Self::deserialize_str(self, visitor)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char bytes byte_buf identifier
        option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum ignored_any
    }
}
