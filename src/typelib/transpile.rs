// Strict encoding schema library, implementing validation and parsing
// strict encoded data against a schema.
//
// SPDX-License-Identifier: Apache-2.0
//
// Written in 2022-2024 by
//     Dr. Maxim Orlovsky <orlovsky@ubideco.org>
//
// Copyright 2022-2024 UBIDECO Institute
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

use amplify::confinement::{Collection, Confined};
use amplify::Wrapper;
use strict_encoding::{
    DefineEnum, DefineStruct, DefineTuple, DefineUnion, FieldName, LibName, Primitive, Sizing,
    SplitParent, StrictDumb, StrictEncode, StrictEnum, StrictParent, StrictStruct, StrictSum,
    StrictTuple, StrictUnion, StrictWriter, StructWriter, TypeName, TypedParent, TypedWrite,
    UnionWriter, VariantName, WriteEnum, WriteStruct, WriteTuple, WriteUnion, LIB_EMBEDDED,
};

use crate::ast::{EnumVariants, Field, NamedFields, UnionVariants, UnnamedFields};
use crate::{Dependency, SemId, SymbolRef, TranspileRef, Ty, TypeLibId};

pub trait BuilderParent: StrictParent<Sink> {
    /// Converts strict-encodable value into a type information. Must be propagated back to the
    /// lib builder which does the TypedWrite implementation to call strict encode on the type
    fn compile_type<T: StrictEncode>(self, value: &T) -> (Self, TranspileRef);
    /// Notifies lib builder about complete type built, even for unnamed inline types, such that it
    /// can register last compiled type for the `compile_type` procedure.
    fn report_compiled(self, lib: LibName, name: Option<TypeName>, ty: Ty<TranspileRef>) -> Self;
}

#[derive(Debug)]
pub struct LibBuilder {
    pub(super) lib_name: LibName,
    pub(super) known_libs: BTreeSet<Dependency>,
    pub(super) extern_types: BTreeMap<LibName, BTreeMap<SemId, TypeName>>,
    pub(super) types: BTreeMap<TypeName, Ty<TranspileRef>>,
    pub(super) roots: BTreeSet<TypeName>,
    last_compiled: Option<TranspileRef>,
}

impl LibBuilder {
    pub fn new(
        name: impl Into<LibName>,
        known_libs: impl IntoIterator<Item = Dependency>,
    ) -> LibBuilder {
        LibBuilder {
            lib_name: name.into(),
            known_libs: known_libs.into_iter().collect(),
            extern_types: empty!(),
            types: empty!(),
            roots: empty!(),
            last_compiled: None,
        }
    }

    pub fn transpile<T: StrictEncode + StrictDumb>(mut self) -> Self {
        self.roots.push(T::strict_name().expect("transpiled types must be named"));
        self = T::strict_dumb().strict_encode(self).expect("memory encoding doesn't error");
        self
    }

    fn dependency_id(&self, lib_name: &LibName) -> TypeLibId {
        self.known_libs
            .iter()
            .find(|dep| &dep.name == lib_name)
            .map(|dep| dep.id)
            .unwrap_or_else(|| panic!("use of library '{lib_name}' which is not a dependency"))
    }
}

impl TypedWrite for LibBuilder {
    type TupleWriter = StructBuilder<Self>;
    type StructWriter = StructBuilder<Self>;
    type UnionDefiner = UnionBuilder;

    fn write_union<T: StrictUnion>(
        self,
        inner: impl FnOnce(Self::UnionDefiner) -> io::Result<Self>,
    ) -> io::Result<Self> {
        let builder = UnionBuilder::with::<T>(self);
        inner(builder)
    }

    fn write_enum<T: StrictEnum>(self, value: T) -> io::Result<Self>
    where u8: From<T> {
        let mut writer = UnionBuilder::with::<T>(self);
        for (_, name) in T::ALL_VARIANTS {
            writer = writer.define_variant(vname!(*name));
        }
        writer = DefineEnum::complete(writer);
        writer = writer.write_variant(vname!(value.variant_name()))?;
        Ok(WriteEnum::complete(writer))
    }

