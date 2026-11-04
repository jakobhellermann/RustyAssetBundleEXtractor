#![allow(unused_variables)]
use std::io::{Cursor, Seek, Write};
use std::marker::PhantomData;

use crate::files::serializedfile::Endianness;
use crate::write_ext::WriteSeekExt;
use byteorder::{ByteOrder, WriteBytesExt};
use serde::Serialize;
use serde::ser::Impossible;

use crate::serde_typetree::Error;
use crate::typetree::TypeTreeNode;

/// `v` in the node's narrower integer type; a value that does not fit is an error, not truncated.
fn narrow<T: TryFrom<V>, V: std::fmt::Display + Copy>(
    v: V,
    node: &TypeTreeNode,
) -> Result<T, Error> {
    T::try_from(v).map_err(|_| {
        Error::custom(format_args!(
            "{v} does not fit {} {}",
            node.m_Type, node.m_Name
        ))
    })
}

/// Serialize the given data to typetree bytes
pub fn to_vec<T: Serialize + ?Sized, B: ByteOrder + 'static>(
    value: &T,
    typetree: &TypeTreeNode,
) -> Result<Vec<u8>, Error> {
    let mut writer = Cursor::new(Vec::with_capacity(128));
    to_writer::<_, _, B>(&mut writer, value, typetree)?;

    Ok(writer.into_inner())
}

/// Serialize the given data to typetree bytes, with the endianness supplied at runtime
pub fn to_vec_endianed<T: Serialize + ?Sized>(
    value: &T,
    typetree: &TypeTreeNode,
    endianness: Endianness,
) -> Result<Vec<u8>, Error> {
    match endianness {
        Endianness::Little => to_vec::<T, byteorder::LE>(value, typetree),
        Endianness::Big => to_vec::<T, byteorder::BE>(value, typetree),
    }
}

/// Serialize the given data into an I/O stream
pub fn to_writer<W: Write + Seek, T: Serialize + ?Sized, B: ByteOrder + 'static>(
    writer: &mut W,
    value: &T,
    typetree: &TypeTreeNode,
) -> Result<(), Error> {
    value.serialize(&mut Serializer::<_, B>::new(writer, typetree))
}

/// A structure for serializing Rust values into typetree data
pub struct Serializer<'cx, W, B> {
    pub typetree: &'cx TypeTreeNode,
    pub writer: &'cx mut W,
    pub marker: PhantomData<B>,
    pub requires_align: bool,
}

impl<'cx, W, B> Serializer<'cx, W, B> {
    pub fn new(writer: &'cx mut W, typetree: &'cx TypeTreeNode) -> Self {
        Serializer {
            typetree,
            writer,
            marker: PhantomData,
            requires_align: typetree.requires_align(),
        }
    }
}

