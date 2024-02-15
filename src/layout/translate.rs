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

use amplify::confinement::Confined;
use encoding::constants::UNIT;
use encoding::Ident;

use super::vesper::{Attr, Pred, VesperType};
use crate::ast::ItemCase;
use crate::typesys::{NestedCase, TypeInfo};
use crate::{TranspileRef, Ty};

impl Ty<TranspileRef> {
    pub(super) fn to_vesper(&self, subject: Ident) -> VesperType {
        let predicate = match self {
            Ty::Primitive(_) => {}
            Ty::UnicodeChar => {}
            Ty::Enum(_) => {}
            Ty::Union(_) => {}
            Ty::Tuple(_) => {}
            Ty::Struct(_) => {}
            Ty::Array(_, _) => {}
            Ty::List(_, _) => {}
            Ty::Set(_, _) => {}
            Ty::Map(_, _, _) => {}
        };

        VesperType {
            subject,
            predicate,
            attributes,
            content,
            comment,
        }
    }
}

impl TypeInfo {
    pub(super) fn to_vesper(&self) -> VesperType {
        let TypeInfo {
            ty,
            fqn,
            item,
            nested,
            ..
        } = self;

        let mut attributes = vec![];
        let mut comment = None;
        let name = fqn.as_ref().map(|f| f.name.clone()).unwrap_or_else(|| tn!("_"));
        let fqn = fqn.as_ref().map(|f| f.name.to_string());
        let subject = match item {
            Some(ItemCase::UnnamedField(pos)) => {
                if name.as_str() == "_" {
                    comment = fqn;
                    Ident::from_uint(*pos)
                } else {
                    name.into_ident()
                }
            }
            Some(ItemCase::NamedField(_, ref fname)) => {
                comment = fqn;
                fname.to_ident()
            }
            Some(ItemCase::UnionVariant(_, ref vname)) => {
                comment = fqn;
                vname.to_ident()
            }
            Some(ItemCase::ListItem) | Some(ItemCase::SetItem) if fqn.is_none() => {
                ident!("element")
            }
            Some(ItemCase::MapKey) if fqn.is_some() => {
                comment = Some(s!("mapped from"));
                name.into_ident()
            }
            Some(ItemCase::MapKey) => ident!("key"),
            Some(ItemCase::MapValue) if fqn.is_some() => {
                comment = Some(s!("mapped to"));
                name.into_ident()
            }
            Some(ItemCase::MapValue) => ident!("value"),
            _ => name.into_ident(),
        };
        let mut predicate = ty.cls().into();
        match ty {
            Ty::Primitive(prim) if *prim == UNIT => {
                attributes.push(Attr::TypeName(tn!("Unit")));
            }
            Ty::Primitive(prim) => {
                attributes.push(Attr::TypeName(tn!("{}", prim)));
            }
            Ty::Array(_, len) => attributes.push(Attr::Len(*len)),
            Ty::List(_, sizing) | Ty::Set(_, sizing) | Ty::Map(_, _, sizing) => {
                attributes.push(Attr::LenRange((*sizing).into()));
            }
            _ => {}
        }
        if ty.is_char_enum() {
            predicate = Pred::Char;
        } else if ty.is_byte_array() {
            predicate = Pred::Bytes;
        }
        for case in nested {
            match case {
                NestedCase::AsciiStr(fqn) => {
                    predicate = Pred::Ascii;
                    if let Some(fqn) = fqn {
                        attributes.push(Attr::AsciiEnum(fqn.name.to_ident()));
                    }
                }
                NestedCase::ByteStr => {
                    predicate = Pred::Bytes;
                }
                NestedCase::UniStr => {
                    predicate = Pred::Str;
                }
                NestedCase::NewType(fqn) => {
                    attributes.push(Attr::Wrapped(fqn.as_ref().map(|f| f.name.to_ident())));
                }
                NestedCase::Option => {
                    attributes.push(Attr::Option);
                }
            }
        }

        if let Ty::Enum(variants) = ty {
            for var in variants {
                attributes.push(Attr::EnumVariant(var.tag, var.name.to_ident()))
            }
        }
        if let Some(ItemCase::UnionVariant(ref pos, _)) = item {
            attributes.push(Attr::Tag(*pos));
        }

        VesperType {
            subject,
            predicate,
            attributes: Confined::from_collection_unsafe(attributes),
            content: none!(),
            comment,
        }
    }
}