    fn write_tuple<T: StrictTuple>(
        self,
        inner: impl FnOnce(Self::TupleWriter) -> io::Result<Self>,
    ) -> io::Result<Self> {
        let writer = StructWriter::tuple::<T>(self);
        let builder =
            StructBuilder::with(libname!(T::STRICT_LIB_NAME), T::strict_name(), writer, false);
        inner(builder)
    }

    fn write_struct<T: StrictStruct>(
        self,
        inner: impl FnOnce(Self::StructWriter) -> io::Result<Self>,
    ) -> io::Result<Self> {
        let writer = StructWriter::structure::<T>(self);
        let builder =
            StructBuilder::with(libname!(T::STRICT_LIB_NAME), T::strict_name(), writer, false);
        inner(builder)
    }

    unsafe fn register_primitive(mut self, prim: Primitive) -> Self {
        self.last_compiled = Some(Ty::Primitive(prim).into());
        self
    }

    unsafe fn register_array(mut self, ty: &impl StrictEncode, len: u16) -> Self {
        self = ty.strict_encode(self).expect("in-memory encoding");
        let ty = self.last_compiled.expect("can't compile type");
        self.last_compiled = Some(Ty::Array(ty, len).into());
        self
    }

    unsafe fn register_unicode(mut self, sizing: Sizing) -> Self {
        self.last_compiled = Some(Ty::List(Ty::UnicodeChar.into(), sizing).into());
        self
    }

    unsafe fn register_list(mut self, ty: &impl StrictEncode, sizing: Sizing) -> Self {
        self = ty.strict_encode(self).expect("in-memory encoding");
        let ty = self.last_compiled.expect("can't compile type");
        self.last_compiled = Some(Ty::List(ty, sizing).into());
        self
    }

    unsafe fn register_set(mut self, ty: &impl StrictEncode, sizing: Sizing) -> Self {
        self = ty.strict_encode(self).expect("in-memory encoding");
        let ty = self.last_compiled.expect("can't compile type");
        self.last_compiled = Some(Ty::Set(ty, sizing).into());
        self
    }

    unsafe fn register_map(
        mut self,
        key: &impl StrictEncode,
        ty: &impl StrictEncode,
        sizing: Sizing,
    ) -> Self {
        self = key.strict_encode(self).expect("in-memory encoding");
        let key_ref = self.last_compiled.clone().expect("can't compile key type");

        self = ty.strict_encode(self).expect("in-memory encoding");
        let val_ref = self.last_compiled.clone().expect("can't compile value type");

        self.last_compiled = Some(Ty::Map(key_ref, val_ref, sizing).into());
        self
    }

    unsafe fn _write_raw<const LEN: usize>(self, _bytes: impl AsRef<[u8]>) -> io::Result<Self> {
        // Nothing to do here
        Ok(self)
    }
}

impl TypedParent for LibBuilder {}
impl StrictParent<Sink> for LibBuilder {
    type Remnant = LibBuilder;
    fn from_write_split(_: StrictWriter<Sink>, remnant: Self::Remnant) -> Self { remnant }
    fn into_write_split(self) -> (StrictWriter<Sink>, Self::Remnant) {
        (StrictWriter::sink(), self)
    }
}
impl BuilderParent for LibBuilder {
    fn compile_type<T: StrictEncode>(self, value: &T) -> (Self, TranspileRef) {
        let _compile = |mut me: Self| -> (Self, TranspileRef) {
            me = value.strict_encode(me).expect("too many types in the library");
            let r =
                me.last_compiled.clone().expect("no type found after strict encoding procedure");
            (me, r)
        };
        match (T::STRICT_LIB_NAME, T::strict_name()) {
            (LIB_EMBEDDED, _) | (_, None) => _compile(self),
            (lib, Some(name)) if lib != self.lib_name.as_str() => {
                let (me, r) = _compile(self);
                let lib_name = libname!(lib);
                let lib_id = me.dependency_id(&lib_name);
                (me, TranspileRef::Extern(SymbolRef::with(lib_name, name, lib_id, r.id())))
            }
            (_, Some(name)) if self.types.contains_key(&name) => (self, TranspileRef::Named(name)),
            (_, Some(_)) => _compile(self),
        }
    }

