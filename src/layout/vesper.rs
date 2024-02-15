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

use std::fmt;
use std::fmt::{Display, Formatter};
use std::ops::Range;

use amplify::num::u24;
use encoding::{Ident, Sizing};
use vesper::{AttrVal, Attribute, Expression, Predicate, TExpr};

use crate::{Cls, SymbolicLib};

#[derive(Wrapper, WrapperMut, Clone, Eq, PartialEq, Debug, From)]
#[wrapper(Deref)]
#[wrapper_mut(DerefMut)]
pub struct VesperLib(Vec<VesperType>);

impl SymbolicLib {
    pub fn to_vesper(&self) -> VesperLib {
        let mut types = Vec::with_capacity(self.roots().len());
        for name in self.roots() {
            let ty = self.types().get(name).expect("symbolic lib inconsistency");
            types.push(ty.to_vesper())
        }
        VesperLib::from(types)
    }
}

pub type VesperType = TExpr<Pred>;

#[derive(Clone, Eq, PartialEq, Hash, Debug, Display)]
#[display(lowercase)]
pub enum Pred {
    /// Type alias
    Is,
    Tuple,
    Rec,
    Enum,
    Union,
    // Composites
    Char,
    Str,
    Ascii,
    Bytes,
    // Collections:
    Array,
    List,
    Set,
    Map,
}

impl Predicate for Pred {
    type Attr = Attr;
}

impl From<Cls> for Pred {
    fn from(cls: Cls) -> Self {
        match cls {
            Cls::Primitive => Pred::Is,
            Cls::Unicode => Pred::Str,
            Cls::AsciiStr => Pred::Ascii,
            Cls::Enum => Pred::Enum,
            Cls::Union => Pred::Union,
            Cls::Struct => Pred::Rec,
            Cls::Tuple => Pred::Tuple,
            Cls::Array => Pred::Array,
            Cls::List => Pred::List,
            Cls::Set => Pred::Set,
            Cls::Map => Pred::Map,
        }
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug, Display)]
#[display(inner)]
pub enum AttrExpr {
    Tag(u8),
    EnumVariant(u8),
    Len(u16),
    LenRange(LenRange),
}

impl Expression for AttrExpr {}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum Attr {
    TypeName(Ident),
    Wrapped(Option<Ident>),
    Option,
    Tag(u8),
    AsciiEnum(Ident),
    EnumVariant(u8, Ident),
    Len(u16),
    LenRange(LenRange),
}

impl Attribute for Attr {
    type Expression = AttrExpr;

    fn name(&self) -> Option<Ident> {
        match self {
            Attr::TypeName(_) => None,
            Attr::Wrapped(name) if name.is_some() => ident!("aka"),
            Attr::Wrapped(_) => None,
            Attr::Option => None,
            Attr::Tag(_) => ident!("tag"),
            Attr::Len(_) => ident!("len"),
            Attr::LenRange(_) => ident!("len"),
            Attr::AsciiEnum(_) => ident!("charset"),
            Attr::EnumVariant(_, name) => Some(name.clone()),
        }
    }

    fn value(&self) -> AttrVal<Self::Expression> {
        match self {
            Attr::TypeName(tn) => AttrVal::Ident(tn.clone()),
            Attr::Wrapped(name) => AttrVal::Ident(name.clone().unwrap_or(ident!("wrapped"))),
            Attr::Option => AttrVal::Ident(ident!("option")),
            Attr::Tag(tag) => AttrVal::Expr(AttrExpr::Tag(*tag)),
            Attr::Len(len) => AttrVal::Expr(AttrExpr::Len(*len)),
            Attr::LenRange(range) => AttrVal::Expr(AttrExpr::LenRange(range.clone())),
            Attr::AsciiEnum(name) => AttrVal::Ident(name.clone()),
            Attr::EnumVariant(pos, _) => AttrVal::Expr(AttrExpr::Tag(*pos)),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct LenRange(Range<u64>);

impl From<Sizing> for LenRange {
    #[inline]
    fn from(sizing: Sizing) -> Self { Self(sizing.min..sizing.max) }
}

impl Display for LenRange {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match (self.0.start, self.0.end) {
            (min, max) if max == u8::MAX as u64 => write!(f, "{min}..MAX8"),
            (min, max) if max == u16::MAX as u64 => write!(f, "{min}..MAX16"),
            (min, max) if max == u24::MAX.into_u64() => write!(f, "{min}..MAX24"),
            (min, max) if max == u32::MAX as u64 => write!(f, "{min}..MAX32"),
            // TODO: Add more numbers
            (min, u64::MAX) => write!(f, "{min}..MAX64"),
            (min, max) => write!(f, "{min}..{max}"),
        }
    }
}