impl<'a, 'cx, W: Write + Seek, B: ByteOrder + 'static> serde::Serializer
    for &'a mut Serializer<'cx, W, B>
{
    type Ok = ();

    type Error = Error;

    type SerializeSeq = SerializerSeq<'a, W, B>;

    type SerializeTuple = SerializerTuple<'a, W, B>;

    type SerializeTupleStruct = Self;

    type SerializeTupleVariant = Self;

    type SerializeMap = SerializerMap<'a, W, B>;

    type SerializeStruct = SerializerStruct<'a, W, B>;

    type SerializeStructVariant = Self;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        ensure_type(self.typetree, "bool")?;
        self.writer.write_u8(v as u8)?;

        self.align_if(self.requires_align)
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        match self.typetree.m_Type.as_str() {
            "SInt8" => self.writer.write_i8(v)?,
            _ => {
                return Err(Error::custom(format_args!(
                    "invalid type: {}, expected {} {}",
                    "i8", self.typetree.m_Type, self.typetree.m_Name,
                )));
            }
        }

        self.align_if(self.requires_align)
    }

    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        match self.typetree.m_Type.as_str() {
            "SInt16" | "short" => self.writer.write_i16::<B>(v)?,
            _ => {
                return Err(Error::custom(format_args!(
                    "invalid type: {}, expected {} {}",
                    "i16", self.typetree.m_Type, self.typetree.m_Name,
                )));
            }
        }

        self.align_if(self.requires_align)
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        match self.typetree.m_Type.as_str() {
            // "UInt8" => self.writer.write_u8(v as u8)?,
            // "UInt16" | "unsigned short" => self.writer.write_u16::<B>(v as u16)?,
            // "UInt32" | "unsigned int" | "Type*" => self.writer.write_u32::<B>(v as u32)?,
            // // "SInt8" => self.writer.write_i8(v as i8)?,
            // "SInt16" | "short" => self.writer.write_i16::<B>(v as i16)?,
            "SInt32" | "int" => self.writer.write_i32::<B>(v)?,
            _ => {
                return Err(Error::custom(format_args!(
                    "invalid type: {}, expected {} {}",
                    "i32", self.typetree.m_Type, self.typetree.m_Name,
                )));
            }
        }

        self.align_if(self.requires_align)
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        match self.typetree.m_Type.as_str() {
            // "UInt8" => self.writer.write_u8(v as u8)?,
            // "UInt16" | "unsigned short" => self.writer.write_u16::<B>(v as u16)?,
            // "UInt32" | "unsigned int" | "Type*" => self.writer.write_u32::<B>(v as u32)?,
            // "UInt64" | "unsigned long long" | "FileSize" => self.writer.write_u64::<B>(v as u64)?,
            "SInt8" => self.writer.write_i8(narrow(v, self.typetree)?)?,
            "SInt16" | "short" => self.writer.write_i16::<B>(narrow(v, self.typetree)?)?,
            "SInt32" | "int" => self.writer.write_i32::<B>(narrow(v, self.typetree)?)?,
            "SInt64" | "long long" => self.writer.write_i64::<B>(v)?,
            _ => {
                return Err(Error::custom(format_args!(
                    "invalid type: {}, expected {} {}",
                    "i64", self.typetree.m_Type, self.typetree.m_Name,
                )));
            }
        }

        self.align_if(self.requires_align)
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        match self.typetree.m_Type.as_str() {
            "UInt8" => self.writer.write_u8(v)?,
            _ => {
                return Err(Error::custom(format_args!(
                    "invalid type: {}, expected {} {}",
                    "u8", self.typetree.m_Type, self.typetree.m_Name,
                )));
            }
        }

        self.align_if(self.requires_align)
    }

    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        match self.typetree.m_Type.as_str() {
            // "UInt8" => self.writer.write_u8(v as u8)?,
            "UInt16" | "unsigned short" => self.writer.write_u16::<B>(v)?,
            // "SInt8" => self.writer.write_i8(v as i8)?,
            // "SInt16" | "short" => self.writer.write_i16::<B>(v as i16)?,
            _ => {
                return Err(Error::custom(format_args!(
                    "invalid type: {}, expected {} {}",
                    "u16", self.typetree.m_Type, self.typetree.m_Name,
                )));
            }
        }

        self.align_if(self.requires_align)
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        match self.typetree.m_Type.as_str() {
            // "UInt8" => self.writer.write_u8(v as u8)?,
            // "UInt16" | "unsigned short" => self.writer.write_u16::<B>(v as u16)?,
            "UInt32" | "unsigned int" | "Type*" => self.writer.write_u32::<B>(v)?,
            // "SInt8" => self.writer.write_i8(v as i8)?,
            // "SInt16" | "short" => self.writer.write_i16::<B>(v as i16)?,
            // "SInt32" | "int" => self.writer.write_i32::<B>(v as i32)?,
            _ => {
                return Err(Error::custom(format_args!(
                    "invalid type: {}, expected {} {}",
                    "u32", self.typetree.m_Type, self.typetree.m_Name,
                )));
            }
        }

        self.align_if(self.requires_align)
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        match self.typetree.m_Type.as_str() {
            "UInt8" => self.writer.write_u8(narrow(v, self.typetree)?)?,
            "UInt16" | "unsigned short" => self.writer.write_u16::<B>(narrow(v, self.typetree)?)?,
            "UInt32" | "unsigned int" | "Type*" => {
                self.writer.write_u32::<B>(narrow(v, self.typetree)?)?
            }
            "UInt64" | "unsigned long long" | "FileSize" => self.writer.write_u64::<B>(v)?,
            "SInt8" => self.writer.write_i8(narrow(v, self.typetree)?)?,
            "SInt16" | "short" => self.writer.write_i16::<B>(narrow(v, self.typetree)?)?,
            "SInt32" | "int" => self.writer.write_i32::<B>(narrow(v, self.typetree)?)?,
            "SInt64" | "long long" => self.writer.write_i64::<B>(narrow(v, self.typetree)?)?,
            _ => {
                return Err(Error::custom(format_args!(
                    "invalid type: {}, expected {} {}",
                    "u64", self.typetree.m_Type, self.typetree.m_Name,
                )));
            }
        }

        self.align_if(self.requires_align)
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        match self.typetree.m_Type.as_str() {
            "float" => self.writer.write_f32::<B>(v)?,
            _ => {
                return Err(Error::custom(format_args!(
                    "invalid type: {}, expected {} {}",
                    "float", self.typetree.m_Type, self.typetree.m_Name,
                )));
            }
        }

        self.align_if(self.requires_align)
    }

    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        match self.typetree.m_Type.as_str() {
            "float" => self.serialize_f32(v as f32)?,
            "double" => self.writer.write_f64::<B>(v)?,
            _ => {
                return Err(Error::custom(format_args!(
                    "invalid type: {}, expected {} {}",
                    "double", self.typetree.m_Type, self.typetree.m_Name,
                )));
            }
        }

        self.align_if(self.requires_align)
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        match u8::try_from(v) {
            Ok(value) => {
                self.writer.write_u8(value)?;
                self.align_if(self.requires_align)
            }
            Err(e) => Err(Error::custom(format!(
                "Cannot serialize char '{}' outside of 0-255",
                v
            ))),
        }
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        ensure_type(self.typetree, "string")?;
        self.writer.write_i32::<B>(v.len() as i32)?;
        self.writer.write_all(v.as_bytes())?;
        self.align_if(true)
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        let Ok(len) = u32::try_from(v.len()) else {
            return Err(Error::custom(format!(
                "Byte sequence of length {} exceeds u32::MAX",
                v.len()
            )));
        };

        self.writer.write_u32::<B>(len)?;

        self.writer.write_all(v)?;
        Ok(())
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Err(Error::custom(format_args!(
            "Attempting to serialize None for {} {}, which exists in the type tree",
            self.typetree.m_Type, self.typetree.m_Name
        )))
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        match self.typetree.m_Type.as_str() {
            "float" => Err(Error::custom(
                "Trying to serialize unit as float. This can happen with e.g. serde_json and infinity/NaN",
            )),
            "Type" => Ok(()),
            _ if self.typetree.children.is_empty() => Ok(()),
            _ => ensure_type(self.typetree, "unit"),
        }
    }

    fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok, Self::Error> {
        todo!("{}", self.typetree.dump())
    }

    fn serialize_unit_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        todo!("{}", self.typetree.dump())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        todo!("{}", self.typetree.dump())
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        todo!("{}", self.typetree.dump())
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        let len = len.expect("todo");

        if self.typetree.m_Type == "TypelessData" {
            self.writer.write_i32::<B>(len as i32)?;
            let byte_type = &self.typetree.children[1];
            assert!(!byte_type.requires_align());
            return Ok(SerializerSeq {
                item_type: byte_type,
                item_type_align: false,
                writer: self.writer,
                marker: self.marker,
                seq_align: self.typetree.requires_align(),
            });
        }

        if self.typetree.children.len() != 1 || self.typetree.children[0].m_Type != "Array" {
            return Err(Error::invalid_typetree_type(self.typetree, "a list"));
        };

        let array_type = &self.typetree.children[0];
        let item_type = &array_type.children[1];
        let requires_align = self.typetree.requires_align() || array_type.requires_align();

        self.writer.write_i32::<B>(len as i32)?;
        Ok(SerializerSeq {
            item_type: &array_type.children[1],
            item_type_align: item_type.requires_align(),
            writer: self.writer,
            marker: self.marker,
            seq_align: requires_align,
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(SerializerTuple {
            typetree: self.typetree,
            writer: self.writer,
            requires_align: self.requires_align,
            marker: self.marker,

            index: 0,
        })
    }

    fn serialize_tuple_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        todo!("{}", self.typetree.dump())
    }

    fn serialize_tuple_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        todo!("{}", self.typetree.dump())
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        match self.typetree.m_Type.as_str() {
            "vector" => todo!(),
            "array" => todo!(),
            "ReferencedObject" | "ReferencedObjectData" | "ManagedReferencesRegistry" => todo!(),
            "map" => {
                self.writer.write_i32::<B>(len.expect("TODO") as i32)?;

                let array_type = &self.typetree.children[0];
                let pair_type = &array_type.children[1];
                let key_type = &pair_type.children[0];
                let value_type = &pair_type.children[1];

                Ok(SerializerMap {
                    typetree: self.typetree,
                    writer: self.writer,
                    requires_align: self.requires_align,
                    marker: PhantomData,
                    mode: SerializerMapMode::Map {
                        key_type,
                        value_type,
                    },
                })
            }
            // "ComponentPair" => todo!("{}", self.typetree.dump()),
            "string" => todo!(),
            _ if is_primitive(self.typetree) => Err(Error::custom(format_args!(
                "invalid type: a struct of {} fields, expected {} {}",
                len.map_or("some".to_string(), |len| len.to_string()),
                self.typetree.m_Type,
                self.typetree.m_Name,
            ))),
            _ => Ok(SerializerMap {
                typetree: self.typetree,
                writer: self.writer,
                requires_align: self.requires_align,
                marker: PhantomData,
                mode: SerializerMapMode::Typed {
                    pending_key: String::new(),
                    next_index: 0,
                    out_of_order: Vec::new(),
                },
            }),
        }
    }

    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        if is_primitive(self.typetree) {
            return Err(Error::custom(format_args!(
                "invalid type: struct {name} of {len} fields, expected {} {}",
                self.typetree.m_Type, self.typetree.m_Name,
            )));
        }
        Ok(SerializerStruct {
            typetree: self.typetree,
            writer: self.writer,
            requires_align: self.requires_align,
            marker: self.marker,

            next_index: 0,
            out_of_order: Vec::new(),
        })
    }

    fn serialize_struct_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        todo!("{}", self.typetree.dump())
    }
}

