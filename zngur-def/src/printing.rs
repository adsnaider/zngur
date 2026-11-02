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

impl crate::RustType {
    /// Writes `self` in `.zng` syntax, using East-`const` (`T const*`) for a leading pointee
    /// `const` so `&'a T` does not collide with `&'a const ...` (a C++ object reference).
    fn write_east_const_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        match self {
            Self::CppPtr(kind, ty) => {
                ty.write_east_const_idl(p)?;
                match kind {
                    crate::CppIndirection::Pointer => write!(p, "*"),
                    crate::CppIndirection::Reference => write!(p, "&"),
                }
            }
            Self::CppConst(ty) => {
                ty.write_east_const_idl(p)?;
                write!(p, " const")
            }
            Self::CppVar(ty) => ty.write_east_const_idl(p),
            _ => p.write_idl(self),
        }
    }
}

impl WriteIDL for crate::RustType {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        match self {
            Self::Primitive(prim) => write!(p, "{prim}"),
            Self::Ref(m, lifetime, ty) => {
                write!(p, "&")?;
                if let Some(lifetime) = lifetime {
                    write!(p, "{lifetime} ")?;
                }
                match ty.as_cpp_object() {
                    // A reference to a C++ object: `&'a [unique] (const | var) T`.
                    Some((constness, inner)) => {
                        if *m == crate::Mutability::Mut {
                            write!(p, "unique ")?;
                        }
                        write!(p, "{} ", constness.keyword())?;
                        inner.write_east_const_idl(p)
                    }
                    None => {
                        if *m == crate::Mutability::Mut {
                            write!(p, "mut ")?;
                            p.write_idl(ty)
                        } else {
                            ty.write_east_const_idl(p)
                        }
                    }
                }
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
            Self::Cpp(path) => {
                write!(p, "c++::{}", path.path.join("::"))?;
                if !path.generics.is_empty() {
                    write!(p, "<")?;
                    p.write_idl(&path.generics)?;
                    write!(p, ">")?;
                }
                Ok(())
            }
            Self::CppPtr(kind, ty) => {
                p.write_idl(ty)?;
                match kind {
                    crate::CppIndirection::Pointer => write!(p, "*"),
                    crate::CppIndirection::Reference => write!(p, "&"),
                }
            }
            Self::CppConst(ty) if matches!(**ty, Self::CppPtr(..)) => {
                p.write_idl(ty)?;
                write!(p, " const")
            }
            Self::CppConst(ty) => {
                write!(p, "const ")?;
                p.write_idl(ty)
            }
            // `T&` / `T*` is a pointer to a non-`const` C++ object.
            Self::CppVar(ty) => p.write_idl(ty),
        }
    }
}

