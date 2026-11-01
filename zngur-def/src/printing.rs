use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IDLItem {
    Function,
    Type,
    TypeWellknownTrait,
    TypeLayout,
    TypeConstructor,
    TypeField,
    TypeVariant,
    TypeVariantField,
    TypeVariantConstructor,
    TypeMethod,
    Trait,
    ExternFn,
    ExternImpl,
}

/// Prints the `.zng` IDL to an output writer.
/// Uses its own trait `WriteIDL` over `std::fmt::Display` to control:
///   - the output of item types
///   - formatting
#[derive(Debug)]
pub struct IDLPrinter<'a, W: Write> {
    enabled_items: Vec<IDLItem>,
    current_indent: usize,
    out: &'a mut W,
    no_std: bool,
}

pub const DEFAULT_IDL_ITEMS: [IDLItem; 13] = [
    IDLItem::Function,
    IDLItem::Type,
    IDLItem::TypeWellknownTrait,
    IDLItem::TypeLayout,
    IDLItem::TypeConstructor,
    IDLItem::TypeField,
    IDLItem::TypeVariant,
    IDLItem::TypeVariantField,
    IDLItem::TypeVariantConstructor,
    IDLItem::TypeMethod,
    IDLItem::Trait,
    IDLItem::ExternFn,
    IDLItem::ExternImpl,
];

impl<'a, W: Write> IDLPrinter<'a, W> {
    pub fn new(out: &'a mut W) -> Self {
        IDLPrinter {
            enabled_items: Vec::from(DEFAULT_IDL_ITEMS),
            current_indent: 0,
            out,
            no_std: false,
        }
    }

    pub fn enable_item(&mut self, item: IDLItem) -> bool {
        if self.enabled_items.contains(&item) {
            return false;
        }
        self.enabled_items.push(item);
        true
    }

    pub fn disable_item(&mut self, item: &IDLItem) -> bool {
        if let Some(pos) = self.enabled_items.iter().position(|i| i == item) {
            self.enabled_items.remove(pos);
            return true;
        }
        false
    }

    pub fn enable_items(&mut self, items: &[IDLItem]) -> bool {
        items.iter().copied().any(|item| self.enable_item(item))
    }

    pub fn disable_items(&mut self, items: &[IDLItem]) -> bool {
        items.into_iter().any(|item| self.disable_item(item))
    }

    pub fn enabled(&self, item: &IDLItem) -> bool {
        self.enabled_items.contains(item)
    }

    pub fn indent(&mut self) {
        self.current_indent += 1;
    }

    pub fn dedent(&mut self) {
        self.current_indent -= 1;
    }

    pub fn write_indent(&mut self) -> std::io::Result<()> {
        let indent = " ".repeat(self.current_indent * 4);
        write!(&mut self.out, "{indent}")
    }

    pub fn write_idl<I: WriteIDL + ?Sized>(&mut self, item: &I) -> std::io::Result<()> {
        item.write_idl(self)
    }

    pub fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> std::io::Result<()> {
        self.out.write_fmt(args)
    }

    /// Retained for API compatibility; not used by built-in `WriteIDL` implementations.
    pub fn std_path(&self) -> &'static str {
        if self.no_std { "::core" } else { "::std" }
    }
}

pub trait WriteIDL {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()>;
}

impl<T: WriteIDL + ?Sized> WriteIDL for Box<T> {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        p.write_idl::<T>(&**self)
    }
}

impl<T: WriteIDL> WriteIDL for Vec<T> {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        for (i, item) in self.iter().enumerate() {
            if i > 0 && i < self.len() {
                write!(p, ", ")?;
            }
            p.write_idl(item)?;
        }
        Ok(())
    }
}

impl<T: WriteIDL> WriteIDL for Option<T> {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        match self {
            Self::Some(t) => p.write_idl(t),
            Self::None => Ok(()),
        }
    }
}

