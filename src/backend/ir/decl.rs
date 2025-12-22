//! IR declaration definitions

use super::{IrSpan, IrStmt, IrType, Mutability};

/// An IR declaration
#[derive(Debug, Clone)]
pub struct IrDecl {
    pub kind: IrDeclKind,
    pub span: IrSpan,
}

impl IrDecl {
    pub fn new(kind: IrDeclKind) -> Self {
        Self {
            kind,
            span: IrSpan::default(),
        }
    }

    pub fn with_span(mut self, span: IrSpan) -> Self {
        self.span = span;
        self
    }
}

/// Declaration kinds
#[derive(Debug, Clone)]
pub enum IrDeclKind {
    /// Function definition
    Function(IrFunction),

    /// Struct definition
    Struct(IrStruct),

    /// Enum definition
    Enum(IrEnum),

    /// Trait definition
    Trait(IrTrait),

    /// Type alias
    TypeAlias { name: String, ty: IrType },

    /// Constant
    Const {
        name: String,
        ty: IrType,
        value: super::IrExpr,
    },

    /// Import (preserved for codegen)
    Import {
        path: Vec<String>,
        alias: Option<String>,
        /// Specific items being imported (for `from x import a, b`)
        items: Vec<IrImportItem>,
    },

    /// Impl block for methods on structs/enums
    Impl(IrImpl),
}

/// An item in a from ... import statement
#[derive(Debug, Clone)]
pub struct IrImportItem {
    pub name: String,
    pub alias: Option<String>,
}

/// IR trait definition
#[derive(Debug, Clone)]
pub struct IrTrait {
    pub name: String,
    /// Methods with default implementations
    pub methods: Vec<IrFunction>,
    pub visibility: Visibility,
}

/// IR impl block definition
#[derive(Debug, Clone)]
pub struct IrImpl {
    /// The type being implemented on (e.g., "Dog")
    pub target_type: String,
    /// The trait being implemented, if any
    pub trait_name: Option<String>,
    /// Methods in this impl block
    pub methods: Vec<IrFunction>,
}

/// IR function definition
#[derive(Debug, Clone)]
pub struct IrFunction {
    pub name: String,
    pub params: Vec<FunctionParam>,
    pub return_type: IrType,
    pub body: Vec<IrStmt>,
    pub is_async: bool,
    pub visibility: Visibility,
    /// Type parameters for generics
    pub type_params: Vec<String>,
}

/// Function parameter
#[derive(Debug, Clone)]
pub struct FunctionParam {
    pub name: String,
    pub ty: IrType,
    pub mutability: Mutability,
    pub is_self: bool,
}

/// IR struct definition
#[derive(Debug, Clone)]
pub struct IrStruct {
    pub name: String,
    pub fields: Vec<StructField>,
    pub derives: Vec<String>,
    pub visibility: Visibility,
    /// Type parameters for generics
    pub type_params: Vec<String>,
}

/// Struct field
#[derive(Debug, Clone)]
pub struct StructField {
    pub name: String,
    pub ty: IrType,
    pub visibility: Visibility,
}

/// IR enum definition
#[derive(Debug, Clone)]
pub struct IrEnum {
    pub name: String,
    pub variants: Vec<EnumVariant>,
    pub derives: Vec<String>,
    pub visibility: Visibility,
    /// Type parameters for generics
    pub type_params: Vec<String>,
}

/// Enum variant
#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub fields: VariantFields,
}

/// Variant fields (unit, tuple, or struct)
#[derive(Debug, Clone)]
pub enum VariantFields {
    Unit,
    Tuple(Vec<IrType>),
    Struct(Vec<StructField>),
}

/// Visibility modifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Private,
    Public,
    Crate,
}

impl Visibility {
    pub fn rust_keyword(&self) -> &'static str {
        match self {
            Visibility::Private => "",
            Visibility::Public => "pub ",
            Visibility::Crate => "pub(crate) ",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visibility_rust_keyword() {
        assert_eq!(Visibility::Private.rust_keyword(), "");
        assert_eq!(Visibility::Public.rust_keyword(), "pub ");
        assert_eq!(Visibility::Crate.rust_keyword(), "pub(crate) ");
    }
}
