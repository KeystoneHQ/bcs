// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use alloc::vec::Vec;
use core2::io::{ErrorKind, Write, Result as core2Result};
use crate::error::{Error, Result};
use serde::{ser, Serialize};

/// Serialize the given data structure as a `Vec<u8>` of BCS.
///
/// Serialization can fail if `T`'s implementation of `Serialize` decides to
/// fail, if `T` contains sequences which are longer than `MAX_SEQUENCE_LENGTH`,
/// or if `T` attempts to serialize an unsupported datatype such as a f32,
/// f64, or char.
///
/// # Examples
///
/// ```
/// use bcs::to_bytes;
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct Ip([u8; 4]);
///
/// #[derive(Serialize)]
/// struct Port(u16);
///
/// #[derive(Serialize)]
/// struct Service {
///     ip: Ip,
///     port: Vec<Port>,
///     connection_max: Option<u32>,
///     enabled: bool,
/// }
///
/// let service = Service {
///     ip: Ip([192, 168, 1, 1]),
///     port: vec![Port(8001), Port(8002), Port(8003)],
///     connection_max: Some(5000),
///     enabled: false,
/// };
///
/// let bytes = to_bytes(&service).unwrap();
/// let expected = vec![
///     0xc0, 0xa8, 0x01, 0x01, 0x03, 0x41, 0x1f, 0x42,
///     0x1f, 0x43, 0x1f, 0x01, 0x88, 0x13, 0x00, 0x00,
///     0x00,
/// ];
/// assert_eq!(bytes, expected);
/// ```
pub fn to_bytes<T>(value: &T) -> Result<Vec<u8>>
where
    T: ?Sized + Serialize,
{
    let mut output = Vec::new();
    serialize_into(&mut output, value)?;
    Ok(output)
}

/// Same as `to_bytes` but use `limit` as max container depth instead of MAX_CONTAINER_DEPTH
/// Note that `limit` has to be lower than MAX_CONTAINER_DEPTH
pub fn to_bytes_with_limit<T>(value: &T, limit: usize) -> Result<Vec<u8>>
where
    T: ?Sized + Serialize,
{
    if limit > crate::MAX_CONTAINER_DEPTH {
        return Err(Error::NotSupported("limit exceeds the max allowed depth"));
    }
    let mut output = Vec::new();
    serialize_into_with_limit(&mut output, value, limit)?;
    Ok(output)
}

/// Same as `to_bytes` but write directly into an `std::io::Write` object.
pub fn serialize_into<W, T>(write: &mut W, value: &T) -> Result<()>
where
    W: ?Sized + Write,
    T: ?Sized + Serialize,
{
    let serializer = Serializer::new(write, crate::MAX_CONTAINER_DEPTH);
    value.serialize(serializer)
}

/// Same as `serialize_into` but use `limit` as max container depth instead of MAX_CONTAINER_DEPTH
/// Note that `limit` has to be lower than MAX_CONTAINER_DEPTH
pub fn serialize_into_with_limit<W, T>(write: &mut W, value: &T, limit: usize) -> Result<()>
where
    W: ?Sized + Write,
    T: ?Sized + Serialize,
{
    if limit > crate::MAX_CONTAINER_DEPTH {
        return Err(Error::NotSupported("limit exceeds the max allowed depth"));
    }
    let serializer = Serializer::new(write, limit);
    value.serialize(serializer)
}

struct WriteCounter(usize);

impl Write for WriteCounter {
    fn write(&mut self, buf: &[u8]) -> core2Result<usize> {
        let len = buf.len();
        self.0 = self.0.checked_add(len).ok_or_else(|| {
            core2::io::Error::new(ErrorKind::Other, "WriteCounter reached max value")
        })?;
        Ok(len)
    }

    fn flush(&mut self) -> core2Result<()> {
        Ok(())
    }
}

/// Same as `to_bytes` but only return the size of the serialized bytes.
pub fn serialized_size<T>(value: &T) -> Result<usize>
where
    T: ?Sized + Serialize,
{
    let mut counter = WriteCounter(0);
    serialize_into(&mut counter, value)?;
    Ok(counter.0)
}

/// Same as `serialized_size` but use `limit` as max container depth instead of MAX_CONTAINER_DEPTH
/// Note that `limit` has to be lower than MAX_CONTAINER_DEPTH
pub fn serialized_size_with_limit<T>(value: &T, limit: usize) -> Result<usize>
where
    T: ?Sized + Serialize,
{
    if limit > crate::MAX_CONTAINER_DEPTH {
        return Err(Error::NotSupported("limit exceeds the max allowed depth"));
    }
    let mut counter = WriteCounter(0);
    serialize_into_with_limit(&mut counter, value, limit)?;
    Ok(counter.0)
}

pub fn is_human_readable() -> bool {
    let mut output = Vec::new();
    let serializer = Serializer::new(&mut output, crate::MAX_CONTAINER_DEPTH);
    ser::Serializer::is_human_readable(&serializer)
}

/// Serialization implementation for BCS
struct Serializer<'a, W: ?Sized> {
    output: &'a mut W,
    max_remaining_depth: usize,
}