impl WriteIDL for crate::RustType {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        match self {
            Self::Primitive(prim) => write!(p, "{prim}"),
            Self::Ref(m, ty) => {
                match m {
                    crate::Mutability::Not => {
                        write!(p, "&")?;
                    }
                    crate::Mutability::Mut => {
                        write!(p, "&mut ")?;
                    }
                }
                p.write_idl(ty)
            }
            Self::Raw(m, ty) => {
                match m {
                    crate::Mutability::Not => {
                        write!(p, "*const ")?;
                    }
                    crate::Mutability::Mut => {
                        write!(p, "*mut ")?;
                    }
                }
                p.write_idl(ty)
            }
            Self::Boxed(ty) => {
                write!(p, "Box<")?;
                p.write_idl(ty)?;
                write!(p, ">")
            }
            Self::Slice(ty) => {
                write!(p, "[")?;
                p.write_idl(ty)?;
                write!(p, "]")
            }
            Self::Dyn(trt, marker_bounds) => {
                write!(p, "dyn ")?;
                p.write_idl(trt)?;
                for bound in marker_bounds {
                    write!(p, " + {bound}")?;
                }
                Ok(())
            }
            Self::Impl(trt, marker_bounds) => {
                write!(p, "impl ")?;
                p.write_idl(trt)?;
                for bound in marker_bounds {
                    write!(p, " + {bound}")?;
                }
                Ok(())
            }
            Self::Tuple(tys) => {
                write!(p, "(")?;
                p.write_idl(tys)?;
                write!(p, ")")
            }
            Self::Adt(pg) => p.write_idl(pg),
            Self::TypeVar(var) => p.write_idl(var),
            Self::Cpp(segs) => write!(p, "c++::{}", segs.join("::")),
        }
    }
}

impl WriteIDL for crate::RustTrait {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        match self {
            Self::Normal(path_and_generics) => p.write_idl(path_and_generics),
            Self::Fn {
                name,
                inputs,
                output,
            } => {
                write!(p, "{name}(")?;
                p.write_idl(inputs)?;
                write!(p, ")")?;
                if **output != crate::RustType::UNIT {
                    write!(p, " -> ")?;
                    p.write_idl(output)?;
                }
                Ok(())
            }
        }
    }
}

impl WriteIDL for crate::RustPathAndGenerics {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        let (first, rest): (_, &[String]) = match &self.path[..] {
            [first] => (first, &[]),
            [first, rest @ ..] => (first, rest),
            _ => {
                // probably an error, a path with no segments
                // return early to not print any generics because that would not be valid
                return Ok(());
            }
        };
        if first != "crate" {
            write!(p, "::")?;
        }
        write!(p, "{first}")?;
        for segment in rest {
            write!(p, "::{segment}")?;
        }

