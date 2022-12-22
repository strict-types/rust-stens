// Strict encoding schema library, implementing validation and parsing
// strict encoded data against a schema.
//
// SPDX-License-Identifier: Apache-2.0
//
// Written in 2022-2023 by
//     Dr. Maxim Orlovsky <orlovsky@ubideco.org>
//
// Copyright 2022-2023 Ubideco Project
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::io::Sink;
use std::marker::PhantomData;

use amplify::WriteCounter;

use crate::ast::Field;
use crate::encoding::{
    DefineEnum, DefineStruct, DefineTuple, DefineUnion, StrictEncode, ToIdent, ToMaybeIdent,
    TypedParent, TypedWrite, WriteEnum, WriteStruct, WriteTuple, WriteUnion,
};
use crate::{FieldName, Ident};

// TODO: Move to amplify crate
#[derive(Debug)]
pub struct CountingWriter<W: io::Write> {
    count: usize,
    limit: usize,
    writer: W,
}

impl<W: io::Write> From<W> for CountingWriter<W> {
    fn from(writer: W) -> Self {
        Self {
            count: 0,
            limit: usize::MAX,
            writer,
        }
    }
}

impl<W: io::Write> CountingWriter<W> {
    pub fn with(limit: usize, writer: W) -> Self {
        Self {
            count: 0,
            limit,
            writer,
        }
    }

    pub fn unbox(self) -> W { self.writer }
}

impl<W: io::Write> io::Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.count + buf.len() > self.limit {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let count = self.writer.write(buf)?;
        self.count += count;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> { self.writer.flush() }
}

#[derive(Debug, From)]
pub struct StrictWriter<W: io::Write>(CountingWriter<W>);

impl StrictWriter<Vec<u8>> {
    pub fn in_memory(limit: usize) -> Self { StrictWriter(CountingWriter::with(limit, vec![])) }
}

impl StrictWriter<WriteCounter> {
    pub fn counter() -> Self { StrictWriter(CountingWriter::from(WriteCounter::default())) }
}

impl StrictWriter<Sink> {
    pub fn sink() -> Self { StrictWriter(CountingWriter::from(Sink::default())) }
}

impl<W: io::Write> StrictWriter<W> {
    pub fn with(limit: usize, writer: W) -> Self {
        StrictWriter(CountingWriter::with(limit, writer))
    }
    pub fn unbox(self) -> W { self.0.unbox() }
}

impl<W: io::Write> TypedWrite for StrictWriter<W> {
    type TupleWriter = StructWriter<W, Self>;
    type StructWriter = StructWriter<W, Self>;
    type UnionDefiner = UnionWriter<W>;
    type EnumDefiner = UnionWriter<W>;

    fn define_union(self, name: Option<impl ToIdent>) -> Self::UnionDefiner {
        UnionWriter::with(name, self)
    }
    fn define_enum(self, name: Option<impl ToIdent>) -> Self::EnumDefiner {
        UnionWriter::with(name, self)
    }
    fn write_tuple(self, name: Option<impl ToIdent>) -> Self::TupleWriter {
        StructWriter::with(name, self)
    }
    fn write_struct(self, name: Option<impl ToIdent>) -> Self::StructWriter {
        StructWriter::with(name, self)
    }
    unsafe fn _write_raw<const LEN: usize>(mut self, bytes: impl AsRef<[u8]>) -> io::Result<Self> {
        use io::Write;
        self.0.write_all(bytes.as_ref())?;
        Ok(self)
    }
}

pub struct StructWriter<W: io::Write, P: StrictParent<W>> {
    name: Option<Ident>,
    fields: BTreeSet<Field>,
    ords: BTreeSet<u8>,
    parent: P,
    defined: bool,
    _phantom: PhantomData<W>,
}

impl<W: io::Write, P: StrictParent<W>> StructWriter<W, P> {
    pub fn with(name: Option<impl ToIdent>, parent: P) -> Self {
        StructWriter {
            name: name.to_maybe_ident(),
            fields: empty!(),
            ords: empty!(),
            parent,
            defined: false,
            _phantom: default!(),
        }
    }

    pub fn is_defined(&self) -> bool { self.defined }

    pub fn as_parent_mut(&mut self) -> &mut P { &mut self.parent }

    pub fn field_ord(&self, field_name: &FieldName) -> Option<u8> {
        self.fields.iter().find(|f| f.name.as_ref() == Some(field_name)).map(|f| f.ord)
    }

    pub fn fields(&self) -> &BTreeSet<Field> { &self.fields }

    pub fn name(&self) -> &str {
        self.name.as_ref().map(|n| n.as_str()).unwrap_or_else(|| "<unnamed>")
    }

    pub fn next_ord(&self) -> u8 { self.fields.iter().max().map(|f| f.ord + 1).unwrap_or_default() }

    fn _define_field(mut self, field: Field) -> Self {
        assert!(
            self.fields.insert(field.clone()),
            "field {:#} is already defined as a part of {}",
            &field,
            self.name()
        );
        self.ords.insert(field.ord);
        self
    }

