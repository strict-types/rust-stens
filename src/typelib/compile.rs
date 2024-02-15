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

use std::collections::BTreeMap;

use encoding::LibName;
use strict_encoding::TypeName;

use crate::typelib::{Dependency, ExternTypes, InlineRef, InlineRef1, InlineRef2, LibRef};
use crate::{SemId, Translate, TranspileError, TranspileRef, Ty};

pub type TypeIndex = BTreeMap<TypeName, SemId>;

#[deprecated(since = "1.3.0", note = "use CompileError")]
pub type TranslateError = CompileError;

#[derive(Clone, Eq, PartialEq, Debug, Display, Error, From)]
#[display(doc_comments)]
pub enum CompileError {
    /// a different type with name `{0}` is already present
    DuplicateName(TypeName),

    /// type `{unknown}` referenced inside `{within}` is not known
    UnknownType {
        unknown: TypeName,
        within: Ty<TranspileRef>,
    },

    /// return type indicating continue operation
    Continue,

    /// dependency {0} is already present in the library
    DuplicatedDependency(Dependency),

    /// too deep type nesting for type {2} inside {0}, path {1}
    NestedInline(TypeName, String, String),

    /// unknown library {0}
    UnknownLib(LibName),

    /// too many dependencies.
    TooManyDependencies,

    /// too many types
    TooManyTypes,

    /// library `{0}` contains too many types.
    LibTooLarge(LibName),
}

impl From<TranspileError> for CompileError {
    fn from(err: TranspileError) -> Self {
        match err {
            TranspileError::UnknownType { unknown, within } => {
                Self::UnknownType { unknown, within }
            }
            TranspileError::UnknownLib(lib) => Self::UnknownLib(lib),
            TranspileError::TooManyDependencies => Self::TooManyDependencies,
            TranspileError::TooManyTypes => Self::TooManyTypes,
            TranspileError::TooManyRootTypes => Self::TooManyTypes,
            TranspileError::LibTooLarge(lib) => Self::LibTooLarge(lib),
        }
    }
}

pub struct NestedContext {
    pub top_name: TypeName,
    pub index: TypeIndex,
    pub extern_types: ExternTypes,
    pub stack: Vec<String>,
}

impl Translate<LibRef> for TranspileRef {
    type Context = ();
    type Builder = NestedContext;
    type Error = CompileError;

    fn translate(
        self,
        builder: &mut Self::Builder,
        ctx: &Self::Context,
    ) -> Result<LibRef, Self::Error> {
        match self {
            TranspileRef::Embedded(ty) => {
                builder.stack.push(ty.cls().to_string());
                let res = ty.translate(builder, ctx).map(LibRef::Inline);
                builder.stack.pop();
                res
            }
            TranspileRef::Named(name) => {
                let id = builder.index.get(&name).ok_or(CompileError::Continue)?;
                Ok(LibRef::Named(*id))
            }
            TranspileRef::Extern(ext) => Ok(LibRef::Extern(ext.into())),
        }
    }
}

impl Translate<InlineRef> for TranspileRef {
    type Context = ();
    type Builder = NestedContext;
    type Error = CompileError;

    fn translate(
        self,
        builder: &mut Self::Builder,
        ctx: &Self::Context,
    ) -> Result<InlineRef, Self::Error> {
        match self {
            TranspileRef::Embedded(ty) => {
                builder.stack.push(ty.cls().to_string());
                let res = ty.translate(builder, ctx).map(InlineRef::Inline);
                builder.stack.pop();
                res
            }
            TranspileRef::Named(name) => {
                let id = builder.index.get(&name).ok_or(CompileError::Continue)?;
                Ok(InlineRef::Named(*id))
            }
            TranspileRef::Extern(ext) => Ok(InlineRef::Extern(ext.into())),
        }
    }
}

impl Translate<InlineRef1> for TranspileRef {
    type Context = ();
    type Builder = NestedContext;
    type Error = CompileError;

    fn translate(
        self,
        builder: &mut Self::Builder,
        ctx: &Self::Context,
    ) -> Result<InlineRef1, Self::Error> {
        match self {
            TranspileRef::Embedded(ty) => {
                builder.stack.push(ty.cls().to_string());
                let res = ty.translate(builder, ctx).map(InlineRef1::Inline);
                builder.stack.pop();
                res
            }
            TranspileRef::Named(name) => {
                let id = builder.index.get(&name).ok_or(CompileError::Continue)?;
                Ok(InlineRef1::Named(*id))
            }
            TranspileRef::Extern(ext) => Ok(InlineRef1::Extern(ext.into())),
        }
    }
}

impl Translate<InlineRef2> for TranspileRef {
    type Context = ();
    type Builder = NestedContext;
    type Error = CompileError;

    fn translate(
        self,
        builder: &mut Self::Builder,
        _ctx: &Self::Context,
    ) -> Result<InlineRef2, Self::Error> {
        match self {
            TranspileRef::Embedded(_ty) => {
                let mut path = builder.stack.clone();
                let name = path.pop().unwrap_or_else(|| s!("<unnamed>"));
                Err(CompileError::NestedInline(builder.top_name.clone(), path.join("."), name))
            }
            TranspileRef::Named(name) => {
                let id = builder.index.get(&name).ok_or(CompileError::Continue)?;
                Ok(InlineRef2::Named(*id))
            }
            TranspileRef::Extern(ext) => Ok(InlineRef2::Extern(ext.into())),
        }
    }
}