    fn report_compiled(
        mut self,
        lib: LibName,
        name: Option<TypeName>,
        ty: Ty<TranspileRef>,
    ) -> Self {
        let r = match (lib, name) {
            (lib, Some(name)) if lib == self.lib_name => {
                if let Some(old_ty) = self.types.get(&name) {
                    assert_eq!(
                        old_ty, &ty,
                        "repeated type name '{name}' for two different types '{old_ty}' and '{ty}'",
                    );
                }
                self.types.insert(name.clone(), ty);
                TranspileRef::Named(name)
            }
            (lib, Some(name)) => {
                let id = ty.sem_id_named(&name);
                self.extern_types.entry(lib.clone()).or_default().insert(id, name.clone());
                let lib_id = self.dependency_id(&lib);
                TranspileRef::Extern(SymbolRef::with(lib, name, lib_id, id))
            }
            (_, None) => TranspileRef::Embedded(Box::new(ty)),
        };
        self.last_compiled = Some(r);
        self
    }
}

#[derive(Debug)]
pub struct StructBuilder<P: BuilderParent> {
    lib: LibName,
    name: Option<TypeName>,
    writer: StructWriter<Sink, P>,
    fields: Vec<(Option<FieldName>, TranspileRef)>,
    cursor: Option<u8>,
}

impl<P: BuilderParent> StructBuilder<P> {
    pub fn with(
        lib: LibName,
        name: Option<TypeName>,
        writer: StructWriter<Sink, P>,
        definer: bool,
    ) -> Self {
        StructBuilder {
            lib,
            name,
            writer,
            fields: empty!(),
            cursor: if definer { Some(0) } else { None },
        }
    }

    pub fn name(&self) -> &str { self.name.as_ref().map(|n| n.as_str()).unwrap_or("<unnamed>") }

    fn _define_field<T: StrictEncode + StrictDumb>(mut self, fname: Option<FieldName>) -> Self {
        let (parent, remnant) = self.writer.into_parent_split();
        let (parent, ty) = parent.compile_type(&T::strict_dumb());
        self.writer = StructWriter::from_parent_split(parent, remnant);
        self.fields.push((fname, ty)); // type repetition is checked by self.parent
        self
    }

    fn _write_field(
        mut self,
        fname: Option<FieldName>,
        value: &impl StrictEncode,
    ) -> io::Result<Self> {
        let (parent, remnant) = self.writer.into_parent_split();
        let (parent, ty) = parent.compile_type(value);
        self.writer = StructWriter::from_parent_split(parent, remnant);
        if let Some(pos) = &mut self.cursor {
            let expect_ty = &self.fields[*pos as usize].1;
            let msg =
                format!("'{}.{pos}' has type '{ty}' instead of '{expect_ty}'", self.writer.name(),);
            assert_eq!(expect_ty, &ty, "{msg}");
            *pos += 1;
        }
        self.fields.push((fname, ty));
        Ok(self)
    }

    fn _build_struct(&self) -> Ty<TranspileRef> {
        if self.fields.is_empty() {
            Ty::UNIT
        } else if self.writer.is_tuple() {
            let fields =
                Confined::try_from_iter(self.fields.iter().map(|(_, field)| field.clone()))
                    .unwrap_or_else(|_| {
                        panic!(
                            "tuple '{}' has invalid number of fields ({})",
                            self.name(),
                            self.fields.len()
                        )
                    });
            Ty::Tuple(UnnamedFields::from_inner(fields))
        } else {
            let fields = Confined::try_from_iter(self.fields.iter().cloned().map(|(name, ty)| {
                let name = name.expect("unnamed field");
                Field { name, ty }
            }))
            .unwrap_or_else(|_| {
                panic!(
                    "tuple '{}' has invalid number of fields ({})",
                    self.name(),
                    self.fields.len()
                )
            });
            Ty::Struct(NamedFields::from_inner(fields))
        }
    }