    fn _write_field(mut self, field: Field, value: &impl StrictEncode) -> io::Result<Self> {
        if self.defined {
            assert!(
                !self.fields.contains(&field),
                "field {:#} was not defined in {}",
                &field,
                self.name()
            )
        } else {
            self = self._define_field(field.clone());
        }
        assert!(
            self.ords.remove(&field.ord),
            "field {:#} was already written before in {} struct",
            &field,
            self.name()
        );
        let (mut writer, remnant) = self.parent.split_typed_write();
        writer = field.ord.strict_encode(writer)?;
        writer = value.strict_encode(writer)?;
        self.parent = P::from_split(writer, remnant);
        Ok(self)
    }

    fn _complete_definition(mut self) -> P {
        assert!(!self.fields.is_empty(), "struct {} does not have fields defined", self.name());
        self.defined = true;
        self.parent
    }

    fn _complete_write(self) -> P {
        assert!(self.ords.is_empty(), "not all fields were written for {}", self.name());
        self.parent
    }
}

impl<W: io::Write, P: StrictParent<W>> DefineStruct for StructWriter<W, P> {
    type Parent = P;
    fn define_field<T: StrictEncode>(self, name: impl ToIdent) -> Self {
        let ord = self.next_ord();
        DefineStruct::define_field_ord::<T>(self, name, ord)
    }
    fn define_field_ord<T: StrictEncode>(self, name: impl ToIdent, ord: u8) -> Self {
        let field = Field::named(name.to_ident(), ord);
        self._define_field(field)
    }
    fn complete(self) -> P { self._complete_definition() }
}

impl<W: io::Write, P: StrictParent<W>> WriteStruct for StructWriter<W, P> {
    type Parent = P;
    fn write_field(self, name: impl ToIdent, value: &impl StrictEncode) -> io::Result<Self> {
        let ord = self.next_ord();
        WriteStruct::write_field_ord(self, name, ord, value)
    }
    fn write_field_ord(
        self,
        name: impl ToIdent,
        ord: u8,
        value: &impl StrictEncode,
    ) -> io::Result<Self> {
        let field = Field::named(name.to_ident(), ord);
        self._write_field(field, value)
    }
    fn complete(self) -> P { self._complete_write() }
}

impl<W: io::Write, P: StrictParent<W>> DefineTuple for StructWriter<W, P> {
    type Parent = P;
    fn define_field<T: StrictEncode>(self) -> Self {
        let ord = self.next_ord();
        DefineTuple::define_field_ord::<T>(self, ord)
    }
    fn define_field_ord<T: StrictEncode>(self, ord: u8) -> Self {
        let field = Field::unnamed(ord);
        self._define_field(field)
    }
    fn complete(self) -> P { self._complete_definition() }
}

impl<W: io::Write, P: StrictParent<W>> WriteTuple for StructWriter<W, P> {
    type Parent = P;
    fn write_field(self, value: &impl StrictEncode) -> io::Result<Self> {
        let ord = self.next_ord();
        WriteTuple::write_field_ord(self, ord, value)
    }
    fn write_field_ord(self, ord: u8, value: &impl StrictEncode) -> io::Result<Self> {
        let field = Field::unnamed(ord);
        self._write_field(field, value)
    }
    fn complete(self) -> P { self._complete_write() }
}

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
enum FieldType {
    Unit,
    Tuple,
    Struct,
}

pub struct UnionWriter<W: io::Write> {
    name: Option<Ident>,
    fields: BTreeMap<Field, FieldType>,
    writer: StrictWriter<W>,
    written: bool,
    parent_ident: Option<Ident>,
}

impl<W: io::Write> UnionWriter<W> {
    fn with(name: Option<impl ToIdent>, writer: StrictWriter<W>) -> Self {
        UnionWriter {
            name: name.to_maybe_ident(),
            fields: empty!(),
            writer,
            written: false,
            parent_ident: None,
        }
    }

    fn inline(name: Option<impl ToIdent>, uw: UnionWriter<W>) -> Self {
        UnionWriter {
            name: name.to_maybe_ident(),
            fields: empty!(),
            writer: uw.writer,
            written: false,
            parent_ident: uw.name,
        }
    }

    fn name(&self) -> &str { self.name.as_ref().map(|n| n.as_str()).unwrap_or("unnamed") }

    fn next_ord(&self) -> u8 { self.fields.keys().max().map(|f| f.ord + 1).unwrap_or_default() }

    fn _define_field(mut self, field: Field, field_type: FieldType) -> Self {
        assert!(
            self.fields.insert(field.clone(), field_type).is_none(),
            "variant {:#} is already defined as a part of {}",
            &field,
            self.name()
        );
        self
    }