fn is_primitive(node: &TypeTreeNode) -> bool {
    use crate::typetree::TypetreeNodeKind::*;
    match node.classify() {
        Bool | U8 | U16 | U32 | U64 | I8 | I16 | I32 | I64 | Float | Double | Char => true,
        String | Map | Untyped | Empty | ArrayWrapper | Array | Struct | TodoReferenced
        | TodoType => false,
    }
}

impl<W: Write + Seek, B> Serializer<'_, W, B> {
    fn align_if(&mut self, requires_align: bool) -> Result<(), Error> {
        if requires_align {
            self.writer.align::<4>()?;
        }
        Ok(())
    }
}

pub struct SerializerSeq<'cx, W, B> {
    item_type: &'cx TypeTreeNode,
    item_type_align: bool,
    writer: &'cx mut W,
    seq_align: bool,
    marker: PhantomData<B>,
}

impl<W: Write + Seek, B: ByteOrder + 'static> serde::ser::SerializeSeq for SerializerSeq<'_, W, B> {
    type Ok = ();

    type Error = Error;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut Serializer {
            typetree: self.item_type,
            writer: self.writer,
            marker: self.marker,
            requires_align: self.item_type_align,
        })
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        if self.seq_align {
            self.writer.align::<4>()?;
        }
        Ok(())
    }
}