impl<'a, W> Serializer<'a, W>
where
    W: ?Sized + Write,
{
    /// Creates a new `Serializer` which will emit BCS.
    fn new(output: &'a mut W, max_remaining_depth: usize) -> Self {
        Self {
            output,
            max_remaining_depth,
        }
    }

    fn output_u32_as_uleb128(&mut self, mut value: u32) -> Result<()> {
        while value >= 0x80 {
            // Write 7 (lowest) bits of data and set the 8th bit to 1.
            let byte = (value & 0x7f) as u8;
            self.output.write_all(&[byte | 0x80]).map_err(|e| Error::from(e))?;
            value >>= 7;
        }
        // Write the remaining bits of data and set the highest bit to 0.
        self.output.write_all(&[value as u8]).map_err(|e| Error::from(e))?;
        Ok(())
    }

    fn output_variant_index(&mut self, v: u32) -> Result<()> {
        self.output_u32_as_uleb128(v)
    }

    /// Serialize a sequence length as a u32.
    fn output_seq_len(&mut self, len: usize) -> Result<()> {
        if len > crate::MAX_SEQUENCE_LENGTH {
            return Err(Error::ExceededMaxLen(len));
        }
        self.output_u32_as_uleb128(len as u32)
    }

    fn enter_named_container(&mut self, name: &'static str) -> Result<()> {
        if self.max_remaining_depth == 0 {
            return Err(Error::ExceededContainerDepthLimit(name));
        }
        self.max_remaining_depth -= 1;
        Ok(())
    }
}

impl<'a, W> ser::Serializer for Serializer<'a, W>
where
    W: ?Sized + Write,
{
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = MapSerializer<'a, W>;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_bool(self, v: bool) -> Result<()> {
        self.serialize_u8(v.into())
    }

    fn serialize_i8(self, v: i8) -> Result<()> {
        self.serialize_u8(v as u8)
    }

    fn serialize_i16(self, v: i16) -> Result<()> {
        self.serialize_u16(v as u16)
    }

    fn serialize_i32(self, v: i32) -> Result<()> {
        self.serialize_u32(v as u32)
    }

    fn serialize_i64(self, v: i64) -> Result<()> {
        self.serialize_u64(v as u64)
    }

    fn serialize_i128(self, v: i128) -> Result<()> {
        self.serialize_u128(v as u128)
    }

    fn serialize_u8(self, v: u8) -> Result<()> {
        self.output.write_all(&[v]).map_err(|e| Error::from(e))?;
        Ok(())
    }

    fn serialize_u16(self, v: u16) -> Result<()> {
        self.output.write_all(&v.to_le_bytes()).map_err(|e| Error::from(e))?;
        Ok(())
    }

    fn serialize_u32(self, v: u32) -> Result<()> {
        self.output.write_all(&v.to_le_bytes()).map_err(|e| Error::from(e))?;
        Ok(())
    }

    fn serialize_u64(self, v: u64) -> Result<()> {
        self.output.write_all(&v.to_le_bytes()).map_err(|e| Error::from(e))?;
        Ok(())
    }

    fn serialize_u128(self, v: u128) -> Result<()> {
        self.output.write_all(&v.to_le_bytes()).map_err(|e| Error::from(e))?;
        Ok(())
    }

    fn serialize_f32(self, _v: f32) -> Result<()> {
        Err(Error::NotSupported("serialize_f32"))
    }

    fn serialize_f64(self, _v: f64) -> Result<()> {
        Err(Error::NotSupported("serialize_f64"))
    }

    fn serialize_char(self, _v: char) -> Result<()> {
        Err(Error::NotSupported("serialize_char"))
    }

    // Just serialize the string as a raw byte array
    fn serialize_str(self, v: &str) -> Result<()> {
        self.serialize_bytes(v.as_bytes())
    }

    // Serialize a byte array as an array of bytes.
    fn serialize_bytes(mut self, v: &[u8]) -> Result<()> {
        self.output_seq_len(v.len())?;
        self.output.write_all(v).map_err(|e| Error::from(e))?;
        Ok(())
    }

    // An absent optional is represented as `00`
    fn serialize_none(self) -> Result<()> {
        self.serialize_u8(0)
    }

    // A present optional is represented as `01` followed by the serialized value
    fn serialize_some<T>(self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        self.output.write_all(&[1]).map_err(|e| Error::from(e))?;
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<()> {
        Ok(())
    }

    fn serialize_unit_struct(mut self, name: &'static str) -> Result<()> {
        self.enter_named_container(name)?;
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        mut self,
        name: &'static str,
        variant_index: u32,
        _variant: &'static str,
    ) -> Result<()> {
        self.enter_named_container(name)?;
        self.output_variant_index(variant_index)
    }

    fn serialize_newtype_struct<T>(mut self, name: &'static str, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        self.enter_named_container(name)?;
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        mut self,
        name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        value: &T,
    ) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        self.enter_named_container(name)?;
        self.output_variant_index(variant_index)?;
        value.serialize(self)
    }

    // The start of the sequence, each value, and the end are three separate
    // method calls. This one is responsible only for serializing the start,
    // which for BCS is either nothing for fixed structures or for variable
    // length structures, the length encoded as a u32.
    fn serialize_seq(mut self, len: Option<usize>) -> Result<Self::SerializeSeq> {
        if let Some(len) = len {
            self.output_seq_len(len)?;
            Ok(self)
        } else {
            Err(Error::MissingLen)
        }
    }

    // Tuples are fixed sized structs so we don't need to encode the length
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple> {
        Ok(self)
    }

    fn serialize_tuple_struct(
        mut self,
        name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        self.enter_named_container(name)?;
        Ok(self)
    }

    fn serialize_tuple_variant(
        mut self,
        name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        self.enter_named_container(name)?;
        self.output_variant_index(variant_index)?;
        Ok(self)
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap> {
        Ok(MapSerializer::new(self))
    }

    fn serialize_struct(
        mut self,
        name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct> {
        self.enter_named_container(name)?;
        Ok(self)
    }

    fn serialize_struct_variant(
        mut self,
        name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant> {
        self.enter_named_container(name)?;
        self.output_variant_index(variant_index)?;
        Ok(self)
    }

    // BCS is not a human readable format
    fn is_human_readable(&self) -> bool {
        false
    }
}

impl<'a, W> ser::SerializeSeq for Serializer<'a, W>
where
    W: ?Sized + Write,
{
    type Ok = ();
    type Error = Error;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(Serializer::new(self.output, self.max_remaining_depth))
    }

    fn end(self) -> Result<()> {
        Ok(())
    }
}

impl<'a, W> ser::SerializeTuple for Serializer<'a, W>
where
    W: ?Sized + Write,
{
    type Ok = ();
    type Error = Error;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(Serializer::new(self.output, self.max_remaining_depth))
    }

    fn end(self) -> Result<()> {
        Ok(())
    }
}

