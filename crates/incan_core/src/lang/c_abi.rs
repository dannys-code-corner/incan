//! Typed vocabulary for the first checked C ABI binding slice.
//!
//! The parser and vocabulary desugarer stay generic. This module is the single language-level source for the C
//! spellings that the typechecker recognizes after vocabulary lowering. Resource ownership, output slots, native
//! artifact classes, and shim policy deliberately belong to later RFC 116 slices.

/// Stable fields accepted by an `@c.binding` descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingArgumentId {
    /// Literal C header spelling.
    Header,
    /// Logical native link capability.
    Link,
}

/// Stable declarative member kinds accepted by the checked C foundation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingMemberId {
    /// A non-executable raw C function declaration.
    Symbol,
    /// A target-verified C enum carrier.
    Enum,
    /// A target-verified by-value C structure.
    Struct,
}

/// Stable fields accepted by a plain C structure declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlainStructArgumentId {
    /// Exact native C struct tag or typedef spelling.
    Native,
}

/// Stable fields accepted by a C symbol declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolArgumentId {
    /// Exact native C symbol spelling.
    Native,
}

/// Stable exact scalar representations accepted by the checked C foundation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarTypeId {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Size,
    CChar,
    CInt,
}

/// Resolve an exact C scalar spelling to its stable identifier.
pub fn scalar_type_from_str(name: &str) -> Option<ScalarTypeId> {
    match name {
        "c.i8" => Some(ScalarTypeId::I8),
        "c.u8" => Some(ScalarTypeId::U8),
        "c.i16" => Some(ScalarTypeId::I16),
        "c.u16" => Some(ScalarTypeId::U16),
        "c.i32" => Some(ScalarTypeId::I32),
        "c.u32" => Some(ScalarTypeId::U32),
        "c.i64" => Some(ScalarTypeId::I64),
        "c.u64" => Some(ScalarTypeId::U64),
        "c.Size" => Some(ScalarTypeId::Size),
        "c.c_char" => Some(ScalarTypeId::CChar),
        "c.c_int" => Some(ScalarTypeId::CInt),
        _ => None,
    }
}

/// Return the canonical source spelling for an exact C scalar representation.
pub const fn scalar_type_as_str(id: ScalarTypeId) -> &'static str {
    match id {
        ScalarTypeId::I8 => "c.i8",
        ScalarTypeId::U8 => "c.u8",
        ScalarTypeId::I16 => "c.i16",
        ScalarTypeId::U16 => "c.u16",
        ScalarTypeId::I32 => "c.i32",
        ScalarTypeId::U32 => "c.u32",
        ScalarTypeId::I64 => "c.i64",
        ScalarTypeId::U64 => "c.u64",
        ScalarTypeId::Size => "c.Size",
        ScalarTypeId::CChar => "c.c_char",
        ScalarTypeId::CInt => "c.c_int",
    }
}

/// Stable pointer constructors accepted by the checked C foundation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerConstructorId {
    /// A read-only C pointer.
    ConstPtr,
    /// A mutable C pointer.
    MutPtr,
}

/// Return the canonical source spelling for an admitted pointer constructor.
pub const fn pointer_constructor_as_str(id: PointerConstructorId) -> &'static str {
    match id {
        PointerConstructorId::ConstPtr => "c.ConstPtr",
        PointerConstructorId::MutPtr => "c.MutPtr",
    }
}

/// Source path that must be imported for C binding vocabulary and descriptor use.
pub const INTEROP_NAMESPACE_PATH: &[&str] = &["std", "interop", "c"];

/// Ordinary marker base that establishes a class as a binding declaration.
pub const BINDING_DECLARATION_BASE: &str = "BindingDeclaration";

/// Canonical Incan spelling for a C `void` return.
pub const VOID_TYPE_SPELLING: &str = "None";

/// Return whether an Incan type spelling represents a C `void` return.
pub fn is_void_type_spelling(name: &str) -> bool {
    name == VOID_TYPE_SPELLING
}

/// Resolve a C binding descriptor field to its stable identifier.
pub fn binding_argument_from_str(name: &str) -> Option<BindingArgumentId> {
    match name {
        "header" => Some(BindingArgumentId::Header),
        "link" => Some(BindingArgumentId::Link),
        _ => None,
    }
}

/// Resolve a C binding member spelling to its stable identifier.
pub fn binding_member_from_str(name: &str) -> Option<BindingMemberId> {
    match name {
        "symbol" => Some(BindingMemberId::Symbol),
        "enum" => Some(BindingMemberId::Enum),
        "struct" => Some(BindingMemberId::Struct),
        _ => None,
    }
}

/// Resolve a plain C structure field to its stable identifier.
pub fn plain_struct_argument_from_str(name: &str) -> Option<PlainStructArgumentId> {
    (name == "native").then_some(PlainStructArgumentId::Native)
}

/// Resolve a raw C symbol field to its stable identifier.
pub fn symbol_argument_from_str(name: &str) -> Option<SymbolArgumentId> {
    (name == "native").then_some(SymbolArgumentId::Native)
}

/// Stable C link capability names admitted by the descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkCapabilityId {
    /// A named system capability that a later target resolver must select explicitly.
    SystemLibrary,
}

/// Resolve a C namespace member name to a supported link capability.
pub fn link_capability_from_str(name: &str) -> Option<LinkCapabilityId> {
    (name == "system_library").then_some(LinkCapabilityId::SystemLibrary)
}

/// Return whether path segments name the imported C interop namespace.
pub fn is_interop_namespace_path<'a>(segments: impl IntoIterator<Item = &'a str>) -> bool {
    segments.into_iter().eq(INTEROP_NAMESPACE_PATH.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::{
        BINDING_DECLARATION_BASE, BindingArgumentId, BindingMemberId, LinkCapabilityId, PlainStructArgumentId,
        PointerConstructorId, ScalarTypeId, SymbolArgumentId, binding_argument_from_str, binding_member_from_str,
        is_interop_namespace_path, is_void_type_spelling, link_capability_from_str, plain_struct_argument_from_str,
        pointer_constructor_as_str, scalar_type_as_str, scalar_type_from_str, symbol_argument_from_str,
    };

    #[test]
    fn checked_c_foundation_vocabulary_is_canonical() {
        assert_eq!(binding_argument_from_str("header"), Some(BindingArgumentId::Header));
        assert_eq!(binding_member_from_str("symbol"), Some(BindingMemberId::Symbol));
        assert_eq!(binding_member_from_str("struct"), Some(BindingMemberId::Struct));
        assert_eq!(
            plain_struct_argument_from_str("native"),
            Some(PlainStructArgumentId::Native)
        );
        assert_eq!(symbol_argument_from_str("native"), Some(SymbolArgumentId::Native));
        assert_eq!(scalar_type_from_str("c.i32"), Some(ScalarTypeId::I32));
        assert_eq!(scalar_type_as_str(ScalarTypeId::CInt), "c.c_int");
        assert_eq!(pointer_constructor_as_str(PointerConstructorId::ConstPtr), "c.ConstPtr");
        assert_eq!(
            link_capability_from_str("system_library"),
            Some(LinkCapabilityId::SystemLibrary)
        );
        assert!(is_interop_namespace_path(["std", "interop", "c"]));
        assert_eq!(BINDING_DECLARATION_BASE, "BindingDeclaration");
        assert!(is_void_type_spelling("None"));
    }
}