impl<W: Write, B: ByteOrder> serde::ser::SerializeTuple for &mut Serializer<'_, W, B> {
    type Ok = ();

    type Error = Error;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        todo!("{}", self.typetree.dump())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        todo!("{}", self.typetree.dump())
    }
}
impl<W: Write, B: ByteOrder> serde::ser::SerializeTupleStruct for &mut Serializer<'_, W, B> {
    type Ok = ();

    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        todo!("{}", self.typetree.dump())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        todo!("{}", self.typetree.dump())
    }
}
impl<W: Write, B: ByteOrder> serde::ser::SerializeTupleVariant for &mut Serializer<'_, W, B> {
    type Ok = ();

    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        todo!("{}", self.typetree.dump())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        todo!("{}", self.typetree.dump())
    }
}

pub struct SerializerMap<'cx, W, B> {
    typetree: &'cx TypeTreeNode,
    writer: &'cx mut W,
    requires_align: bool,
    marker: PhantomData<B>,

    mode: SerializerMapMode<'cx>,
}

enum SerializerMapMode<'cx> {
    Typed {
        pending_key: String,
        next_index: usize,
        out_of_order: Vec<(usize, Vec<u8>)>,
    },
    Map {
        key_type: &'cx TypeTreeNode,
        value_type: &'cx TypeTreeNode,
    },
}