impl<'a, W> ser::SerializeTupleStruct for Serializer<'a, W>
where
    W: ?Sized + Write,
{
    type Ok = ();
    type Error = Error;

    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(Serializer::new(self.output, self.max_remaining_depth))
    }

    fn end(self) -> Result<()> {
        Ok(())
    }
}

impl<'a, W> ser::SerializeTupleVariant for Serializer<'a, W>
where
    W: ?Sized + Write,
{
    type Ok = ();
    type Error = Error;

    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(Serializer::new(self.output, self.max_remaining_depth))
    }

    fn end(self) -> Result<()> {
        Ok(())
    }
}

#[doc(hidden)]
struct MapSerializer<'a, W: ?Sized> {
    serializer: Serializer<'a, W>,
    entries: Vec<(Vec<u8>, Vec<u8>)>,
    next_key: Option<Vec<u8>>,
}

impl<'a, W: ?Sized> MapSerializer<'a, W> {
    fn new(serializer: Serializer<'a, W>) -> Self {
        MapSerializer {
            serializer,
            entries: Vec::new(),
            next_key: None,
        }
    }
}

impl<'a, W> ser::SerializeMap for MapSerializer<'a, W>
where
    W: ?Sized + Write,
{
    type Ok = ();
    type Error = Error;

    fn serialize_key<T>(&mut self, key: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        if self.next_key.is_some() {
            return Err(Error::ExpectedMapValue);
        }

        let mut output = Vec::new();
        key.serialize(Serializer::new(
            &mut output,
            self.serializer.max_remaining_depth,
        ))?;
        self.next_key = Some(output);
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        match self.next_key.take() {
            Some(key) => {
                let mut output = Vec::new();
                value.serialize(Serializer::new(
                    &mut output,
                    self.serializer.max_remaining_depth,
                ))?;
                self.entries.push((key, output));
                Ok(())
            }
            None => Err(Error::ExpectedMapKey),
        }
    }

    fn end(mut self) -> Result<()> {
        if self.next_key.is_some() {
            return Err(Error::ExpectedMapValue);
        }
        self.entries.sort_by(|e1, e2| e1.0.cmp(&e2.0));
        self.entries.dedup_by(|e1, e2| e1.0.eq(&e2.0));

        let len = self.entries.len();
        self.serializer.output_seq_len(len)?;

        for (key, value) in &self.entries {
            self.serializer.output.write_all(key).map_err(|e| Error::from(e))?;
            self.serializer.output.write_all(value).map_err(|e| Error::from(e))?;
        }

        Ok(())
    }
}

impl<'a, W> ser::SerializeStruct for Serializer<'a, W>
where
    W: ?Sized + Write,
{
    type Ok = ();
    type Error = Error;

    fn serialize_field<T>(&mut self, _key: &'static str, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(Serializer::new(self.output, self.max_remaining_depth))
    }

    fn end(self) -> Result<()> {
        Ok(())
    }
}

impl<'a, W> ser::SerializeStructVariant for Serializer<'a, W>
where
    W: ?Sized + Write,
{
    type Ok = ();
    type Error = Error;

    fn serialize_field<T>(&mut self, _key: &'static str, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(Serializer::new(self.output, self.max_remaining_depth))
    }

    fn end(self) -> Result<()> {
        Ok(())
    }
}