    fn _write_field(mut self, name: Ident, field_type: FieldType) -> io::Result<Self> {
        let (field, t) = self
            .fields
            .iter()
            .find(|(f, _)| f.name.as_ref() == Some(&name))
            .expect(&format!("variant {:#} was not defined in {}", &name, self.name()));
        assert_eq!(
            *t,
            field_type,
            "variant {:#} in {} must be a {:?} while it is written as {:?}",
            &field,
            self.name(),
            t,
            field_type
        );
        assert!(!self.written, "multiple attempts to write variants of {}", self.name());
        self.written = true;
        self.writer = field.ord.strict_encode(self.writer)?;
        Ok(self)
    }

    fn _complete_definition(self) -> Self {
        assert!(
            !self.fields.is_empty(),
            "unit or enum {} does not have fields defined",
            self.name()
        );
        self
    }

    fn _complete_write(self) -> StrictWriter<W> {
        assert!(self.written, "not a single variant is written for {}", self.name());
        self.writer
    }
}

impl<W: io::Write> DefineUnion for UnionWriter<W> {
    type Parent = StrictWriter<W>;
    type TupleDefiner = StructWriter<W, Self>;
    type StructDefiner = StructWriter<W, Self>;
    type UnionWriter = UnionWriter<W>;

    fn define_unit(self, name: impl ToIdent) -> Self {
        let field = Field::named(name.to_ident(), self.next_ord());
        self._define_field(field, FieldType::Unit)
    }
    fn define_tuple(mut self, name: impl ToIdent) -> Self::TupleDefiner {
        let field = Field::named(name.to_ident(), self.next_ord());
        self = self._define_field(field, FieldType::Tuple);
        StructWriter::with(Some(name), self)
    }
    fn define_struct(mut self, name: impl ToIdent) -> Self::StructDefiner {
        let field = Field::named(name.to_ident(), self.next_ord());
        self = self._define_field(field, FieldType::Struct);
        StructWriter::with(Some(name), self)
    }
    fn complete(self) -> Self::UnionWriter { self._complete_definition() }
}

impl<W: io::Write> WriteUnion for UnionWriter<W> {
    type Parent = StrictWriter<W>;
    type TupleWriter = StructWriter<W, Self>;
    type StructWriter = StructWriter<W, Self>;

    fn write_unit(self, name: impl ToIdent) -> io::Result<Self> {
        self._write_field(name.to_ident(), FieldType::Unit)
    }
    fn write_tuple(mut self, name: impl ToIdent) -> io::Result<Self::TupleWriter> {
        self = self._write_field(name.to_ident(), FieldType::Tuple)?;
        Ok(StructWriter::with(Some(name), self))
    }
    fn write_struct(mut self, name: impl ToIdent) -> io::Result<Self::StructWriter> {
        self = self._write_field(name.to_ident(), FieldType::Struct)?;
        Ok(StructWriter::with(Some(name), self))
    }
    fn complete(self) -> Self::Parent { self._complete_write() }
}

impl<W: io::Write> DefineEnum for UnionWriter<W> {
    type Parent = StrictWriter<W>;
    type EnumWriter = UnionWriter<W>;
    fn define_variant(self, name: impl ToIdent, value: u8) -> Self {
        let field = Field::named(name.to_ident(), value);
        self._define_field(field, FieldType::Unit)
    }
    fn complete(self) -> Self::EnumWriter { self._complete_definition() }
}

impl<W: io::Write> WriteEnum for UnionWriter<W> {
    type Parent = StrictWriter<W>;
    fn write_variant(self, name: impl ToIdent) -> io::Result<Self> {
        self._write_field(name.to_ident(), FieldType::Unit)
    }
    fn complete(self) -> Self::Parent { self._complete_write() }
}

pub trait StrictParent<W: io::Write>: TypedParent {
    type Remnant;
    fn from_split(writer: StrictWriter<W>, remnant: Self::Remnant) -> Self;
    fn split_typed_write(self) -> (StrictWriter<W>, Self::Remnant);
}
impl<W: io::Write> TypedParent for StrictWriter<W> {}
impl<W: io::Write> TypedParent for UnionWriter<W> {}
impl<W: io::Write> StrictParent<W> for StrictWriter<W> {
    type Remnant = ();
    fn from_split(writer: StrictWriter<W>, _: Self::Remnant) -> Self { writer }
    fn split_typed_write(self) -> (StrictWriter<W>, Self::Remnant) { (self, ()) }
}
impl<W: io::Write> StrictParent<W> for UnionWriter<W> {
    type Remnant = UnionWriter<Vec<u8>>;
    fn from_split(writer: StrictWriter<W>, remnant: Self::Remnant) -> Self {
        Self {
            name: remnant.name,
            fields: remnant.fields,
            writer,
            written: remnant.written,
            parent_ident: remnant.parent_ident,
        }
    }
    fn split_typed_write(self) -> (StrictWriter<W>, Self::Remnant) {
        let remnant = UnionWriter {
            name: self.name,
            fields: self.fields,
            writer: StrictWriter::<Vec<u8>>::in_memory(0),
            written: self.written,
            parent_ident: self.parent_ident,
        };
        (self.writer, remnant)
    }
}