impl<W: Write + Seek, B> SerializerMap<'_, W, B> {
    pub fn align_if(&mut self, requires_align: bool) -> Result<(), Error> {
        if requires_align {
            self.writer.align::<4>()?;
        }
        Ok(())
    }
}

impl<'cx, W: Write + Seek, B: ByteOrder + 'static> serde::ser::SerializeMap
    for SerializerMap<'cx, W, B>
{
    type Ok = ();

    type Error = Error;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Self::Error> {
        match &mut self.mode {
            SerializerMapMode::Typed { pending_key, .. } => {
                key.serialize(StringSerializer {
                    string: pending_key,
                })?;
                Ok(())
            }
            SerializerMapMode::Map { key_type, .. } => key.serialize(&mut Serializer {
                typetree: key_type,
                writer: self.writer,
                marker: self.marker,
                requires_align: key_type.requires_align(),
            }),
        }
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        match &mut self.mode {
            SerializerMapMode::Typed {
                pending_key,
                next_index,
                out_of_order,
            } => {
                let key = std::mem::take(pending_key);
                write_struct_field::<W, B>(
                    self.typetree,
                    self.writer,
                    next_index,
                    out_of_order,
                    &key,
                    self.marker,
                    value,
                )
            }
            SerializerMapMode::Map { value_type, .. } => value.serialize(&mut Serializer {
                typetree: value_type,
                writer: self.writer,
                marker: self.marker,
                requires_align: value_type.requires_align(),
            }),
        }
    }

    fn end(mut self) -> Result<Self::Ok, Self::Error> {
        if let SerializerMapMode::Typed {
            next_index,
            ref out_of_order,
            ..
        } = self.mode
        {
            if let Some(missing) = missing_struct_fields(self.typetree, next_index, out_of_order) {
                return Err(Error::custom(format_args!(
                    "Missing fields of type {}: {}",
                    self.typetree.m_Type, missing,
                )));
            }
        }
        self.align_if(self.requires_align)
    }
}