    fn _complete_definition(self) -> P {
        let ty = self._build_struct();
        if self.writer.is_tuple() {
            DefineTuple::complete(self.writer).report_compiled(self.lib, self.name, ty)
        } else {
            DefineStruct::complete(self.writer).report_compiled(self.lib, self.name, ty)
        }
    }

    fn _complete_write(self) -> P {
        let ty = self._build_struct();
        if let Some(pos) = self.cursor {
            assert_eq!(
                pos as usize,
                self.fields.len(),
                "not all fields were written for '{}'",
                self.writer.name()
            );
        }
        if self.writer.is_tuple() {
            WriteTuple::complete(self.writer).report_compiled(self.lib, self.name, ty)
        } else {
            WriteStruct::complete(self.writer).report_compiled(self.lib, self.name, ty)
        }
    }
}

impl<P: BuilderParent> DefineStruct for StructBuilder<P> {
    type Parent = P;

    fn define_field<T: StrictEncode + StrictDumb>(mut self, name: FieldName) -> Self {
        self.writer = DefineStruct::define_field::<T>(self.writer, name.clone());
        self._define_field::<T>(Some(name))
    }

    fn complete(self) -> P { self._complete_definition() }
}

impl<P: BuilderParent> WriteStruct for StructBuilder<P> {
    type Parent = P;

    fn write_field(mut self, name: FieldName, value: &impl StrictEncode) -> io::Result<Self> {
        self.writer = WriteStruct::write_field(self.writer, name.clone(), value)?;
        self._write_field(Some(name), value)
    }

    fn complete(self) -> P { self._complete_write() }
}

impl<P: BuilderParent> DefineTuple for StructBuilder<P> {
    type Parent = P;

    fn define_field<T: StrictEncode + StrictDumb>(mut self) -> Self {
        self.writer = DefineTuple::define_field::<T>(self.writer);
        self._define_field::<T>(None)
    }

    fn complete(self) -> P { self._complete_definition() }
}

impl<P: BuilderParent> WriteTuple for StructBuilder<P> {
    type Parent = P;

    fn write_field(mut self, value: &impl StrictEncode) -> io::Result<Self> {
        self.writer = WriteTuple::write_field(self.writer, value)?;
        self._write_field(None, value)
    }

    fn complete(self) -> P { self._complete_write() }
}

#[derive(Debug)]
pub struct UnionBuilder {
    lib: LibName,
    name: Option<TypeName>,
    variants: BTreeMap<u8, TranspileRef>,
    parent: LibBuilder,
    writer: UnionWriter<Sink>,
}

impl UnionBuilder {
    pub fn with<T: StrictSum>(parent: LibBuilder) -> Self {
        UnionBuilder {
            lib: libname!(T::STRICT_LIB_NAME),
            name: T::strict_name(),
            variants: empty!(),
            parent,
            writer: UnionWriter::with::<T>(StrictWriter::sink()),
        }
    }

    fn _fork(&self) -> Self {
        Self {
            lib: self.lib.clone(),
            name: self.name.clone(),
            variants: self.variants.clone(),
            parent: LibBuilder::new(self.lib.clone(), None),
            writer: UnionWriter::sink(),
        }
    }

    pub fn name(&self) -> &str { self.name.as_ref().map(|n| n.as_str()).unwrap_or("<unnamed>") }

    fn _define_variant(&mut self, name: &VariantName) {
        let ty = self.parent.last_compiled.clone().expect("no compiled type found");
        let tag = self.writer.tag_by_name(name);
        self.variants.insert(tag, ty);
    }

    fn _build_union(&self) -> Ty<TranspileRef> {
        let variants = self
            .writer
            .variants()
            .keys()
            .map(|variant| {
                let lib_ref = self.variants.get(&variant.tag).expect("type guarantees");
                (variant.clone(), lib_ref.clone())
            })
            .collect::<BTreeMap<_, _>>();
        let variants = UnionVariants::try_from(variants)
            .unwrap_or_else(|_| panic!("union '{}' has invalid number of variants", self.name()));
        Ty::Union(variants)
    }