        if (self.generics.len() + self.named_generics.len()) > 0 {
            write!(p, "<")?;
            let mut generic_count = 0;
            for generic in &self.generics {
                if generic_count > 0 {
                    write!(p, ", ")?;
                }
                p.write_idl(generic)?;
                generic_count += 1;
            }
            for (name, generic) in &self.named_generics {
                if generic_count > 0 {
                    write!(p, ", ")?;
                }
                write!(p, "{name} = ")?;
                p.write_idl(generic)?;
                generic_count += 1;
            }
            write!(p, ">")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::TypeVar {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        write!(p, "{}", self.0)
    }
}

impl WriteIDL for crate::ZngurSpec {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        // Each top-level section is separated from the previous one by a blank line.
        let mut need_separator = false;
        let mut separate = |p: &mut IDLPrinter<W>| -> std::io::Result<()> {
            if need_separator {
                writeln!(p)?;
            }
            need_separator = true;
            Ok(())
        };
        if !self.imported_modules.is_empty() {
            separate(p)?;
            for import in &self.imported_modules {
                writeln!(p, r#"import "{}";"#, import.path.display())?;
            }
        }
        if self.convert_panic_to_exception.0 {
            separate(p)?;
            writeln!(p, "#convert_panic_to_exception")?;
        }
        if !self.additional_includes.0.is_empty() {
            separate(p)?;
            let includes = &self.additional_includes.0;
            // The lexer has no escapes, so use a raw string with enough `#`s
            // that its terminator can't appear in the contents.
            let hashes = "#".repeat(
                (0..)
                    .find(|&n| !includes.contains(&format!("\"{}", "#".repeat(n))))
                    .unwrap(),
            );
            writeln!(
                p,
                "#cpp_additional_includes r{hashes}\"{includes}\"{hashes}"
            )?;
        }
        if !self.types.is_empty() && p.enabled(&IDLItem::Type) {
            separate(p)?;
            for (i, ty) in self.types.iter().enumerate() {
                if i > 0 {
                    writeln!(p)?;
                }
                p.write_indent()?;
                p.write_idl(ty)?;
            }
        }
        if !self.funcs.is_empty() && p.enabled(&IDLItem::Function) {
            separate(p)?;
            // Consecutive functions in the same module share a `mod` block.
            let mut open_mod: Option<&[String]> = None;
            for func in &self.funcs {
                let Some((name, parent)) = func.path.path.split_last() else {
                    // A function with an empty path can't be expressed.
                    continue;
                };
                if open_mod != Some(parent) {
                    if let Some(prev) = open_mod {
                        write_mod_close(p, prev)?;
                    }
                    write_mod_open(p, parent)?;
                    open_mod = Some(parent);
                }
                write_fn_decl(p, name, func)?;
            }
            if let Some(prev) = open_mod {
                write_mod_close(p, prev)?;
            }
        }
        if !self.traits.is_empty() && p.enabled(&IDLItem::Trait) {
            separate(p)?;
            for (i, trt) in self.traits.values().enumerate() {
                if i > 0 {
                    writeln!(p)?;
                }
                p.write_idl(trt)?;
            }
        }
        let has_extern_fns = !self.extern_cpp_funcs.is_empty() && p.enabled(&IDLItem::ExternFn);
        let has_extern_impls = !self.extern_cpp_impls.is_empty() && p.enabled(&IDLItem::ExternImpl);
        if has_extern_fns || has_extern_impls {
            separate(p)?;
            p.write_indent()?;
            writeln!(p, r#"extern "C++" {{"#)?;
            p.indent();
            if has_extern_fns {
                writeln!(p)?;
                for extern_fn in &self.extern_cpp_funcs {
                    p.write_indent()?;
                    p.write_idl(extern_fn)?;
                }
            }
            if has_extern_impls {
                if has_extern_fns {
                    writeln!(p)?;
                }
                for extern_impl in &self.extern_cpp_impls {
                    p.write_indent()?;
                    p.write_idl(extern_impl)?;
                }
            }
            writeln!(p)?;
            p.dedent();
            p.write_indent()?;
            writeln!(p, "}}")?;
        }
        Ok(())
    }
}

/// Opens `mod <parent> {` for free functions. An empty `parent` (a function
/// path with a single segment) needs no `mod` block.
fn write_mod_open<W: Write>(p: &mut IDLPrinter<W>, parent: &[String]) -> std::io::Result<()> {
    if parent.is_empty() {
        return Ok(());
    }
    p.write_indent()?;
    write!(p, "mod ")?;
    // An empty `generics` list means just the path is printed.
    p.write_idl(&crate::RustPathAndGenerics {
        path: parent.to_vec(),
        generics: vec![],
        named_generics: vec![],
    })?;
    writeln!(p, " {{")?;
    p.indent();
    Ok(())
}

/// Closes a block opened by [`write_mod_open`] with the same `parent`.
fn write_mod_close<W: Write>(p: &mut IDLPrinter<W>, parent: &[String]) -> std::io::Result<()> {
    if parent.is_empty() {
        return Ok(());
    }
    p.dedent();
    p.write_indent()?;
    writeln!(p, "}}")
}

/// Writes `[safe|unsafe] fn <name>(..) -> ..;`, i.e. the part of a free function that goes
/// inside its `mod` block.
fn write_fn_decl<W: Write>(
    p: &mut IDLPrinter<W>,
    name: &str,
    func: &crate::ZngurFn,
) -> std::io::Result<()> {
    p.write_indent()?;
    let safety = match func.is_safe {
        Some(true) => "safe ",
        Some(false) => "unsafe ",
        None => "",
    };
    write!(p, "{safety}fn {name}")?;
    if !func.path.generics.is_empty() {
        write!(p, "<")?;
        p.write_idl(&func.path.generics)?;
        write!(p, ">")?;
    }
    write!(p, "(")?;
    p.write_idl(&func.inputs)?;
    write!(p, ")")?;
    if func.output != crate::RustType::UNIT {
        write!(p, " -> ")?;
        p.write_idl(&func.output)?;
    }
    writeln!(p, ";")
}

impl WriteIDL for crate::ZngurFn {
    /// Free functions can only be declared by name inside a `mod`, so
    /// `crate::a::f` is printed as `mod crate::a { fn f(..); }`.
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::Function) {
            let Some((name, parent)) = self.path.path.split_last() else {
                // A function with an empty path can't be expressed.
                return Ok(());
            };
            write_mod_open(p, parent)?;
            write_fn_decl(p, name, self)?;
            write_mod_close(p, parent)?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurType {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::Type) {
            write!(p, "type ")?;
            p.write_idl(&self.ty)?;
            writeln!(p, " {{")?;
            p.indent();

            // Don't duplicate layout info: `#cpp_ref` and `?Sized` both imply a
            // layout, and restating it is a parse error.
            let is_unsized = self
                .wellknown_traits
                .contains(&crate::ZngurWellknownTrait::Unsized);
            if self.cpp_ref.is_none() && self.layout.is_some() && !is_unsized {
                p.write_indent()?;
                p.write_idl(&self.layout)?;
            }
            if self.trivially_relocatable && p.enabled(&IDLItem::TypeLayout) {
                p.write_indent()?;
                writeln!(p, "#trivially_relocatable;")?;
            }
            if self.cpp_heap_allocated.is_some() {
                p.write_indent()?;
                p.write_idl(&self.cpp_heap_allocated)?;
            }
            if self.cpp_ref.is_some() {
                p.write_indent()?;
                p.write_idl(&self.cpp_ref)?;
            }
            if self.cpp_stack_owned.is_some() {
                p.write_indent()?;
                p.write_idl(&self.cpp_stack_owned)?;
            }

            if p.enabled(&IDLItem::TypeWellknownTrait) {
                // `Drop` is implied by the parser and isn't valid in `wellknown_traits`.
                let drop_filtered: Vec<_> = self
                    .wellknown_traits
                    .iter()
                    .filter(|wk| !matches!(wk, crate::ZngurWellknownTrait::Drop))
                    .copied()
                    .collect();
                if !drop_filtered.is_empty() {
                    p.write_indent()?;
                    write!(p, "wellknown_traits(")?;
                    p.write_idl(&drop_filtered)?;
                    writeln!(p, ");")?;
                }
            }

            if self.constructor.is_some() {
                p.write_indent()?;
                p.write_idl(&self.constructor)?;
            }

            if p.enabled(&IDLItem::TypeVariant) {
                if !self.exhaustive {
                    p.write_indent()?;
                    writeln!(p, "non_exhaustive;")?;
                }
                for variant in &self.variants {
                    p.write_indent()?;
                    p.write_idl(variant)?;
                }
            }

            if !self.fields.is_empty() {
                writeln!(p)?;
                for field in &self.fields {
                    p.write_indent()?;
                    p.write_idl(field)?;
                }
            }

            if !self.methods.is_empty() {
                writeln!(p)?;
                for method in &self.methods {
                    p.write_indent()?;
                    p.write_idl(method)?;
                }
            }

            p.dedent();
            p.write_indent()?;
            writeln!(p, "}}")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurTrait {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::Trait) {
            write!(p, "trait ")?;
            p.write_idl(&self.tr)?;
            write!(p, " {{")?;
            if !self.methods.is_empty() {
                writeln!(p, "")?;
                p.indent();
                for method in &self.methods {
                    p.write_indent()?;
                    match method.is_safe {
                        Some(true) => write!(p, "safe ")?,
                        Some(false) => write!(p, "unsafe ")?,
                        None => {}
                    }
                    p.write_idl(method)?;
                    writeln!(p, ";")?;
                }
                p.dedent();
            }
            p.write_indent()?;
            writeln!(p, "}}")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurExternCppFn {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::ExternFn) {
            let safety = if self.is_safe { "safe" } else { "unsafe" };
            let name = &self.name;
            write!(p, "{safety} fn {name}(")?;
            p.write_idl(&self.inputs)?;
            write!(p, ")")?;
            if self.output != crate::RustType::UNIT {
                write!(p, " -> ")?;
                p.write_idl(&self.output)?;
            }
            writeln!(p, ";")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurExternCppImpl {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::ExternImpl) {
            write!(p, "impl ")?;
            if self.tr.is_some() {
                p.write_idl(&self.tr)?;
                write!(p, " for ")?;
            }
            p.write_idl(&self.ty)?;
            writeln!(p, " {{")?;
            p.indent();
            for method in &self.methods {
                p.write_indent()?;
                let safety = if method.is_safe == Some(true) {
                    "safe"
                } else {
                    "unsafe"
                };
                write!(p, "{safety} ")?;
                p.write_idl(method)?;
                writeln!(p, ";")?;
            }
            p.dedent();
            p.write_indent()?;
            writeln!(p, "}}")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::LayoutPolicy {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeLayout) {
            write!(p, "#")?;
            match self {
                Self::StackAllocated { size, align } => {
                    writeln!(p, "layout(size = {size}, align = {align});")?;
                }
                Self::Conservative { size, align } => {
                    writeln!(p, "layout_conservative(size = {size}, align = {align});")?;
                }
                Self::HeapAllocated => {
                    writeln!(p, "heap_allocated;")?;
                }
                Self::OnlyByRef => {
                    writeln!(p, "only_by_ref;")?;
                }
            }
        }
        Ok(())
    }
}

impl WriteIDL for crate::CppStackOwned {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeLayout) {
            let ty = &self.0;
            writeln!(p, r#"#cpp_stack_owned "{ty}";"#)?;
        }
        Ok(())
    }
}
impl WriteIDL for crate::CppHeapAllocated {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeLayout) {
            let ty = &self.0;
            writeln!(p, r#"#cpp_heap_allocated "{ty}";"#)?;
        }
        Ok(())
    }
}
impl WriteIDL for crate::CppRef {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeLayout) {
            let ty = &self.0;
            writeln!(p, r#"#cpp_ref "{ty}";"#)?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurWellknownTrait {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        match self {
            Self::Debug => write!(p, "Debug"),
            Self::Drop => write!(p, "Drop"),
            Self::Unsized => write!(p, "?Sized"),
            Self::Copy => write!(p, "Copy"),
        }
    }
}

impl WriteIDL for crate::ZngurConstructor {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeConstructor) {
            let valid_tuple_repr = self
                .inputs
                .iter()
                .all(|(name, _)| name.parse::<usize>().is_ok());
            let (open, close) = if valid_tuple_repr {
                ("(", ")")
            } else {
                ("{", "}")
            };
            write!(p, "constructor {open}")?;
            for (i, (name, input)) in self.inputs.iter().enumerate() {
                if i > 0 && i < self.inputs.len() {
                    write!(p, ", ")?;
                }
                if !valid_tuple_repr {
                    write!(p, "{name}: ")?;
                }
                p.write_idl(input)?;
            }
            writeln!(p, "{close};")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurVariant {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeVariant) {
            let name = &self.name;
            write!(p, "variant {name} {{")?;
            let print_fields = !self.fields.is_empty() && p.enabled(&IDLItem::TypeVariantField);
            if print_fields || !self.exhaustive {
                writeln!(p, "")?;
            }
            p.indent();
            if !self.exhaustive {
                p.write_indent()?;
                writeln!(p, "non_exhaustive;")?;
            }

            if print_fields {
                for field in &self.fields {
                    p.write_indent()?;
                    p.write_idl(field)?;
                }
            }
            p.dedent();
            if print_fields || !self.exhaustive {
                p.write_indent()?;
            }
            writeln!(p, "}}")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurField {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeField) {
            let name = &self.name;
            let offset = self
                .offset
                .map_or("auto".to_string(), |size| size.to_string());
            write!(p, "field {name} (offset = {offset}, type = ")?;
            p.write_idl(&self.ty)?;
            writeln!(p, ");")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurMethodDetails {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeMethod) {
            let safety = match self.data.is_safe {
                Some(true) => "safe ",
                Some(false) => "unsafe ",
                None => "",
            };
            write!(p, "{safety}")?;
            p.write_idl(&self.data)?;
            if let Some(use_path) = &self.use_path {
                // `use_path` is already a Rust path (`crate::...` or `::...`).
                let path = use_path;
                write!(p, " use {path}")?;
            }
            if let Some((ty, _)) = &self.deref {
                write!(p, " deref ")?;
                p.write_idl(ty)?;
            }
            if let Some(name) = &self.cpp_name {
                write!(p, " as {name}")?;
            }
            writeln!(p, ";")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurMethod {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        let name = &self.name;
        let receiver = match self.receiver {
            crate::ZngurMethodReceiver::Static => "",
            crate::ZngurMethodReceiver::Move => "self",
            crate::ZngurMethodReceiver::Ref(mu) => match mu {
                crate::Mutability::Not => "&self",
                crate::Mutability::Mut => "&mut self",
            },
        };
        let rec_sep = if self.inputs.len() > 0
            && !matches!(self.receiver, crate::ZngurMethodReceiver::Static)
        {
            ", "
        } else {
            ""
        };
        write!(p, "fn {name}")?;
        if self.generics.len() > 0 {
            write!(p, "<")?;
            p.write_idl(&self.generics)?;
            write!(p, ">")?;
        }
        write!(p, "({receiver}{rec_sep}")?;
        for (i, input) in self.inputs.iter().enumerate() {
            if i > 0 && i < self.inputs.len() {
                write!(p, ", ")?;
            }
            p.write_idl(input)?;
        }
        write!(p, ")")?;
        if self.output != crate::RustType::UNIT {
            write!(p, " -> ")?;
            p.write_idl(&self.output)?;
        }
        Ok(())
    }
}

#[test]
fn print_spec() {
    let spec = crate::ZngurSpec {
        types: vec![crate::ZngurType {
            ty: crate::RustType::Adt(crate::RustPathAndGenerics {
                path: vec!["crate".into(), "Example".into()],
                generics: vec![],
                named_generics: vec![],
            }),
            layout: Some(crate::LayoutPolicy::Conservative { size: 16, align: 8 }),
            wellknown_traits: vec![crate::ZngurWellknownTrait::Debug],
            exhaustive: true,
            constructor: None,
            variants: vec![],
            fields: vec![
                crate::ZngurField {
                    name: "foo".into(),
                    ty: crate::RustType::Primitive(crate::PrimitiveRustType::Uint(64)),
                    offset: Some(0),
                },
                crate::ZngurField {
                    name: "bar".into(),
                    ty: crate::RustType::Primitive(crate::PrimitiveRustType::Float(64)),
                    offset: None,
                },
            ],
            methods: vec![crate::ZngurMethodDetails {
                data: crate::ZngurMethod {
                    name: "foo_bar".into(),
                    generics: vec![],
                    receiver: crate::ZngurMethodReceiver::Ref(crate::Mutability::Not),
                    inputs: vec![],
                    output: crate::RustType::UNIT,
                    is_safe: Some(true),
                },
                use_path: None,
                deref: None,
                cpp_name: Some("fooBar".into()),
            }],
            cpp_heap_allocated: None,
            cpp_ref: None,
            cpp_stack_owned: None,
            trivially_relocatable: false,
        }],
        traits: indexmap::IndexMap::default(),
        funcs: vec![crate::ZngurFn {
            path: crate::RustPathAndGenerics {
                path: vec!["crate".into(), "do_w_example".into()],
                generics: vec![],
                named_generics: vec![],
            },
            inputs: vec![crate::RustType::Ref(
                crate::Mutability::Not,
                Box::new(crate::RustType::Adt(crate::RustPathAndGenerics {
                    path: vec!["crate".into(), "Example".into()],
                    generics: vec![],
                    named_generics: vec![],
                })),
            )],
            output: crate::RustType::Primitive(crate::PrimitiveRustType::Bool),
            is_safe: Some(true),
        }],
        extern_cpp_funcs: vec![],
        extern_cpp_impls: vec![],
        ..Default::default()
    };

    let mut buf = std::io::BufWriter::new(Vec::new());

    let mut printer = crate::printing::IDLPrinter::new(&mut buf);

    printer.write_idl(&spec).expect("write error");

    let bytes = buf.into_inner().expect("buffer flush error");

    let idl = String::from_utf8(bytes).expect("utf8 error");

    assert_eq!(
        "type crate::Example {
    #layout_conservative(size = 16, align = 8);
    wellknown_traits(Debug);

    field foo (offset = 0, type = u64);
    field bar (offset = auto, type = f64);

    safe fn foo_bar(&self) as fooBar;
}

mod crate {
    safe fn do_w_example(&crate::Example) -> bool;
}
",
        idl
    );
}

#[test]
fn print_funcs_grouped_by_mod() {
    let func = |path: &[&str]| crate::ZngurFn {
        path: crate::RustPathAndGenerics {
            path: path.iter().map(|s| s.to_string()).collect(),
            generics: vec![],
            named_generics: vec![],
        },
        inputs: vec![],
        output: crate::RustType::UNIT,
        is_safe: None,
    };
    let spec = crate::ZngurSpec {
        funcs: vec![
            func(&["top1"]),
            func(&["crate", "a"]),
            func(&["crate", "b"]),
            func(&["top2"]),
            func(&["crate", "m", "c"]),
            func(&["crate", "m", "d"]),
            func(&["crate", "e"]),
        ],
        ..Default::default()
    };

    let mut buf = Vec::new();
    crate::printing::IDLPrinter::new(&mut buf)
        .write_idl(&spec)
        .expect("write error");
    let idl = String::from_utf8(buf).expect("utf8 error");

    // Consecutive functions share a `mod`; single-segment functions emit at top level;
    // the printer never reorders them.
    assert_eq!(
        "fn top1();
mod crate {
    fn a();
    fn b();
}
fn top2();
mod crate::m {
    fn c();
    fn d();
}
mod crate {
    fn e();
}
",
        idl
    );

    // Standalone `ZngurFn::write_idl` handles both single-segment and `mod`-scoped paths.
    let mut buf = Vec::new();
    crate::printing::IDLPrinter::new(&mut buf)
        .write_idl(&func(&["standalone"]))
        .expect("write error");
    assert_eq!("fn standalone();\n", String::from_utf8(buf).unwrap());

    let mut buf = Vec::new();
    crate::printing::IDLPrinter::new(&mut buf)
        .write_idl(&func(&["crate", "m", "f"]))
        .expect("write error");
    assert_eq!(
        "mod crate::m {\n    fn f();\n}\n",
        String::from_utf8(buf).unwrap()
    );
}

#[test]
fn disabled_section_does_not_emit_blank_separator() {
    let spec = crate::ZngurSpec {
        convert_panic_to_exception: crate::ConvertPanicToException(true),
        funcs: vec![crate::ZngurFn {
            path: crate::RustPathAndGenerics {
                path: vec!["crate".into(), "f".into()],
                generics: vec![],
                named_generics: vec![],
            },
            inputs: vec![],
            output: crate::RustType::UNIT,
            is_safe: None,
        }],
        extern_cpp_funcs: vec![crate::ZngurExternCppFn {
            name: "ext".into(),
            inputs: vec![],
            output: crate::RustType::UNIT,
            is_safe: true,
        }],
        ..Default::default()
    };

    let mut buf = Vec::new();
    let mut printer = crate::printing::IDLPrinter::new(&mut buf);
    printer.disable_item(&IDLItem::Function);
    printer.write_idl(&spec).expect("write error");
    let idl = String::from_utf8(buf).expect("utf8 error");

    assert_eq!(
        "#convert_panic_to_exception

extern \"C++\" {

    safe fn ext();

}
",
        idl
    );
}

#[test]
fn test_write_cpp_type_idl() {
    let spec = crate::ZngurSpec {
        types: vec![
            crate::ZngurType {
                ty: crate::RustType::Cpp(vec!["Way".into()]),
                layout: Some(crate::LayoutPolicy::ZERO_SIZED_TYPE),
                wellknown_traits: vec![],
                exhaustive: true,
                constructor: None,
                variants: vec![],
                fields: vec![],
                methods: vec![],
                cpp_heap_allocated: None,
                cpp_ref: Some(crate::CppRef("::osmium::Way".into())),
                cpp_stack_owned: None,
                trivially_relocatable: false,
            },
            crate::ZngurType {
                ty: crate::RustType::Cpp(vec!["a".into(), "b".into(), "Name".into()]),
                layout: Some(crate::LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                wellknown_traits: vec![],
                exhaustive: true,
                constructor: None,
                variants: vec![],
                fields: vec![],
                methods: vec![crate::ZngurMethodDetails {
                    data: crate::ZngurMethod {
                        name: "deref".into(),
                        generics: vec![],
                        receiver: crate::ZngurMethodReceiver::Ref(crate::Mutability::Not),
                        inputs: vec![],
                        output: crate::RustType::UNIT,
                        is_safe: None,
                    },
                    use_path: Some("::core::ops::Deref".into()),
                    deref: None,
                    cpp_name: None,
                }],
                cpp_heap_allocated: Some(crate::CppHeapAllocated("::x::Name".into())),
                cpp_ref: None,
                cpp_stack_owned: None,
                trivially_relocatable: false,
            },
        ],
        ..Default::default()
    };

    let mut buf = Vec::new();
    crate::printing::IDLPrinter::new(&mut buf)
        .write_idl(&spec)
        .expect("write error");
    let idl = String::from_utf8(buf).expect("utf8 error");

    assert_eq!(
        "type c++::Way {
    #cpp_ref \"::osmium::Way\";
}

type c++::a::b::Name {
    #layout(size = 16, align = 8);
    #cpp_heap_allocated \"::x::Name\";

    fn deref(&self) use ::core::ops::Deref;
}
",
        idl
    );
}