/// Write one field of a struct, regardless of the order they appear in.
/// Out-of-order fields get buffered until they're expected.
fn write_struct_field<W: Write + Seek, B: ByteOrder + 'static>(
    typetree: &TypeTreeNode,
    writer: &mut W,
    next_index: &mut usize,
    out_of_order: &mut Vec<(usize, Vec<u8>)>,
    key: &str,
    marker: PhantomData<B>,
    value: &(impl Serialize + ?Sized),
) -> Result<(), Error> {
    let (found_index, child_type) = typetree
        .children
        .iter()
        .enumerate()
        .find(|(_, child)| child.m_Name == key)
        .ok_or_else(|| {
            Error::custom(format_args!(
                "field {key} isn't in type {}",
                typetree.m_Type
            ))
        })?;
    if found_index < *next_index || out_of_order.iter().any(|(index, _)| *index == found_index) {
        return Err(Error::custom(format_args!(
            "duplicate field {key} of type {}",
            typetree.m_Type
        )));
    }

    if found_index == *next_index {
        value.serialize(&mut Serializer {
            typetree: child_type,
            writer,
            marker,
            requires_align: child_type.requires_align(),
        })?;
        *next_index += 1;
        // drain buffered fields that follow contiguously
        while out_of_order
            .first()
            .is_some_and(|(index, _)| *index == *next_index)
        {
            let (_, bytes) = out_of_order.remove(0);
            writer.write_all(&bytes)?;
            *next_index += 1;
        }
        return Ok(());
    }

    let mut buffer = std::io::Cursor::new(Vec::new());
    value.serialize(&mut Serializer {
        typetree: child_type,
        writer: &mut buffer,
        marker,
        requires_align: child_type.requires_align(),
    })?;
    let at = out_of_order.partition_point(|(index, _)| *index < found_index);
    out_of_order.insert(at, (found_index, buffer.into_inner()));

    Ok(())
}

fn missing_struct_fields(
    typetree: &TypeTreeNode,
    next_index: usize,
    out_of_order: &[(usize, Vec<u8>)],
) -> Option<String> {
    if out_of_order.is_empty() && next_index == typetree.children.len() {
        return None;
    }
    let missing: Vec<_> = typetree
        .children
        .iter()
        .enumerate()
        .filter(|(index, _)| *index >= next_index && !out_of_order.iter().any(|(i, _)| i == index))
        .map(|(_, child)| format!("{} {}", child.m_Type, child.m_Name))
        .collect();
    Some(missing.join(", "))
}

/// Fields are deserialized in the typetree order, regardless of the declaration order of the struct.
/// Having the correct order still makes sense, because it doesn't require buffering.
pub struct SerializerStruct<'cx, W, B> {
    typetree: &'cx TypeTreeNode,
    writer: &'cx mut W,
    requires_align: bool,
    marker: PhantomData<B>,

    /// The next expected typetree position to write. Anything after it gets buffered.
    next_index: usize,
    /// `(typetree position, bytes)`
    out_of_order: Vec<(usize, Vec<u8>)>,
}

impl<W: Write + Seek, B: ByteOrder + 'static> serde::ser::SerializeStruct
    for SerializerStruct<'_, W, B>
{
    type Ok = ();

    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &str,
        value: &T,
    ) -> Result<(), Self::Error> {
        write_struct_field::<W, B>(
            self.typetree,
            self.writer,
            &mut self.next_index,
            &mut self.out_of_order,
            key,
            self.marker,
            value,
        )
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        if let Some(missing) =
            missing_struct_fields(self.typetree, self.next_index, &self.out_of_order)
        {
            return Err(Error::custom(format_args!(
                "Missing fields of type {}: {}",
                self.typetree.m_Type, missing,
            )));
        }
        if self.requires_align {
            self.writer.align::<4>()?;
        }
        Ok(())
    }
}

impl<W: Write, B: ByteOrder> serde::ser::SerializeStructVariant for &mut Serializer<'_, W, B> {
    type Ok = ();

    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &str,
        value: &T,
    ) -> Result<(), Self::Error> {
        todo!("{}", self.typetree.dump())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        todo!("{}", self.typetree.dump())
    }
}

pub struct SerializerTuple<'cx, W, B> {
    typetree: &'cx TypeTreeNode,
    writer: &'cx mut W,
    requires_align: bool,
    marker: PhantomData<B>,

    index: usize,
}