    fn _build_enum(&self) -> Ty<TranspileRef> {
        let variants = self.writer.variants().keys().cloned().collect::<BTreeSet<_>>();
        let variants = EnumVariants::try_from(variants)
            .unwrap_or_else(|_| panic!("enum '{}' has invalid number of variants", self.name()));
        Ty::Enum(variants)
    }

    fn _complete_definition(mut self, ty: Ty<TranspileRef>) -> UnionBuilder {
        self.writer = DefineUnion::complete(self.writer);
        self.parent = self.parent.report_compiled(self.lib.clone(), self.name.clone(), ty);
        self
    }

    fn _complete_write(self, ty: Ty<TranspileRef>) -> LibBuilder {
        let _ = WriteUnion::complete(self.writer);
        self.parent.report_compiled(self.lib, self.name, ty)
    }

    fn from_split(writer: UnionWriter<Sink>, mut remnant: Self) -> Self {
        remnant.writer = writer;
        remnant
    }

    fn into_split(self) -> (UnionWriter<Sink>, UnionBuilder) {
        let (_, writer) = self.writer.into_write_split();
        let remnant = Self {
            lib: self.lib,
            name: self.name,
            variants: self.variants,
            parent: self.parent,
            writer: UnionWriter::sink(),
        };
        (writer, remnant)
    }
}

impl TypedParent for UnionBuilder {}
impl StrictParent<Sink> for UnionBuilder {
    type Remnant = UnionBuilder;
    fn from_write_split(_: StrictWriter<Sink>, remnant: Self::Remnant) -> Self { remnant }
    fn into_write_split(self) -> (StrictWriter<Sink>, Self::Remnant) {
        (StrictWriter::sink(), self)
    }
}
impl BuilderParent for UnionBuilder {
    fn compile_type<T: StrictEncode>(mut self, value: &T) -> (Self, TranspileRef) {
        let (parent, r) = self.parent.compile_type(value);
        self.parent = parent;
        (self, r)
    }
    fn report_compiled(
        mut self,
        lib: LibName,
        name: Option<TypeName>,
        ty: Ty<TranspileRef>,
    ) -> Self {
        self.parent = self.parent.report_compiled(lib, name, ty);
        self
    }
}

impl DefineEnum for UnionBuilder {
    type Parent = LibBuilder;
    type EnumWriter = Self;

    fn define_variant(mut self, name: VariantName) -> Self {
        self.parent = self.parent.report_compiled(self.lib.clone(), None, Ty::U8);
        self.writer = DefineEnum::define_variant(self.writer, name.clone());
        self._define_variant(&name);
        self
    }

    fn complete(self) -> Self::EnumWriter {
        let ty = self._build_enum();
        self._complete_definition(ty)
    }
}

impl WriteEnum for UnionBuilder {
    type Parent = LibBuilder;

    fn write_variant(mut self, name: VariantName) -> io::Result<Self> {
        self.parent = self.parent.report_compiled(self.lib.clone(), None, Ty::U8);
        self.writer = WriteEnum::write_variant(self.writer, name)?;
        Ok(self)
    }

    fn complete(self) -> LibBuilder {
        let ty = self._build_enum();
        self._complete_write(ty)
    }
}

impl DefineUnion for UnionBuilder {
    type Parent = LibBuilder;
    type TupleDefiner = StructBuilder<Self>;
    type StructDefiner = StructBuilder<Self>;
    type UnionWriter = Self;

    fn define_unit(mut self, name: VariantName) -> Self {
        self.parent = self.parent.report_compiled(self.lib.clone(), None, Ty::UNIT);
        self.writer = DefineUnion::define_unit(self.writer, name.clone());
        self._define_variant(&name);
        self
    }