impl WriteIDL for crate::RustTrait {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        match self {
            Self::Normal(path_and_generics) => p.write_idl(path_and_generics),
            Self::Fn {
                name,
                lifetimes,
                inputs,
                output,
            } => {
                if !lifetimes.is_empty() {
                    write!(p, "for{} ", crate::lifetime_params(lifetimes))?;
                }
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

        if (self.lifetimes.len() + self.generics.len() + self.named_generics.len()) > 0 {
            write!(p, "<")?;
            let mut generic_count = 0;
            for lifetime in &self.lifetimes {
                if generic_count > 0 {
                    write!(p, ", ")?;
                }
                write!(p, "{lifetime}")?;
                generic_count += 1;
            }
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
        if let Some(crate_name) = &self.crate_name {
            separate(p)?;
            writeln!(p, "#crate {crate_name}")?;
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
        if (!self.types.is_empty() || !self.cpp_templates.is_empty()) && p.enabled(&IDLItem::Type) {
            separate(p)?;
            for (i, ty) in self.cpp_templates.iter().chain(&self.types).enumerate() {
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
        if !self.cpp_funcs.is_empty() && p.enabled(&IDLItem::Function) {
            separate(p)?;
            // Consecutive C++ functions in the same namespace share a `mod c++::` block.
            let mut open_namespace: Option<&[String]> = None;
            for func in &self.cpp_funcs {
                if open_namespace != Some(&func.namespace) {
                    if open_namespace.is_some() {
                        p.dedent();
                        p.write_indent()?;
                        writeln!(p, "}}")?;
                    }
                    p.write_indent()?;
                    writeln!(p, "mod c++::{} {{", func.namespace.join("::"))?;
                    p.indent();
                    open_namespace = Some(&func.namespace);
                }
                write_cpp_fn_decl(p, func)?;
            }
            if open_namespace.is_some() {
                p.dedent();
                p.write_indent()?;
                writeln!(p, "}}")?;
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
        ..Default::default()
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

/// Writes `<'a, 'b, T1, T2>`: the lifetimes that an item declares, then its type arguments (or
/// nothing if there are neither).
fn write_lifetimes_and_generics<W: Write>(
    p: &mut IDLPrinter<W>,
    lifetimes: &[crate::Lifetime],
    generics: &[crate::RustType],
) -> std::io::Result<()> {
    if lifetimes.is_empty() && generics.is_empty() {
        return Ok(());
    }
    write!(p, "<")?;
    for (i, lifetime) in lifetimes.iter().enumerate() {
        if i > 0 {
            write!(p, ", ")?;
        }
        write!(p, "{lifetime}")?;
    }
    if !lifetimes.is_empty() && !generics.is_empty() {
        write!(p, ", ")?;
    }
    p.write_idl(&generics.to_vec())?;
    write!(p, ">")
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
    write_lifetimes_and_generics(p, &func.lifetimes, &func.path.generics)?;
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

/// Writes `(safe | unsafe) fn <name>(..) -> .. [as ..];`, i.e. the part of a free C++
/// function that goes inside its `mod c++::<namespace>` block.
fn write_cpp_fn_decl<W: Write>(
    p: &mut IDLPrinter<W>,
    func: &crate::ZngurCppFn,
) -> std::io::Result<()> {
    p.write_indent()?;
    let safety = if func.is_safe { "safe" } else { "unsafe" };
    let name = &func.cpp_name;
    write!(p, "{safety} fn {name}")?;
    write_lifetimes_and_generics(p, &func.lifetimes, &func.template_args)?;
    write!(p, "(")?;
    p.write_idl(&func.inputs)?;
    write!(p, ")")?;
    if func.output != crate::RustType::UNIT {
        write!(p, " -> ")?;
        p.write_idl(&func.output)?;
    }
    if let Some(rust_name) = &func.rust_name {
        write!(p, " as {rust_name}")?;
    }
    writeln!(p, ";")
}

impl WriteIDL for crate::ZngurCppFn {
    /// Free C++ functions are printed inside a `mod c++::<namespace>` block.
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::Function) && !self.namespace.is_empty() {
            p.write_indent()?;
            writeln!(p, "mod c++::{} {{", self.namespace.join("::"))?;
            p.indent();
            write_cpp_fn_decl(p, self)?;
            p.dedent();
            p.write_indent()?;
            writeln!(p, "}}")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurType {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::Type) {
            // Templates (`type<T> c++::std::unique_ptr<T>`) list their type variables, after the
            // type's lifetime parameters (`type<'a> crate::Parser<'a>`).
            let type_vars = self.ty.type_vars();
            let params: Vec<String> = self
                .lifetimes
                .iter()
                .map(|l| l.to_string())
                .chain(type_vars.iter().map(|v| v.0.clone()))
                .collect();
            if params.is_empty() {
                write!(p, "type ")?;
            } else {
                write!(p, "type<{}> ", params.join(", "))?;
            }
            p.write_idl(&self.ty)?;
            writeln!(p, " {{")?;
            p.indent();

            // `?Sized` implies a layout, and restating it is a parse error.
            let is_unsized = self
                .wellknown_traits
                .contains(&crate::ZngurWellknownTrait::Unsized);
            if self.layout.is_some() && !is_unsized {
                p.write_indent()?;
                p.write_idl(&self.layout)?;
            }
            if p.enabled(&IDLItem::TypeLayout)
                && let Some(directive) = self.trivially_relocatable.directive()
            {
                p.write_indent()?;
                writeln!(p, "{directive};")?;
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

            if !self.cpp_fields.is_empty() && p.enabled(&IDLItem::TypeField) {
                writeln!(p)?;
                for field in &self.cpp_fields {
                    p.write_indent()?;
                    p.write_idl(field)?;
                }
            }

            if !self.cpp_constructors.is_empty() && p.enabled(&IDLItem::TypeConstructor) {
                writeln!(p)?;
                for constructor in &self.cpp_constructors {
                    p.write_indent()?;
                    p.write_idl(constructor)?;
                }
            }

            if !self.cpp_methods.is_empty() && p.enabled(&IDLItem::TypeMethod) {
                writeln!(p)?;
                for method in &self.cpp_methods {
                    p.write_indent()?;
                    p.write_idl(method)?;
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
            write!(p, "{safety} fn {name}")?;
            write_lifetimes_and_generics(p, &self.lifetimes, &[])?;
            write!(p, "(")?;
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
            write!(p, "impl")?;
            write_lifetimes_and_generics(p, &self.lifetimes, &[])?;
            write!(p, " ")?;
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
        let receiver = self.rust_receiver();
        let rec_sep = if self.inputs.len() > 0
            && !matches!(self.receiver, crate::ZngurMethodReceiver::Static)
        {
            ", "
        } else {
            ""
        };
        write!(p, "fn {name}")?;
        write_lifetimes_and_generics(p, &self.lifetimes, &self.generics)?;
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

impl WriteIDL for crate::ZngurCppField {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeField) {
            let safety = if self.is_safe { "" } else { "unsafe " };
            let access = match self.access {
                crate::CppFieldAccess::Ref { unique, constness } => {
                    let unique = if unique { "unique " } else { "" };
                    format!("&{unique}{} ", constness.keyword())
                }
                crate::CppFieldAccess::Copy { constness } => format!("{} ", constness.keyword()),
            };
            let name = &self.name;
            write!(p, "{safety}field {access}{name} (type = ")?;
            p.write_idl(&self.ty)?;
            write!(p, ")")?;
            if let Some(rust_name) = &self.rust_name {
                write!(p, " as {rust_name}")?;
            }
            writeln!(p, ";")?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurCppConstructor {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeConstructor) {
            let safety = if self.is_safe { "safe" } else { "unsafe" };
            write!(p, "{safety} constructor")?;
            write_lifetimes_and_generics(p, &self.lifetimes, &[])?;
            write!(p, "(")?;
            for (i, input) in self.inputs.iter().enumerate() {
                if i > 0 {
                    write!(p, ", ")?;
                }
                p.write_idl(input)?;
            }
            writeln!(p, ") as {};", self.rust_name)?;
        }
        Ok(())
    }
}

impl WriteIDL for crate::ZngurCppMethod {
    fn write_idl<W: Write>(&self, p: &mut IDLPrinter<W>) -> std::io::Result<()> {
        if p.enabled(&IDLItem::TypeMethod) {
            let safety = if self.is_safe { "safe" } else { "unsafe" };
            // Static member functions are the ones without a receiver.
            let name = &self.cpp_name;
            write!(p, "{safety} fn {name}")?;
            write_lifetimes_and_generics(p, &self.lifetimes, &self.template_args)?;
            write!(p, "(")?;
            let receiver = match (self.receiver, self.raw_receiver) {
                (crate::ZngurMethodReceiver::Ref(_), Some(indirection)) => {
                    let constness = match self.receiver_constness {
                        crate::CppConstness::Const => "const ",
                        crate::CppConstness::Var => "",
                    };
                    let sigil = match indirection {
                        crate::CppIndirection::Pointer => "*",
                        crate::CppIndirection::Reference => "&",
                    };
                    Some(format!("{constness}self{sigil}"))
                }
                (crate::ZngurMethodReceiver::Ref(m), None) => {
                    let lifetime = self
                        .receiver_lifetime
                        .as_ref()
                        .map(|l| format!("{l} "))
                        .unwrap_or_default();
                    let unique = if m == crate::Mutability::Mut {
                        "unique "
                    } else {
                        ""
                    };
                    let constness = self.receiver_constness.keyword();
                    Some(format!("&{lifetime}{unique}{constness} self"))
                }
                _ => None,
            };
            if let Some(receiver) = &receiver {
                write!(p, "{receiver}")?;
            }
            for (i, input) in self.inputs.iter().enumerate() {
                if i > 0 || receiver.is_some() {
                    write!(p, ", ")?;
                }
                p.write_idl(input)?;
            }
            write!(p, ")")?;
            if self.output != crate::RustType::UNIT {
                write!(p, " -> ")?;
                p.write_idl(&self.output)?;
            }
            if let Some(rust_name) = &self.rust_name {
                write!(p, " as {rust_name}")?;
            }
            writeln!(p, ";")?;
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
                ..Default::default()
            }),
            lifetimes: vec![],
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
                    lifetimes: vec![],
                    generics: vec![],
                    receiver: crate::ZngurMethodReceiver::Ref(crate::Mutability::Not),
                    receiver_lifetime: None,
                    inputs: vec![],
                    output: crate::RustType::UNIT,
                    is_safe: Some(true),
                },
                use_path: None,
                deref: None,
                cpp_name: Some("fooBar".into()),
            }],
            cpp_methods: vec![],
            cpp_constructors: vec![],
            cpp_fields: vec![],
            trivially_relocatable: crate::TriviallyRelocatable::No,
        }],
        traits: indexmap::IndexMap::default(),
        funcs: vec![crate::ZngurFn {
            path: crate::RustPathAndGenerics {
                path: vec!["crate".into(), "do_w_example".into()],
                ..Default::default()
            },
            lifetimes: vec![],
            inputs: vec![crate::RustType::Ref(
                crate::Mutability::Not,
                None,
                Box::new(crate::RustType::Adt(crate::RustPathAndGenerics {
                    path: vec!["crate".into(), "Example".into()],
                    ..Default::default()
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
            ..Default::default()
        },
        lifetimes: vec![],
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
                ..Default::default()
            },
            lifetimes: vec![],
            inputs: vec![],
            output: crate::RustType::UNIT,
            is_safe: None,
        }],
        extern_cpp_funcs: vec![crate::ZngurExternCppFn {
            name: "ext".into(),
            lifetimes: vec![],
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
                ty: crate::RustType::Cpp(vec!["Way".into()].into()),
                lifetimes: vec![],
                layout: None,
                wellknown_traits: vec![],
                exhaustive: true,
                constructor: None,
                variants: vec![],
                fields: vec![],
                methods: vec![],
                cpp_methods: vec![],
                cpp_constructors: vec![],
                cpp_fields: vec![],
                trivially_relocatable: crate::TriviallyRelocatable::No,
            },
            crate::ZngurType {
                ty: crate::RustType::Cpp(vec!["a".into(), "b".into(), "Name".into()].into()),
                lifetimes: vec![],
                layout: Some(crate::LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                wellknown_traits: vec![],
                exhaustive: true,
                constructor: None,
                variants: vec![],
                fields: vec![],
                methods: vec![crate::ZngurMethodDetails {
                    data: crate::ZngurMethod {
                        name: "deref".into(),
                        lifetimes: vec![],
                        generics: vec![],
                        receiver: crate::ZngurMethodReceiver::Ref(crate::Mutability::Not),
                        receiver_lifetime: None,
                        inputs: vec![],
                        output: crate::RustType::UNIT,
                        is_safe: None,
                    },
                    use_path: Some("::core::ops::Deref".into()),
                    deref: None,
                    cpp_name: None,
                }],
                cpp_methods: vec![],
                cpp_constructors: vec![],
                cpp_fields: vec![],
                trivially_relocatable: crate::TriviallyRelocatable::Checked,
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
}

type c++::a::b::Name {
    #layout(size = 16, align = 8);
    #trivially_relocatable;

    fn deref(&self) use ::core::ops::Deref;
}
",
        idl
    );
}

#[test]
fn print_template_type_vars_in_dyn_and_fn_traits() {
    let t_var = crate::RustType::TypeVar(crate::TypeVar("T".into()));
    let mk_ty = |ty: crate::RustType, lifetimes: Vec<crate::Lifetime>| crate::ZngurType {
        ty,
        lifetimes,
        layout: Some(crate::LayoutPolicy::StackAllocated { size: 16, align: 8 }),
        wellknown_traits: vec![],
        exhaustive: true,
        constructor: None,
        variants: vec![],
        fields: vec![],
        methods: vec![],
        cpp_methods: vec![],
        cpp_constructors: vec![],
        cpp_fields: vec![],
        trivially_relocatable: crate::TriviallyRelocatable::No,
    };
    let spec = crate::ZngurSpec {
        types: vec![
            mk_ty(
                crate::RustType::Boxed(Box::new(crate::RustType::Dyn(
                    crate::RustTrait::Normal(crate::RustPathAndGenerics {
                        path: vec!["std".into(), "future".into(), "Future".into()],
                        named_generics: vec![("Output".into(), t_var.clone())],
                        ..Default::default()
                    }),
                    vec![],
                ))),
                vec![],
            ),
            mk_ty(
                crate::RustType::Boxed(Box::new(crate::RustType::Dyn(
                    crate::RustTrait::Fn {
                        name: "Fn".into(),
                        lifetimes: vec![],
                        inputs: vec![t_var.clone()],
                        output: Box::new(t_var.clone()),
                    },
                    vec![],
                ))),
                vec![],
            ),
            mk_ty(
                crate::RustType::Adt(crate::RustPathAndGenerics {
                    path: vec!["std".into(), "slice".into(), "Iter".into()],
                    lifetimes: vec![crate::Lifetime("a".into())],
                    generics: vec![t_var],
                    named_generics: vec![],
                }),
                vec![crate::Lifetime("a".into())],
            ),
        ],
        ..Default::default()
    };

    let mut buf = Vec::new();
    crate::printing::IDLPrinter::new(&mut buf)
        .write_idl(&spec)
        .expect("write error");
    let idl = String::from_utf8(buf).expect("utf8 error");

    assert_eq!(
        "type<T> Box<dyn ::std::future::Future<Output = T>> {
    #layout(size = 16, align = 8);
}

type<T> Box<dyn Fn(T) -> T> {
    #layout(size = 16, align = 8);
}

type<'a, T> ::std::slice::Iter<'a, T> {
    #layout(size = 16, align = 8);
}
",
        idl
    );
}

#[test]
fn print_cpp_function_template_args() {
    let point = crate::RustType::Cpp(vec!["geo".to_owned(), "Point".to_owned()].into());
    let spec = crate::ZngurSpec {
        cpp_funcs: vec![crate::ZngurCppFn {
            lifetimes: vec![],
            namespace: vec!["std".to_owned()],
            cpp_name: "make_unique".to_owned(),
            template_args: vec![point.clone()],
            rust_name: Some("make_point".to_owned()),
            inputs: vec![crate::RustType::Primitive(crate::PrimitiveRustType::Int(
                32,
            ))],
            output: crate::RustType::UNIT,
            is_safe: true,
        }],
        ..Default::default()
    };

    let mut buf = Vec::new();
    crate::printing::IDLPrinter::new(&mut buf)
        .write_idl(&spec)
        .expect("write error");
    let idl = String::from_utf8(buf).expect("utf8 error");

    assert_eq!(
        "mod c++::std {
    safe fn make_unique<c++::geo::Point>(i32) as make_point;
}
",
        idl
    );
}