impl<W: Write + Seek, B: ByteOrder + 'static> serde::ser::SerializeTuple
    for SerializerTuple<'_, W, B>
{
    type Ok = ();

    type Error = Error;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        let child_type = &self.typetree.children[self.index];

        value.serialize(&mut Serializer {
            typetree: child_type,
            writer: self.writer,
            marker: self.marker,
            requires_align: child_type.requires_align(),
        })?;

        self.index += 1;

        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        if self.requires_align {
            self.writer.align::<4>()?;
        }
        assert_eq!(self.index, self.typetree.children.len());

        Ok(())
    }
}

fn ensure_type(tt: &TypeTreeNode, expected: &str) -> Result<(), Error> {
    if tt.m_Type != expected {
        return Err(Error::custom(format_args!(
            "invalid type: {}, expected {} (at {} {})\n{}",
            tt.m_Type,
            expected,
            tt.m_Type,
            tt.m_Name,
            tt.dump(),
        )));
    };

    Ok(())
}

struct StringSerializer<'a> {
    string: &'a mut String,
}
impl serde::Serializer for StringSerializer<'_> {
    type Ok = ();

    type Error = Error;

    type SerializeSeq = Impossible<(), Error>;

    type SerializeTuple = Impossible<(), Error>;

    type SerializeTupleStruct = Impossible<(), Error>;

    type SerializeTupleVariant = Impossible<(), Error>;

    type SerializeMap = Impossible<(), Error>;

    type SerializeStruct = Impossible<(), Error>;

    type SerializeStructVariant = Impossible<(), Error>;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        self.string.push_str(v);
        Ok(())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        unreachable!()
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_unit_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }

    fn serialize_newtype_struct<T>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        unreachable!()
    }

    fn serialize_newtype_variant<T>(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        unreachable!()
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        unreachable!()
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        unreachable!()
    }

    fn serialize_tuple_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        unreachable!()
    }

    fn serialize_tuple_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        unreachable!()
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        unreachable!()
    }

    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        unreachable!()
    }

    fn serialize_struct_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use crate::typetree::TypeTreeNode;

    use super::to_vec;
    use crate::serde_typetree::from_slice;

    fn node(ty: &str, name: &str, children: Vec<TypeTreeNode>) -> TypeTreeNode {
        TypeTreeNode {
            m_Type: ty.to_owned(),
            m_Name: name.to_owned(),
            children,
            ..Default::default()
        }
    }

    #[derive(Debug, serde_derive::Serialize, serde_derive::Deserialize)]
    struct Pair {
        second: u32,
        first: u32,
    }

    #[test]
    fn struct_fields_write_in_typetree_order() {
        let typetree = node(
            "Pair",
            "value",
            vec![
                node("UInt32", "first", vec![]),
                node("UInt32", "second", vec![]),
            ],
        );
        let bytes = to_vec::<_, byteorder::LE>(
            &Pair {
                second: 2,
                first: 1,
            },
            &typetree,
        )
        .unwrap();
        assert_eq!(bytes, [1_u32.to_le_bytes(), 2_u32.to_le_bytes()].concat());

        let value: Pair = from_slice::<_, byteorder::LE>(&bytes, &typetree).unwrap();
        assert_eq!((value.first, value.second), (1, 2));
    }

    #[test]
    fn a_struct_for_a_plain_node_rejects_instead_of_writing_nothing() {
        #[derive(serde_derive::Serialize)]
        struct Rational {
            count: i64,
        }
        let float = node("float", "seconds", vec![]);
        let error = to_vec::<_, byteorder::LE>(&Rational { count: 1 }, &float)
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "invalid type: struct Rational of 1 fields, expected float seconds"
        );

        let empty: std::collections::BTreeMap<String, u32> = Default::default();
        let error = to_vec::<_, byteorder::LE>(&empty, &float).unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid type: a struct of 0 fields, expected float seconds"
        );
    }

    #[test]
    fn missing_fields_reject_instead_of_short_writing() {
        #[derive(serde_derive::Serialize)]
        struct OnlySecond {
            second: u32,
        }
        let typetree = node(
            "Pair",
            "value",
            vec![
                node("UInt32", "first", vec![]),
                node("UInt32", "second", vec![]),
            ],
        );
        let error = to_vec::<_, byteorder::LE>(&OnlySecond { second: 2 }, &typetree)
            .unwrap_err()
            .to_string();
        assert_eq!(error, "Missing fields of type Pair: UInt32 first");
    }

    #[test]
    fn serialize_integer_sizes() {
        let mut errors = Vec::new();
        for (ty, min, max) in [
            ("SInt8", i8::MIN as i64, i8::MAX as i64),
            ("short", i16::MIN as i64, i16::MAX as i64),
            ("int", i32::MIN as i64, i32::MAX as i64),
        ] {
            let node = node(ty, "value", vec![]);
            to_vec::<_, byteorder::LE>(&min, &node).expect(ty);
            to_vec::<_, byteorder::LE>(&max, &node).expect(ty);
            for outside in [min - 1, max + 1] {
                let error = to_vec::<_, byteorder::LE>(&outside, &node).unwrap_err();
                errors.push(error.to_string());
            }
        }
        for (ty, max) in [
            ("UInt8", u8::MAX as u64),
            ("UInt16", u16::MAX as u64),
            ("unsigned int", u32::MAX as u64),
            ("SInt8", i8::MAX as u64),
            ("short", i16::MAX as u64),
            ("int", i32::MAX as u64),
            ("SInt64", i64::MAX as u64),
        ] {
            let node = node(ty, "value", vec![]);
            to_vec::<_, byteorder::LE>(&max, &node).expect(ty);
            let error = to_vec::<_, byteorder::LE>(&(max + 1), &node).unwrap_err();
            errors.push(error.to_string());
        }
        assert_eq!(
            errors,
            [
                "-129 does not fit SInt8 value",
                "128 does not fit SInt8 value",
                "-32769 does not fit short value",
                "32768 does not fit short value",
                "-2147483649 does not fit int value",
                "2147483648 does not fit int value",
                "256 does not fit UInt8 value",
                "65536 does not fit UInt16 value",
                "4294967296 does not fit unsigned int value",
                "128 does not fit SInt8 value",
                "32768 does not fit short value",
                "2147483648 does not fit int value",
                "9223372036854775808 does not fit SInt64 value",
            ]
        );
    }

    #[test]
    fn map_fields_write_in_typetree_order() {
        let typetree = node(
            "Pair",
            "value",
            vec![
                node("UInt32", "first", vec![]),
                node("UInt32", "second", vec![]),
            ],
        );
        let value = serde_json::json!({ "second": 2, "first": 1 });
        let bytes = to_vec::<_, byteorder::LE>(&value, &typetree).unwrap();
        assert_eq!(bytes, [1_u32.to_le_bytes(), 2_u32.to_le_bytes()].concat());
    }

    #[test]
    fn map_missing_fields_reject_instead_of_short_writing() {
        let typetree = node(
            "Pair",
            "value",
            vec![
                node("UInt32", "first", vec![]),
                node("UInt32", "second", vec![]),
            ],
        );
        let value = serde_json::json!({ "second": 2 });
        let error = to_vec::<_, byteorder::LE>(&value, &typetree)
            .unwrap_err()
            .to_string();
        assert_eq!(error, "Missing fields of type Pair: UInt32 first");
    }

    #[test]
    fn map_unknown_fields_error_instead_of_panicking() {
        let typetree = node(
            "Pair",
            "value",
            vec![
                node("UInt32", "first", vec![]),
                node("UInt32", "second", vec![]),
            ],
        );
        let value = serde_json::json!({ "first": 1, "second": 2, "third": 3 });
        let error =
            std::panic::catch_unwind(|| to_vec::<_, byteorder::LE>(&value, &typetree).map(|_| ()))
                .unwrap_or_else(|_| panic!("serializing an unknown map field panics"))
                .unwrap_err()
                .to_string();
        assert_eq!(error, "field third isn't in type Pair");
    }
}