    fn define_tuple(
        mut self,
        name: VariantName,
        inner: impl FnOnce(Self::TupleDefiner) -> Self,
    ) -> Self {
        let lib = self.lib.clone();
        let (writer, remnant) = self.into_split();
        let mut clone = remnant._fork();
        let mut lib_builder = clone.parent;
        let writer = writer.define_tuple(name.clone(), |d| {
            let (writer, _) = d.into_parent_split();
            let mut reconstructed_self = Self::from_split(writer, remnant);
            let struct_writer = StructWriter::unnamed(reconstructed_self, true);
            let struct_builder = StructBuilder::with(lib, None, struct_writer, true);
            reconstructed_self = inner(struct_builder);
            lib_builder = reconstructed_self.parent;
            reconstructed_self.writer
        });
        clone.parent = lib_builder;
        self = Self::from_split(writer, clone);
        self._define_variant(&name);
        self
    }

    fn define_struct(
        mut self,
        name: VariantName,
        inner: impl FnOnce(Self::StructDefiner) -> Self,
    ) -> Self {
        let lib = self.lib.clone();
        let (writer, remnant) = self.into_split();
        let mut clone = remnant._fork();
        let mut lib_builder = clone.parent;
        let writer = writer.define_struct(name.clone(), |d| {
            let (writer, _) = d.into_parent_split();
            let mut reconstructed_self = Self::from_split(writer, remnant);
            let struct_writer = StructWriter::unnamed(reconstructed_self, false);
            let struct_builder = StructBuilder::with(lib, None, struct_writer, true);
            reconstructed_self = inner(struct_builder);
            lib_builder = reconstructed_self.parent;
            reconstructed_self.writer
        });
        clone.parent = lib_builder;
        self = Self::from_split(writer, clone);
        self._define_variant(&name);
        self
    }

    fn complete(self) -> Self::UnionWriter {
        let ty = self._build_union();
        self._complete_definition(ty)
    }
}

impl WriteUnion for UnionBuilder {
    type Parent = LibBuilder;
    type TupleWriter = StructBuilder<Self>;
    type StructWriter = StructBuilder<Self>;

    fn write_unit(mut self, name: VariantName) -> io::Result<Self> {
        self.parent = self.parent.report_compiled(self.lib.clone(), None, Ty::UNIT);
        self.writer = WriteUnion::write_unit(self.writer, name)?;
        Ok(self)
    }

    fn write_tuple(
        mut self,
        name: VariantName,
        inner: impl FnOnce(Self::TupleWriter) -> io::Result<Self>,
    ) -> io::Result<Self> {
        let lib = self.lib.clone();
        let (writer, remnant) = self.into_split();
        let mut clone = remnant._fork();
        let mut lib_builder = clone.parent;
        let writer = writer.write_tuple(name, |d| {
            let (writer, _) = d.into_parent_split();
            let mut reconstructed_self = Self::from_split(writer, remnant);
            let struct_writer = StructWriter::unnamed(reconstructed_self, true);
            let struct_builder = StructBuilder::with(lib, None, struct_writer, false);
            reconstructed_self = inner(struct_builder)?;
            lib_builder = reconstructed_self.parent;
            Ok(reconstructed_self.writer)
        })?;
        clone.parent = lib_builder;
        self = Self::from_split(writer, clone);
        Ok(self)
    }

    fn write_struct(
        mut self,
        name: VariantName,
        inner: impl FnOnce(Self::StructWriter) -> io::Result<Self>,
    ) -> io::Result<Self> {
        let lib = self.lib.clone();
        let (writer, remnant) = self.into_split();
        let mut clone = remnant._fork();
        let mut lib_builder = clone.parent;
        let writer = writer.write_struct(name, |d| {
            let (writer, _) = d.into_parent_split();
            let mut reconstructed_self = Self::from_split(writer, remnant);
            let struct_writer = StructWriter::unnamed(reconstructed_self, false);
            let struct_builder = StructBuilder::with(lib, None, struct_writer, false);
            reconstructed_self = inner(struct_builder)?;
            lib_builder = reconstructed_self.parent;
            Ok(reconstructed_self.writer)
        })?;
        clone.parent = lib_builder;
        self = Self::from_split(writer, clone);
        Ok(self)
    }

    fn complete(self) -> LibBuilder {
        let ty = self._build_union();
        self._complete_write(ty)
    }
}
