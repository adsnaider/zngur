use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    ops::{Deref, DerefMut},
    path::Component,
};

use ariadne::{Color, Label, Report, ReportKind};
use chumsky::{input::MapExtra, prelude::*};
use itertools::{Either, Itertools};

use zngur_def::{
    AdditionalIncludes, ConflictSource, ConvertPanicToException, CppConstness, CppFieldAccess,
    CppHeapAllocated, CppIndirection, CppRef, CppStackOwned, CppTemplateHome, CppTypePath, Import,
    LayoutPolicy, Lifetime, Merge, MergeFailure, ModuleImport, Mutability, PrimitiveRustType,
    RustPathAndGenerics, RustTrait, RustType, TriviallyRelocatable, TypeVar, ZngurConstructor,
    ZngurCppConstructor, ZngurCppField, ZngurCppFn, ZngurCppMethod, ZngurExternCppFn,
    ZngurExternCppImpl, ZngurField, ZngurFn, ZngurMethod, ZngurMethodDetails, ZngurMethodReceiver,
    ZngurSpec, ZngurTrait, ZngurType, ZngurVariant, ZngurWellknownTrait, cpp_handle_field_name,
    cpp_handle_keyword,
};

pub type Span = SimpleSpan<usize>;

/// Result of parsing a .zng file, containing both the spec and the list of all processed files.
#[derive(Debug)]
pub struct ParseResult {
    /// The parsed Zngur specification
    pub spec: ZngurSpec,
    /// All .zng files that were processed (main file + transitive imports)
    pub processed_files: Vec<std::path::PathBuf>,
    /// count of errors reported
    pub errors: usize,
    /// count of warnings reported
    pub warnings: usize,
}

#[cfg(test)]
mod tests;

pub mod cfg;
mod conditional;
mod template_types;

use crate::{
    cfg::{CfgConditional, RustCfgProvider},
    conditional::{Condition, ConditionalItem, NItems, conditional_item},
    template_types::{try_match_template, unify_cpp_template_types},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spanned<T> {
    inner: T,
    span: Span,
}

type ParserInput<'a> = chumsky::input::MappedInput<
    Token<'a>,
    Span,
    &'a [(Token<'a>, Span)],
    Box<
        dyn for<'x> Fn(
            &'x (Token<'_>, chumsky::span::SimpleSpan),
        ) -> (&'x Token<'x>, &'x SimpleSpan),
    >,
>;

#[derive(Default)]
pub struct UnstableFeatures {
    pub cfg_match: bool,
    pub cfg_if: bool,
    pub template_types: bool,
}

#[derive(Default)]
pub struct ZngParserState {
    pub unstable_features: UnstableFeatures,
}

type ZngParserExtra<'a> =
    extra::Full<Rich<'a, Token<'a>, Span>, extra::SimpleState<ZngParserState>, ()>;

type BoxedZngParser<'a, Item> = chumsky::Boxed<'a, 'a, ParserInput<'a>, Item, ZngParserExtra<'a>>;

/// Effective trait alias for verbose chumsky Parser Trait
pub(crate) trait ZngParser<'a, Item>:
    Parser<'a, ParserInput<'a>, Item, ZngParserExtra<'a>> + Clone
{
}
impl<'a, T, Item> ZngParser<'a, Item> for T where
    T: Parser<'a, ParserInput<'a>, Item, ZngParserExtra<'a>> + Clone
{
}

#[derive(Debug)]
pub struct ParsedZngFile<'a>(Vec<ParsedItem<'a>>);

#[derive(Debug)]
pub struct ProcessedZngFile<'a> {
    aliases: Vec<ParsedAlias<'a>>,
    items: Vec<ProcessedItem<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ParsedPathStart {
    Absolute,
    Relative,
    Crate,
    Cpp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedPath<'a> {
    start: ParsedPathStart,
    segments: Vec<&'a str>,
    span: Span,
}

/// A path to some named entity in either the Rust or C++ universe -- today
/// always a namespace/module path, but named generally since a scope's base
/// may need to refer to other kinds of entities (e.g. structs) later.
#[derive(Debug, Clone)]
enum EntityPath {
    /// If initial element is "crate", interpreted as crate relative.
    /// Otherwise, understood to be a global path.
    Rust(Vec<String>),
    Cpp(Vec<String>),
}

impl EntityPath {
    fn rust<'s>(segments: impl IntoIterator<Item = &'s str>) -> EntityPath {
        EntityPath::Rust(segments.into_iter().map(str::to_owned).collect())
    }

    fn cpp<'s>(segments: impl IntoIterator<Item = &'s str>) -> EntityPath {
        EntityPath::Cpp(segments.into_iter().map(str::to_owned).collect())
    }

    fn crate_relative<'s>(segments: impl IntoIterator<Item = &'s str>) -> EntityPath {
        EntityPath::rust(["crate"].into_iter().chain(segments))
    }

    fn child(&self, name: &str) -> EntityPath {
        let mut child = self.clone();
        match &mut child {
            EntityPath::Rust(v) | EntityPath::Cpp(v) => v.push(name.to_owned()),
        }
        child
    }

    fn join<'s>(&self, extra: impl IntoIterator<Item = &'s str>) -> EntityPath {
        let mut joined = self.clone();
        match &mut joined {
            EntityPath::Rust(v) | EntityPath::Cpp(v) => {
                v.extend(extra.into_iter().map(str::to_owned))
            }
        }
        joined
    }

    /// Return Rust source text that references a Rust `target` path.
    /// `EntityPath::Cpp` targets and the empty root `EntityPath::Rust` cannot be referenced here.
    fn rust_reference(&self) -> Option<String> {
        match self {
            EntityPath::Rust(v) if v.is_empty() => None,
            EntityPath::Rust(v) => {
                if v.first().is_some_and(|s| s == "crate") {
                    Some(v.join("::"))
                } else {
                    Some(format!("::{}", v.join("::")))
                }
            }
            EntityPath::Cpp(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedAlias<'a> {
    name: &'a str,
    target: EntityPath,
}

impl ResolvedAlias<'_> {
    fn expand(&self, path: &ParsedPath<'_>) -> Option<EntityPath> {
        if path.start == ParsedPathStart::Relative
            && path.segments.first().is_some_and(|part| *part == self.name)
        {
            let extra = path.segments.iter().skip(1).copied();
            Some(self.target.join(extra))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
struct Scope<'a> {
    aliases: Vec<ResolvedAlias<'a>>,
    base: EntityPath,
    type_vars: HashSet<ParsedTypeVar<'a>>,
}

impl<'a> Scope<'a> {
    /// Create a new root scope containing the specified aliases.
    fn new_root(aliases: Vec<ParsedAlias<'a>>, ctx: &mut ParseContext) -> Scope<'a> {
        let base = EntityPath::Rust(Default::default());
        let resolved_aliases = Self::resolve_aliases(&aliases, &base, &[], ctx);
        Scope {
            aliases: resolved_aliases,
            base,
            type_vars: Default::default(),
        }
    }

    fn resolve_aliases(
        new_aliases: &[ParsedAlias<'a>],
        base: &EntityPath,
        parent_aliases: &[ResolvedAlias<'a>],
        ctx: &mut ParseContext,
    ) -> Vec<ResolvedAlias<'a>> {
        fn resolve_one<'a>(
            idx: usize,
            unique_aliases: &[&ParsedAlias<'a>],
            base: &EntityPath,
            parent_aliases: &[ResolvedAlias<'a>],
            visited: &mut Vec<usize>,
            reported_cycles: &mut HashSet<usize>,
            ctx: &mut ParseContext,
        ) -> EntityPath {
            let alias = unique_aliases[idx];
            if alias.path.start == ParsedPathStart::Relative {
                if let Some(&first) = alias.path.segments.first() {
                    if let Some(target_idx) = unique_aliases.iter().position(|a| a.name == first) {
                        if let Some(cycle_pos) = visited.iter().position(|&v| v == target_idx) {
                            let cycle_head = *visited[cycle_pos..].iter().min().unwrap();
                            if reported_cycles.insert(cycle_head) {
                                let head_alias = unique_aliases[cycle_head];
                                ctx.add_error_str(
                                    &format!("cyclic `use` alias definition `{}`", head_alias.name),
                                    head_alias.span,
                                );
                            }
                        } else {
                            visited.push(target_idx);
                            let target = resolve_one(
                                target_idx,
                                unique_aliases,
                                base,
                                parent_aliases,
                                visited,
                                reported_cycles,
                                ctx,
                            );
                            visited.pop();
                            return target.join(alias.path.segments.iter().skip(1).copied());
                        }
                    } else if let Some(expanded) = parent_aliases
                        .iter()
                        .find_map(|existing| existing.expand(&alias.path))
                    {
                        return expanded;
                    }
                }
            }
            alias.path.clone().to_zngur(base)
        }

        let mut unique_aliases: Vec<&ParsedAlias<'a>> = Vec::with_capacity(new_aliases.len());
        let mut seen_names: std::collections::HashMap<&'a str, Span> =
            std::collections::HashMap::with_capacity(new_aliases.len());
        for alias in new_aliases {
            if let Some(&first_span) = seen_names.get(alias.name) {
                let dup_span = ctx.report_span_for_range(alias.span.into_range());
                let prev_span = ctx.report_span_for_range(first_span.into_range());
                let msg = format!("duplicate `use` alias `{}` in the same module", alias.name);
                let report = Report::build(ReportKind::Error, dup_span.clone())
                    .with_message(&msg)
                    .with_label(
                        Label::new(dup_span)
                            .with_message(&msg)
                            .with_color(Color::Red),
                    )
                    .with_label(
                        Label::new(prev_span)
                            .with_message("previously defined here")
                            .with_color(Color::Blue),
                    )
                    .finish();
                ctx.add_fatal_report(report);
            } else {
                seen_names.insert(alias.name, alias.span);
                unique_aliases.push(alias);
            }
        }

        let mut local: Vec<ResolvedAlias<'a>> =
            Vec::with_capacity(unique_aliases.len() + parent_aliases.len());
        let mut visited = Vec::new();
        let mut reported_cycles = HashSet::new();
        for idx in 0..unique_aliases.len() {
            visited.clear();
            visited.push(idx);
            let target = resolve_one(
                idx,
                &unique_aliases,
                base,
                parent_aliases,
                &mut visited,
                &mut reported_cycles,
                ctx,
            );
            local.push(ResolvedAlias {
                name: unique_aliases[idx].name,
                target,
            });
        }
        local.extend_from_slice(parent_aliases);
        local
    }

    fn has_alias(&self, path: &ParsedPath<'_>) -> bool {
        path.start == ParsedPathStart::Relative
            && path
                .segments
                .first()
                .is_some_and(|part| self.aliases.iter().any(|alias| alias.name == *part))
    }

    /// Resolve a path according to the current scope.
    fn resolve_path(&self, path: ParsedPath<'a>) -> EntityPath {
        // Check to see if the path refers to an alias:
        if let Some(expanded_alias) = self.aliases.iter().find_map(|alias| alias.expand(&path)) {
            expanded_alias
        } else {
            path.to_zngur(&self.base)
        }
    }

    fn report_invalid_cpp_path(
        &self,
        ctx: &mut ParseContext,
        path: &ParsedPath<'_>,
        message: &str,
        rust_kind_plural: &str,
    ) {
        let is_bare_relative = path.start == ParsedPathStart::Relative && !self.has_alias(path);
        let report_span = ctx.report_span_for_range(path.span.into_range());
        let mut report = Report::build(ReportKind::Error, report_span.clone())
            .with_message(message)
            .with_label(
                Label::new(report_span)
                    .with_message(message)
                    .with_color(Color::Red),
            );
        if is_bare_relative && matches!(self.base, EntityPath::Cpp(_)) {
            report = report.with_help(format!(
                "inside a `c++::` module, prefix Rust {rust_kind_plural} with `::` or `crate::`"
            ));
        }
        ctx.add_fatal_report(report.finish());
    }

    fn sub_scope(
        &self,
        new_aliases: &[ParsedAlias<'a>],
        nested_path: ParsedPath<'a>,
        ctx: &mut ParseContext,
    ) -> Scope<'_> {
        let base = nested_path.to_zngur(&self.base);
        let mod_aliases = Self::resolve_aliases(new_aliases, &base, &self.aliases, ctx);

        Scope {
            aliases: mod_aliases,
            base,
            type_vars: self.type_vars.clone(),
        }
    }

    fn with_type_vars(&self, type_vars: HashSet<ParsedTypeVar<'a>>) -> Scope<'_> {
        Scope {
            aliases: self.aliases.clone(),
            base: self.base.clone(),
            type_vars,
        }
    }

    fn as_type_var(&self, ty: &ParsedRustPathAndGenerics<'a>) -> Option<TypeVar> {
        if let ParsedRustPathAndGenerics {
            path:
                ParsedPath {
                    start: ParsedPathStart::Relative,
                    segments,
                    span: _,
                },
            lifetimes,
            generics,
            named_generics,
            has_explicit_angle_brackets: false,
            span: _,
        } = ty
            && lifetimes.is_empty()
            && generics.is_empty()
            && named_generics.is_empty()
            && let &[single_elem] = segments.as_slice()
            && self.type_vars.contains(&ParsedTypeVar(single_elem))
        {
            Some(TypeVar(single_elem.to_owned()))
        } else {
            None
        }
    }
}

impl ParsedPath<'_> {
    fn to_zngur(self, base: &EntityPath) -> EntityPath {
        match self.start {
            ParsedPathStart::Absolute => EntityPath::rust(self.segments),
            ParsedPathStart::Relative => base.join(self.segments),
            ParsedPathStart::Crate => EntityPath::crate_relative(self.segments),
            ParsedPathStart::Cpp => EntityPath::cpp(self.segments),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAlias<'a> {
    name: &'a str,
    path: ParsedPath<'a>,
    span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedImportPath {
    path: std::path::PathBuf,
    span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedItem<'a> {
    ConvertPanicToException(Span),
    CrateName(&'a str, Span),
    CppAdditionalInclude(&'a str),
    UnstableFeature(&'a str),
    Allow(&'a str, Span),
    Mod {
        path: ParsedPath<'a>,
        items: Vec<ParsedItem<'a>>,
    },
    Type {
        ty: Spanned<ParsedRustType<'a>>,
        items: Vec<Spanned<ParsedTypeItem<'a>>>,
        /// The lifetime parameters in `type<'a, ...>`.
        lifetimes: Vec<&'a str>,
        type_vars: Option<HashSet<ParsedTypeVar<'a>>>,
        unstable_template_types: bool,
    },
    Trait {
        tr: Spanned<ParsedRustTrait<'a>>,
        methods: Vec<Spanned<ParsedMethod<'a>>>,
    },
    Fn(Spanned<ParsedFunction<'a>>),
    ExternCpp(Vec<ParsedExternCppItem<'a>>),
    Alias(ParsedAlias<'a>),
    Import(ParsedImportPath),
    ModuleImport {
        path: std::path::PathBuf,
        span: Span,
    },
    MatchOnCfg(Condition<CfgConditional<'a>, ParsedItem<'a>, NItems>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProcessedItem<'a> {
    ConvertPanicToException(Span),
    CrateName(&'a str, Span),
    CppAdditionalInclude(&'a str),
    Mod {
        path: ParsedPath<'a>,
        items: Vec<ProcessedItem<'a>>,
        aliases: Vec<ParsedAlias<'a>>,
    },
    Type {
        ty: Spanned<ParsedRustType<'a>>,
        items: Vec<Spanned<ParsedTypeItem<'a>>>,
        /// The lifetime parameters in `type<'a, ...>`.
        lifetimes: Vec<&'a str>,
        type_vars: Option<HashSet<ParsedTypeVar<'a>>>,
        unstable_template_types: bool,
    },
    Trait {
        tr: Spanned<ParsedRustTrait<'a>>,
        methods: Vec<Spanned<ParsedMethod<'a>>>,
    },
    Fn(Spanned<ParsedFunction<'a>>),
    ExternCpp(Vec<ParsedExternCppItem<'a>>),
    Import(ParsedImportPath),
    ModuleImport {
        path: std::path::PathBuf,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedExternCppItem<'a> {
    Function {
        is_safe: bool,
        method: Spanned<ParsedMethod<'a>>,
    },
    Impl {
        /// The lifetime parameters in `impl<'a>`.
        lifetimes: Vec<&'a str>,
        tr: Option<ParsedRustTrait<'a>>,
        ty: Spanned<ParsedRustType<'a>>,
        methods: Vec<(bool, Spanned<ParsedMethod<'a>>)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedConstructorArgs<'a> {
    Unit,
    Tuple(Vec<ParsedRustType<'a>>),
    Named(Vec<(&'a str, ParsedRustType<'a>)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedLayoutPolicy<'a> {
    StackAllocated(Vec<(Spanned<&'a str>, usize)>),
    Conservative(Vec<(Spanned<&'a str>, usize)>),
    HeapAllocated,
    OnlyByRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedTypeItem<'a> {
    Layout(Span, ParsedLayoutPolicy<'a>),
    Traits(Vec<Spanned<ZngurWellknownTrait>>),
    NonExhaustive,
    Constructor {
        lifetimes: Vec<&'a str>,
        args: ParsedConstructorArgs<'a>,
        rust_name: Option<&'a str>,
    },
    Variant {
        name: &'a str,
        items: Vec<Spanned<ParsedTypeItem<'a>>>,
    },
    Field {
        name: String,
        ty: ParsedRustType<'a>,
        offset: Option<usize>,
    },
    /// A unified `fn` item inside a `type` block:
    /// `[safe | unsafe] [static] [async] fn <name>[<'a, ..., T, ...>](<inputs...>) [const]
    /// [-> <output>] [use <path>] [deref <ty>] [as <alias>];`
    ///
    /// Lowered to `ZngurCppMethod` on `c++::` types (requiring `safe` or `unsafe`) and to
    /// `ZngurMethod` on Rust types.
    Fn {
        is_safe: Option<bool>,
        is_static: bool,
        async_span: Option<Span>,
        name: &'a str,
        name_is_ident: bool,
        lifetimes: Vec<&'a str>,
        generics: Vec<ParsedRustType<'a>>,
        inputs: Vec<ParsedRustType<'a>>,
        is_const: bool,
        output: ParsedRustType<'a>,
        sig_span: Span,
        use_path: Option<ParsedPath<'a>>,
        deref: Option<ParsedRustType<'a>>,
        alias: Option<(&'a str, bool)>,
    },
    /// A C++ constructor:
    /// `(safe | unsafe) constructor[<lifetimes...>](types...) as rust_name;` (`rust_name` is
    /// `None` if `as` was left off, to give a clear error).
    CppConstructor {
        is_safe: bool,
        lifetimes: Vec<&'a str>,
        inputs: Vec<ParsedRustType<'a>>,
        rust_name: Option<&'a str>,
    },
    /// An accessor for a field of a C++ type:
    /// `[safe | unsafe] field [& [unique]] (const | var) name (type = T) [as rust_name];`.
    /// `constness` is `None` if it's missing, and `unique` without `&` is an error, both reported
    /// when it's processed.
    CppField {
        is_safe: bool,
        is_ref: bool,
        unique: bool,
        constness: Option<CppConstness>,
        name: &'a str,
        ty: ParsedRustType<'a>,
        rust_name: Option<&'a str>,
    },
    CppValue {
        field: &'a str,
        cpp_type: &'a str,
    },
    // For the `#cpp_*` directives, `cpp_type` is `None` if the C++ type name is omitted, which is
    // allowed for `c++::` types: the name is the type's path.
    CppHeapAllocated {
        cpp_type: Option<&'a str>,
    },
    CppRef {
        cpp_type: Option<&'a str>,
    },
    CppStackOwned {
        cpp_type: Option<&'a str>,
        props: Option<Vec<(Spanned<&'a str>, usize)>>,
    },
    /// `#trivially_relocatable`, or `#unsafe(trivially_relocatable)` if `unchecked`.
    TriviallyRelocatable {
        unchecked: bool,
    },
    MatchOnCfg(Condition<CfgConditional<'a>, ParsedTypeItem<'a>, NItems>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ParsedTypeVar<'a>(&'a str);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedMethod<'a> {
    name: &'a str,
    /// The lifetimes it declares (`fn f<'a>(...)`), without the `'`.
    lifetimes: Vec<&'a str>,
    is_safe: Option<bool>,
    receiver: ZngurMethodReceiver,
    /// The lifetime of a `&'a self` / `&'a mut self` receiver.
    receiver_lifetime: Option<&'a str>,
    generics: Vec<ParsedRustType<'a>>,
    inputs: Vec<ParsedRustType<'a>>,
    output: ParsedRustType<'a>,
}

/// A free function item (Rust or C++), disambiguated after resolving `path` in scope:
/// `[safe | unsafe] [async] fn path[<'a, ..., T, ...>](types...) [-> type] [as alias];`
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedFunction<'a> {
    is_safe: Option<bool>,
    async_span: Option<Span>,
    path: ParsedPath<'a>,
    name_is_ident: bool,
    lifetimes: Vec<&'a str>,
    generics: Vec<ParsedRustType<'a>>,
    inputs: Vec<ParsedRustType<'a>>,
    output: ParsedRustType<'a>,
    alias: Option<(&'a str, bool)>,
    sig_span: Span,
}

impl ParsedMethod<'_> {
    fn to_zngur(self, scope: &Scope<'_>, ctx: &mut ParseContext) -> ZngurMethod {
        ZngurMethod {
            name: self.name.to_owned(),
            lifetimes: lifetimes(&self.lifetimes),
            generics: self
                .generics
                .into_iter()
                .map(|x| x.to_zngur(scope, ctx))
                .collect(),
            receiver: self.receiver,
            receiver_lifetime: self.receiver_lifetime.map(lifetime),
            inputs: self
                .inputs
                .into_iter()
                .map(|x| x.to_zngur(scope, ctx))
                .collect(),
            output: self.output.to_zngur(scope, ctx),
            is_safe: self.is_safe,
        }
    }
}

fn lifetime(name: &str) -> Lifetime {
    Lifetime(name.to_owned())
}

fn lifetimes(names: &[&str]) -> Vec<Lifetime> {
    names.iter().map(|n| lifetime(n)).collect()
}

fn validate_declared_lifetimes(ctx: &mut ParseContext, span: Span, declared: &[Lifetime]) -> bool {
    for (i, l) in declared.iter().enumerate() {
        if l.is_builtin() {
            ctx.add_error_str(&format!("`{l}` can't be declared"), span);
            return false;
        }
        if declared[..i].contains(l) {
            ctx.add_error_str(&format!("lifetime `{l}` is declared twice"), span);
            return false;
        }
    }
    true
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LifetimeContext {
    Item,
    TraitHeader,
}

fn check_used_lifetime(
    ctx: &mut ParseContext,
    span: Span,
    all_outer: &[Lifetime],
    allowed: &[Lifetime],
    fn_trait_name: Option<&str>,
    context: LifetimeContext,
    l: &Lifetime,
) -> bool {
    if context == LifetimeContext::TraitHeader && fn_trait_name.is_none() && l.0 == "_" {
        ctx.add_error_str(
            "anonymous lifetime `'_` cannot be used in a `trait` header: \
             `trait` declarations don't take lifetime parameters (only `'static` can be used here)",
            span,
        );
        return false;
    }
    if l.is_builtin() || allowed.contains(l) {
        return true;
    }
    if let Some(name) = fn_trait_name {
        if all_outer.contains(l) {
            ctx.add_error_str(
                &format!(
                    "outer lifetime `{l}` can't be used inside `{name}(...)`: \
                     use `for<{l}> {name}(...)` for a higher-ranked function trait"
                ),
                span,
            );
        } else {
            ctx.add_error_str(
                &format!("undeclared lifetime `{l}`: declare it with `for<{l}> {name}(...)`"),
                span,
            );
        }
    } else if context == LifetimeContext::TraitHeader {
        ctx.add_error_str(
            &format!(
                "undeclared lifetime `{l}`: `trait` declarations don't take lifetime parameters \
                 (only `'static` can be used here)"
            ),
            span,
        );
    } else {
        ctx.add_error_str(
            &format!(
                "undeclared lifetime `{l}`: declare it on the function, `impl` or `type` \
                 (e.g. `fn f<{l}>(...)`)"
            ),
            span,
        );
    }
    false
}

fn check_ty_lifetimes(
    ctx: &mut ParseContext,
    span: Span,
    all_outer: &[Lifetime],
    allowed: &[Lifetime],
    fn_trait_name: Option<&str>,
    context: LifetimeContext,
    ty: &RustType,
) -> bool {
    match ty {
        RustType::Primitive(_) | RustType::TypeVar(_) => true,
        RustType::Cpp(path) => path
            .generics
            .iter()
            .all(|t| check_ty_lifetimes(ctx, span, all_outer, allowed, fn_trait_name, context, t)),
        RustType::Ref(_, lt, inner) => {
            match lt {
                Some(l) => {
                    if !check_used_lifetime(
                        ctx,
                        span,
                        all_outer,
                        allowed,
                        fn_trait_name,
                        context,
                        l,
                    ) {
                        return false;
                    }
                }
                None if context == LifetimeContext::TraitHeader && fn_trait_name.is_none() => {
                    ctx.add_error_str(
                        "elided reference lifetime cannot be used in a `trait` header: \
                         `trait` declarations don't take lifetime parameters \
                         (use `&'static ...` if `'static` is intended)",
                        span,
                    );
                    return false;
                }
                None => {}
            }
            check_ty_lifetimes(ctx, span, all_outer, allowed, fn_trait_name, context, inner)
        }
        RustType::Raw(_, inner)
        | RustType::Boxed(inner)
        | RustType::Slice(inner)
        | RustType::CppPtr(_, inner)
        | RustType::CppConst(inner)
        | RustType::CppVar(inner) => {
            check_ty_lifetimes(ctx, span, all_outer, allowed, fn_trait_name, context, inner)
        }
        RustType::Tuple(tys) => tys
            .iter()
            .all(|t| check_ty_lifetimes(ctx, span, all_outer, allowed, fn_trait_name, context, t)),
        RustType::Adt(pg) => {
            pg.lifetimes.iter().all(|l| {
                check_used_lifetime(ctx, span, all_outer, allowed, fn_trait_name, context, l)
            }) && pg
                .generics
                .iter()
                .chain(pg.named_generics.iter().map(|(_, t)| t))
                .all(|t| {
                    check_ty_lifetimes(ctx, span, all_outer, allowed, fn_trait_name, context, t)
                })
        }
        RustType::Dyn(tr, _) | RustType::Impl(tr, _) => match tr {
            RustTrait::Normal(pg) => {
                pg.lifetimes.iter().all(|l| {
                    check_used_lifetime(ctx, span, all_outer, allowed, fn_trait_name, context, l)
                }) && pg
                    .generics
                    .iter()
                    .chain(pg.named_generics.iter().map(|(_, t)| t))
                    .all(|t| {
                        check_ty_lifetimes(ctx, span, all_outer, allowed, fn_trait_name, context, t)
                    })
            }
            RustTrait::Fn {
                name,
                lifetimes: fn_lts,
                inputs,
                output,
            } => {
                if !validate_declared_lifetimes(ctx, span, fn_lts) {
                    return false;
                }
                if let Some(l) = fn_lts.iter().find(|l| all_outer.contains(l)) {
                    ctx.add_error_str(
                        &format!(
                            "lifetime `{l}` in `for<...>` is already declared in an outer scope"
                        ),
                        span,
                    );
                    return false;
                }
                let mut inner_outer = all_outer.to_vec();
                inner_outer.extend(fn_lts.iter().cloned());
                inputs.iter().chain([&**output]).all(|t| {
                    check_ty_lifetimes(ctx, span, &inner_outer, fn_lts, Some(name), context, t)
                })
            }
        },
    }
}

fn check_used_types<'t>(
    ctx: &mut ParseContext,
    span: Span,
    declared: &[Lifetime],
    types: impl IntoIterator<Item = &'t RustType>,
    extra: impl IntoIterator<Item = &'t Lifetime>,
    context: LifetimeContext,
    allow_cpp_ref: bool,
    allow_cpp_object_ref: bool,
) -> bool {
    let types: Vec<&RustType> = types.into_iter().collect();
    if !report_cpp_objects(
        ctx,
        span,
        types.iter().copied(),
        allow_cpp_ref,
        allow_cpp_object_ref,
    ) {
        return false;
    }
    types
        .into_iter()
        .all(|t| check_ty_lifetimes(ctx, span, declared, declared, None, context, t))
        && extra
            .into_iter()
            .all(|l| check_used_lifetime(ctx, span, declared, declared, None, context, l))
}

/// Checks the types of a declaration: that C++ object references/pointers in `types` are valid
/// (see `check_cpp_objects_inner`), that `declared` is valid (no builtins or duplicates), and that
/// all lifetimes used in `types` and `extra` are declared (or `'static` / `'_`).
fn check_types<'t>(
    ctx: &mut ParseContext,
    span: Span,
    declared: &[Lifetime],
    types: impl IntoIterator<Item = &'t RustType>,
    extra: impl IntoIterator<Item = &'t Lifetime>,
    context: LifetimeContext,
    allow_cpp_ref: bool,
    allow_cpp_object_ref: bool,
) -> bool {
    let types: Vec<&RustType> = types.into_iter().collect();
    if !report_cpp_objects(
        ctx,
        span,
        types.iter().copied(),
        allow_cpp_ref,
        allow_cpp_object_ref,
    ) {
        return false;
    }
    validate_declared_lifetimes(ctx, span, declared)
        && types
            .into_iter()
            .all(|t| check_ty_lifetimes(ctx, span, declared, declared, None, context, t))
        && extra
            .into_iter()
            .all(|l| check_used_lifetime(ctx, span, declared, declared, None, context, l))
}

/// Checks the types and lifetimes of `method`, which can use its own lifetimes and `outer` (those
/// of its `impl` or `type` block).
fn check_method_types<'t>(
    ctx: &mut ParseContext,
    span: Span,
    outer: &[Lifetime],
    method: &'t ZngurMethod,
    extra_types: impl IntoIterator<Item = &'t RustType>,
) -> bool {
    let extra_types: Vec<&RustType> = extra_types.into_iter().collect();
    if !report_cpp_objects(
        ctx,
        span,
        method.inputs.iter().chain([&method.output]),
        true,
        true,
    ) || !report_cpp_objects(
        ctx,
        span,
        method.generics.iter().chain(extra_types.iter().copied()),
        false,
        false,
    ) {
        return false;
    }
    if let Some(l) = method.lifetimes.iter().find(|l| outer.contains(l)) {
        ctx.add_error_str(
            &format!("lifetime `{l}` is already declared by the `impl` or `type`"),
            span,
        );
        return false;
    }
    if !validate_declared_lifetimes(ctx, span, &method.lifetimes) {
        return false;
    }
    let declared: Vec<Lifetime> = outer.iter().chain(&method.lifetimes).cloned().collect();
    method
        .inputs
        .iter()
        .chain([&method.output])
        .chain(&method.generics)
        .chain(extra_types)
        .all(|t| {
            check_ty_lifetimes(
                ctx,
                span,
                &declared,
                &declared,
                None,
                LifetimeContext::Item,
                t,
            )
        })
        && method.receiver_lifetime.iter().all(|l| {
            check_used_lifetime(
                ctx,
                span,
                &declared,
                &declared,
                None,
                LifetimeContext::Item,
                l,
            )
        })
}

/// The receiver of a C++ member function: `&'a [unique] (const | var) self`, or a raw one
/// (`const self&`, `self*`, ...). `RustRef` is a (no longer valid) `&'a self` / `&'a mut self`.
enum ParsedReceiver<'a> {
    Ref {
        unique: bool,
        constness: CppConstness,
        lifetime: Option<&'a str>,
    },
    Raw(CppConstness, CppIndirection),
    RustRef(Mutability, Option<&'a str>),
}

fn as_receiver<'a>(ty: &ParsedRustType<'a>) -> Option<ParsedReceiver<'a>> {
    let is_self = |ty: &ParsedRustType<'_>| {
        matches!(ty, ParsedRustType::Adt(pg)
            if pg.path.start == ParsedPathStart::Relative
                && pg.path.segments == ["self"]
                && pg.generics.is_empty()
                && pg.lifetimes.is_empty()
                && pg.named_generics.is_empty())
    };
    match ty {
        ParsedRustType::Ref(kind, l, inner) if is_self(inner) => Some(match *kind {
            ParsedRefKind::Cpp { unique, constness } => ParsedReceiver::Ref {
                unique,
                // `&'a unique self` is rejected during `rust_type()` parsing.
                constness: constness.unwrap_or(CppConstness::Var),
                lifetime: *l,
            },
            ParsedRefKind::Rust(m) => ParsedReceiver::RustRef(m, *l),
        }),
        ParsedRustType::CppPtr(k, inner) => match &**inner {
            ParsedRustType::CppConst(inner) if is_self(inner) => {
                Some(ParsedReceiver::Raw(CppConstness::Const, *k))
            }
            inner if is_self(inner) => Some(ParsedReceiver::Raw(CppConstness::Var, *k)),
            _ => None,
        },
        _ => None,
    }
}

fn contains_relative_self(ty: &ParsedRustType<'_>) -> bool {
    let in_pg = |pg: &ParsedRustPathAndGenerics<'_>| {
        (pg.path.start == ParsedPathStart::Relative && pg.path.segments == ["self"])
            || pg
                .generics
                .iter()
                .chain(pg.named_generics.iter().map(|(_, t)| t))
                .any(contains_relative_self)
    };
    match ty {
        ParsedRustType::Primitive(_) => false,
        ParsedRustType::Ref(_, _, inner)
        | ParsedRustType::Raw(_, inner)
        | ParsedRustType::Boxed(inner)
        | ParsedRustType::Slice(inner)
        | ParsedRustType::CppPtr(_, inner)
        | ParsedRustType::CppConst(inner) => contains_relative_self(inner),
        ParsedRustType::Tuple(tys) => tys.iter().any(contains_relative_self),
        ParsedRustType::Adt(pg) => in_pg(pg),
        ParsedRustType::Dyn(tr, _) | ParsedRustType::Impl(tr, _) => match tr {
            ParsedRustTrait::Normal(pg) => in_pg(pg),
            ParsedRustTrait::Fn { inputs, output, .. } => {
                inputs.iter().chain([&**output]).any(contains_relative_self)
            }
        },
    }
}

fn is_rust_keyword(name: &str) -> bool {
    matches!(
        name,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "gen"
            | "macro"
            | "override"
            | "priv"
            | "try"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
    )
}

fn rust_keyword_fn_error(what: &str, name: &str, has_rename: bool) -> String {
    if has_rename {
        format!(
            "`{name}` is a Rust keyword and cannot be used as the Rust function name in `as {name}`"
        )
    } else {
        format!(
            "`{name}` is a Rust keyword; give this {what} a non-keyword Rust name with \
             `as <rust_name>`"
        )
    }
}

fn check_rust_fn_name(
    ctx: &mut ParseContext,
    span: Span,
    what: &str,
    name: &str,
    has_rename: bool,
    is_type_member: bool,
) -> bool {
    if is_rust_keyword(name) {
        ctx.add_error_str(&rust_keyword_fn_error(what, name, has_rename), span);
        return false;
    }
    if is_type_member && matches!(name, "as_const" | "as_const_mut" | "into_const") {
        let hint = if has_rename {
            "choose a different name in `as <rust_name>`"
        } else {
            "rename it with `as <rust_name>`"
        };
        ctx.add_error_str(
            &format!(
                "`{name}` collides with an inherent conversion method on `c++::` wrappers; {hint}"
            ),
            span,
        );
        return false;
    }
    true
}

/// Whether `ty` has a Rust reference without a lifetime (or with `'_`).
fn has_elided_ref(ty: &RustType) -> bool {
    match ty {
        RustType::Ref(_, None, _) => true,
        RustType::Ref(_, Some(l), t) => l.0 == "_" || has_elided_ref(t),
        RustType::Raw(_, t)
        | RustType::Boxed(t)
        | RustType::Slice(t)
        | RustType::CppPtr(_, t)
        | RustType::CppConst(t)
        | RustType::CppVar(t) => has_elided_ref(t),
        RustType::Tuple(tys) => tys.iter().any(has_elided_ref),
        RustType::Adt(pg)
        | RustType::Dyn(RustTrait::Normal(pg), _)
        | RustType::Impl(RustTrait::Normal(pg), _) => pg
            .generics
            .iter()
            .chain(pg.named_generics.iter().map(|(_, t)| t))
            .any(has_elided_ref),
        RustType::Dyn(RustTrait::Fn { inputs, output, .. }, _)
        | RustType::Impl(RustTrait::Fn { inputs, output, .. }, _) => {
            inputs.iter().chain([&**output]).any(has_elided_ref)
        }
        RustType::Cpp(path) => path.generics.iter().any(has_elided_ref),
        _ => false,
    }
}

/// The opt-out warnings for the signature of a C++ function: `elided_lifetimes` (a Rust reference
/// without a lifetime, which can't be checked against C++) and `safe_raw_pointer` (a `safe`
/// function that takes a raw pointer, which may be null or invalid).
#[allow(clippy::too_many_arguments)]
fn check_cpp_signature_lints(
    ctx: &mut ParseContext,
    span: Span,
    what: &str,
    is_safe: bool,
    elided_receiver: bool,
    raw_receiver: bool,
    inputs: &[RustType],
    output: &RustType,
) {
    let report_span = ctx.report_span_for_range(span.into_range());
    check_cpp_signature_lints_at(
        ctx,
        report_span,
        None,
        what,
        is_safe,
        elided_receiver,
        raw_receiver,
        inputs,
        output,
    );
}

const SAFE_RAW_POINTER_FIELD_WARNING: &str = "this C++ field accessor is `safe` but exposes \
    mutable access to a raw pointer, which may be set to a null or invalid pointer; declare it \
    `unsafe field` (or silence this with `#allow(safe_raw_pointer)`)";

fn elided_lifetimes_warning(what: &str) -> String {
    let example = match what {
        "constructor" => "safe constructor<'a>(&'a const c++::T) as from_ref",
        "member function" => "fn f<'a>(&'a const self) -> &'a const T",
        _ => "fn f<'a>(&'a const T) -> &'a const U",
    };
    format!(
        "this C++ {what} has a reference without a lifetime, which can't be checked \
         against C++; name the lifetimes (e.g. `{example}`), \
         or silence this with `#allow(elided_lifetimes)`"
    )
}

fn safe_raw_pointer_warning(what: &str) -> String {
    format!(
        "this C++ {what} is `safe` but takes a raw pointer, which may be null or invalid; \
         it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)"
    )
}

#[allow(clippy::too_many_arguments)]
fn check_cpp_signature_lints_at(
    ctx: &mut ParseContext,
    report_span: ReportSpan,
    extra_allowed: Option<&HashSet<String>>,
    what: &str,
    is_safe: bool,
    elided_receiver: bool,
    raw_receiver: bool,
    inputs: &[RustType],
    output: &RustType,
) {
    let is_allowed = |ctx: &ParseContext, lint: &str| {
        extra_allowed.map_or_else(|| ctx.allows(lint), |s| s.contains(lint))
    };
    if !is_allowed(ctx, "elided_lifetimes")
        && (elided_receiver || inputs.iter().chain([output]).any(has_elided_ref))
    {
        let warning = elided_lifetimes_warning(what);
        ctx.add_report(
            Report::build(ReportKind::Warning, report_span.clone())
                .with_message(&warning)
                .with_label(
                    Label::new(report_span.clone())
                        .with_message(&warning)
                        .with_color(Color::Yellow),
                )
                .finish(),
        );
    }
    if !is_allowed(ctx, "safe_raw_pointer")
        && is_safe
        && (raw_receiver || inputs.iter().any(contains_raw_ptr))
    {
        let warning = safe_raw_pointer_warning(what);
        ctx.add_report(
            Report::build(ReportKind::Warning, report_span.clone())
                .with_message(&warning)
                .with_label(
                    Label::new(report_span)
                        .with_message(&warning)
                        .with_color(Color::Yellow),
                )
                .finish(),
        );
    }
}

fn check_cpp_field_lints(ctx: &mut ParseContext, span: Span, field: &ZngurCppField) {
    if !ctx.allows("safe_raw_pointer") && is_mutable_raw_ptr_field(field) {
        ctx.add_warning_str(SAFE_RAW_POINTER_FIELD_WARNING, span);
    }
}

fn is_mutable_raw_ptr_field(field: &ZngurCppField) -> bool {
    field.is_safe
        && matches!(
            field.access,
            CppFieldAccess::Ref {
                unique,
                constness,
            } if unique || constness == CppConstness::Var
        )
        && contains_raw_ptr(&field.ty)
}

/// Reports named lifetimes in `types`, for constructs like C++ fields that don't support them.
fn reject_lifetimes<'t>(
    ctx: &mut ParseContext,
    span: Span,
    what: &str,
    types: impl IntoIterator<Item = &'t RustType>,
) -> bool {
    match types.into_iter().flat_map(RustType::free_lifetimes).next() {
        Some(l) => {
            ctx.add_error_str(
                &format!("lifetimes (`{l}`) aren't supported in the signatures of {what} yet"),
                span,
            );
            false
        }
        None => true,
    }
}

/// Whether `ty` is or contains a raw pointer (a C++-style `T&` / `T*`, or `*const T` / `*mut T`),
/// which may be null or invalid.
fn contains_raw_ptr(ty: &RustType) -> bool {
    match ty {
        RustType::CppPtr(..) | RustType::Raw(..) => true,
        RustType::CppConst(t)
        | RustType::CppVar(t)
        | RustType::Ref(_, _, t)
        | RustType::Boxed(t)
        | RustType::Slice(t) => contains_raw_ptr(t),
        RustType::Tuple(tys) => tys.iter().any(contains_raw_ptr),
        RustType::Adt(pg) | RustType::Dyn(RustTrait::Normal(pg), _) => pg
            .generics
            .iter()
            .chain(pg.named_generics.iter().map(|(_, t)| t))
            .any(contains_raw_ptr),
        RustType::Dyn(RustTrait::Fn { inputs, output, .. }, _) => {
            inputs.iter().chain([&**output]).any(contains_raw_ptr)
        }
        RustType::Cpp(path) => path.generics.iter().any(contains_raw_ptr),
        _ => false,
    }
}

/// Reports a C++ member whose Rust name `name` an earlier constructor, member function or field
/// accessor of the same type already has, and returns whether there was none.
fn check_member_name(
    ctx: &mut ParseContext,
    span: Span,
    name: &str,
    constructors: &[ZngurCppConstructor],
    methods: &[ZngurCppMethod],
    fields: &[ZngurCppField],
) -> bool {
    let taken = constructors.iter().any(|c| c.rust_name == name)
        || methods.iter().any(|m| m.rust_name() == name)
        || fields.iter().any(|f| f.rust_name() == name);
    if taken {
        ctx.add_error_str(&duplicate_member_name(name), span);
    }
    !taken
}

fn duplicate_member_name(name: &str) -> String {
    format!(
        "`{name}` is already the Rust name of another member of this type \
         (a constructor, member function or field accessor)"
    )
}

/// How `ty` is spelled in a `.zng` file (for diagnostics).
fn zng_spelling(ty: &RustType) -> String {
    fn zng_east_const_spelling(ty: &RustType) -> String {
        match ty {
            RustType::CppPtr(CppIndirection::Pointer, t) => {
                format!("{}*", zng_east_const_spelling(t))
            }
            RustType::CppPtr(CppIndirection::Reference, t) => {
                format!("{}&", zng_east_const_spelling(t))
            }
            RustType::CppConst(t) => format!("{} const", zng_east_const_spelling(t)),
            RustType::CppVar(t) => zng_east_const_spelling(t),
            _ => zng_spelling(ty),
        }
    }

    fn zng_path_spelling(pg: &RustPathAndGenerics) -> String {
        let Some((first, rest)) = pg.path.split_first() else {
            return String::new();
        };
        let mut out = String::new();
        if first != "crate" {
            out.push_str("::");
        }
        out.push_str(first);
        for seg in rest {
            out.push_str("::");
            out.push_str(seg);
        }
        if !pg.lifetimes.is_empty() || !pg.generics.is_empty() || !pg.named_generics.is_empty() {
            let args = pg
                .lifetimes
                .iter()
                .map(|l| l.to_string())
                .chain(pg.generics.iter().map(zng_spelling))
                .chain(
                    pg.named_generics
                        .iter()
                        .map(|(k, v)| format!("{k} = {}", zng_spelling(v))),
                )
                .join(", ");
            out.push('<');
            out.push_str(&args);
            out.push('>');
        }
        out
    }

    fn zng_trait_spelling(tr: &RustTrait) -> String {
        match tr {
            RustTrait::Normal(pg) => zng_path_spelling(pg),
            RustTrait::Fn {
                name,
                lifetimes,
                inputs,
                output,
            } => {
                let mut out = String::new();
                if !lifetimes.is_empty() {
                    out.push_str("for");
                    out.push_str(&zngur_def::lifetime_params(lifetimes));
                    out.push(' ');
                }
                out.push_str(name);
                out.push('(');
                out.push_str(&inputs.iter().map(zng_spelling).join(", "));
                out.push(')');
                if **output != RustType::UNIT {
                    out.push_str(" -> ");
                    out.push_str(&zng_spelling(output));
                }
                out
            }
        }
    }

    match ty {
        RustType::Primitive(prim) => prim.to_string(),
        RustType::Ref(m, l, inner) => {
            let lt = l.as_ref().map(|l| format!("{l} ")).unwrap_or_default();
            match inner.as_cpp_object() {
                Some((constness, obj_inner)) => {
                    let u = if *m == Mutability::Mut { "unique " } else { "" };
                    format!(
                        "&{lt}{u}{} {}",
                        constness.keyword(),
                        zng_east_const_spelling(obj_inner)
                    )
                }
                None => match m {
                    Mutability::Mut => format!("&{lt}mut {}", zng_spelling(inner)),
                    Mutability::Not => format!("&{lt}{}", zng_east_const_spelling(inner)),
                },
            }
        }
        RustType::Raw(Mutability::Not, t) => format!("*const {}", zng_spelling(t)),
        RustType::Raw(Mutability::Mut, t) => format!("*mut {}", zng_spelling(t)),
        RustType::Boxed(t) => format!("Box<{}>", zng_spelling(t)),
        RustType::Slice(t) => format!("[{}]", zng_spelling(t)),
        RustType::Dyn(tr, bounds) => {
            let mut out = format!("dyn {}", zng_trait_spelling(tr));
            for b in bounds {
                out.push_str(&format!(" + {b}"));
            }
            out
        }
        RustType::Impl(tr, bounds) => {
            let mut out = format!("impl {}", zng_trait_spelling(tr));
            for b in bounds {
                out.push_str(&format!(" + {b}"));
            }
            out
        }
        RustType::Tuple(tys) => match tys.as_slice() {
            [single] => format!("({},)", zng_spelling(single)),
            _ => format!("({})", tys.iter().map(zng_spelling).join(", ")),
        },
        RustType::Adt(pg) => zng_path_spelling(pg),
        RustType::TypeVar(var) => var.0.clone(),
        RustType::Cpp(path) if path.generics.is_empty() => format!("c++::{}", path.path.join("::")),
        RustType::Cpp(path) => format!(
            "c++::{}<{}>",
            path.path.join("::"),
            path.generics.iter().map(zng_spelling).join(", ")
        ),
        RustType::CppConst(t) if matches!(**t, RustType::CppPtr(..)) => {
            format!("{} const", zng_spelling(t))
        }
        RustType::CppConst(t) => format!("const {}", zng_spelling(t)),
        RustType::CppVar(t) => zng_spelling(t),
        RustType::CppPtr(CppIndirection::Pointer, t) => format!("{}*", zng_spelling(t)),
        RustType::CppPtr(CppIndirection::Reference, t) => format!("{}&", zng_spelling(t)),
    }
}

fn is_valid_cpp_object_inner(ty: &RustType) -> bool {
    matches!(
        ty,
        RustType::Cpp(_)
            | RustType::CppPtr(CppIndirection::Pointer, _)
            | RustType::TypeVar(_)
            | RustType::Primitive(
                PrimitiveRustType::Int(8 | 16 | 32 | 64)
                    | PrimitiveRustType::Uint(8 | 16 | 32 | 64)
                    | PrimitiveRustType::Float(32 | 64)
                    | PrimitiveRustType::Usize
                    | PrimitiveRustType::Bool,
            )
    )
}

/// Checks `RustType::Cpp(_)` occurrences in the signature of a direct C++ item (member function,
/// constructor, free function, field): every `c++::` type must be declared locally or imported
/// (when `check_undeclared` is true), and can only appear by value if it is held by value in Rust
/// (`#layout(...)` and `#trivially_relocatable`).
fn check_cpp_signature_type(
    ty: &RustType,
    in_indirection: bool,
    known_cpp_types: &HashSet<CppTypePath>,
    known_cpp_paths: &HashSet<Vec<String>>,
    by_value_cpp_types: &HashSet<CppTypePath>,
    check_undeclared: bool,
) -> Result<(), String> {
    let check_all = |tys: &[RustType], ind: bool| {
        tys.iter().try_for_each(|t| {
            check_cpp_signature_type(
                t,
                ind,
                known_cpp_types,
                known_cpp_paths,
                by_value_cpp_types,
                check_undeclared,
            )
        })
    };
    match ty {
        RustType::Cpp(path) => {
            check_all(&path.generics, true)?;
            let RustType::Cpp(erased_path) = ty.with_erased_lifetimes() else {
                unreachable!()
            };
            let is_known = known_cpp_types.contains(&erased_path);
            if !is_known {
                // Arity mismatches on declared/imported `c++::` types and undeclared template
                // instantiations are reported with dedicated diagnostics by
                // `check_cpp_template_uses`.
                if check_undeclared
                    && path.generics.is_empty()
                    && !known_cpp_paths.contains(&path.path)
                {
                    return Err(format!(
                        "`c++::{}` is not declared or imported",
                        path.path.join("::")
                    ));
                }
            } else if !in_indirection && !by_value_cpp_types.contains(&erased_path) {
                return Err(format!(
                    "`{}` cannot be passed or returned by value without `#layout(...)` and \
                     `#trivially_relocatable`",
                    zng_spelling(ty)
                ));
            }
            Ok(())
        }
        RustType::Ref(_, _, t) | RustType::Raw(_, t) | RustType::CppPtr(_, t) => {
            check_cpp_signature_type(
                t,
                true,
                known_cpp_types,
                known_cpp_paths,
                by_value_cpp_types,
                check_undeclared,
            )
        }
        RustType::CppConst(t) | RustType::CppVar(t) => check_cpp_signature_type(
            t,
            in_indirection,
            known_cpp_types,
            known_cpp_paths,
            by_value_cpp_types,
            check_undeclared,
        ),
        RustType::Boxed(t) | RustType::Slice(t) => check_cpp_signature_type(
            t,
            false,
            known_cpp_types,
            known_cpp_paths,
            by_value_cpp_types,
            check_undeclared,
        ),
        RustType::Tuple(tys) => check_all(tys, false),
        RustType::Adt(pg) => {
            check_all(&pg.generics, false)?;
            pg.named_generics.iter().try_for_each(|(_, t)| {
                check_cpp_signature_type(
                    t,
                    false,
                    known_cpp_types,
                    known_cpp_paths,
                    by_value_cpp_types,
                    check_undeclared,
                )
            })
        }
        RustType::Impl(..) => Err(
            "`impl Trait` cannot be used in C++ function signatures; use `Box<dyn Trait>` or \
             `&dyn Trait` instead"
                .to_owned(),
        ),
        RustType::Dyn(tr, _) => match tr {
            RustTrait::Normal(pg) => {
                check_all(&pg.generics, false)?;
                pg.named_generics.iter().try_for_each(|(_, t)| {
                    check_cpp_signature_type(
                        t,
                        false,
                        known_cpp_types,
                        known_cpp_paths,
                        by_value_cpp_types,
                        check_undeclared,
                    )
                })
            }
            RustTrait::Fn { inputs, output, .. } => {
                check_all(inputs, false)?;
                check_cpp_signature_type(
                    output,
                    false,
                    known_cpp_types,
                    known_cpp_paths,
                    by_value_cpp_types,
                    check_undeclared,
                )
            }
        },
        RustType::Primitive(_) | RustType::TypeVar(_) => Ok(()),
    }
}

fn contains_cpp_type(ty: &RustType) -> bool {
    match ty {
        RustType::Cpp(_) => true,
        RustType::Ref(_, _, t)
        | RustType::Raw(_, t)
        | RustType::Boxed(t)
        | RustType::Slice(t)
        | RustType::CppPtr(_, t)
        | RustType::CppConst(t)
        | RustType::CppVar(t) => contains_cpp_type(t),
        RustType::Tuple(tys) => tys.iter().any(contains_cpp_type),
        RustType::Adt(pg) => {
            pg.generics.iter().any(contains_cpp_type)
                || pg.named_generics.iter().any(|(_, t)| contains_cpp_type(t))
        }
        RustType::Dyn(tr, _) | RustType::Impl(tr, _) => match tr {
            RustTrait::Normal(pg) => {
                pg.generics.iter().any(contains_cpp_type)
                    || pg.named_generics.iter().any(|(_, t)| contains_cpp_type(t))
            }
            RustTrait::Fn { inputs, output, .. } => {
                inputs.iter().any(contains_cpp_type) || contains_cpp_type(output)
            }
        },
        RustType::Primitive(_) | RustType::TypeVar(_) => false,
    }
}

struct CppHeapAllocatedInfo<'a> {
    heap_allocated: &'a HashSet<Vec<String>>,
    /// When parsing an `import`ed module (whose own `import`s aren't resolved), only `c++::`
    /// types declared locally in that module have a known `#cpp_heap_allocated` status.
    known_only_local: Option<&'a HashSet<Vec<String>>>,
}

impl CppHeapAllocatedInfo<'_> {
    fn is_heap_allocated(&self, path: &CppTypePath) -> Option<bool> {
        if !path.generics.is_empty() {
            return Some(false);
        }
        if let Some(local) = self.known_only_local
            && !local.contains(&path.path)
        {
            return None;
        }
        Some(self.heap_allocated.contains(&path.path))
    }
}

struct DeferredCppObjectCheck {
    report_span: ReportSpan,
    types: Vec<RustType>,
    allow_cpp_ref: bool,
    allow_cpp_object_ref: bool,
}

/// Checks the references and pointers to C++ objects in `ty`, once its paths are resolved: a
/// reference to a C++ class must say whether the object is `const` (`&'a const T`,
/// `&'a unique var T`, ...), and only C++ objects (C++ classes, builtins and C++ pointers) can be
/// `const` or `var` or pointed to by `T*` / `T&`. Type variables of templates are checked once
/// they're substituted, and checks that depend on whether a `c++::` type is `#cpp_heap_allocated`
/// run once all merged files and imported modules have been resolved (`heap_info.is_some()`).
fn check_cpp_objects_inner(
    ty: &RustType,
    allow_cpp_ref: bool,
    allow_cpp_object_ref: bool,
    heap_info: Option<&CppHeapAllocatedInfo<'_>>,
) -> Result<(), String> {
    check_cpp_objects_ctx(ty, allow_cpp_ref, allow_cpp_object_ref, false, heap_info)
}

fn check_cpp_objects_ctx(
    ty: &RustType,
    allow_cpp_ref: bool,
    allow_cpp_object_ref: bool,
    allow_cpp_const: bool,
    heap_info: Option<&CppHeapAllocatedInfo<'_>>,
) -> Result<(), String> {
    let all = |tys: &[RustType], allow_ref: bool, allow_obj_ref: bool, allow_const: bool| {
        tys.iter().try_for_each(|t| {
            check_cpp_objects_ctx(t, allow_ref, allow_obj_ref, allow_const, heap_info)
        })
    };
    let is_not_heap_allocated = |path: &CppTypePath| {
        !path.generics.is_empty()
            || heap_info.and_then(|i| i.is_heap_allocated(path)) == Some(false)
    };
    match ty {
        RustType::Ref(m, l, inner) => {
            let l = l.as_ref().map(|l| format!("{l} ")).unwrap_or_default();
            match &**inner {
                RustType::Cpp(path) => {
                    if is_not_heap_allocated(path) {
                        let inner = zng_spelling(inner);
                        let suggestion = match m {
                            Mutability::Not => format!(
                                "`&{l}const {inner}` (a C++ `const T&`, `&T<Const>` in Rust) or \
                                 `&{l}var {inner}` (a shared `T&`, `&T` in Rust)"
                            ),
                            Mutability::Mut => format!(
                                "`&{l}unique var {inner}` (a `T&` that Rust uses uniquely, `&mut T` \
                                 in Rust)"
                            ),
                        };
                        Err(format!(
                            "a reference to a C++ class must say whether the object is `const`: \
                             {suggestion}"
                        ))
                    } else {
                        Ok(())
                    }
                }
                RustType::CppConst(t) | RustType::CppVar(t) => {
                    if let RustType::Cpp(path) = &**t
                        && heap_info.and_then(|i| i.is_heap_allocated(path)) == Some(true)
                    {
                        let m = if *m == Mutability::Mut { "mut " } else { "" };
                        let inner = zng_spelling(t);
                        return Err(format!(
                            "`{inner}` is `#cpp_heap_allocated` (its Rust wrapper holds an owning \
                             pointer rather than the C++ object inline), so references to it \
                             cannot be `const` or `var`: use `&{l}{m}{inner}`"
                        ));
                    }
                    if is_valid_cpp_object_inner(t) {
                        check_cpp_objects_ctx(t, false, false, false, heap_info)?;
                        if !allow_cpp_object_ref {
                            let ty_str = zng_spelling(ty);
                            return Err(format!(
                                "reference to a C++ object `{ty_str}` is a C++ reference in C++ \
                                 and can only be a function, method, or constructor parameter or \
                                 return type (use `::std::option::Option<{ty_str}>` for a \
                                 nullable C++ pointer)"
                            ));
                        }
                        Ok(())
                    } else if matches!(**t, RustType::Primitive(PrimitiveRustType::Char)) {
                        let m = if *m == Mutability::Mut { "mut " } else { "" };
                        Err(format!(
                            "`char` can't be a C++ object (C++ can store invalid code points in \
                             it), so references to it can't be `const` or `var`: use \
                             `&{l}{m}char` (a Rust value)"
                        ))
                    } else if matches!(
                        **t,
                        RustType::Primitive(
                            PrimitiveRustType::Int(_)
                                | PrimitiveRustType::Uint(_)
                                | PrimitiveRustType::Float(_)
                        )
                    ) {
                        let m = if *m == Mutability::Mut { "mut " } else { "" };
                        let t = zng_spelling(t);
                        Err(format!(
                            "`{t}` is not a C++ builtin scalar (only `bool`, 8/16/32/64-bit \
                             integers, `usize`, `f32`, and `f64` are), so references to it \
                             can't be `const` or `var`: use `&{l}{m}{t}` (a Rust value)"
                        ))
                    } else if matches!(**t, RustType::CppPtr(CppIndirection::Reference, _)) {
                        Err(
                            "references to C++ references aren't valid; use `&'a const T` / \
                             `&'a var T` or `const T&` / `T&`"
                                .to_owned(),
                        )
                    } else {
                        let m = if *m == Mutability::Mut { "mut " } else { "" };
                        let t = zng_spelling(t);
                        Err(format!(
                            "only C++ objects (`c++::` types, builtins and C++ pointers) can be \
                             `const` or `var`, and `{t}` is a Rust type: use `&{l}{m}{t}`"
                        ))
                    }
                }
                RustType::CppPtr(CppIndirection::Reference, _) => Err(
                    "references to C++ references aren't valid; use `&'a const T` / \
                     `&'a var T` or `const T&` / `T&`"
                        .to_owned(),
                ),
                _ => check_cpp_objects_ctx(inner, false, false, false, heap_info),
            }
        }
        RustType::Raw(m, inner) => match &**inner {
            RustType::Cpp(path) => {
                if is_not_heap_allocated(path) {
                    let inner_str = zng_spelling(inner);
                    let suggestion = match m {
                        Mutability::Not => format!("`const {inner_str}*`"),
                        Mutability::Mut => format!("`{inner_str}*`"),
                    };
                    Err(format!(
                        "a raw pointer to a C++ class must say whether the object is `const`: \
                         use {suggestion}"
                    ))
                } else {
                    Ok(())
                }
            }
            RustType::CppConst(t) | RustType::CppVar(t) => {
                let inner_str = zng_spelling(t);
                let suggestion = match m {
                    Mutability::Not => format!("`const {inner_str}*`"),
                    Mutability::Mut => format!("`{inner_str}*`"),
                };
                Err(format!(
                    "a raw pointer to a C++ object is written in C++ style: use {suggestion}"
                ))
            }
            RustType::CppPtr(CppIndirection::Reference, _) => {
                Err("pointers to C++ references aren't valid".to_owned())
            }
            _ => check_cpp_objects_ctx(inner, false, false, false, heap_info),
        },
        RustType::CppPtr(indirection, pointee) => {
            let inner = match &**pointee {
                RustType::CppConst(t) | RustType::CppVar(t) => &**t,
                t => t,
            };
            if matches!(inner, RustType::CppPtr(CppIndirection::Reference, _)) {
                return match indirection {
                    CppIndirection::Pointer => {
                        Err("pointers to C++ references (`T&*`) aren't valid".to_owned())
                    }
                    CppIndirection::Reference => {
                        Err("references to C++ references (`T&&`) aren't valid".to_owned())
                    }
                };
            }
            if matches!(inner, RustType::Primitive(PrimitiveRustType::Char)) {
                return Err(
                    "`char` can't be a C++ object (C++ can store invalid code points in it), so \
                     C++ pointers and references can't point to it"
                        .to_owned(),
                );
            }
            if matches!(
                inner,
                RustType::Primitive(
                    PrimitiveRustType::Int(_)
                        | PrimitiveRustType::Uint(_)
                        | PrimitiveRustType::Float(_)
                )
            ) && !is_valid_cpp_object_inner(inner)
            {
                return Err(format!(
                    "`{}` is not a C++ builtin scalar (only `bool`, 8/16/32/64-bit integers, \
                     `usize`, `f32`, and `f64` are), so C++ pointers and references can't point \
                     to it",
                    zng_spelling(inner)
                ));
            }
            if !is_valid_cpp_object_inner(inner) {
                return Err(format!(
                    "C++ pointers and references (`T*`, `T&`) can only point to C++ objects \
                     (`c++::` types, builtins and C++ pointers), not `{}`",
                    zng_spelling(inner)
                ));
            }
            if *indirection == CppIndirection::Reference && !allow_cpp_ref {
                return Err(format!(
                    "C++ reference `{}` can only be a function parameter, return type, or C++ \
                     template argument",
                    zng_spelling(ty)
                ));
            }
            if let RustType::Cpp(path) = inner
                && heap_info.and_then(|i| i.is_heap_allocated(path)) == Some(true)
            {
                let inner_str = zng_spelling(inner);
                return Err(format!(
                    "`{inner_str}` is `#cpp_heap_allocated` (its Rust wrapper holds an owning \
                     pointer rather than the C++ object inline), so C++ pointers and references \
                     (`{inner_str}*`, `{inner_str}&`) cannot point to it: use `&{inner_str}` or \
                     `&mut {inner_str}`"
                ));
            }
            check_cpp_objects_ctx(pointee, false, false, true, heap_info)
        }
        RustType::CppConst(t) | RustType::CppVar(t) => {
            if let RustType::Cpp(path) = &**t
                && heap_info.and_then(|i| i.is_heap_allocated(path)) == Some(true)
            {
                let inner_str = zng_spelling(t);
                return Err(format!(
                    "`{inner_str}` is `#cpp_heap_allocated` (its Rust wrapper holds an owning \
                     pointer rather than the C++ object inline), so it cannot be `const` or `var`"
                ));
            }
            if !is_valid_cpp_object_inner(t) {
                if matches!(**t, RustType::Primitive(PrimitiveRustType::Char)) {
                    Err(
                        "`char` can't be a C++ object (C++ can store invalid code points in it)"
                            .to_owned(),
                    )
                } else {
                    Err(format!(
                        "only C++ objects (`c++::` types, builtins and C++ pointers) can be \
                         `const` or `var`, not `{}`",
                        zng_spelling(t)
                    ))
                }
            } else if !allow_cpp_const {
                Err(
                    "`const T` can only be the type of a C++ pointer or reference \
                     (`const T&`, `const T*`, `&'a const T`) or a C++ template argument"
                        .to_owned(),
                )
            } else {
                check_cpp_objects_ctx(t, false, false, false, heap_info)
            }
        }
        RustType::Boxed(t) | RustType::Slice(t) => {
            check_cpp_objects_ctx(t, false, false, false, heap_info)
        }
        RustType::Tuple(tys) => all(tys, false, false, false),
        RustType::Adt(pg) => {
            if ty.as_nullable_cpp_ref().is_some() {
                return check_cpp_objects_ctx(&pg.generics[0], false, true, false, heap_info);
            }
            all(&pg.generics, false, false, false)?;
            pg.named_generics
                .iter()
                .try_for_each(|(_, t)| check_cpp_objects_ctx(t, false, false, false, heap_info))
        }
        RustType::Cpp(path) => {
            for g in &path.generics {
                check_cpp_objects_ctx(g, true, false, true, heap_info)?;
                if g.type_vars().is_empty() {
                    g.cpp_spelling()?;
                }
            }
            Ok(())
        }
        RustType::Dyn(tr, _) | RustType::Impl(tr, _) => match tr {
            RustTrait::Normal(pg) => {
                check_cpp_objects_ctx(&RustType::Adt(pg.clone()), false, false, false, heap_info)
            }
            RustTrait::Fn { inputs, output, .. } => {
                all(inputs, false, true, false)?;
                check_cpp_objects_ctx(output, false, true, false, heap_info)
            }
        },
        RustType::Primitive(_) | RustType::TypeVar(_) => Ok(()),
    }
}

/// Reports the first immediate error of [`check_cpp_objects_inner`] in `types` (recording any
/// `c++::`-bearing types for post-merge `#cpp_heap_allocated` validation), and returns whether
/// there was none.
fn report_cpp_objects<'t>(
    ctx: &mut ParseContext,
    span: Span,
    types: impl IntoIterator<Item = &'t RustType>,
    allow_cpp_ref: bool,
    allow_cpp_object_ref: bool,
) -> bool {
    let types: Vec<&RustType> = types.into_iter().collect();
    match types
        .iter()
        .try_for_each(|t| check_cpp_objects_inner(t, allow_cpp_ref, allow_cpp_object_ref, None))
    {
        Ok(()) => {
            let cpp_types: Vec<RustType> = types
                .into_iter()
                .filter(|t| contains_cpp_type(t))
                .cloned()
                .collect();
            if !cpp_types.is_empty() {
                let report_span = ctx.report_span_for_range(span.into_range());
                ctx.deferred_cpp_object_checks.push(DeferredCppObjectCheck {
                    report_span,
                    types: cpp_types,
                    allow_cpp_ref,
                    allow_cpp_object_ref,
                });
            }
            true
        }
        Err(message) => {
            ctx.add_error_str(&message, span);
            false
        }
    }
}

/// Checks the type of a C++ field accessor: a C++ object (a `c++::` type, a builtin or a C++
/// pointer), or a type variable of a template (checked once it's substituted).
fn check_cpp_field(
    field: &ZngurCppField,
    heap_info: Option<&CppHeapAllocatedInfo<'_>>,
) -> Result<(), String> {
    let ty = match &field.ty {
        RustType::CppConst(inner) => {
            if let CppFieldAccess::Ref { unique, constness } = field.access {
                if constness == CppConstness::Var {
                    return Err(format!(
                        "the field `{}` has `const` type `{}`, so its reference accessor must be \
                         `const` (`field &const {} ...`), not `var`",
                        field.name,
                        zng_spelling(&field.ty),
                        field.name
                    ));
                }
                if unique {
                    return Err(format!(
                        "the field `{}` has `const` type `{}`, so `field &unique const` cannot be \
                         used (`field &const {} ...` instead)",
                        field.name,
                        zng_spelling(&field.ty),
                        field.name
                    ));
                }
            }
            &**inner
        }
        ty => ty,
    };
    match ty {
        RustType::CppPtr(CppIndirection::Reference, _) => Err(format!(
            "the field `{}` has the reference type `{}`, and fields of reference type aren't \
             supported",
            field.name,
            zng_spelling(ty)
        )),
        ty if is_valid_cpp_object_inner(ty) => {
            if let RustType::Cpp(path) = ty
                && heap_info.and_then(|i| i.is_heap_allocated(path)) == Some(true)
            {
                let inner_str = zng_spelling(ty);
                return Err(format!(
                    "`{inner_str}` is `#cpp_heap_allocated` (its Rust wrapper holds an owning \
                     pointer rather than the C++ object inline), so it cannot be a C++ field type"
                ));
            }
            check_cpp_objects_inner(ty, false, false, heap_info)
        }
        ty => Err(format!(
            "the field `{}` has the type `{}`, but C++ fields are C++ objects: `c++::` types, \
             builtins (other than `char` and `str`) or C++ pointers",
            field.name,
            zng_spelling(ty)
        )),
    }
}

struct MergeContext {
    outer_ty: RustType,
    variant: Option<String>,
}

/// The C++ type name of a `#cpp_heap_allocated`, `#cpp_ref` or `#cpp_stack_owned` directive on
/// `ty`: the explicit one if given, otherwise the path of a `c++::` type (`c++::a::B` is `::a::B`).
fn cpp_type_name(
    explicit: Option<&str>,
    ty: &RustType,
    span: Span,
    ctx: &mut ParseContext,
) -> Option<String> {
    match (explicit, ty) {
        (Some(name), _) => Some(name.to_owned()),
        (None, RustType::Cpp(_)) => ty.cpp_spelling().ok(),
        (None, _) => {
            ctx.add_error_str(
                "the C++ type name can only be omitted for `c++::` types, whose path is the name",
                span,
            );
            None
        }
    }
}

fn checked_merge<T, U>(
    src: T,
    dst: &mut U,
    span: Span,
    ctx: &mut ParseContext,
    src_ctx: Option<MergeContext>,
) where
    T: Merge<U>,
{
    match src.merge(dst) {
        Ok(()) => {}
        Err(e) => match e {
            MergeFailure::Conflict(s, conflict) => {
                ctx.add_fatal_report(build_merge_conflict_report(
                    ctx, span, &s, conflict, src_ctx,
                ));
            }
        },
    }
}

fn build_template_conflict_report(
    ctx: &ParseContext,
    template: &TemplateDef,
    target_ty: &ZngurType,
    template_span: ReportSpan,
    msg: &str,
    conflict: (ConflictSource, ConflictSource),
) -> ParseReport<'static> {
    let spans = ctx.fetch_spans_global(target_ty);
    let (first, rest) = {
        let mut it = spans.into_iter();
        let first = it.next().cloned();
        (first, it.collect::<Vec<_>>())
    };

    let mut report = Report::build(ReportKind::Error, (0usize, 0usize..0))
        .with_message(format!(
            "Failed to apply template {} to type {}: {msg}",
            template.ty.ty, target_ty.ty
        ))
        .with_label(
            Label::new(template_span)
                .with_message("Template declared here")
                .with_color(Color::Blue),
        );

    if let Some(first) = first {
        report.add_label(
            Label::new(first)
                .with_message("Type first declared here.")
                .with_color(Color::Blue),
        );
    }

    add_conflict_labels(
        ctx,
        &mut report,
        conflict,
        &MergeContext {
            outer_ty: target_ty.ty.clone(),
            variant: None,
        },
    );

    let count = rest.len();
    for (i, span) in rest.into_iter().enumerate() {
        report.add_label(
            Label::new(span.clone())
                .with_message("Type also declared here")
                .with_color(Color::Blue),
        );
        if i >= 2 {
            report.add_note(format!(
                "{} additional type declaration sites omitted",
                count - i
            ));
            break;
        }
    }
    report.finish()
}

fn build_template_pair_conflict_report(
    ctx: &ParseContext,
    earlier_template: &TemplateDef,
    later_template: &TemplateDef,
    msg: &str,
    conflict: (ConflictSource, ConflictSource),
) -> ParseReport<'static> {
    let earlier_span = (
        earlier_template.source_id,
        earlier_template.span.into_range(),
    );
    let later_span = (later_template.source_id, later_template.span.into_range());
    let mut report = Report::build(ReportKind::Error, later_span.clone())
        .with_message(format!(
            "Failed to apply template {} to template {}: {msg}",
            zng_spelling(&later_template.ty.ty),
            zng_spelling(&earlier_template.ty.ty),
        ))
        .with_label(
            Label::new(later_span)
                .with_message("Template declared here")
                .with_color(Color::Blue),
        )
        .with_label(
            Label::new(earlier_span)
                .with_message("Earlier template declared here")
                .with_color(Color::Blue),
        );

    if later_template.ty.ty == earlier_template.ty.ty {
        add_conflict_labels(
            ctx,
            &mut report,
            conflict,
            &MergeContext {
                outer_ty: later_template.ty.ty.clone(),
                variant: None,
            },
        );
    } else {
        let (src_conflict, dst_conflict) = conflict;
        let src_key = src_conflict.into_span_key_with_variant(later_template.ty.ty.clone(), None);
        let dst_key = dst_conflict.into_span_key_with_variant(earlier_template.ty.ty.clone(), None);
        if let Some(src_span) = ctx.fetch_spans_global(&src_key).last().copied().cloned() {
            report.add_label(
                Label::new(src_span)
                    .with_message("declaration here ...")
                    .with_color(Color::Yellow),
            );
        }
        if let Some(dst_span) = ctx.fetch_spans_global(&dst_key).last().copied().cloned() {
            report.add_label(
                Label::new(dst_span)
                    .with_message("conflicts with the declaration here.")
                    .with_color(Color::Yellow),
            );
        }
    }

    report.finish()
}

fn build_merge_conflict_report(
    ctx: &ParseContext,
    span: Span,
    msg: &str,
    conflict: (ConflictSource, ConflictSource),
    merge_ctx: Option<MergeContext>,
) -> ParseReport<'static> {
    let report_span = ctx.report_span_for_range(span.into_range());
    let mut report = Report::build(ReportKind::Error, report_span.clone())
        .with_message(msg)
        .with_label(
            Label::new(report_span)
                .with_message(msg)
                .with_color(Color::Red),
        );
    if let Some(merge_ctx) = &merge_ctx {
        add_conflict_labels(ctx, &mut report, conflict, merge_ctx);
    }
    report.finish()
}

fn add_conflict_labels(
    ctx: &ParseContext,
    report: &mut ariadne::ReportBuilder<'static, ReportSpan>,
    conflict: (ConflictSource, ConflictSource),
    merge_ctx: &MergeContext,
) {
    let (src_conflict, dst_conflict) = conflict;
    let src_span_key = src_conflict
        .clone()
        .into_span_key_with_variant(merge_ctx.outer_ty.clone(), merge_ctx.variant.clone());
    let dst_span_key = dst_conflict
        .clone()
        .into_span_key_with_variant(merge_ctx.outer_ty.clone(), merge_ctx.variant.clone());
    let src_spans = ctx.fetch_spans_global(&src_span_key);
    let dst_spans = ctx.fetch_spans_global(&dst_span_key);
    let (src_span, dst_span, first_span, rest) = if src_span_key == dst_span_key {
        let mut src = src_spans.into_iter();
        (
            src.next_back().cloned(),
            src.next_back().cloned(),
            src.next().cloned(),
            src.collect::<Vec<_>>(),
        )
    } else {
        let mut src = src_spans.into_iter();
        let mut dst = dst_spans.into_iter();
        (
            src.next_back().cloned(),
            dst.next_back().cloned(),
            dst.next().cloned(),
            dst.collect::<Vec<_>>(),
        )
    };
    if let Some(src_span) = src_span {
        report.add_label(
            Label::new(src_span)
                .with_message("declaration here ...")
                .with_color(Color::Yellow),
        );
    }
    if let Some(dst_span) = dst_span {
        report.add_label(
            Label::new(dst_span)
                .with_message("conflicts with the declaration here.")
                .with_color(Color::Yellow),
        );
    }
    if let Some(first_span) = first_span {
        report.add_label(
            Label::new(first_span)
                .with_message("first declared here.")
                .with_color(Color::Yellow),
        );
    }
    let count = rest.len();
    for (i, span) in rest.into_iter().enumerate() {
        report.add_label(
            Label::new(span.clone())
                .with_message("also declared here.")
                .with_color(Color::Blue),
        );
        if i >= 2 {
            report.add_note(format!("{} additional conflict sites omitted", count - i));
        }
    }
}

impl ProcessedItem<'_> {
    fn add_to_zngur_spec(
        self,
        r: &mut ZngurSpecBuilder,
        scope: &Scope<'_>,
        ctx: &mut ParseContext,
    ) {
        match self {
            ProcessedItem::Mod {
                path,
                items,
                aliases,
            } => {
                let is_root = matches!(scope.base, EntityPath::Rust(ref v) if v.is_empty());
                // Only a relative mod path composes onto the enclosing module;
                // every anchored start (`c++::`, `crate::`, `::`) discards it,
                // so nesting one silently ignores the surrounding module. Allow
                // anchored paths only at the file root and require nested mods
                // to be relative.
                let anchor = match path.start {
                    ParsedPathStart::Cpp => Some(if path.segments.is_empty() {
                        "c++"
                    } else {
                        "c++::"
                    }),
                    ParsedPathStart::Crate => Some("crate::"),
                    ParsedPathStart::Absolute => Some("::"),
                    ParsedPathStart::Relative => None,
                };
                if let Some(anchor) = anchor
                    && !is_root
                {
                    ctx.add_error_str(
                        &format!(
                            "`{anchor}` modules can only appear at the top level of a file, not nested inside another module"
                        ),
                        path.span,
                    );
                } else {
                    let sub_scope = scope.sub_scope(&aliases, path, ctx);
                    for item in items {
                        item.add_to_zngur_spec(r, &sub_scope, ctx);
                    }
                }
            }
            ProcessedItem::Import(path) => {
                if path.path.is_absolute() {
                    ctx.add_error_str("Absolute paths imports are not supported.", path.span)
                }
                match path.path.components().next() {
                    Some(Component::CurDir) | Some(Component::ParentDir) => {
                        let import = Import(path.path);
                        ctx.record_span(&import, path.span.into_range());
                        r.imports.push(import);
                    }
                    _ => ctx.add_error_str(
                        "Module import is not supported. Use a relative path instead.",
                        path.span,
                    ),
                }
            }
            ProcessedItem::ModuleImport { path, span } => {
                let resolved_path = if ctx.depth > 0
                    && let Some((root_dir, current_dir)) =
                        ctx.module_root_dir.as_deref().zip(ctx.path.parent())
                {
                    rebase_relative_import_path(root_dir, current_dir, &path)
                } else {
                    path
                };
                r.spec.imported_modules.push(ModuleImport {
                    path: resolved_path,
                    ..Default::default()
                });
                r.module_import_spans
                    .push(ctx.report_span_for_range(span.into_range()));
                r.module_import_resolved_paths.push(None);
            }
            ProcessedItem::Type {
                ty,
                items,
                lifetimes: type_lifetimes,
                type_vars,
                unstable_template_types,
            } => {
                if ty.inner == ParsedRustType::Tuple(vec![]) {
                    // We add unit type implicitly.
                    ctx.add_error_str(
                        "Unit type is declared implicitly. Remove this entirely.",
                        ty.span,
                    );
                }
                let type_lifetimes = lifetimes(&type_lifetimes);
                let declared_type_vars: Option<Vec<TypeVar>> = type_vars
                    .as_ref()
                    .map(|tvs| tvs.iter().map(|v| TypeVar(v.0.to_owned())).collect());

                let (is_template, scope) = match type_vars {
                    Some(type_vars) => (true, &scope.with_type_vars(type_vars)),
                    None => (false, scope),
                };

                let mut methods = vec![];
                let mut cpp_methods: Vec<ZngurCppMethod> = vec![];
                let mut cpp_constructors: Vec<ZngurCppConstructor> = vec![];
                let mut cpp_fields: Vec<ZngurCppField> = vec![];
                let mut constructor = None;
                let mut variants = vec![];
                let mut fields = vec![];
                let mut wellknown_traits = vec![];
                let mut layout = None;
                let mut layout_span = None;
                let mut exhaustive = true;
                let mut cpp_heap_allocated = None;
                let mut cpp_ref = None;
                let mut cpp_stack_owned = None;
                let mut trivially_relocatable = TriviallyRelocatable::No;
                let mut to_process = items;
                to_process.reverse(); // create a stack of items to process
                let check_size_align = |props: Vec<(Spanned<&str>, usize)>| {
                    let mut size = None;
                    let mut align = None;
                    let mut errors = vec![];
                    for (key, value) in props {
                        match key.inner {
                            "size" => size = Some(value),
                            "align" => {
                                if !value.is_power_of_two() {
                                    errors.push(("Alignment must be a power of two", key.span));
                                }
                                align = Some(value);
                            }
                            _ => errors.push(("Unknown property", key.span)),
                        }
                    }
                    if size.is_none() {
                        errors.push(("Size is not declared for this type", ty.span));
                    }
                    if align.is_none() {
                        errors.push(("Align is not declared for this type", ty.span));
                    }
                    if errors.is_empty() {
                        Ok((size.unwrap(), align.unwrap()))
                    } else {
                        Err(errors)
                    }
                };
                let rust_ty = ty.inner.to_zngur(scope, ctx);
                if !type_lifetimes.is_empty() && matches!(rust_ty, RustType::Cpp(_)) {
                    ctx.add_error_str("`c++::` types don't have lifetime parameters", ty.span);
                    return;
                }
                if is_template && !matches!(rust_ty, RustType::Cpp(_)) && !unstable_template_types {
                    ctx.add_error_str(
                        "Template types are unstable. Enable them by using `#unstable(template_types)` at the top of the file.",
                        ty.span,
                    );
                    return;
                }
                if let Some(declared) = &declared_type_vars {
                    if let RustType::Cpp(path) = &rust_ty
                        && path.generics.is_empty()
                    {
                        ctx.add_error_str(
                            &format!(
                                "`{}` has no template arguments; C++ template declarations need \
                                 template arguments (e.g. `type<T> {}<T> {{ ... }}`)",
                                zng_spelling(&rust_ty),
                                zng_spelling(&rust_ty),
                            ),
                            ty.span,
                        );
                        return;
                    }
                    let used = rust_ty.type_vars();
                    let mut unused: Vec<&str> = declared
                        .iter()
                        .filter(|v| !used.contains(v))
                        .map(|v| v.0.as_str())
                        .collect();
                    unused.sort_unstable();
                    if !unused.is_empty() {
                        for var in unused {
                            ctx.add_error_str(
                                &format!(
                                    "type parameter `{var}` is not used in the template type `{}`",
                                    zng_spelling(&rust_ty),
                                ),
                                ty.span,
                            );
                        }
                        return;
                    }
                }
                let is_top_cpp_obj_ref = matches!(
                    &rust_ty,
                    RustType::Ref(_, _, inner) if inner.as_cpp_object().is_some()
                );
                let deferred_before = ctx.deferred_cpp_object_checks.len();
                let mut ty_ok = check_types(
                    ctx,
                    ty.span,
                    &type_lifetimes,
                    [&rust_ty],
                    [],
                    LifetimeContext::Item,
                    false,
                    is_top_cpp_obj_ref,
                );
                if ty_ok && (matches!(rust_ty, RustType::CppPtr(..)) || is_top_cpp_obj_ref) {
                    ctx.deferred_cpp_object_checks.truncate(deferred_before);
                    ctx.add_error_str(
                        &format!(
                            "C++ pointers and references (`{}`) are built-in, not types to \
                             declare: use them in signatures directly (only `c++::` classes take \
                             a `type` block)",
                            zng_spelling(&rust_ty)
                        ),
                        ty.span,
                    );
                    return;
                }
                // It's a C++ pointer in C++, which isn't a type to declare.
                if ty_ok && let Some((_, _, pointee)) = rust_ty.as_nullable_cpp_ref() {
                    ctx.deferred_cpp_object_checks.truncate(deferred_before);
                    ctx.add_error_str(
                        &format!(
                            "a nullable reference to a C++ object (`Option<&'a const T>`, \
                             `Option<&'a var T>`, ...) is a C++ pointer in C++ (here to `{}`), not \
                             a type to declare: use it in signatures directly",
                            zng_spelling(pointee)
                        ),
                        ty.span,
                    );
                    return;
                }
                if ty_ok && !matches!(rust_ty, RustType::Cpp(_)) {
                    let used = rust_ty.free_lifetimes();
                    for lt in &type_lifetimes {
                        if !used.contains(lt) {
                            ctx.add_error_str(
                                &format!(
                                    "unconstrained lifetime `{lt}` in `type`: `{lt}` must be used \
                                     in `{rust_ty}` (if `{lt}` is only used by methods, declare it \
                                     on each method instead, e.g. `fn f<{lt}>(...)`)"
                                ),
                                ty.span,
                            );
                            ty_ok = false;
                        }
                    }
                }
                while let Some(item) = to_process.pop() {
                    let item_span = item.span;
                    let item = item.inner;
                    match item {
                        ParsedTypeItem::Layout(span, p) => {
                            let l = match p {
                                ParsedLayoutPolicy::StackAllocated(p) => {
                                    match check_size_align(p) {
                                        Ok((size, align)) => {
                                            LayoutPolicy::StackAllocated { size, align }
                                        }
                                        Err(errs) => {
                                            for (msg, err_span) in errs {
                                                ctx.add_error_str(msg, err_span);
                                            }
                                            if layout.is_none() {
                                                layout = Some(LayoutPolicy::ZERO_SIZED_TYPE);
                                            }
                                            if layout_span.is_none() {
                                                layout_span = Some(span);
                                            }
                                            continue;
                                        }
                                    }
                                }
                                ParsedLayoutPolicy::Conservative(p) => match check_size_align(p) {
                                    Ok((size, align)) => LayoutPolicy::Conservative { size, align },
                                    Err(errs) => {
                                        for (msg, err_span) in errs {
                                            ctx.add_error_str(msg, err_span);
                                        }
                                        if layout.is_none() {
                                            layout = Some(LayoutPolicy::ZERO_SIZED_TYPE);
                                        }
                                        if layout_span.is_none() {
                                            layout_span = Some(span);
                                        }
                                        continue;
                                    }
                                },
                                ParsedLayoutPolicy::HeapAllocated => LayoutPolicy::HeapAllocated,
                                ParsedLayoutPolicy::OnlyByRef => LayoutPolicy::OnlyByRef,
                            };
                            ctx.record_span(
                                &l.into_span_key_with(rust_ty.clone()),
                                span.into_range(),
                            );
                            layout = Some(l);
                            match layout_span {
                                Some(_) => {
                                    ctx.add_error_str("Duplicate layout policy found", span);
                                }
                                None => layout_span = Some(span),
                            }
                        }
                        ParsedTypeItem::Traits(tr) => {
                            for t in &tr {
                                ctx.record_span(
                                    &PartialSpanKey::wellknown_trait(rust_ty.clone(), t.inner),
                                    t.span.into_range(),
                                );
                            }
                            wellknown_traits.extend(tr);
                        }
                        ParsedTypeItem::NonExhaustive => {
                            if !exhaustive {
                                ctx.add_error_str(
                                    "Duplicate non_exhaustive annotation found",
                                    item_span,
                                );
                            }
                            exhaustive = false;
                        }
                        ParsedTypeItem::Constructor {
                            lifetimes: ctor_lifetimes,
                            args,
                            rust_name,
                        } => {
                            if matches!(rust_ty, RustType::Cpp(_)) {
                                ctx.add_error_str(
                                    "C++ constructors start with `safe` or `unsafe` and need a \
                                     Rust name with `as <name>` (e.g. `safe constructor(...) as \
                                     new;`)",
                                    item_span,
                                );
                                continue;
                            }
                            if !ctor_lifetimes.is_empty() || rust_name.is_some() {
                                ctx.add_error_str(
                                    "Rust tuple/struct `constructor` declarations don't take \
                                     lifetime parameters or `as <name>` (C++ constructors start \
                                     with `safe` or `unsafe` on `c++::` types)",
                                    item_span,
                                );
                                continue;
                            }
                            if constructor.is_some() {
                                ctx.add_error_str("Duplicate constructor found", item_span);
                            }
                            let c = ZngurConstructor {
                                inputs: match args {
                                    ParsedConstructorArgs::Unit => vec![],
                                    ParsedConstructorArgs::Tuple(t) => t
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, t)| (i.to_string(), t.to_zngur(scope, ctx)))
                                        .collect(),
                                    ParsedConstructorArgs::Named(t) => t
                                        .into_iter()
                                        .map(|(i, t)| (i.to_owned(), t.to_zngur(scope, ctx)))
                                        .collect(),
                                },
                            };
                            if !check_used_types(
                                ctx,
                                item_span,
                                &type_lifetimes,
                                c.inputs.iter().map(|(_, t)| t),
                                [],
                                LifetimeContext::Item,
                                true,
                                true,
                            ) {
                                continue;
                            }
                            ctx.record_span(
                                &c.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            constructor = Some(c);
                        }
                        ParsedTypeItem::Variant { name, items } => {
                            if matches!(rust_ty, RustType::Cpp(_)) {
                                ctx.add_error_str(
                                    "enum variants cannot be declared on `c++::` types",
                                    item_span,
                                );
                                continue;
                            }
                            let mut exhaustive = true;
                            let mut fields = vec![];
                            for item in items {
                                match item.inner {
                                    ParsedTypeItem::NonExhaustive => {
                                        if !exhaustive {
                                            ctx.add_error_str(
                                                "Duplicate non_exhaustive annotation found",
                                                item.span,
                                            );
                                        }
                                        exhaustive = false;
                                    }
                                    ParsedTypeItem::Field { name, ty, offset } => {
                                        if offset.is_some() {
                                            ctx.add_error_str(
                                                "static offsets on enum fields are not supported",
                                                item.span,
                                            );
                                        }
                                        let field = ZngurField {
                                            name: name.to_owned(),
                                            ty: ty.to_zngur(scope, ctx),
                                            offset,
                                        };
                                        if !check_used_types(
                                            ctx,
                                            item.span,
                                            &type_lifetimes,
                                            [&field.ty],
                                            [],
                                            LifetimeContext::Item,
                                            false,
                                            false,
                                        ) {
                                            continue;
                                        }
                                        ctx.record_span(
                                            &field.into_span_key_with_variant(
                                                rust_ty.clone(),
                                                Some(name.clone()),
                                            ),
                                            item.span.into_range(),
                                        );
                                        fields.push(field);
                                    }
                                    _ => panic!("bug: invalid variant item found: {item:?}"),
                                }
                            }
                            variants.push(ZngurVariant {
                                name: name.to_owned(),
                                fields,
                                exhaustive,
                            });
                        }
                        ParsedTypeItem::Field { name, ty, offset } => {
                            if matches!(rust_ty, RustType::Cpp(_)) {
                                ctx.add_error_str(
                                    &format!(
                                        "C++ field accessors omit `offset` and say what they \
                                         return (e.g. `field &const {name} (type = ...);` or \
                                         `field var {name} (type = ...);`)"
                                    ),
                                    item_span,
                                );
                                continue;
                            }
                            let field = ZngurField {
                                name: name.to_owned(),
                                ty: ty.to_zngur(scope, ctx),
                                offset,
                            };
                            if !check_used_types(
                                ctx,
                                item_span,
                                &type_lifetimes,
                                [&field.ty],
                                [],
                                LifetimeContext::Item,
                                false,
                                false,
                            ) {
                                continue;
                            }
                            ctx.record_span(
                                &field.into_span_key_with_variant(rust_ty.clone(), None),
                                item_span.into_range(),
                            );
                            fields.push(field);
                        }
                        ParsedTypeItem::Fn {
                            is_safe,
                            is_static,
                            async_span,
                            name,
                            name_is_ident,
                            lifetimes: method_lifetimes,
                            generics: method_generics,
                            mut inputs,
                            is_const,
                            output,
                            sig_span,
                            use_path,
                            deref,
                            alias,
                        } => {
                            let is_cpp_method_target = match &rust_ty {
                                RustType::Cpp(_) => true,
                                RustType::TypeVar(_) => {
                                    is_static
                                        || is_const
                                        || !name_is_ident
                                        || matches!(alias, Some((_, false)))
                                        || matches!(
                                            inputs.first().and_then(as_receiver),
                                            Some(
                                                ParsedReceiver::Ref { .. }
                                                    | ParsedReceiver::Raw(..)
                                            )
                                        )
                                }
                                _ => false,
                            };
                            if is_cpp_method_target {
                                let Some(is_safe) = is_safe else {
                                    ctx.add_error_str(
                                        "C++ member functions must be marked `safe` or `unsafe`",
                                        item_span,
                                    );
                                    continue;
                                };
                                if async_span.is_some() {
                                    ctx.add_error_str(
                                        "C++ member functions cannot be `async`",
                                        item_span,
                                    );
                                    continue;
                                }
                                if use_path.is_some() || deref.is_some() {
                                    ctx.add_error_str(
                                        "C++ member functions do not support `use` or `deref` \
                                         (those are for Rust methods; implement Rust traits on \
                                         `c++::` types via \
                                         `extern \"C++\" { impl Trait for c++::T { ... } }`)",
                                        item_span,
                                    );
                                    continue;
                                }
                                if !method_generics.is_empty() {
                                    ctx.add_error_str(
                                        "C++ member functions do not support generic type \
                                         arguments",
                                        item_span,
                                    );
                                    continue;
                                }
                                if is_const {
                                    ctx.add_error_str(
                                        "C++ member functions take an explicit receiver instead \
                                         of a trailing `const`: `&'a const self` for `const` \
                                         member functions, and `&'a unique var self` (or \
                                         `&'a var self`) for the others",
                                        item_span,
                                    );
                                    continue;
                                }
                                if is_static {
                                    ctx.add_error_str(
                                        "C++ static member functions are declared without a \
                                         `self` receiver; remove `static`",
                                        item_span,
                                    );
                                    continue;
                                }
                                let parsed_receiver = inputs.first().and_then(as_receiver);
                                if parsed_receiver.is_some() {
                                    inputs.remove(0);
                                }
                                if inputs.iter().chain([&output]).any(contains_relative_self) {
                                    ctx.add_error_str(
                                        "invalid C++ member function receiver: expected \
                                         `&'a const self`, `&'a unique var self`, `&'a var self`, \
                                         `&'a unique const self`, `const self*`, `self*`, \
                                         `const self&`, `self&`, or no `self` for a static member \
                                         function",
                                        item_span,
                                    );
                                    continue;
                                }
                                // A member function without a `self` receiver is a static one.
                                let (receiver, receiver_constness, receiver_lifetime, raw_receiver) =
                                    match parsed_receiver {
                                        None => (
                                            ZngurMethodReceiver::Static,
                                            CppConstness::Var,
                                            None,
                                            None,
                                        ),
                                        Some(ParsedReceiver::RustRef(m, l)) => {
                                            let l = l.map(|l| format!("'{l} ")).unwrap_or_default();
                                            let suggestion = match m {
                                                Mutability::Not => format!(
                                                    "`&{l}const self` (a `const` member function)"
                                                ),
                                                Mutability::Mut => format!(
                                                    "`&{l}unique var self` (a non-`const` member \
                                                 function, `&mut self` in Rust)"
                                                ),
                                            };
                                            ctx.add_error_str(
                                                &format!(
                                                    "the receiver of a C++ member function is a \
                                                 reference to a C++ object, which says whether \
                                                 it's `const`: {suggestion}"
                                                ),
                                                item_span,
                                            );
                                            continue;
                                        }
                                        Some(ParsedReceiver::Ref {
                                            unique,
                                            constness,
                                            lifetime: l,
                                        }) => {
                                            let m = if unique {
                                                Mutability::Mut
                                            } else {
                                                Mutability::Not
                                            };
                                            (
                                                ZngurMethodReceiver::Ref(m),
                                                constness,
                                                l.map(lifetime),
                                                None,
                                            )
                                        }
                                        Some(ParsedReceiver::Raw(constness, indirection)) => {
                                            // Raw receivers are associated functions; the `Ref`
                                            // only says that there is a `this`.
                                            let m = match constness {
                                                CppConstness::Const => Mutability::Not,
                                                CppConstness::Var => Mutability::Mut,
                                            };
                                            (
                                                ZngurMethodReceiver::Ref(m),
                                                constness,
                                                None,
                                                Some(indirection),
                                            )
                                        }
                                    };
                                let rust_name = alias.map(|(s, _)| s);
                                let inputs: Vec<RustType> =
                                    inputs.into_iter().map(|x| x.to_zngur(scope, ctx)).collect();
                                let output = output.to_zngur(scope, ctx);
                                let method = ZngurCppMethod {
                                    cpp_name: name.to_owned(),
                                    lifetimes: lifetimes(&method_lifetimes),
                                    rust_name: rust_name.filter(|&n| n != name).map(str::to_owned),
                                    receiver,
                                    receiver_constness,
                                    receiver_lifetime,
                                    raw_receiver,
                                    inputs,
                                    output,
                                    is_safe,
                                };
                                if !check_types(
                                    ctx,
                                    item_span,
                                    &method.lifetimes,
                                    method.inputs.iter().chain([&method.output]),
                                    method.receiver_lifetime.as_ref(),
                                    LifetimeContext::Item,
                                    true,
                                    true,
                                ) {
                                    continue;
                                }
                                if !check_rust_fn_name(
                                    ctx,
                                    item_span,
                                    "C++ function",
                                    method.rust_name(),
                                    rust_name.is_some(),
                                    true,
                                ) {
                                    continue;
                                }
                                let elided_receiver =
                                    method.receiver_lifetime.as_ref().is_none_or(|l| l.0 == "_")
                                        && method.raw_receiver.is_none()
                                        && method.receiver != ZngurMethodReceiver::Static;
                                check_cpp_signature_lints(
                                    ctx,
                                    item_span,
                                    "member function",
                                    is_safe,
                                    elided_receiver,
                                    method.raw_receiver.is_some(),
                                    &method.inputs,
                                    &method.output,
                                );
                                if !check_member_name(
                                    ctx,
                                    item_span,
                                    method.rust_name(),
                                    &cpp_constructors,
                                    &cpp_methods,
                                    &cpp_fields,
                                ) {
                                    continue;
                                }
                                ctx.record_span(
                                    &method.into_span_key_with(rust_ty.clone()),
                                    item_span.into_range(),
                                );
                                cpp_methods.push(method);
                            } else {
                                if is_static {
                                    ctx.add_error_str(
                                        "Rust static methods are declared without a `self` \
                                         receiver; remove `static`",
                                        item_span,
                                    );
                                    continue;
                                }
                                if is_const {
                                    ctx.add_error_str(
                                        "methods of Rust types cannot have a trailing `const` \
                                         qualifier",
                                        item_span,
                                    );
                                    continue;
                                }
                                if !name_is_ident {
                                    ctx.add_error_str(
                                        &format!(
                                            "`{name}` is a keyword and cannot be used as a Rust \
                                             method name"
                                        ),
                                        item_span,
                                    );
                                    continue;
                                }
                                if let Some((alias_name, false)) = alias {
                                    ctx.add_error_str(
                                        &format!(
                                            "`{alias_name}` is a keyword and cannot be used as a \
                                             method alias"
                                        ),
                                        item_span,
                                    );
                                    continue;
                                }
                                if matches!(
                                    inputs.first().and_then(as_receiver),
                                    Some(ParsedReceiver::Raw(..))
                                ) {
                                    ctx.add_error_str(
                                        "methods of Rust types and `extern \"C++\"` impls take \
                                         `&self` or `&mut self`, not `const self*` / `self&`",
                                        sig_span,
                                    );
                                    continue;
                                }
                                let mut data = match parsed_method_from_signature(
                                    name,
                                    async_span,
                                    method_lifetimes,
                                    method_generics,
                                    (inputs, output),
                                    sig_span,
                                ) {
                                    Ok(data) => data,
                                    Err(e) => {
                                        ctx.add_error_str(&e.reason().to_string(), *e.span());
                                        continue;
                                    }
                                };
                                data.is_safe = is_safe;
                                let cpp_name = alias.map(|(s, _)| s);
                                let deref = deref.and_then(|x| {
                                    let deref_type = x.to_zngur(scope, ctx);
                                    let receiver_mutability = match data.receiver {
                                        ZngurMethodReceiver::Ref(mutability) => mutability,
                                        ZngurMethodReceiver::Static | ZngurMethodReceiver::Move => {
                                            ctx.add_error_str(
                                                "Deref needs reference receiver",
                                                item_span,
                                            );
                                            return None;
                                        }
                                    };
                                    Some((deref_type, receiver_mutability))
                                });
                                let use_path = use_path.and_then(|x| {
                                    let target = scope.resolve_path(x.clone());
                                    match target.rust_reference() {
                                        Some(reference) => Some(reference),
                                        None => {
                                            scope.report_invalid_cpp_path(
                                                ctx,
                                                &x,
                                                "`c++::` paths cannot be used in a method's `use` path",
                                                "paths",
                                            );
                                            None
                                        }
                                    }
                                });
                                let method = ZngurMethodDetails {
                                    data: data.to_zngur(scope, ctx),
                                    use_path,
                                    deref,
                                    cpp_name: cpp_name.map(|s| s.to_owned()),
                                };
                                if !check_method_types(
                                    ctx,
                                    item_span,
                                    &type_lifetimes,
                                    &method.data,
                                    method.deref.as_ref().map(|d| &d.0),
                                ) {
                                    continue;
                                }
                                ctx.record_span(
                                    &method.data.into_span_key_with(rust_ty.clone()),
                                    item_span.into_range(),
                                );
                                methods.push(method);
                            }
                        }
                        ParsedTypeItem::CppConstructor {
                            is_safe,
                            lifetimes: constructor_lifetimes,
                            inputs,
                            rust_name,
                        } => {
                            let Some(rust_name) = rust_name else {
                                ctx.add_error_str(
                                    "C++ constructors need a Rust name: `as <name>`",
                                    item_span,
                                );
                                continue;
                            };
                            if inputs.iter().any(contains_relative_self) {
                                ctx.add_error_str(
                                    "C++ constructors do not have a `self` receiver; spell out the \
                                     C++ type path explicitly (e.g. `&'a const c++::ns::Foo`)",
                                    item_span,
                                );
                                continue;
                            }
                            let inputs: Vec<RustType> =
                                inputs.into_iter().map(|x| x.to_zngur(scope, ctx)).collect();
                            let constructor_lifetimes = lifetimes(&constructor_lifetimes);
                            if !check_types(
                                ctx,
                                item_span,
                                &constructor_lifetimes,
                                &inputs,
                                [],
                                LifetimeContext::Item,
                                true,
                                true,
                            ) {
                                continue;
                            }
                            if !check_rust_fn_name(
                                ctx,
                                item_span,
                                "C++ function",
                                rust_name,
                                true,
                                true,
                            ) {
                                continue;
                            }
                            check_cpp_signature_lints(
                                ctx,
                                item_span,
                                "constructor",
                                is_safe,
                                false,
                                false,
                                &inputs,
                                &RustType::UNIT,
                            );
                            if !check_member_name(
                                ctx,
                                item_span,
                                rust_name,
                                &cpp_constructors,
                                &cpp_methods,
                                &cpp_fields,
                            ) {
                                continue;
                            }
                            let constructor = ZngurCppConstructor {
                                rust_name: rust_name.to_owned(),
                                lifetimes: constructor_lifetimes,
                                inputs,
                                is_safe,
                            };
                            ctx.record_span(
                                &PartialSpanKey::cpp_constructor(
                                    rust_ty.clone(),
                                    constructor.rust_name.clone(),
                                ),
                                item_span.into_range(),
                            );
                            cpp_constructors.push(constructor);
                        }
                        ParsedTypeItem::CppField {
                            is_safe,
                            is_ref,
                            unique,
                            constness,
                            name,
                            ty,
                            rust_name,
                        } => {
                            let Some(constness) = constness else {
                                let unique = if unique { "unique " } else { "" };
                                ctx.add_error_str(
                                    &format!(
                                        "a C++ field accessor says what it returns: \
                                         `field &{unique}const {name} ...` / \
                                         `field &{unique}var {name} ...` (a reference to the \
                                         field, `unique` if Rust uses it uniquely), or \
                                         `field const {name} ...` / `field var {name} ...` (a \
                                         copy of it)"
                                    ),
                                    item_span,
                                );
                                continue;
                            };
                            let access = match (is_ref, unique) {
                                (true, unique) => CppFieldAccess::Ref { unique, constness },
                                (false, false) => CppFieldAccess::Copy { constness },
                                (false, true) => {
                                    let c = constness.keyword();
                                    ctx.add_error_str(
                                        &format!(
                                            "`unique` only applies to references: \
                                             `field &unique {c} {name} ...` returns a unique \
                                             reference to the field, and `field {c} {name} ...` \
                                             a copy of it"
                                        ),
                                        item_span,
                                    );
                                    continue;
                                }
                            };
                            let has_rename = rust_name.is_some();
                            let field = ZngurCppField {
                                name: name.to_owned(),
                                rust_name: rust_name.filter(|&n| n != name).map(str::to_owned),
                                ty: ty.to_zngur(scope, ctx),
                                access,
                                is_safe,
                            };
                            if !reject_lifetimes(ctx, item_span, "C++ fields", [&field.ty]) {
                                continue;
                            }
                            if let Err(e) = check_cpp_field(&field, None) {
                                ctx.add_error_str(&e, item_span);
                                continue;
                            }
                            if !check_rust_fn_name(
                                ctx,
                                item_span,
                                "C++ field accessor",
                                field.rust_name(),
                                has_rename,
                                true,
                            ) {
                                continue;
                            }
                            check_cpp_field_lints(ctx, item_span, &field);
                            if !check_member_name(
                                ctx,
                                item_span,
                                field.rust_name(),
                                &cpp_constructors,
                                &cpp_methods,
                                &cpp_fields,
                            ) {
                                continue;
                            }
                            ctx.record_span(
                                &field.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_fields.push(field);
                        }
                        ParsedTypeItem::CppValue { field: _, cpp_type } => {
                            if is_template && matches!(rust_ty, RustType::Cpp(_)) {
                                ctx.add_error_str(
                                    "C++ class templates can't be `#cpp_heap_allocated`",
                                    item_span,
                                );
                                continue;
                            }
                            ctx.add_warning_str(
                                "#cpp_value is deprecated; use #cpp_heap_allocated instead",
                                item_span,
                            );
                            let cpp = CppHeapAllocated(cpp_type.to_owned());
                            ctx.record_span(
                                &cpp.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_heap_allocated = Some(cpp);
                        }
                        ParsedTypeItem::CppHeapAllocated { cpp_type } => {
                            if is_template && matches!(rust_ty, RustType::Cpp(_)) {
                                ctx.add_error_str(
                                    "C++ class templates can't be `#cpp_heap_allocated`",
                                    item_span,
                                );
                                continue;
                            }
                            let Some(cpp_type) = cpp_type_name(cpp_type, &rust_ty, item_span, ctx)
                            else {
                                continue;
                            };
                            let cpp = CppHeapAllocated(cpp_type);
                            ctx.record_span(
                                &cpp.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_heap_allocated = Some(cpp);
                        }
                        ParsedTypeItem::CppRef { cpp_type } => {
                            if matches!(&rust_ty, RustType::Cpp(p) if !p.generics.is_empty()) {
                                ctx.add_error_str(
                                    "`#cpp_ref` cannot be used on C++ class templates; a `c++::` \
                                     type without a layout is already reference-only",
                                    item_span,
                                );
                                continue;
                            }
                            let Some(cpp_type) = cpp_type_name(cpp_type, &rust_ty, item_span, ctx)
                            else {
                                continue;
                            };
                            match layout_span {
                                Some(span) => {
                                    ctx.add_error_str("Duplicate layout policy found", span);
                                    continue;
                                }
                                None => {
                                    layout = Some(LayoutPolicy::ZERO_SIZED_TYPE);
                                    layout_span = Some(item_span);
                                }
                            }
                            let cpp = CppRef(cpp_type);
                            ctx.record_span(
                                &cpp.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_ref = Some(cpp);
                        }
                        ParsedTypeItem::CppStackOwned { cpp_type, props } => {
                            if matches!(&rust_ty, RustType::Cpp(p) if !p.generics.is_empty()) {
                                ctx.add_error_str(
                                    "`#cpp_stack_owned` cannot be used on C++ class templates; \
                                     use `#layout(...)` and `#trivially_relocatable` instead",
                                    item_span,
                                );
                                continue;
                            }
                            let Some(cpp_type) = cpp_type_name(cpp_type, &rust_ty, item_span, ctx)
                            else {
                                continue;
                            };
                            let has_legacy_props = props.is_some();
                            if let Some(props) = props {
                                let size = props
                                    .iter()
                                    .find(|(k, _)| k.inner == "size")
                                    .map(|(_, v)| v.to_string())
                                    .unwrap_or_else(|| "X".to_string());
                                let align = props
                                    .iter()
                                    .find(|(k, _)| k.inner == "align")
                                    .map(|(_, v)| v.to_string())
                                    .unwrap_or_else(|| "Y".to_string());
                                let report_span = ctx.report_span_for_range(item_span.into_range());
                                let report = Report::build(ReportKind::Error, report_span.clone())
                                    .with_message("Old syntax `#cpp_stack_owned` with layout properties is no longer supported.")
                                    .with_label(
                                        Label::new(report_span)
                                            .with_message("layout parameters no longer accepted here")
                                            .with_color(Color::Red),
                                    )
                                    .with_help(format!(
                                        "layout moved to its own directive: `#cpp_stack_owned \"{cpp_type}\"; #layout(size = {size}, align = {align});`"
                                    ))
                                    .finish();
                                ctx.add_fatal_report(report);
                            }
                            let cpp = CppStackOwned(cpp_type);
                            ctx.record_span(
                                &cpp.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_stack_owned = Some(cpp);
                            if has_legacy_props && layout.is_none() {
                                layout = Some(LayoutPolicy::ZERO_SIZED_TYPE);
                            }
                        }
                        ParsedTypeItem::TriviallyRelocatable { unchecked } => {
                            let kind = if unchecked {
                                TriviallyRelocatable::Unchecked
                            } else {
                                TriviallyRelocatable::Checked
                            };
                            let directive = kind.directive().unwrap();
                            if !is_template && !matches!(rust_ty, RustType::Cpp(_)) {
                                ctx.add_error_str(
                                    &format!(
                                        "`{directive}` is only for `c++::` types (Rust types \
                                         are always trivially relocatable)"
                                    ),
                                    item_span,
                                );
                                continue;
                            }
                            ctx.record_span(
                                &PartialSpanKey::trivially_relocatable(rust_ty.clone(), kind),
                                item_span.into_range(),
                            );
                            trivially_relocatable = trivially_relocatable.combine(kind);
                        }
                        ParsedTypeItem::MatchOnCfg(match_) => {
                            let result = match_.eval(ctx);
                            if let Some(result) = result {
                                to_process.extend(result);
                            }
                        }
                    }
                }
                let is_unsized = wellknown_traits
                    .iter()
                    .find(|x| x.inner == ZngurWellknownTrait::Unsized)
                    .cloned();
                let wt = wellknown_traits
                    .into_iter()
                    .map(|x| x.inner)
                    .collect::<Vec<_>>();
                if let Some(is_unsized) = is_unsized {
                    if let Some(span) = layout_span {
                        ctx.add_fatal_report(
                            Report::build(
                                ReportKind::Error,
                                ctx.report_span_for_range(span.into_range()),
                            )
                            .with_message("Duplicate layout policy found for unsized type.")
                            .with_label(
                                Label::new(ctx.report_span_for_range(span.into_range()))
                                    .with_message(
                                        "Unsized types have implicit layout policy, remove this.",
                                    )
                                    .with_color(Color::Red),
                            )
                            .with_label(
                                Label::new(ctx.report_span_for_range(is_unsized.span.into_range()))
                                    .with_message("Type declared as unsized here.")
                                    .with_color(Color::Blue),
                            )
                            .finish(),
                        )
                    }
                    layout = Some(LayoutPolicy::OnlyByRef);
                }
                let zngur_type = ZngurType {
                    ty: rust_ty.clone(),
                    lifetimes: type_lifetimes,
                    layout,
                    methods,
                    cpp_methods,
                    cpp_constructors,
                    cpp_fields,
                    wellknown_traits: wt,
                    exhaustive,
                    constructor,
                    variants,
                    fields,
                    cpp_heap_allocated,
                    cpp_ref,
                    cpp_stack_owned,
                    trivially_relocatable,
                };
                if !ty_ok {
                    return;
                }
                if is_template {
                    r.templates.push(TemplateDef {
                        ty: zngur_type,
                        source_id: ctx.source_id(),
                        span: ty.span,
                        allowed_lints: ctx.allowed_lints.clone(),
                        imported: false,
                    });
                } else {
                    r.type_allowed_lints
                        .entry(rust_ty.with_erased_lifetimes())
                        .or_insert_with(|| ctx.allowed_lints.clone());
                    ctx.record_span(&zngur_type, ty.span.into_range());
                    checked_merge(
                        zngur_type,
                        &mut r.spec,
                        ty.span,
                        ctx,
                        Some(MergeContext {
                            outer_ty: rust_ty,
                            variant: None,
                        }),
                    );
                }
            }
            ProcessedItem::Trait { tr, methods } => {
                let tr_zngur = tr.inner.to_zngur(scope, ctx);
                let mut ok = check_types(
                    ctx,
                    tr.span,
                    &[],
                    [&RustType::Dyn(tr_zngur.clone(), vec![])],
                    [],
                    LifetimeContext::TraitHeader,
                    false,
                    false,
                );
                let mut zngur_methods = Vec::with_capacity(methods.len());
                for m in methods {
                    let m_span = m.span;
                    let m_zngur = m.inner.to_zngur(scope, ctx);
                    if !check_method_types(ctx, m_span, &[], &m_zngur, []) {
                        ok = false;
                    }
                    ctx.record_span(
                        &PartialSpanKey::trait_method(tr_zngur.clone(), m_zngur.name.clone()),
                        m_span.into_range(),
                    );
                    zngur_methods.push(m_zngur);
                }
                if !ok {
                    return;
                }
                let trt = ZngurTrait {
                    tr: tr_zngur,
                    methods: zngur_methods,
                };
                ctx.record_span(&trt, tr.span.into_range());
                checked_merge(trt, &mut r.spec, tr.span, ctx, None);
            }
            ProcessedItem::Fn(f) => {
                let item_span = f.span;
                let f = f.inner;
                let target =
                    if f.path.start == ParsedPathStart::Relative && f.path.segments.len() == 1 {
                        scope.base.child(f.path.segments[0])
                    } else {
                        scope.resolve_path(f.path.clone())
                    };
                match target {
                    EntityPath::Rust(path) => {
                        let sig_span = f.sig_span;
                        if !f.name_is_ident {
                            let name = f.path.segments.last().expect("at_least(1)");
                            ctx.add_error_str(
                                &format!(
                                    "`{name}` is a keyword and cannot be used as a Rust function name"
                                ),
                                sig_span,
                            );
                            return;
                        }
                        if f.alias.is_some() {
                            ctx.add_error_str(
                                "Rust free functions cannot be renamed with `as`",
                                sig_span,
                            );
                            return;
                        }
                        let name = *f.path.segments.last().expect("at_least(1)");
                        let parsed_method = match parsed_method_from_signature(
                            name,
                            f.async_span,
                            f.lifetimes,
                            f.generics,
                            (f.inputs, f.output),
                            sig_span,
                        ) {
                            Ok(m) => m,
                            Err(e) => {
                                ctx.add_error_str(&e.reason().to_string(), *e.span());
                                return;
                            }
                        };
                        let method = parsed_method.to_zngur(scope, ctx);
                        if !check_method_types(ctx, sig_span, &[], &method, []) {
                            return;
                        }
                        let func = ZngurFn {
                            path: RustPathAndGenerics {
                                path,
                                generics: method.generics,
                                ..Default::default()
                            },
                            lifetimes: method.lifetimes,
                            inputs: method.inputs,
                            output: method.output,
                            is_safe: f.is_safe,
                        };
                        ctx.record_span(&func, sig_span.into_range());
                        checked_merge(func, &mut r.spec, sig_span, ctx, None);
                    }
                    EntityPath::Cpp(mut segments) => {
                        let cpp_name = segments.pop().expect("at_least(1)");
                        let namespace = segments;
                        let Some(is_safe) = f.is_safe else {
                            let is_bare_relative = f.path.start == ParsedPathStart::Relative
                                && (f.path.segments.len() == 1 || !scope.has_alias(&f.path));
                            let report_span = ctx.report_span_for_range(item_span.into_range());
                            let msg = "C++ functions must be marked `safe` or `unsafe`";
                            let mut report = Report::build(ReportKind::Error, report_span.clone())
                                .with_message(msg)
                                .with_label(
                                    Label::new(report_span)
                                        .with_message(msg)
                                        .with_color(Color::Red),
                                );
                            if is_bare_relative && matches!(scope.base, EntityPath::Cpp(_)) {
                                report = report.with_help(
                                    "inside a `c++::` module, prefix Rust functions with `::` or `crate::`",
                                );
                            }
                            ctx.add_fatal_report(report.finish());
                            return;
                        };
                        if f.async_span.is_some() {
                            ctx.add_error_str("C++ functions cannot be `async`", item_span);
                            return;
                        }
                        if !f.generics.is_empty() {
                            ctx.add_error_str(
                                "C++ functions do not support generic type arguments",
                                item_span,
                            );
                            return;
                        }
                        if namespace.is_empty() {
                            ctx.add_error_str(
                                "C++ functions in the global namespace aren't supported",
                                item_span,
                            );
                            return;
                        }
                        if f.inputs
                            .iter()
                            .chain([&f.output])
                            .any(contains_relative_self)
                        {
                            ctx.add_error_str(
                                "free C++ functions do not take a `self` receiver (declare member \
                                 functions inside a `type c++::... { ... }` block)",
                                item_span,
                            );
                            return;
                        }
                        let inputs: Vec<RustType> = f
                            .inputs
                            .into_iter()
                            .map(|x| x.to_zngur(scope, ctx))
                            .collect();
                        let output = f.output.to_zngur(scope, ctx);
                        let fn_lifetimes = lifetimes(&f.lifetimes);
                        if !check_types(
                            ctx,
                            item_span,
                            &fn_lifetimes,
                            inputs.iter().chain([&output]),
                            [],
                            LifetimeContext::Item,
                            true,
                            true,
                        ) {
                            return;
                        }
                        let rust_name = f.alias.map(|(s, _)| s);
                        let has_rename = rust_name.is_some();
                        let func = ZngurCppFn {
                            namespace,
                            cpp_name: cpp_name.clone(),
                            rust_name: rust_name.filter(|&n| n != cpp_name).map(str::to_owned),
                            lifetimes: fn_lifetimes,
                            inputs,
                            output,
                            is_safe,
                        };
                        if !check_rust_fn_name(
                            ctx,
                            item_span,
                            "C++ function",
                            func.rust_name(),
                            has_rename,
                            false,
                        ) {
                            return;
                        }
                        check_cpp_signature_lints(
                            ctx,
                            item_span,
                            "function",
                            is_safe,
                            false,
                            false,
                            &func.inputs,
                            &func.output,
                        );
                        if !ctx.fetch_spans_current(&func).is_empty() {
                            ctx.add_error_str(
                                &format!(
                                    "`{}` is already the Rust name of another C++ function in `c++::{}` \
                                     (use `as <rust_name>` to give C++ overloads distinct Rust names)",
                                    func.rust_name(),
                                    func.namespace.join("::")
                                ),
                                item_span,
                            );
                            return;
                        }
                        ctx.record_span(&func, item_span.into_range());
                        checked_merge(
                            func,
                            &mut r.spec,
                            item_span,
                            ctx,
                            Some(MergeContext {
                                outer_ty: RustType::UNIT,
                                variant: None,
                            }),
                        );
                    }
                }
            }
            ProcessedItem::ExternCpp(items) => {
                for item in items {
                    match item {
                        ParsedExternCppItem::Function { is_safe, method } => {
                            let span = method.span;
                            let method = method.inner.to_zngur(scope, ctx);
                            if !check_method_types(ctx, span, &[], &method, []) {
                                continue;
                            }
                            let extern_fn = ZngurExternCppFn {
                                name: method.name.to_string(),
                                lifetimes: method.lifetimes,
                                inputs: method.inputs,
                                output: method.output,
                                is_safe,
                            };
                            ctx.record_span(&extern_fn, span.into_range());
                            checked_merge(extern_fn, &mut r.spec, span, ctx, None);
                        }
                        ParsedExternCppItem::Impl {
                            lifetimes: impl_lifetimes,
                            tr,
                            ty,
                            methods,
                        } => {
                            let impl_lifetimes = lifetimes(&impl_lifetimes);
                            let tr_zngur = tr.map(|x| x.to_zngur(scope, ctx));
                            let ty_zngur = ty.inner.to_zngur(scope, ctx);
                            let trait_ty = tr_zngur.clone().map(|tr| RustType::Dyn(tr, vec![]));
                            let is_inherent_cpp_obj_ref = tr_zngur.is_none()
                                && matches!(
                                    &ty_zngur,
                                    RustType::Ref(_, _, inner) if inner.as_cpp_object().is_some()
                                );
                            let deferred_before = ctx.deferred_cpp_object_checks.len();
                            let mut ok = check_types(
                                ctx,
                                ty.span,
                                &impl_lifetimes,
                                [&ty_zngur].into_iter().chain(trait_ty.as_ref()),
                                [],
                                LifetimeContext::Item,
                                false,
                                is_inherent_cpp_obj_ref,
                            );
                            if ok
                                && tr_zngur.is_none()
                                && (matches!(ty_zngur, RustType::CppPtr(..))
                                    || is_inherent_cpp_obj_ref
                                    || ty_zngur.as_nullable_cpp_ref().is_some())
                            {
                                ctx.deferred_cpp_object_checks.truncate(deferred_before);
                                ctx.add_error_str(
                                    &format!(
                                        "inherent `extern \"C++\"` impls are not supported on C++ \
                                         pointers or references (`{}`)",
                                        zng_spelling(&ty_zngur)
                                    ),
                                    ty.span,
                                );
                                ok = false;
                            }
                            if ok {
                                let mut constrained: HashSet<Lifetime> =
                                    ty_zngur.free_lifetimes().into_iter().collect();
                                if let Some(tr) = &tr_zngur {
                                    // Associated type bindings (`Trait<Assoc = T>`) are stripped
                                    // from the `impl` header into `type Assoc = T;` items inside
                                    // the block by `take_assocs()`, so Rust E0207 does not
                                    // consider lifetimes appearing only in `named_generics` to
                                    // constrain the `impl`'s lifetime parameters.
                                    let (tr_without_assocs, _) = tr.clone().take_assocs();
                                    constrained.extend(
                                        RustType::Dyn(tr_without_assocs, vec![]).free_lifetimes(),
                                    );
                                }
                                for lt in &impl_lifetimes {
                                    if !constrained.contains(lt) {
                                        ctx.add_error_str(
                                            &format!(
                                                "unconstrained lifetime `{lt}` in `impl`: `{lt}` \
                                                 must be used in the implemented trait or self \
                                                 type (if `{lt}` is only used by methods, declare \
                                                 it on each method instead, e.g. `fn f<{lt}>(...)`)"
                                            ),
                                            ty.span,
                                        );
                                        ok = false;
                                    }
                                }
                            }
                            let mut zngur_methods = Vec::with_capacity(methods.len());
                            for (is_safe, x) in methods {
                                let m_span = x.span;
                                let mut m = x.inner.to_zngur(scope, ctx);
                                m.is_safe = Some(is_safe);
                                if !check_method_types(ctx, m_span, &impl_lifetimes, &m, []) {
                                    ok = false;
                                }
                                ctx.record_span(
                                    &PartialSpanKey::extern_cpp_method(
                                        tr_zngur.clone(),
                                        ty_zngur.clone(),
                                        m.name.clone(),
                                    ),
                                    m_span.into_range(),
                                );
                                zngur_methods.push(m);
                            }
                            if !ok {
                                continue;
                            }
                            let extern_impl = ZngurExternCppImpl {
                                lifetimes: impl_lifetimes,
                                tr: tr_zngur,
                                ty: ty_zngur,
                                methods: zngur_methods,
                            };
                            ctx.record_span(&extern_impl, ty.span.into_range());
                            checked_merge(extern_impl, &mut r.spec, ty.span, ctx, None);
                        }
                    }
                }
            }
            ProcessedItem::CppAdditionalInclude(s) => {
                match AdditionalIncludes(s.to_owned()).merge(&mut r.spec) {
                    Ok(()) => {}
                    Err(_) => {
                        unreachable!() // For now, additional includes can't have conflicts.
                    }
                }
            }
            ProcessedItem::ConvertPanicToException(span) => {
                if ctx.depth > 0 {
                    ctx.add_error_str(
                        "Using `#convert_panic_to_exception` in imported zngur files is not supported. This directive can only be used in the main zngur file.",
                        span,
                    );
                    return;
                }
                match ConvertPanicToException(true).merge(&mut r.spec) {
                    Ok(()) => {}
                    Err(_) => {
                        unreachable!() // For now, CPtE also can't have conflicts.
                    }
                }
            }
            ProcessedItem::CrateName(name, span) => {
                if !matches!(&scope.base, EntityPath::Rust(v) if v.is_empty()) {
                    ctx.add_error_str(
                        "`#crate` can only be used at the top level of a file, not inside a `mod` block",
                        span,
                    );
                    return;
                }
                if ctx.depth > 0 {
                    ctx.add_error_str(
                        "`#crate` can only be used in the top-level file of a zngur module, not in merged files",
                        span,
                    );
                    return;
                }
                if is_reserved_crate_name(name) {
                    ctx.add_error_str(&format!("`{name}` cannot be used as a `#crate` name"), span);
                    return;
                }
                match &r.spec.crate_name {
                    Some(existing) if existing != name => ctx.add_error_str(
                        &format!("conflicting `#crate`: already set to `{existing}`"),
                        span,
                    ),
                    _ => r.spec.crate_name = Some(name.to_owned()),
                }
            }
        }
    }
}

/// The kind of a reference in the `.zng` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedRefKind {
    /// `&'a T` / `&'a mut T`: a reference to a Rust value.
    Rust(Mutability),
    /// `&'a [unique] (const | var) T`: a reference to a C++ object. `constness` is `None` for
    /// `&'a unique T`, which is an error (reported by `check_cpp_const`).
    Cpp {
        unique: bool,
        constness: Option<CppConstness>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedRustType<'a> {
    Primitive(PrimitiveRustType),
    /// `&T`, `&'a T`, `&'a mut T`, `&'a const T`, `&'a unique var T`, ... (the lifetime is without
    /// the `'`, and `T` is without the `const` / `var`).
    Ref(ParsedRefKind, Option<&'a str>, Box<ParsedRustType<'a>>),
    Raw(Mutability, Box<ParsedRustType<'a>>),
    Boxed(Box<ParsedRustType<'a>>),
    Slice(Box<ParsedRustType<'a>>),
    Dyn(ParsedRustTrait<'a>, Vec<&'a str>),
    Impl(ParsedRustTrait<'a>, Vec<&'a str>),
    Tuple(Vec<ParsedRustType<'a>>),
    Adt(ParsedRustPathAndGenerics<'a>),
    /// `T*` or `T&`, where `T` may be `const`.
    CppPtr(CppIndirection, Box<ParsedRustType<'a>>),
    /// `const T`.
    CppConst(Box<ParsedRustType<'a>>),
}

impl ParsedRustType<'_> {
    fn to_zngur(self, scope: &Scope<'_>, ctx: &mut ParseContext) -> RustType {
        match self {
            ParsedRustType::Primitive(s) => RustType::Primitive(s),
            // The pointee is a `CppConst` or a `CppVar`.
            ParsedRustType::CppPtr(kind, s) => RustType::CppPtr(
                kind,
                Box::new(RustType::cpp_object(
                    CppConstness::Var,
                    s.to_zngur(scope, ctx),
                )),
            ),
            ParsedRustType::CppConst(s) => RustType::CppConst(Box::new(s.to_zngur(scope, ctx))),
            ParsedRustType::Ref(ParsedRefKind::Rust(m), l, s) => {
                RustType::Ref(m, l.map(lifetime), Box::new(s.to_zngur(scope, ctx)))
            }
            ParsedRustType::Ref(ParsedRefKind::Cpp { unique, constness }, l, s) => {
                let m = if unique {
                    Mutability::Mut
                } else {
                    Mutability::Not
                };
                // `&'a unique T` is rejected by `check_cpp_const`.
                let constness = constness.unwrap_or(CppConstness::Var);
                RustType::Ref(
                    m,
                    l.map(lifetime),
                    Box::new(RustType::cpp_object(constness, s.to_zngur(scope, ctx))),
                )
            }
            ParsedRustType::Raw(m, s) => RustType::Raw(m, Box::new(s.to_zngur(scope, ctx))),
            ParsedRustType::Boxed(s) => RustType::Boxed(Box::new(s.to_zngur(scope, ctx))),
            ParsedRustType::Slice(s) => RustType::Slice(Box::new(s.to_zngur(scope, ctx))),
            ParsedRustType::Dyn(tr, bounds) => RustType::Dyn(
                tr.to_zngur(scope, ctx),
                bounds.into_iter().map(|x| x.to_owned()).collect(),
            ),
            ParsedRustType::Impl(tr, bounds) => RustType::Impl(
                tr.to_zngur(scope, ctx),
                bounds.into_iter().map(|x| x.to_owned()).collect(),
            ),
            ParsedRustType::Tuple(v) => {
                RustType::Tuple(v.into_iter().map(|s| s.to_zngur(scope, ctx)).collect())
            }
            ParsedRustType::Adt(s) => match scope.as_type_var(&s) {
                Some(v) => RustType::TypeVar(v),
                None => match scope.resolve_path(s.path.clone()) {
                    EntityPath::Cpp(segs) => {
                        let is_bare_relative =
                            s.path.start == ParsedPathStart::Relative && !scope.has_alias(&s.path);
                        let bad_cpp_msg = if !s.lifetimes.is_empty() {
                            Some("`c++::` types don't take lifetime arguments")
                        } else if !s.named_generics.is_empty() {
                            Some(
                                "C++ template arguments are positional; `Name = T` is only for \
                                 Rust types",
                            )
                        } else if s.has_explicit_angle_brackets && s.generics.is_empty() {
                            Some(
                                "C++ class template instantiations in `.zng` must specify at \
                                 least one template argument (write out default C++ template \
                                 arguments explicitly)",
                            )
                        } else {
                            None
                        };
                        if let Some(msg) = bad_cpp_msg {
                            let report_span = ctx.report_span_for_range(s.span.into_range());
                            let mut report = Report::build(ReportKind::Error, report_span.clone())
                                .with_message(msg)
                                .with_label(
                                    Label::new(report_span)
                                        .with_message(msg)
                                        .with_color(Color::Red),
                                );
                            if is_bare_relative && matches!(scope.base, EntityPath::Cpp(_)) {
                                report = report.with_help(
                                    "inside a `c++::` module, prefix Rust paths with `::` or `crate::`",
                                );
                            }
                            ctx.add_fatal_report(report.finish());
                        }
                        for (_, g) in s.named_generics {
                            let _ = g.to_zngur(scope, ctx);
                        }
                        let generics: Vec<RustType> = s
                            .generics
                            .into_iter()
                            .map(|x| x.to_zngur(scope, ctx))
                            .collect();
                        RustType::Cpp(CppTypePath {
                            path: segs,
                            generics: if bad_cpp_msg.is_some() {
                                vec![]
                            } else {
                                generics
                            },
                        })
                    }
                    EntityPath::Rust(_) => RustType::Adt(s.to_zngur(scope, ctx)),
                },
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedRustTrait<'a> {
    Normal(ParsedRustPathAndGenerics<'a>),
    Fn {
        name: &'a str,
        /// The lifetimes of a `for<'a>` binder.
        lifetimes: Vec<&'a str>,
        inputs: Vec<ParsedRustType<'a>>,
        output: Box<ParsedRustType<'a>>,
    },
}

impl ParsedRustTrait<'_> {
    fn to_zngur(self, scope: &Scope<'_>, ctx: &mut ParseContext) -> RustTrait {
        match self {
            ParsedRustTrait::Normal(s) => RustTrait::Normal(s.to_zngur(scope, ctx)),
            ParsedRustTrait::Fn {
                name,
                lifetimes: fn_lifetimes,
                inputs,
                output,
            } => RustTrait::Fn {
                name: name.to_owned(),
                lifetimes: lifetimes(&fn_lifetimes),
                inputs: inputs.into_iter().map(|s| s.to_zngur(scope, ctx)).collect(),
                output: Box::new(output.to_zngur(scope, ctx)),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedRustPathAndGenerics<'a> {
    path: ParsedPath<'a>,
    /// Lifetime arguments, without the `'`.
    lifetimes: Vec<&'a str>,
    generics: Vec<ParsedRustType<'a>>,
    named_generics: Vec<(&'a str, ParsedRustType<'a>)>,
    has_explicit_angle_brackets: bool,
    span: Span,
}

impl ParsedRustPathAndGenerics<'_> {
    fn to_zngur(self, scope: &Scope<'_>, ctx: &mut ParseContext) -> RustPathAndGenerics {
        let orig_path = self.path.clone();
        let full_span = self.span;
        if self
            .generics
            .iter()
            .any(|g| matches!(g, ParsedRustType::CppConst(_)))
        {
            ctx.add_error_str(
                "`const T` can only be the type of a C++ pointer or reference \
                 (`const T&`, `const T*`, `&'a const T`) or a C++ template argument",
                full_span,
            );
        }
        RustPathAndGenerics {
            path: match scope.resolve_path(self.path) {
                EntityPath::Rust(v) => v,
                EntityPath::Cpp(segs) => {
                    scope.report_invalid_cpp_path(
                        ctx,
                        &orig_path,
                        "`c++::` paths cannot be used as a trait",
                        "traits",
                    );
                    segs
                }
            },
            lifetimes: lifetimes(&self.lifetimes),
            generics: self
                .generics
                .into_iter()
                .map(|x| match x {
                    ParsedRustType::CppConst(inner) => inner.to_zngur(scope, ctx),
                    x => x.to_zngur(scope, ctx),
                })
                .collect(),
            named_generics: self
                .named_generics
                .into_iter()
                .map(|(name, x)| (name.to_owned(), x.to_zngur(scope, ctx)))
                .collect(),
        }
    }
}

pub type SourceId = usize;
pub type ReportSpan = (SourceId, std::ops::Range<usize>);
pub type ParseReport<'b> = Report<'b, ReportSpan>;

/// A diagnostic report, tagged with whether it should abort the parse.
pub struct ReportEntry<'b> {
    pub fatal: bool,
    pub report: ParseReport<'b>,
}

/// One half of a split key to identify a parsed item type
/// to store a list of spans for that type. The other half is a [`SourceId`](SourceId).
///
/// Uniquely identifying an individual span would require generating
/// and storing a declaration id in the parser and most spans are only
/// needed to identify conflicts when merging declarations so identifying
/// a declaration type is enough
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PartialSpanKey {
    /// an [`Import`](zngur_def::Import)
    Import(Box<std::path::Path>),
    /// a [`Type`](zngur_def::ZngurType)
    Ty(RustType),
    /// a [`Trait`](zngur_def::ZngurTrait)
    Trait(RustTrait),
    /// a [`Fn`](zngur_def::ZngurFn)
    Fn(ZngurFn),

    // spans likely to be inside others
    /// a [`LayoutPolicy`](zngur_def::LayoutPolicy)
    /// outer_ty
    Layout(RustType),
    /// a [`Constructor`](zngur_def::ZngurConstructor)
    /// outer_ty, constructor_sig?
    Constructor(RustType, Option<Vec<(String, RustType)>>),
    /// a [`CppRef`](zngur_def::CppRef)
    /// outer_ty
    CppRef(RustType),
    /// a [`CppHeapAllocated`](zngur_def::CppHeapAllocated)
    /// outer_ty
    CppHeapAllocated(RustType),
    /// a [`CppStackOwned`](zngur_def::CppStackOwned)
    /// outer_ty
    CppStackOwned(RustType),
    /// a `#trivially_relocatable` or `#unsafe(trivially_relocatable)` directive
    /// outer_ty, kind
    TriviallyRelocatable(RustType, TriviallyRelocatable),
    /// a [`ZngurWellknownTrait`](zngur_def::ZngurWellknownTrait)
    /// outer_ty, trait
    WellknownTrait(RustType, ZngurWellknownTrait),
    /// a [`Method`](zngur_def::ZngurMethod) on a
    /// [`Type`](zngur_def::ZngurType) or [`Trait`](zngur_def::ZngurTrait)
    /// outer_ty, method
    Method(RustType, ZngurMethod),
    /// a [`ZngurCppMethod`](zngur_def::ZngurCppMethod) on a [`Type`](zngur_def::ZngurType)
    /// outer_ty, method
    CppMethod(RustType, ZngurCppMethod),
    /// a [`CppConstructor`](zngur_def::ZngurCppConstructor)
    /// outer_ty, rust_name
    CppConstructor(RustType, String),
    /// a [`ZngurCppField`](zngur_def::ZngurCppField)
    /// outer_ty, rust_name
    CppField(RustType, String),
    /// a [`ZngurCppFn`](zngur_def::ZngurCppFn)
    /// namespace, rust_name
    CppFn(Vec<String>, String),
    /// a [`ZngurExternCppFn`](zngur_def::ZngurExternCppFn)
    ExternCppFn(String),
    /// a [`ZngurExternCppImpl`](zngur_def::ZngurExternCppImpl)
    ExternCppImpl(Option<RustTrait>, RustType),
    /// a method inside an `extern "C++" { impl ... }`
    ExternCppMethod(Option<RustTrait>, RustType, String),
    /// a method inside a `trait ...`
    TraitMethod(RustTrait, String),
    /// a [`Field`](`zngur_def::ZngurField`) on a [`Type`](zngur_def::ZngurType)
    /// or it's internal [`Variant`](zngur_def::ZngurVariant)
    /// outer_ty, variant?, field_name
    Field(RustType, Option<String>, String),
}

impl PartialSpanKey {
    /// combine with a [`SourceId`] to make a full [`SpanKey`]
    fn full_key_with(self, source_id: SourceId) -> SpanKey {
        SpanKey(source_id, self)
    }

    fn layout(outer_ty: RustType) -> Self {
        Self::Layout(outer_ty.with_erased_lifetimes())
    }

    fn constructor(outer_ty: RustType, sig: Option<Vec<(String, RustType)>>) -> Self {
        Self::Constructor(
            outer_ty.with_erased_lifetimes(),
            sig.map(|v| {
                v.into_iter()
                    .map(|(n, t)| (n, t.with_erased_lifetimes()))
                    .collect()
            }),
        )
    }

    fn cpp_ref(outer_ty: RustType) -> Self {
        Self::CppRef(outer_ty.with_erased_lifetimes())
    }

    fn cpp_heap_allocated(outer_ty: RustType) -> Self {
        Self::CppHeapAllocated(outer_ty.with_erased_lifetimes())
    }

    fn cpp_stack_owned(outer_ty: RustType) -> Self {
        Self::CppStackOwned(outer_ty.with_erased_lifetimes())
    }

    fn trivially_relocatable(outer_ty: RustType, kind: TriviallyRelocatable) -> Self {
        Self::TriviallyRelocatable(outer_ty.with_erased_lifetimes(), kind)
    }

    fn wellknown_trait(outer_ty: RustType, tr: ZngurWellknownTrait) -> Self {
        Self::WellknownTrait(outer_ty.with_erased_lifetimes(), tr)
    }

    fn method(outer_ty: RustType, method: ZngurMethod) -> Self {
        Self::Method(
            outer_ty.with_erased_lifetimes(),
            ZngurMethod {
                name: method.name,
                lifetimes: vec![],
                generics: method
                    .generics
                    .into_iter()
                    .map(|t| t.with_erased_lifetimes())
                    .collect(),
                receiver: method.receiver,
                receiver_lifetime: None,
                inputs: method
                    .inputs
                    .into_iter()
                    .map(|t| t.with_erased_lifetimes())
                    .collect(),
                output: method.output.with_erased_lifetimes(),
                is_safe: method.is_safe,
            },
        )
    }

    fn cpp_method(outer_ty: RustType, method: ZngurCppMethod) -> Self {
        Self::CppMethod(
            outer_ty.with_erased_lifetimes(),
            method.with_erased_lifetimes(),
        )
    }

    fn field(outer_ty: RustType, variant: Option<String>, name: String) -> Self {
        Self::Field(outer_ty.with_erased_lifetimes(), variant, name)
    }

    fn cpp_constructor(outer_ty: RustType, name: String) -> Self {
        Self::CppConstructor(outer_ty.with_erased_lifetimes(), name)
    }
    fn cpp_field(outer_ty: RustType, name: String) -> Self {
        Self::CppField(outer_ty.with_erased_lifetimes(), name)
    }

    fn extern_cpp_impl(tr: Option<RustTrait>, ty: RustType) -> Self {
        let tr = tr.map(|tr| {
            let RustType::Dyn(erased, _) = RustType::Dyn(tr, vec![]).with_erased_lifetimes() else {
                unreachable!()
            };
            erased
        });
        Self::ExternCppImpl(tr, ty.with_erased_lifetimes())
    }

    fn extern_cpp_method(tr: Option<RustTrait>, ty: RustType, name: String) -> Self {
        let Self::ExternCppImpl(tr, ty) = Self::extern_cpp_impl(tr, ty) else {
            unreachable!()
        };
        Self::ExternCppMethod(tr, ty, name)
    }

    fn trait_method(tr: RustTrait, name: String) -> Self {
        let RustType::Dyn(erased, _) = RustType::Dyn(tr, vec![]).with_erased_lifetimes() else {
            unreachable!()
        };
        Self::TraitMethod(erased, name)
    }
}

/// a full key identifying a list of spans for a given type in a particular source
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SpanKey(SourceId, PartialSpanKey);

/// trait to impl for types to allow easy span lookup
trait IntoSpanKey {
    fn into_span_key(&self) -> PartialSpanKey;
}

impl IntoSpanKey for SpanKey {
    fn into_span_key(&self) -> PartialSpanKey {
        self.1.clone()
    }
}

impl IntoSpanKey for PartialSpanKey {
    fn into_span_key(&self) -> PartialSpanKey {
        self.clone()
    }
}

impl IntoSpanKey for zngur_def::Import {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Import(self.0.as_path().into())
    }
}

impl IntoSpanKey for zngur_def::ZngurType {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Ty(self.ty.canonicalize_lifetimes(&self.lifetimes))
    }
}
impl IntoSpanKey for zngur_def::RustType {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Ty(self.clone())
    }
}

impl IntoSpanKey for zngur_def::ZngurTrait {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Trait(self.tr.clone())
    }
}
impl IntoSpanKey for zngur_def::RustTrait {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Trait(self.clone())
    }
}

impl IntoSpanKey for zngur_def::ZngurFn {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::Fn(self.clone())
    }
}

impl IntoSpanKey for zngur_def::ZngurCppFn {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::CppFn(self.namespace.clone(), self.rust_name().to_owned())
    }
}

impl IntoSpanKey for zngur_def::ZngurExternCppFn {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::ExternCppFn(self.name.clone())
    }
}

impl IntoSpanKey for zngur_def::ZngurExternCppImpl {
    fn into_span_key(&self) -> PartialSpanKey {
        PartialSpanKey::extern_cpp_impl(self.tr.clone(), self.ty.clone())
    }
}

/// trait to impl for type that require variant and type qualifications for lookup
trait VariantQualifiedSpanKeyExt {
    fn into_span_key_with_variant(
        &self,
        outer_ty: RustType,
        variant: Option<String>,
    ) -> PartialSpanKey;
}

/// trait to impl for type that require type qualifications for lookup
trait QualifiedSpanKeyExt {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey;
}

impl VariantQualifiedSpanKeyExt for zngur_def::ConflictSource {
    fn into_span_key_with_variant(
        &self,
        outer_ty: RustType,
        variant: Option<String>,
    ) -> PartialSpanKey {
        match self {
            zngur_def::ConflictSource::Type(ty) => PartialSpanKey::Ty(ty.clone()),
            zngur_def::ConflictSource::Layout => PartialSpanKey::layout(outer_ty),
            zngur_def::ConflictSource::Constructor(sig) => {
                PartialSpanKey::constructor(outer_ty, sig.clone())
            }
            zngur_def::ConflictSource::CppRef => PartialSpanKey::cpp_ref(outer_ty),
            zngur_def::ConflictSource::CppHeapAllocated => {
                PartialSpanKey::cpp_heap_allocated(outer_ty)
            }
            zngur_def::ConflictSource::CppStackOwned => PartialSpanKey::cpp_stack_owned(outer_ty),
            zngur_def::ConflictSource::Method(method) => {
                PartialSpanKey::method(outer_ty, method.clone())
            }
            zngur_def::ConflictSource::CppMethod(method) => {
                PartialSpanKey::cpp_method(outer_ty, (**method).clone())
            }
            zngur_def::ConflictSource::CppConstructor(name) => {
                PartialSpanKey::cpp_constructor(outer_ty, name.clone())
            }
            zngur_def::ConflictSource::CppField(name) => {
                PartialSpanKey::cpp_field(outer_ty, name.clone())
            }
            zngur_def::ConflictSource::CppFn(namespace, name) => {
                PartialSpanKey::CppFn(namespace.clone(), name.clone())
            }
            zngur_def::ConflictSource::Field(name) => {
                PartialSpanKey::field(outer_ty, variant, name.clone())
            }
        }
    }
}

impl VariantQualifiedSpanKeyExt for zngur_def::ZngurField {
    fn into_span_key_with_variant(
        &self,
        outer_ty: RustType,
        variant: Option<String>,
    ) -> PartialSpanKey {
        PartialSpanKey::field(outer_ty, variant, self.name.clone())
    }
}

impl QualifiedSpanKeyExt for zngur_def::LayoutPolicy {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::layout(outer_ty)
    }
}

impl QualifiedSpanKeyExt for zngur_def::ZngurMethod {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::method(outer_ty, self.clone())
    }
}

impl QualifiedSpanKeyExt for zngur_def::ZngurCppMethod {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::cpp_method(outer_ty, self.clone())
    }
}

impl QualifiedSpanKeyExt for zngur_def::ZngurCppField {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::cpp_field(outer_ty, self.rust_name().to_owned())
    }
}

impl QualifiedSpanKeyExt for zngur_def::CppHeapAllocated {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::cpp_heap_allocated(outer_ty)
    }
}

impl QualifiedSpanKeyExt for zngur_def::CppRef {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::cpp_ref(outer_ty)
    }
}

impl QualifiedSpanKeyExt for zngur_def::CppStackOwned {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::cpp_stack_owned(outer_ty)
    }
}

impl QualifiedSpanKeyExt for zngur_def::ZngurConstructor {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::constructor(outer_ty, Some(self.inputs.clone()))
    }
}

/// A wrapper around both an owned value and a exclusive mutable
/// reference to that same value. Used to allow nested parse contexts
/// to internally borrow from their parent.
///
/// Implements the important [`AsMut`](AsMut) and
/// [`Deref`](Deref)/[`DerefMut`](DerefMut) traits to allow the type
/// to be used transparently.
enum OwnedRefMut<'a, T> {
    Owned(T),
    Borrowed(&'a mut T),
}

impl<'a, T> Deref for OwnedRefMut<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(t) => t,
            Self::Borrowed(t) => t,
        }
    }
}

impl<'a, T> DerefMut for OwnedRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Owned(t) => t,
            Self::Borrowed(t) => t,
        }
    }
}

impl<'a, T, U> AsMut<U> for OwnedRefMut<'a, T>
where
    <OwnedRefMut<'a, T> as Deref>::Target: AsMut<U>,
{
    fn as_mut(&mut self) -> &mut U {
        self.deref_mut().as_mut()
    }
}

impl<'a, T, U> AsRef<U> for OwnedRefMut<'a, T>
where
    <OwnedRefMut<'a, T> as Deref>::Target: AsRef<U>,
{
    fn as_ref(&self) -> &U {
        self.deref().as_ref()
    }
}

impl<'a, T> OwnedRefMut<'a, T> {
    /// exclusively borrows from self returning a wrapped value
    pub fn into_borrowed<'t>(&'t mut self) -> OwnedRefMut<'t, T> {
        match self {
            Self::Owned(t) => OwnedRefMut::<'t, T>::Borrowed(t),
            Self::Borrowed(t) => OwnedRefMut::<'t, T>::Borrowed(t),
        }
    }

    /// consume self to return the inner value if owned
    #[allow(dead_code)]
    pub fn into_inner(self) -> Option<T> {
        match self {
            Self::Owned(t) => Some(t),
            Self::Borrowed(_) => None,
        }
    }
}

impl<'a, T: Default> Default for OwnedRefMut<'a, T> {
    fn default() -> Self {
        Self::Owned(Default::default())
    }
}

impl<'a, T: Clone> OwnedRefMut<'a, T> {
    /// clones the inner value unconditionally
    pub fn clone_inner(&self) -> T {
        match self {
            Self::Owned(t) => t.clone(),
            Self::Borrowed(t) => (*t).clone(),
        }
    }

    /// returns the inner value, cloning a borrowed value if necessary
    #[allow(dead_code)]
    pub fn unwrap_or_clone(self) -> T {
        match self {
            Self::Owned(t) => t,
            Self::Borrowed(t) => t.clone(),
        }
    }
}

/// Lexically normalizes `.` and `..` components in `path` without touching the filesystem.
pub(crate) fn normalize_lexical_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !out.is_absolute() {
                    out.push("..");
                }
            }
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(comp.as_os_str()),
            Component::Normal(c) => out.push(c),
        }
    }
    if out.as_os_str().is_empty() {
        std::path::PathBuf::from(".")
    } else {
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IndirectTemplateHome {
    home: CppTemplateHome,
    home_module_path: std::path::PathBuf,
    home_crate_name: Option<String>,
}

#[derive(Clone)]
struct CachedImportedModule {
    crate_name: Option<String>,
    cpp_types: Vec<Vec<String>>,
    cpp_heap_allocated_types: Vec<Vec<String>>,
    cpp_by_value_types: Vec<(RustType, String)>,
    cpp_template_homes: Vec<CppTemplateHome>,
    indirect_cpp_template_homes: Vec<IndirectTemplateHome>,
    cpp_templates: Vec<Vec<String>>,
    cpp_template_instantiations: Vec<CppTypePath>,
    indirect_cpp_template_instantiations: Vec<(CppTypePath, std::path::PathBuf)>,
    own_cpp_templates: Vec<TemplateDef>,
    had_errors: bool,
}

fn rewrite_crate_in_rust_path_and_generics(pg: &mut RustPathAndGenerics, home_crate: &str) {
    if pg.path.first().is_some_and(|s| s == "crate") {
        pg.path[0] = home_crate.to_owned();
    }
    for g in &mut pg.generics {
        rewrite_crate_in_rust_type(g, home_crate);
    }
    for (_, g) in &mut pg.named_generics {
        rewrite_crate_in_rust_type(g, home_crate);
    }
}

fn rewrite_crate_in_rust_type(ty: &mut RustType, home_crate: &str) {
    match ty {
        RustType::Adt(pg) => rewrite_crate_in_rust_path_and_generics(pg, home_crate),
        RustType::Cpp(path) => {
            for g in &mut path.generics {
                rewrite_crate_in_rust_type(g, home_crate);
            }
        }
        RustType::Ref(_, _, inner)
        | RustType::Raw(_, inner)
        | RustType::Boxed(inner)
        | RustType::Slice(inner)
        | RustType::CppPtr(_, inner)
        | RustType::CppConst(inner)
        | RustType::CppVar(inner) => rewrite_crate_in_rust_type(inner, home_crate),
        RustType::Tuple(tys) => {
            for t in tys {
                rewrite_crate_in_rust_type(t, home_crate);
            }
        }
        RustType::Dyn(tr, _) | RustType::Impl(tr, _) => match tr {
            RustTrait::Normal(pg) => rewrite_crate_in_rust_path_and_generics(pg, home_crate),
            RustTrait::Fn { inputs, output, .. } => {
                for t in inputs {
                    rewrite_crate_in_rust_type(t, home_crate);
                }
                rewrite_crate_in_rust_type(output, home_crate);
            }
        },
        RustType::Primitive(_) | RustType::TypeVar(_) => {}
    }
}

fn rewrite_crate_paths_in_zngur_type(ty: &mut ZngurType, home_crate: &str) {
    rewrite_crate_in_rust_type(&mut ty.ty, home_crate);
    for method in &mut ty.methods {
        for g in &mut method.data.generics {
            rewrite_crate_in_rust_type(g, home_crate);
        }
        for input in &mut method.data.inputs {
            rewrite_crate_in_rust_type(input, home_crate);
        }
        rewrite_crate_in_rust_type(&mut method.data.output, home_crate);
        if let Some(use_path) = &mut method.use_path {
            if use_path == "crate" {
                *use_path = format!("::{home_crate}");
            } else if let Some(rest) = use_path.strip_prefix("crate::") {
                *use_path = format!("::{home_crate}::{rest}");
            }
        }
        if let Some((deref_ty, _)) = &mut method.deref {
            rewrite_crate_in_rust_type(deref_ty, home_crate);
        }
    }
    for method in &mut ty.cpp_methods {
        for input in &mut method.inputs {
            rewrite_crate_in_rust_type(input, home_crate);
        }
        rewrite_crate_in_rust_type(&mut method.output, home_crate);
    }
    for ctor in &mut ty.cpp_constructors {
        for input in &mut ctor.inputs {
            rewrite_crate_in_rust_type(input, home_crate);
        }
    }
    for field in &mut ty.cpp_fields {
        rewrite_crate_in_rust_type(&mut field.ty, home_crate);
    }
    if let Some(ctor) = &mut ty.constructor {
        for (_, input) in &mut ctor.inputs {
            rewrite_crate_in_rust_type(input, home_crate);
        }
    }
    for variant in &mut ty.variants {
        for field in &mut variant.fields {
            rewrite_crate_in_rust_type(&mut field.ty, home_crate);
        }
    }
    for field in &mut ty.fields {
        rewrite_crate_in_rust_type(&mut field.ty, home_crate);
    }
}

#[derive(Debug, Default)]
struct ReportCounter {
    pub errors: usize,
    pub warnings: usize,
}

/// The parse context. Holds references to the current source path and text and tracks
/// errors and declaration spans. Also holds the [configuration provider](RustCfgProvider)
/// and [report sink](ReportSink) used for this parse.
struct ParseContext<'this, 'source, 'cfg> {
    path: &'source std::path::Path,
    /// Parent directory of the top-level `.zng` file of the module currently being parsed.
    /// Relative `import` paths inside nested `merge` files are rebased onto this directory.
    module_root_dir: Option<std::path::PathBuf>,
    source: &'source str,
    source_id: SourceId,
    depth: usize,
    /// Lints silenced for the current file via `#allow(...)`.
    allowed_lints: HashSet<String>,
    /// Whether non-fatal warnings are suppressed (when recursively parsing `import`ed modules,
    /// whose own builds emit their warnings).
    suppress_warnings: bool,
    cfg_provider: &'cfg dyn RustCfgProvider,
    report_sink: &'cfg mut dyn ReportSink,
    /// All .zng files processed during parsing (main file + imports)
    processed_files: OwnedRefMut<'this, Vec<std::path::PathBuf>>,
    report_counter: OwnedRefMut<'this, ReportCounter>,
    sources: OwnedRefMut<'this, indexmap::IndexMap<std::path::PathBuf, ariadne::Source<String>>>,
    recorded_spans: OwnedRefMut<'this, indexmap::IndexMap<SpanKey, Vec<ReportSpan>>>,
    deferred_cpp_object_checks: OwnedRefMut<'this, Vec<DeferredCppObjectCheck>>,
    /// Ancestor modules currently being parsed via `import`, normalized lexically, to detect
    /// import cycles.
    active_module_stack: OwnedRefMut<'this, Vec<std::path::PathBuf>>,
    /// Already-parsed `import`ed modules, keyed by normalized path, so diamond module DAGs parse
    /// each module once.
    parsed_module_cache:
        OwnedRefMut<'this, std::collections::HashMap<std::path::PathBuf, CachedImportedModule>>,
}

impl<'this, 'source, 'cfg> ParseContext<'this, 'source, 'cfg> {
    fn new(
        path: &'source std::path::Path,
        source: &'source str,
        cfg: &'cfg dyn RustCfgProvider,
        report_sink: &'cfg mut dyn ReportSink,
    ) -> Self {
        let processed_files = OwnedRefMut::Owned(vec![path.to_path_buf()]);
        let active_module_stack = OwnedRefMut::Owned(vec![normalize_lexical_path(path)]);
        let mut sources = OwnedRefMut::Owned(indexmap::IndexMap::default());
        let (source_id, _) = sources.insert_full(
            path.to_path_buf(),
            ariadne::Source::from(source.to_string()),
        );
        Self {
            path,
            module_root_dir: path.parent().map(std::path::Path::to_path_buf),
            source_id,
            source,
            depth: 0,
            allowed_lints: Default::default(),
            suppress_warnings: false,
            cfg_provider: cfg,
            report_sink,
            processed_files,
            report_counter: Default::default(),
            sources,
            recorded_spans: Default::default(),
            deferred_cpp_object_checks: Default::default(),
            active_module_stack,
            parsed_module_cache: Default::default(),
        }
    }

    /// build a nested parse context for parsing a new source
    /// to be merged into the current one
    fn nested<'borrowed, 'src>(
        &'borrowed mut self,
        path: &'src std::path::Path,
        source: &'src str,
    ) -> ParseContext<'borrowed, 'src, 'borrowed> {
        let (source_id, _) = self.sources.insert_full(
            path.to_path_buf(),
            ariadne::Source::from(source.to_string()),
        );
        self.processed_files.push(path.to_path_buf());
        ParseContext {
            path,
            module_root_dir: self.module_root_dir.clone(),
            source_id,
            source,
            depth: self.depth + 1,
            allowed_lints: Default::default(),
            suppress_warnings: self.suppress_warnings,
            cfg_provider: self.cfg_provider,
            report_sink: self.report_sink,
            processed_files: self.processed_files.into_borrowed(),
            report_counter: self.report_counter.into_borrowed(),
            sources: self.sources.into_borrowed(),
            recorded_spans: self.recorded_spans.into_borrowed(),
            deferred_cpp_object_checks: self.deferred_cpp_object_checks.into_borrowed(),
            active_module_stack: self.active_module_stack.into_borrowed(),
            parsed_module_cache: self.parsed_module_cache.into_borrowed(),
        }
    }

    /// Build a parse context for the top-level file of an `import`ed module, which is parsed
    /// into its own spec (it is the top level of that module with its own isolated span table).
    fn nested_module<'borrowed, 'src>(
        &'borrowed mut self,
        path: &'src std::path::Path,
        source: &'src str,
    ) -> ParseContext<'borrowed, 'src, 'borrowed> {
        let mut ctx = self.nested(path, source);
        ctx.module_root_dir = path.parent().map(std::path::Path::to_path_buf);
        ctx.depth = 0;
        ctx.suppress_warnings = true;
        ctx.recorded_spans = OwnedRefMut::Owned(Default::default());
        ctx.deferred_cpp_object_checks = OwnedRefMut::Owned(Default::default());
        ctx
    }

    /// the [`SourceId`] of the current source which can be fetched from the cache
    /// retrieved by calling [`source_cache`](Self::source_cache)
    fn source_id(&self) -> SourceId {
        self.source_id
    }

    fn allows(&self, lint: &str) -> bool {
        self.allowed_lints.contains(lint)
    }

    /// returns a borrowed source cache that implements the ariadne Cache trait
    #[allow(dead_code)]
    fn source_cache(&self) -> SourceCache<'_> {
        SourceCache::new(&self.sources)
    }

    /// Renders a single report to text (stripped of ANSI color codes).
    #[allow(dead_code)]
    fn render_report(&self, report: &ParseReport) -> String {
        let mut buf = Vec::new();
        report.write(self.source_cache(), &mut buf).unwrap();
        String::from_utf8(strip_ansi_escapes::strip(buf)).unwrap()
    }

    /// Record a report that should invalidate the current parse
    fn add_fatal_report<'r>(&mut self, report: ParseReport<'r>) {
        self.sink_report(ReportEntry {
            fatal: true,
            report,
        });
    }

    /// Record a non-fatal report
    fn add_report<'r>(&mut self, report: ParseReport<'r>) {
        self.sink_report(ReportEntry {
            fatal: false,
            report,
        });
    }

    /// Record fatal reports from the passed parse errors
    fn add_errors<'err_src>(&mut self, errs: impl Iterator<Item = Rich<'err_src, String>>) {
        let path = self.source_id;
        for e in errs {
            let e_span = (path.to_owned(), e.span().into_range());
            let report = Report::build(ReportKind::Error, e_span.clone())
                .with_message(e.to_string())
                .with_label(
                    Label::new(e_span)
                        .with_message(e.reason().to_string())
                        .with_color(Color::Red),
                )
                .with_labels(e.contexts().map(|(label, span)| {
                    let l_span = (path.to_owned(), span.into_range());
                    Label::new(l_span)
                        .with_message(format!("while parsing this {}", label))
                        .with_color(Color::Yellow)
                }))
                .finish();
            self.sink_report(ReportEntry {
                fatal: true,
                report,
            });
        }
    }

    /// Record a fatal report from the passed message and span
    fn add_error_str(&mut self, error: &str, span: Span) {
        self.add_errors([Rich::custom(span, error)].into_iter());
    }

    /// Records a non-fatal warning as an ariadne diagnostic pointing at `span`.
    /// Rendering is deferred until dispatch time (see `render_report`).
    fn add_warning_str(&mut self, warning: &str, span: Span) {
        let report = Report::build(
            ReportKind::Warning,
            (self.source_id.to_owned(), span.into_range()),
        )
        .with_message(warning)
        .with_label(
            Label::new((self.source_id.to_owned(), span.into_range()))
                .with_message(warning)
                .with_color(Color::Yellow),
        )
        .finish();
        self.sink_report(ReportEntry {
            fatal: false,
            report,
        });
    }

    /// a count of the errors recorded by this parse context and any nested ones
    fn reports(&self) -> &ReportCounter {
        &self.report_counter
    }

    /// drains and sinks the current reports
    /// returns true if any errors were reported
    fn sink_report<'r>(&mut self, report: ReportEntry<'r>) {
        if !report.fatal && self.suppress_warnings {
            return;
        }
        let ParseContext {
            report_sink,
            sources,
            report_counter,
            ..
        } = self;
        let report_counter = report_counter.deref_mut();
        if report.fatal {
            report_counter.errors += 1;
        } else {
            report_counter.warnings += 1;
        }
        let source_cache = SourceCache::new(sources);
        report_sink.sink_report(&report, source_cache);
    }

    fn get_config_provider(&self) -> &dyn RustCfgProvider {
        self.cfg_provider
    }

    fn get_processed_files(&self) -> Vec<std::path::PathBuf> {
        self.processed_files.clone_inner()
    }

    /// Record a span for the key as existing in the currently processing file
    fn record_span<K: IntoSpanKey>(&mut self, key: &K, range: std::ops::Range<usize>) {
        let span = self.report_span_for_range(range);
        use indexmap::map::Entry;
        match self
            .recorded_spans
            .entry(key.into_span_key().full_key_with(self.source_id))
        {
            Entry::Occupied(mut spans) => {
                let spans = spans.get_mut();
                if !spans.contains(&span) {
                    spans.push(span);
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![span]);
            }
        }
    }

    /// Transform a range into a [report span](ReportSpan) for the currently processing file
    fn report_span_for_range(&self, range: std::ops::Range<usize>) -> ReportSpan {
        (self.source_id.to_owned(), range)
    }

    /// fetch the recorded spans for the item in the currently processing source
    /// stored in order of declaration
    fn fetch_spans_current<K: IntoSpanKey>(&self, key: &K) -> Vec<&ReportSpan> {
        self.fetch_spans_for(key, self.source_id)
    }

    /// fetch the recorded spans for the item in the given source
    /// stored in order of declaration
    fn fetch_spans_for<K: IntoSpanKey>(&self, key: &K, source_id: SourceId) -> Vec<&ReportSpan> {
        self.recorded_spans
            .get(&key.into_span_key().full_key_with(source_id))
            .into_iter()
            .flatten()
            .collect()
    }

    /// fetch the recorded spans for the item in all sources
    /// stored in order of declaration
    fn fetch_spans_global<K: IntoSpanKey>(&self, key: &K) -> Vec<&ReportSpan> {
        let partial = key.into_span_key();
        self.recorded_spans
            .iter()
            .filter_map(
                |(SpanKey(_, part), spans)| {
                    if *part == partial { Some(spans) } else { None }
                },
            )
            .flatten()
            .collect()
    }
}

/// A trait for types which can resolve filesystem-like paths relative to a given directory.
pub trait ImportResolver {
    fn resolve_import(
        &self,
        cwd: &std::path::Path,
        relpath: &std::path::Path,
    ) -> Result<String, String>;
}

/// A default implementation of ImportResolver which uses conventional filesystem paths and semantics.
pub struct DefaultImportResolver;

/// Make snapshot stable for windows.
fn import_error_message(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        "No such file or directory".to_owned()
    } else {
        error.to_string()
    }
}

impl ImportResolver for DefaultImportResolver {
    fn resolve_import(
        &self,
        cwd: &std::path::Path,
        relpath: &std::path::Path,
    ) -> Result<String, String> {
        let path = cwd
            .join(relpath)
            .canonicalize()
            .map_err(import_error_message)?;
        std::fs::read_to_string(path).map_err(import_error_message)
    }
}

impl<T: ImportResolver + ?Sized> ImportResolver for Box<T> {
    fn resolve_import(
        &self,
        cwd: &std::path::Path,
        relpath: &std::path::Path,
    ) -> Result<String, String> {
        self.deref().resolve_import(cwd, relpath)
    }
}

/// a [`ariadne::Cache<SourceId>`] compatible source cache
/// internally maps a [`SourceId`] to a [`ariadne::Source<String>`]
/// borrowed from the [`ParseContext`]
#[derive(Debug, Clone, Copy)]
pub struct SourceCache<'c> {
    sources: &'c indexmap::IndexMap<std::path::PathBuf, ariadne::Source<String>>,
}

impl<'c> SourceCache<'c> {
    fn new(sources: &'c indexmap::IndexMap<std::path::PathBuf, ariadne::Source<String>>) -> Self {
        Self { sources }
    }
}

impl<'c> ariadne::Cache<SourceId> for SourceCache<'c> {
    type Storage = String;
    fn fetch(
        &mut self,
        id: &SourceId,
    ) -> Result<&ariadne::Source<Self::Storage>, impl std::fmt::Debug> {
        match self.sources.get_index(*id) {
            Some((_path, source)) => Ok(source),
            None => Err(format!("Unknown source id '{}'", id)),
        }
    }

    fn display<'a>(&self, id: &'a SourceId) -> Option<impl std::fmt::Display + 'a> {
        self.sources
            .get_index(*id)
            .map(|(path, _source)| path.display().to_string())
    }
}

/// a type that can receive reports from the zngur parser as they are generated
pub trait ReportSink {
    fn sink_report(&mut self, report: &ReportEntry, source_cache: SourceCache);
}

impl<T> ReportSink for T
where
    T: for<'r, 'c> FnMut(bool, &ParseReport<'r>, SourceCache<'c>),
{
    fn sink_report(&mut self, report: &ReportEntry, source_cache: SourceCache) {
        self(report.fatal, &report.report, source_cache)
    }
}

/// a [`ReportSink`] that prints to `stderr`
pub struct StdErrReportSink<const ERRORS: bool = true, const WARNINGS: bool = true>;

impl<const ERRORS: bool, const WARNINGS: bool> ReportSink for StdErrReportSink<ERRORS, WARNINGS> {
    fn sink_report(&mut self, report: &ReportEntry, source_cache: SourceCache) {
        if (report.fatal && ERRORS) || (!report.fatal && WARNINGS) {
            let _ = report.report.eprint(source_cache);
        }
    }
}

fn is_reserved_crate_name(name: &str) -> bool {
    matches!(
        name,
        "Self"
            | "abstract"
            | "as"
            | "async"
            | "await"
            | "become"
            | "box"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "do"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "final"
            | "fn"
            | "for"
            | "gen"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "macro"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "override"
            | "priv"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "try"
            | "type"
            | "typeof"
            | "unsafe"
            | "unsized"
            | "use"
            | "virtual"
            | "where"
            | "while"
            | "yield"
    )
}

fn normalize_lexical_components(path: &std::path::Path) -> Vec<std::path::Component<'_>> {
    use std::path::Component;
    let mut out = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir) => {}
                _ => out.push(Component::ParentDir),
            },
            other => out.push(other),
        }
    }
    out
}

/// Rebases a `./`- or `../`-prefixed `import` path written inside a `merge`d file (`current_dir`)
/// so that it is relative to the top-level `.zng` file's directory (`module_root_dir`). Bare
/// C++ include-search paths (like `"packet.zng"`) are returned unchanged.
fn rebase_relative_import_path(
    module_root_dir: &std::path::Path,
    current_dir: &std::path::Path,
    import_path: &std::path::Path,
) -> std::path::PathBuf {
    use std::path::{Component, Path, PathBuf};
    if !matches!(
        import_path.components().next(),
        Some(Component::CurDir | Component::ParentDir)
    ) {
        return import_path.to_path_buf();
    }
    let joined = current_dir.join(import_path);
    let root_comps = normalize_lexical_components(module_root_dir);
    let joined_comps = normalize_lexical_components(&joined);
    let common = root_comps
        .iter()
        .zip(&joined_comps)
        .take_while(|(a, b)| a == b)
        .count();
    if !root_comps[common..]
        .iter()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return joined;
    }
    let up_count = root_comps[common..].len();
    if let Some(tail_utf8) = joined_comps[common..]
        .iter()
        .map(|comp| comp.as_os_str().to_str())
        .collect::<Option<Vec<&str>>>()
    {
        let mut parts: Vec<&str> = Vec::with_capacity(up_count + tail_utf8.len());
        parts.extend(std::iter::repeat_n("..", up_count));
        parts.extend(tail_utf8);
        return if parts.is_empty() {
            PathBuf::from(".")
        } else if parts[0] == ".." {
            PathBuf::from(parts.join("/"))
        } else {
            PathBuf::from(format!("./{}", parts.join("/")))
        };
    }
    let mut out = PathBuf::new();
    for _ in 0..up_count {
        out.push("..");
    }
    for comp in &joined_comps[common..] {
        out.push(comp.as_os_str());
    }
    if matches!(out.components().next(), Some(Component::ParentDir)) {
        out
    } else {
        Path::new(".").join(out)
    }
}

impl<'a> ParsedZngFile<'a> {
    /// Parses the modules that the current file `import`s (from `first` on), to find their
    /// `#crate`, the `c++::` types they declare, and the C++ class templates they're the home of.
    fn resolve_module_imports(
        zngur: &mut ZngurSpecBuilder,
        first: usize,
        ctx: &mut ParseContext,
        resolver: &impl ImportResolver,
    ) {
        let Some(base_dir) = ctx
            .module_root_dir
            .clone()
            .or_else(|| ctx.path.parent().map(std::path::Path::to_path_buf))
        else {
            return;
        };
        for index in first..zngur.spec.imported_modules.len() {
            let import_path = zngur.spec.imported_modules[index].path.clone();
            let report_span = zngur.module_import_spans[index].clone();
            let path = base_dir.join(&import_path);
            let norm_path = normalize_lexical_path(&path);
            if ctx.active_module_stack.contains(&norm_path) {
                let msg = format!(
                    "cyclic module `import` involving `{}`",
                    import_path.display()
                );
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, report_span.clone())
                        .with_message(&msg)
                        .with_label(
                            Label::new(report_span)
                                .with_message(&msg)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
                continue;
            }
            let cached = if let Some(cached) = ctx.parsed_module_cache.get(&norm_path).cloned() {
                cached
            } else {
                let Ok(text) = resolver.resolve_import(&base_dir, &import_path) else {
                    // Not an error: an `import` only has to name the generated header (found via
                    // the C++ include path), so the `.zng` file may not be at this relative path.
                    // Its `c++::` types just can't be referred to from this module then.
                    continue;
                };
                let errors_before = ctx.reports().errors;
                ctx.active_module_stack.push(norm_path.clone());
                let (imported, mut own_cpp_templates, sub_resolved_paths) = {
                    let mut module_ctx = ctx.nested_module(&path, &text);
                    let mut module = ZngurSpecBuilder::default();
                    Self::parse_into(&mut module, &mut module_ctx, resolver);
                    let sub_resolved_paths = module.module_import_resolved_paths.clone();
                    // Templates of C++ class templates (`type<T> c++::...`) declared in the
                    // imported module also apply to this module's instantiations. (Templates of
                    // Rust types stay local to their module.)
                    let own_cpp_templates: Vec<TemplateDef> = module
                        .templates
                        .iter()
                        .filter(|t| matches!(t.ty.ty, RustType::Cpp(_)) && !t.imported)
                        .cloned()
                        .collect();
                    (
                        module.to_zngur(&mut module_ctx),
                        own_cpp_templates,
                        sub_resolved_paths,
                    )
                };
                ctx.active_module_stack.pop();
                let had_errors = ctx.reports().errors > errors_before;

                if let Some(crate_name) = &imported.crate_name {
                    let home_crate = crate_name.replace('-', "_");
                    for template in &mut own_cpp_templates {
                        rewrite_crate_paths_in_zngur_type(&mut template.ty, &home_crate);
                    }
                }

                let mut cpp_types: Vec<Vec<String>> = vec![];
                for ty in &imported.types {
                    if let RustType::Cpp(path) = &ty.ty
                        && path.generics.is_empty()
                        && !cpp_types.contains(&path.path)
                    {
                        cpp_types.push(path.path.clone());
                    }
                }
                let cpp_heap_allocated_types: Vec<Vec<String>> = imported
                    .types
                    .iter()
                    .filter(|ty| ty.cpp_heap_allocated.is_some())
                    .filter_map(|ty| match &ty.ty {
                        RustType::Cpp(path) if path.generics.is_empty() => Some(path.path.clone()),
                        _ => None,
                    })
                    .collect();
                let mut cpp_templates: Vec<Vec<String>> = vec![];
                for home in &imported.cpp_template_homes {
                    if !cpp_templates.contains(&home.path) {
                        cpp_templates.push(home.path.clone());
                    }
                }
                let mut cpp_template_instantiations: Vec<CppTypePath> = vec![];
                for ty in &imported.types {
                    if let RustType::Cpp(path) = &ty.ty
                        && !path.generics.is_empty()
                    {
                        if !cpp_templates.contains(&path.path) {
                            cpp_templates.push(path.path.clone());
                        }
                        if let RustType::Cpp(erased_path) = ty.ty.with_erased_lifetimes()
                            && !cpp_template_instantiations.contains(&erased_path)
                        {
                            cpp_template_instantiations.push(erased_path);
                        }
                    }
                }
                let mut indirect_cpp_template_homes: Vec<IndirectTemplateHome> = vec![];
                let mut indirect_cpp_template_instantiations: Vec<(
                    CppTypePath,
                    std::path::PathBuf,
                )> = vec![];
                for sub_norm in sub_resolved_paths.iter().flatten() {
                    let Some(sub_cached) = ctx.parsed_module_cache.get(sub_norm) else {
                        continue;
                    };
                    for inst in sub_cached
                        .cpp_template_instantiations
                        .iter()
                        .map(|inst| (inst.clone(), sub_norm.clone()))
                        .chain(
                            sub_cached
                                .indirect_cpp_template_instantiations
                                .iter()
                                .cloned(),
                        )
                    {
                        if !cpp_template_instantiations.contains(&inst.0)
                            && !indirect_cpp_template_instantiations
                                .iter()
                                .any(|(existing, _)| existing == &inst.0)
                        {
                            indirect_cpp_template_instantiations.push(inst);
                        }
                    }
                    for ind in sub_cached
                        .cpp_template_homes
                        .iter()
                        .map(|h| IndirectTemplateHome {
                            home: h.clone(),
                            home_module_path: sub_norm.clone(),
                            home_crate_name: sub_cached.crate_name.clone(),
                        })
                        .chain(sub_cached.indirect_cpp_template_homes.iter().cloned())
                    {
                        if !imported
                            .cpp_template_homes
                            .iter()
                            .any(|h| h.path == ind.home.path)
                            && !indirect_cpp_template_homes.iter().any(|existing| {
                                existing.home.path == ind.home.path
                                    && existing.home_module_path == ind.home_module_path
                            })
                        {
                            indirect_cpp_template_homes.push(ind);
                        }
                    }
                }
                let cpp_by_value_types: Vec<(RustType, String)> = imported
                    .types
                    .iter()
                    .filter(|ty| matches!(ty.ty, RustType::Cpp(_)))
                    .filter_map(|ty| {
                        ty.cpp_by_value_layout()?;
                        Some((ty.ty.clone(), ty.cpp_name()?))
                    })
                    .collect();

                let cached = CachedImportedModule {
                    crate_name: imported.crate_name,
                    cpp_types,
                    cpp_heap_allocated_types,
                    cpp_by_value_types,
                    cpp_template_homes: imported.cpp_template_homes,
                    indirect_cpp_template_homes,
                    cpp_templates,
                    cpp_template_instantiations,
                    indirect_cpp_template_instantiations,
                    own_cpp_templates,
                    had_errors,
                };
                ctx.parsed_module_cache
                    .insert(norm_path.clone(), cached.clone());
                cached
            };

            zngur.module_import_resolved_paths[index] = Some(norm_path.clone());

            if !cached.had_errors
                && (!cached.cpp_types.is_empty()
                    || !cached.cpp_template_homes.is_empty()
                    || !cached.cpp_templates.is_empty())
                && cached.crate_name.is_none()
            {
                let msg = format!(
                    "imported module `{}` declares `c++::` types, so it needs a `#crate <name>` \
                     directive for this module to find their Rust wrappers \
                     (at `::<name>::generated::...`)",
                    import_path.display()
                );
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, report_span.clone())
                        .with_message(&msg)
                        .with_label(
                            Label::new(report_span)
                                .with_message(&msg)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
            }
            if zngur.imported_template_modules.insert(norm_path) {
                zngur
                    .templates
                    .extend(cached.own_cpp_templates.into_iter().map(|t| TemplateDef {
                        imported: true,
                        ..t
                    }));
            }
            let module_import = &mut zngur.spec.imported_modules[index];
            module_import.crate_name = cached.crate_name;
            module_import.cpp_types = cached.cpp_types;
            module_import.cpp_template_instantiations = cached.cpp_template_instantiations;
            module_import.cpp_heap_allocated_types = cached.cpp_heap_allocated_types;
            module_import.cpp_by_value_types = cached.cpp_by_value_types;
            module_import.cpp_template_homes = cached.cpp_template_homes;
            module_import.cpp_templates = cached.cpp_templates;
        }
    }

    fn parse_into(
        zngur: &mut ZngurSpecBuilder,
        ctx: &mut ParseContext,
        resolver: &impl ImportResolver,
    ) {
        let (tokens, errs) = lexer().parse(ctx.source).into_output_errors();
        let Some(tokens) = tokens else {
            ctx.add_errors(errs.into_iter().map(|e| e.map_token(|c| c.to_string())));
            return;
        };
        let tokens: ParserInput<'_> = tokens.as_slice().map(
            (ctx.source.len()..ctx.source.len()).into(),
            Box::new(|(t, s)| (t, s)),
        );
        let (ast, errs) = file_parser()
            .map_with(|ast, extra| (ast, extra.span()))
            .parse_with_state(tokens, &mut extra::SimpleState(ZngParserState::default()))
            .into_output_errors();
        let Some(ast) = ast else {
            ctx.add_errors(errs.into_iter().map(|e| e.map_token(|c| c.to_string())));
            return;
        };

        let mut top_items = Vec::with_capacity(ast.0.0.len());
        for item in ast.0.0 {
            if let ParsedItem::Allow(lint, span) = item {
                if KNOWN_LINTS.contains(&lint) {
                    ctx.allowed_lints.insert(lint.to_owned());
                } else {
                    ctx.add_error_str(
                        &format!(
                            "unknown warning `{lint}` (known ones: {})",
                            KNOWN_LINTS.join(", ")
                        ),
                        span,
                    );
                }
            } else {
                top_items.push(item);
            }
        }
        let (aliases, items) = partition_parsed_items(
            top_items
                .into_iter()
                .map(|item| process_parsed_item(item, ctx)),
        );
        let errors_before = ctx.reports().errors;
        let first_new_module_import = zngur.spec.imported_modules.len();
        ProcessedZngFile::new(aliases, items).into_zngur_spec(zngur, ctx);
        if ctx.reports().errors > errors_before {
            return;
        }
        Self::resolve_module_imports(zngur, first_new_module_import, ctx, resolver);

        if let Some(dirname) = ctx.path.to_owned().parent() {
            for import in std::mem::take(&mut zngur.imports) {
                match resolver.resolve_import(dirname, &import.0) {
                    Ok(text) => {
                        let path = dirname.join(&import.0);
                        let mut nested_ctx = ctx.nested(&path, &text);
                        Self::parse_into(zngur, &mut nested_ctx, resolver);
                    }
                    Err(err) => {
                        let path = import.0.display();
                        let spans = ctx.fetch_spans_current(&import);
                        let (first, rest) = {
                            let mut iter = spans.into_iter();
                            let first = iter
                                .next()
                                .cloned()
                                .unwrap_or_else(|| ctx.report_span_for_range(0..0));
                            let rest: Vec<_> = iter.collect();
                            (first, rest)
                        };
                        let mut report = Report::build(ReportKind::Error, first)
                            .with_message(format!("Failed to process merge file `{path}`: {err}",));
                        let count = rest.len();
                        for (i, span) in rest.into_iter().enumerate() {
                            report = report.with_label(
                                ariadne::Label::new(span.clone())
                                    .with_message("Also merged here")
                                    .with_color(Color::Blue),
                            );
                            if i >= 2 {
                                report = report.with_note(format!(
                                    "{} additional locations omitted",
                                    count - i
                                ));
                                break;
                            }
                        }

                        ctx.add_fatal_report(report.finish());
                    }
                }
            }
        }
    }

    /// Parse a .zng file and return both the spec and list of all processed files.
    ///
    /// `warning_sink` is called once per deprecation or other non-fatal warning
    /// produced while parsing.
    pub fn parse(
        path: &std::path::Path,
        cfg: impl RustCfgProvider + 'static,
        report_sink: &mut impl ReportSink,
    ) -> ParseResult {
        let mut zngur = ZngurSpecBuilder::default();
        zngur.spec.rust_cfg.extend(cfg.get_cfg_pairs());
        zngur.spec.rust_cfg.sort();
        let text = std::fs::read_to_string(&path).unwrap();
        let cfg: &dyn RustCfgProvider = &cfg;
        let mut ctx = ParseContext::new(path, &text, cfg, report_sink);
        Self::parse_into(&mut zngur, &mut ctx, &DefaultImportResolver);
        let spec = zngur.to_zngur(&mut ctx);
        if ctx.reports().errors > 0 {
            // add report of cfg values used
            ctx.add_report(
                Report::build(
                    ReportKind::Custom("cfg values used", ariadne::Color::Green),
                    ctx.report_span_for_range(0..0),
                )
                .with_message(
                    cfg.get_cfg_pairs()
                        .into_iter()
                        .map(|(key, value)| match value {
                            Some(value) => format!("{key}=\"{value}\""),
                            None => key,
                        })
                        .join("\n")
                        .to_string(),
                )
                .finish(),
            );
        }
        ParseResult {
            spec,
            processed_files: ctx.get_processed_files(),
            errors: ctx.reports().errors,
            warnings: ctx.reports().warnings,
        }
    }

    /// parse a IDL source str with with given options.
    /// sinks errors and warnings instead of panicking
    pub fn parse_str_with_resolver(
        text: &str,
        path: &str,
        cfg: impl RustCfgProvider + 'static,
        resolver: &impl ImportResolver,
        report_sink: &mut impl ReportSink,
    ) -> ParseResult {
        let mut zngur = ZngurSpecBuilder::default();
        let path = std::path::PathBuf::from(path);
        let mut ctx = ParseContext::new(&path, text, &cfg, report_sink);
        Self::parse_into(&mut zngur, &mut ctx, resolver);
        let spec = zngur.to_zngur(&mut ctx);
        ParseResult {
            spec,
            processed_files: ctx.get_processed_files(),
            errors: ctx.reports().errors,
            warnings: ctx.reports().warnings,
        }
    }
}

pub(crate) enum ProcessedItemOrAlias<'a> {
    Ignore,
    Processed(ProcessedItem<'a>),
    Alias(ParsedAlias<'a>),
    ChildItems(Vec<ProcessedItemOrAlias<'a>>),
}

fn process_parsed_item<'a>(
    item: ParsedItem<'a>,
    ctx: &mut ParseContext,
) -> ProcessedItemOrAlias<'a> {
    use ProcessedItemOrAlias as Ret;
    match item {
        ParsedItem::Alias(alias) => Ret::Alias(alias),
        ParsedItem::ConvertPanicToException(span) => {
            Ret::Processed(ProcessedItem::ConvertPanicToException(span))
        }
        ParsedItem::CrateName(name, span) => Ret::Processed(ProcessedItem::CrateName(name, span)),
        ParsedItem::UnstableFeature(_) => {
            // ignore
            Ret::Ignore
        }
        ParsedItem::Allow(_, span) => {
            ctx.add_error_str(
                "`#allow(...)` applies to the whole file and can only appear at the top level",
                span,
            );
            Ret::Ignore
        }
        ParsedItem::CppAdditionalInclude(inc) => {
            Ret::Processed(ProcessedItem::CppAdditionalInclude(inc))
        }
        ParsedItem::Mod { path, items } => {
            let (aliases, items) = partition_parsed_items(
                items.into_iter().map(|item| process_parsed_item(item, ctx)),
            );
            Ret::Processed(ProcessedItem::Mod {
                path,
                items,
                aliases,
            })
        }
        ParsedItem::Type {
            ty,
            items,
            lifetimes,
            type_vars,
            unstable_template_types,
        } => Ret::Processed(ProcessedItem::Type {
            ty,
            items,
            lifetimes,
            type_vars,
            unstable_template_types,
        }),
        ParsedItem::Trait { tr, methods } => Ret::Processed(ProcessedItem::Trait { tr, methods }),
        ParsedItem::Fn(function) => Ret::Processed(ProcessedItem::Fn(function)),
        ParsedItem::ExternCpp(items) => Ret::Processed(ProcessedItem::ExternCpp(items)),
        ParsedItem::Import(path) => Ret::Processed(ProcessedItem::Import(path)),
        ParsedItem::ModuleImport { path, span } => {
            Ret::Processed(ProcessedItem::ModuleImport { path, span })
        }
        ParsedItem::MatchOnCfg(match_) => Ret::ChildItems(
            match_
                .eval(ctx)
                .unwrap_or_default() // unwrap or empty
                .into_iter()
                .map(|item| item.inner)
                .collect(),
        ),
    }
}

fn partition_parsed_items<'a>(
    items: impl IntoIterator<Item = ProcessedItemOrAlias<'a>>,
) -> (Vec<ParsedAlias<'a>>, Vec<ProcessedItem<'a>>) {
    let mut aliases = Vec::new();
    let mut processed = Vec::new();
    for item in items.into_iter() {
        match item {
            ProcessedItemOrAlias::Ignore => continue,
            ProcessedItemOrAlias::Processed(p) => processed.push(p),
            ProcessedItemOrAlias::Alias(a) => aliases.push(a),
            ProcessedItemOrAlias::ChildItems(children) => {
                let (child_aliases, child_items) = partition_parsed_items(children);
                aliases.extend(child_aliases);
                processed.extend(child_items);
            }
        }
    }
    (aliases, processed)
}

impl<'a> ProcessedZngFile<'a> {
    fn new(aliases: Vec<ParsedAlias<'a>>, items: Vec<ProcessedItem<'a>>) -> Self {
        ProcessedZngFile { aliases, items }
    }

    fn into_zngur_spec(self, zngur: &mut ZngurSpecBuilder, ctx: &mut ParseContext) {
        let root_scope = Scope::new_root(self.aliases, ctx);

        for item in self.items {
            item.add_to_zngur_spec(zngur, &root_scope, ctx);
        }
    }
}

#[derive(Clone)]
struct TemplateDef {
    ty: ZngurType,
    source_id: SourceId,
    span: Span,
    allowed_lints: HashSet<String>,
    /// Whether the template comes from an imported module (only templates of C++ class templates
    /// do), rather than from this one.
    imported: bool,
}

struct TemplateItemLintOrigin {
    template_span: ReportSpan,
    template_allowed: HashSet<String>,
    /// True when the member already ran C++ lints when the template was parsed (`false` for a
    /// static `fn` in a `type<T> Foo<T>` template without `c++::` that was converted to C++).
    linted_at_parse: bool,
    orig_had_elided_lifetimes: bool,
    orig_had_safe_raw_pointer: bool,
}

#[derive(Default)]
struct TemplateMemberLints {
    methods: HashMap<String, TemplateItemLintOrigin>,
    constructors: HashMap<String, TemplateItemLintOrigin>,
    fields: HashMap<String, TemplateItemLintOrigin>,
}

#[allow(clippy::too_many_arguments)]
fn report_template_member_lint(
    ctx: &mut ParseContext,
    origin: &TemplateItemLintOrigin,
    instantiation_span: &ReportSpan,
    instantiation_allowed: Option<&HashSet<String>>,
    lint: &'static str,
    orig_had: bool,
    sub_had: bool,
    warning: &str,
    reported_deferred: &mut HashSet<(ReportSpan, &'static str)>,
) {
    if !sub_had {
        return;
    }
    if !origin.linted_at_parse && orig_had {
        if !origin.template_allowed.contains(lint)
            && reported_deferred.insert((origin.template_span.clone(), lint))
        {
            ctx.add_report(
                Report::build(ReportKind::Warning, origin.template_span.clone())
                    .with_message(warning)
                    .with_label(
                        Label::new(origin.template_span.clone())
                            .with_message(warning)
                            .with_color(Color::Yellow),
                    )
                    .finish(),
            );
        }
    } else if !orig_had {
        let allowed_in_inst = instantiation_allowed.is_some_and(|s| s.contains(lint));
        if !origin.template_allowed.contains(lint) && !allowed_in_inst {
            let mut report = Report::build(ReportKind::Warning, instantiation_span.clone())
                .with_message(warning)
                .with_label(
                    Label::new(instantiation_span.clone())
                        .with_message(warning)
                        .with_color(Color::Yellow),
                );
            if *instantiation_span != origin.template_span {
                report = report.with_label(
                    Label::new(origin.template_span.clone())
                        .with_message("in template member declared here")
                        .with_color(Color::Blue),
                );
            }
            ctx.add_report(report.finish());
        }
    }
}

#[derive(Default)]
struct ZngurSpecBuilder {
    spec: ZngurSpec,
    templates: Vec<TemplateDef>,
    /// Lints silenced via `#allow(...)` in the file of each concrete `type` declaration.
    type_allowed_lints: HashMap<RustType, HashSet<String>>,
    imports: Vec<Import>,
    /// The span of each entry of `spec.imported_modules`, in the file that imports it.
    module_import_spans: Vec<ReportSpan>,
    /// Normalized path of each entry of `spec.imported_modules`, when resolved.
    module_import_resolved_paths: Vec<Option<std::path::PathBuf>>,
    /// Normalized paths of imported modules whose `type<T> c++::...` templates have already been
    /// added to `templates`.
    imported_template_modules: HashSet<std::path::PathBuf>,
}

impl ZngurSpecBuilder {
    fn to_zngur(self, ctx: &mut ParseContext) -> ZngurSpec {
        let ZngurSpecBuilder {
            mut spec,
            mut templates,
            type_allowed_lints,
            imports,
            module_import_spans,
            module_import_resolved_paths,
            imported_template_modules: _,
        } = self;
        let mut seen_module_imports: HashSet<std::path::PathBuf> = HashSet::new();
        let mut dedup_modules = Vec::with_capacity(spec.imported_modules.len());
        let mut dedup_spans = Vec::with_capacity(module_import_spans.len());
        let mut dedup_resolved = Vec::with_capacity(module_import_resolved_paths.len());
        for ((module, span), resolved) in spec
            .imported_modules
            .into_iter()
            .zip(module_import_spans)
            .zip(module_import_resolved_paths)
        {
            let key = resolved
                .clone()
                .unwrap_or_else(|| normalize_lexical_path(&module.path));
            if seen_module_imports.insert(key) {
                dedup_modules.push(module);
                dedup_spans.push(span);
                dedup_resolved.push(resolved);
            }
        }
        spec.imported_modules = dedup_modules;
        let module_import_spans = dedup_spans;
        let module_import_resolved_paths = dedup_resolved;
        for template in &mut templates {
            if !matches!(template.ty.ty, RustType::Cpp(_)) || template.imported {
                continue;
            }
            let template_span = (template.source_id, template.span.into_range());
            if template
                .ty
                .wellknown_traits
                .contains(&ZngurWellknownTrait::Unsized)
            {
                let unsized_spans = ctx.fetch_spans_global(&PartialSpanKey::wellknown_trait(
                    template.ty.ty.clone(),
                    ZngurWellknownTrait::Unsized,
                ));
                let report_span = unsized_spans
                    .last()
                    .copied()
                    .cloned()
                    .unwrap_or_else(|| template_span.clone());
                let msg = "`?Sized` is not supported on C++ types; a `c++::` type without a \
                           layout is already reference-only";
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, report_span.clone())
                        .with_message(msg)
                        .with_label(
                            Label::new(report_span)
                                .with_message(msg)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
                template
                    .ty
                    .wellknown_traits
                    .retain(|wkt| *wkt != ZngurWellknownTrait::Unsized);
            }
            if matches!(
                template.ty.layout,
                Some(LayoutPolicy::OnlyByRef | LayoutPolicy::HeapAllocated)
            ) {
                let layout_spans =
                    ctx.fetch_spans_global(&PartialSpanKey::layout(template.ty.ty.clone()));
                let report_span = layout_spans
                    .last()
                    .copied()
                    .cloned()
                    .unwrap_or(template_span);
                let msg = "`#heap_allocated` and `#only_by_ref` are for Rust types; a `c++::` \
                           type without a layout is already reference-only";
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, report_span.clone())
                        .with_message(msg)
                        .with_label(
                            Label::new(report_span)
                                .with_message(msg)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
                template.ty.layout = None;
            }
        }
        let mut all_template_locations = Vec::with_capacity(spec.types.len());
        let mut all_template_member_lints = Vec::with_capacity(spec.types.len());
        let mut matched_template_indices: HashSet<usize> = HashSet::new();
        let mut checked_template_pairs: HashSet<(usize, usize)> = HashSet::new();
        for ty in &mut spec.types {
            let concrete_had_layout = ty.layout.is_some();
            let mut template_locations = Vec::new();
            let mut matched_on_ty: Vec<usize> = Vec::new();
            let mut member_lints = TemplateMemberLints::default();
            for (t_idx, template) in templates.iter().enumerate() {
                if let Some(template_match) = try_match_template(ty, &template.ty) {
                    matched_template_indices.insert(t_idx);
                    if !concrete_had_layout {
                        for &prev_idx in &matched_on_ty {
                            checked_template_pairs.insert((prev_idx, t_idx));
                        }
                        matched_on_ty.push(t_idx);
                    }
                    let location = (template.source_id, template.span.into_range());
                    let tpl_ty = &template.ty.ty;
                    let method_origins: Vec<(String, TemplateItemLintOrigin)> = template
                        .ty
                        .cpp_methods
                        .iter()
                        .map(|orig_m| {
                            let orig_key = orig_m.into_span_key_with(tpl_ty.clone());
                            let span = ctx
                                .fetch_spans_global(&orig_key)
                                .first()
                                .copied()
                                .cloned()
                                .unwrap_or_else(|| location.clone());
                            let elided_recv =
                                orig_m.receiver_lifetime.as_ref().is_none_or(|l| l.0 == "_")
                                    && orig_m.raw_receiver.is_none()
                                    && orig_m.receiver != ZngurMethodReceiver::Static;
                            (
                                orig_m.rust_name().to_owned(),
                                TemplateItemLintOrigin {
                                    template_span: span,
                                    template_allowed: template.allowed_lints.clone(),
                                    linted_at_parse: true,
                                    orig_had_elided_lifetimes: elided_recv
                                        || orig_m
                                            .inputs
                                            .iter()
                                            .chain([&orig_m.output])
                                            .any(has_elided_ref),
                                    orig_had_safe_raw_pointer: orig_m.is_safe
                                        && (orig_m.raw_receiver.is_some()
                                            || orig_m.inputs.iter().any(contains_raw_ptr)),
                                },
                            )
                        })
                        .chain(
                            template
                                .ty
                                .methods
                                .iter()
                                .filter(|m| {
                                    matches!(ty.ty, RustType::Cpp(_))
                                        && m.data.is_safe.is_some()
                                        && m.data.receiver == ZngurMethodReceiver::Static
                                        && m.data.generics.is_empty()
                                        && m.use_path.is_none()
                                        && m.deref.is_none()
                                })
                                .zip(&template_match.0.cpp_methods[template.ty.cpp_methods.len()..])
                                .map(|(orig_m, sub_m)| {
                                    let orig_key = orig_m.data.into_span_key_with(tpl_ty.clone());
                                    let span = ctx
                                        .fetch_spans_global(&orig_key)
                                        .first()
                                        .copied()
                                        .cloned()
                                        .unwrap_or_else(|| location.clone());
                                    (
                                        sub_m.rust_name().to_owned(),
                                        TemplateItemLintOrigin {
                                            template_span: span,
                                            template_allowed: template.allowed_lints.clone(),
                                            linted_at_parse: template.imported,
                                            orig_had_elided_lifetimes: orig_m
                                                .data
                                                .inputs
                                                .iter()
                                                .chain([&orig_m.data.output])
                                                .any(has_elided_ref),
                                            orig_had_safe_raw_pointer: orig_m.data.is_safe
                                                == Some(true)
                                                && orig_m.data.inputs.iter().any(contains_raw_ptr),
                                        },
                                    )
                                }),
                        )
                        .collect();
                    let ctor_origins: Vec<(String, TemplateItemLintOrigin)> = template
                        .ty
                        .cpp_constructors
                        .iter()
                        .map(|orig_c| {
                            let orig_key = PartialSpanKey::cpp_constructor(
                                tpl_ty.clone(),
                                orig_c.rust_name.clone(),
                            );
                            let span = ctx
                                .fetch_spans_global(&orig_key)
                                .first()
                                .copied()
                                .cloned()
                                .unwrap_or_else(|| location.clone());
                            (
                                orig_c.rust_name.clone(),
                                TemplateItemLintOrigin {
                                    template_span: span,
                                    template_allowed: template.allowed_lints.clone(),
                                    linted_at_parse: true,
                                    orig_had_elided_lifetimes: orig_c
                                        .inputs
                                        .iter()
                                        .any(has_elided_ref),
                                    orig_had_safe_raw_pointer: orig_c.is_safe
                                        && orig_c.inputs.iter().any(contains_raw_ptr),
                                },
                            )
                        })
                        .collect();
                    let field_origins: Vec<(String, TemplateItemLintOrigin)> = template
                        .ty
                        .cpp_fields
                        .iter()
                        .map(|orig_f| {
                            let orig_key = orig_f.into_span_key_with(tpl_ty.clone());
                            let span = ctx
                                .fetch_spans_global(&orig_key)
                                .first()
                                .copied()
                                .cloned()
                                .unwrap_or_else(|| location.clone());
                            (
                                orig_f.rust_name().to_owned(),
                                TemplateItemLintOrigin {
                                    template_span: span,
                                    template_allowed: template.allowed_lints.clone(),
                                    linted_at_parse: true,
                                    orig_had_elided_lifetimes: false,
                                    orig_had_safe_raw_pointer: is_mutable_raw_ptr_field(orig_f),
                                },
                            )
                        })
                        .collect();
                    if let Err(e) = template_match.merge_into(ty, concrete_had_layout) {
                        match e {
                            MergeFailure::Conflict(msg, conflict) => {
                                let report = build_template_conflict_report(
                                    ctx,
                                    template,
                                    &ty,
                                    location.clone(),
                                    &msg,
                                    conflict,
                                );

                                ctx.add_fatal_report(report);
                            }
                        }
                    } else {
                        for (name, origin) in method_origins {
                            member_lints.methods.entry(name).or_insert(origin);
                        }
                        for (name, origin) in ctor_origins {
                            member_lints.constructors.entry(name).or_insert(origin);
                        }
                        for (name, origin) in field_origins {
                            member_lints.fields.entry(name).or_insert(origin);
                        }
                        template_locations.push(location);
                    }
                }
            }
            all_template_locations.push(template_locations);
            all_template_member_lints.push(member_lints);
        }
        // Validate mutual compatibility across overlapping local `type<...> c++::...` blocks even
        // when the home module declares no concrete instantiations (or only overrides layout).
        for (i, t1) in templates.iter().enumerate() {
            if !matches!(t1.ty.ty, RustType::Cpp(_)) || t1.imported {
                continue;
            }
            for (j, t2) in templates.iter().enumerate().skip(i + 1) {
                if !matches!(t2.ty.ty, RustType::Cpp(_))
                    || t2.imported
                    || checked_template_pairs.contains(&(i, j))
                {
                    continue;
                }
                let Some(unified_ty) = unify_cpp_template_types(&t1.ty.ty, &t2.ty.ty) else {
                    continue;
                };
                let mut synthetic_target = ZngurType {
                    ty: unified_ty,
                    lifetimes: vec![],
                    layout: None,
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    cpp_methods: vec![],
                    cpp_constructors: vec![],
                    cpp_fields: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                    cpp_heap_allocated: None,
                    cpp_ref: None,
                    cpp_stack_owned: None,
                };
                if let (Some(m1), Some(m2)) = (
                    try_match_template(&synthetic_target, &t1.ty),
                    try_match_template(&synthetic_target, &t2.ty),
                ) && m1.0.merge(&mut synthetic_target).is_ok()
                    && let Err(MergeFailure::Conflict(msg, conflict)) =
                        m2.0.merge(&mut synthetic_target)
                {
                    let report = build_template_pair_conflict_report(ctx, t1, t2, &msg, conflict);
                    ctx.add_fatal_report(report);
                }
            }
        }

        let cpp_heap_allocated: HashSet<Vec<String>> = spec
            .types
            .iter()
            .filter(|ty| ty.cpp_heap_allocated.is_some())
            .filter_map(|ty| match &ty.ty {
                RustType::Cpp(path) if path.generics.is_empty() => Some(path.path.clone()),
                _ => None,
            })
            .chain(
                spec.imported_modules
                    .iter()
                    .flat_map(|m| m.cpp_heap_allocated_types.iter().cloned()),
            )
            .collect();
        let local_cpp_paths: HashSet<Vec<String>> = spec
            .types
            .iter()
            .filter_map(|ty| match &ty.ty {
                RustType::Cpp(path) => Some(path.path.clone()),
                _ => None,
            })
            .collect();
        let known_local_cpp_types: HashSet<CppTypePath> = spec
            .types
            .iter()
            .filter_map(|ty| match ty.ty.with_erased_lifetimes() {
                RustType::Cpp(path) => Some(path),
                _ => None,
            })
            .collect();
        let had_incomplete_imports =
            !imports.is_empty() || (ctx.reports().errors > 0 && !spec.imported_modules.is_empty());
        let heap_info = CppHeapAllocatedInfo {
            heap_allocated: &cpp_heap_allocated,
            known_only_local: had_incomplete_imports.then_some(&local_cpp_paths),
        };
        let known_cpp_types: HashSet<CppTypePath> = known_local_cpp_types
            .iter()
            .cloned()
            .chain(spec.imported_modules.iter().flat_map(|m| {
                m.cpp_types
                    .iter()
                    .cloned()
                    .map(CppTypePath::from)
                    .chain(m.cpp_template_instantiations.iter().cloned())
            }))
            .collect();
        let known_cpp_paths: HashSet<Vec<String>> = local_cpp_paths
            .iter()
            .cloned()
            .chain(templates.iter().filter_map(|t| match &t.ty.ty {
                RustType::Cpp(path) if !t.imported => Some(path.path.clone()),
                _ => None,
            }))
            .chain(
                spec.imported_modules
                    .iter()
                    .flat_map(|m| m.cpp_types.iter().chain(&m.cpp_templates).cloned()),
            )
            .chain(
                module_import_resolved_paths
                    .iter()
                    .filter_map(|p| p.as_ref())
                    .filter_map(|norm_path| ctx.parsed_module_cache.get(norm_path))
                    .flat_map(|cached| {
                        cached
                            .indirect_cpp_template_homes
                            .iter()
                            .map(|ind| ind.home.path.clone())
                    }),
            )
            .collect();
        let by_value_cpp_types: HashSet<CppTypePath> = spec
            .types
            .iter()
            .filter_map(|ty| {
                ty.cpp_by_value_layout()?;
                match ty.ty.with_erased_lifetimes() {
                    RustType::Cpp(path) => Some(path),
                    _ => None,
                }
            })
            .chain(spec.imported_modules.iter().flat_map(|m| {
                m.cpp_by_value_types
                    .iter()
                    .filter_map(|(ty, _)| match ty.with_erased_lifetimes() {
                        RustType::Cpp(path) => Some(path),
                        _ => None,
                    })
            }))
            .collect();

        for check in std::mem::take(&mut *ctx.deferred_cpp_object_checks) {
            if let Err(message) = check.types.iter().try_for_each(|t| {
                check_cpp_objects_inner(
                    t,
                    check.allow_cpp_ref,
                    check.allow_cpp_object_ref,
                    Some(&heap_info),
                )
            }) {
                let report = Report::build(ReportKind::Error, check.report_span.clone())
                    .with_message(&message)
                    .with_label(
                        Label::new(check.report_span)
                            .with_message(&message)
                            .with_color(Color::Red),
                    )
                    .finish();
                ctx.add_fatal_report(report);
            }
        }

        let mut reported_deferred: HashSet<(ReportSpan, &'static str)> = HashSet::new();
        for ((ty, template_locations), template_member_lints) in spec
            .types
            .iter_mut()
            .zip(&all_template_locations)
            .zip(&all_template_member_lints)
        {
            let instantiation_allowed = type_allowed_lints.get(&ty.ty.with_erased_lifetimes());
            if !template_locations.is_empty() {
                let check_res = ty
                    .constructor
                    .iter()
                    .flat_map(|c| c.inputs.iter().map(|(_, t)| t))
                    .chain(
                        ty.methods
                            .iter()
                            .flat_map(|m| m.data.inputs.iter().chain([&m.data.output])),
                    )
                    .chain(
                        ty.cpp_methods
                            .iter()
                            .flat_map(|m| m.inputs.iter().chain([&m.output])),
                    )
                    .chain(ty.cpp_constructors.iter().flat_map(|c| c.inputs.iter()))
                    .try_for_each(|t| check_cpp_objects_inner(t, true, true, Some(&heap_info)))
                    .and_then(|()| {
                        [&ty.ty]
                            .into_iter()
                            .chain(ty.fields.iter().map(|f| &f.ty))
                            .chain(
                                ty.variants
                                    .iter()
                                    .flat_map(|v| v.fields.iter().map(|f| &f.ty)),
                            )
                            .chain(ty.methods.iter().flat_map(|m| {
                                m.data
                                    .generics
                                    .iter()
                                    .chain(m.deref.as_ref().map(|(d, _)| d))
                            }))
                            .try_for_each(|t| {
                                check_cpp_objects_inner(t, false, false, Some(&heap_info))
                            })
                    })
                    .and_then(|()| {
                        ty.cpp_fields
                            .iter()
                            .try_for_each(|f| check_cpp_field(f, Some(&heap_info)))
                    });
                if let Err(err) = check_res {
                    let report_span = ctx
                        .fetch_spans_global(ty)
                        .first()
                        .copied()
                        .cloned()
                        .or_else(|| template_locations.last().cloned())
                        .unwrap_or((0, 0usize..0));
                    let mut report = Report::build(ReportKind::Error, report_span.clone())
                        .with_message(&err)
                        .with_label(
                            Label::new(report_span)
                                .with_message(&err)
                                .with_color(Color::Red),
                        );
                    for location in template_locations {
                        report = report.with_label(
                            Label::new(location.clone())
                                .with_message("in template instantiated here")
                                .with_color(Color::Blue),
                        );
                    }
                    ctx.add_fatal_report(report.finish());
                }
                for field in &mut ty.cpp_fields {
                    if let RustType::CppConst(inner) = &field.ty {
                        field.ty = (**inner).clone();
                    }
                }
            }

            let field_spans = ty
                .fields
                .iter()
                .filter_map(|field| {
                    let span = ctx
                        .fetch_spans_global(&field.into_span_key_with_variant(ty.ty.clone(), None))
                        .into_iter()
                        .next()
                        .cloned()
                        .or_else(|| ctx.fetch_spans_global(ty).into_iter().next().cloned())?;
                    Some((cpp_handle_field_name(&field.name), span))
                })
                .collect::<HashMap<_, _>>();
            let mut reported_names = HashSet::new();
            for method in &ty.methods {
                let name =
                    cpp_handle_keyword(method.cpp_name.as_deref().unwrap_or(&method.data.name));
                let Some(field_span) = field_spans.get(&name) else {
                    continue;
                };
                if !reported_names.insert(name.clone()) {
                    continue;
                }
                let method_span = ctx
                    .fetch_spans_global(&method.data.into_span_key_with(ty.ty.clone()))
                    .into_iter()
                    .next()
                    .cloned()
                    .or_else(|| ctx.fetch_spans_global(ty).into_iter().next().cloned());
                let Some(method_span) = method_span else {
                    continue;
                };
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, method_span.clone())
                        .with_message(format!(
                            "C++ field and method both use the name `{name}` on `{}`.",
                            ty.ty
                        ))
                        .with_label(
                            Label::new((*field_span).clone())
                                .with_message("field is exposed with this C++ name")
                                .with_color(Color::Yellow),
                        )
                        .with_label(
                            Label::new(method_span)
                                .with_message("method is exposed with this C++ name")
                                .with_color(Color::Red),
                        )
                        .with_note(
                            "Rename the field or give the method a different C++ name with `as`.",
                        )
                        .finish(),
                );
            }

            if !ty.wellknown_traits.iter().any(|wkt| {
                matches!(
                    wkt,
                    ZngurWellknownTrait::Copy | ZngurWellknownTrait::Unsized
                )
            }) {
                ty.wellknown_traits.push(ZngurWellknownTrait::Drop);
            }
            let mut cpp_directives: Vec<(&str, PartialSpanKey)> = Vec::new();
            if ty.cpp_stack_owned.is_some() {
                cpp_directives.push((
                    "#cpp_stack_owned",
                    PartialSpanKey::cpp_stack_owned(ty.ty.clone()),
                ));
            }
            if ty.cpp_ref.is_some() {
                cpp_directives.push(("#cpp_ref", PartialSpanKey::cpp_ref(ty.ty.clone())));
            }
            if ty.cpp_heap_allocated.is_some() {
                cpp_directives.push((
                    "#cpp_heap_allocated",
                    PartialSpanKey::cpp_heap_allocated(ty.ty.clone()),
                ));
            }
            if cpp_directives.len() > 1 {
                let (first_name, ref first_key) = cpp_directives[0];
                let (second_name, ref second_key) = cpp_directives[1];
                let first_spans = ctx.fetch_spans_global(first_key);
                let second_spans = ctx.fetch_spans_global(second_key);
                let report_span = second_spans
                    .last()
                    .or_else(|| first_spans.last())
                    .copied()
                    .cloned()
                    .unwrap_or((0, 0usize..0));
                let mut report =
                    Report::build(ReportKind::Error, report_span).with_message(format!(
                        "`{first_name}` cannot be used with `{second_name}` for type {}.",
                        ty.ty
                    ));
                for (dir_name, dir_key) in &cpp_directives {
                    if let Some(span) = ctx.fetch_spans_global(dir_key).last() {
                        report = report.with_label(
                            Label::new((*span).clone())
                                .with_message(format!("`{dir_name}` declared here"))
                                .with_color(Color::Yellow),
                        );
                    }
                }
                ctx.add_fatal_report(report.finish());
                continue;
            }
            if ty.wellknown_traits.contains(&ZngurWellknownTrait::Unsized)
                && (matches!(ty.ty, RustType::Cpp(_)) || !cpp_directives.is_empty())
            {
                let unsized_spans = ctx.fetch_spans_global(&PartialSpanKey::wellknown_trait(
                    ty.ty.clone(),
                    ZngurWellknownTrait::Unsized,
                ));
                let report_span = unsized_spans
                    .last()
                    .copied()
                    .cloned()
                    .or_else(|| template_locations.last().cloned())
                    .or_else(|| ctx.fetch_spans_global(ty).first().copied().cloned())
                    .unwrap_or((0, 0usize..0));
                let msg = "`?Sized` is not supported on C++ types; a `c++::` type without a \
                           layout is already reference-only";
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, report_span.clone())
                        .with_message(msg)
                        .with_label(
                            Label::new(report_span)
                                .with_message(msg)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
                continue;
            }
            let needs_layout = !matches!(ty.ty, RustType::Cpp(_))
                || ty.cpp_stack_owned.is_some()
                || ty.cpp_heap_allocated.is_some();
            if ty.layout.is_none() && needs_layout {
                let spans = ctx.fetch_spans_global(ty);
                let (first, rest) = {
                    let mut it = spans.into_iter();
                    let first = it.next().cloned();
                    (first, it.collect::<Vec<_>>())
                };
                let mut report = Report::build(ReportKind::Error, (0, 0usize..0))
                    .with_message(format!("No layout policy found for type {}.", ty.ty));
                if ty.cpp_stack_owned.is_some() {
                    report = report.with_note(
                        "Type is declared with `#cpp_stack_owned` which requires a layout directive like `#layout(size = X, align = Y)` or `#layout_conservative(size = X, align = Y)`.",
                    );
                } else if ty.cpp_heap_allocated.is_some() && matches!(ty.ty, RustType::Cpp(_)) {
                    report = report.with_note(
                        "Type is declared with `#cpp_heap_allocated` which requires `#layout(size = 16, align = 8)` or `#heap_allocated`.",
                    );
                } else {
                    report = report.with_note(
                        "Use one of `#layout(size = X, align = Y)`, `#heap_allocated` or `#only_by_ref`.",
                    );
                }

                if let Some(first) = first {
                    report.add_label(
                        Label::new(first)
                            .with_message("Type first declared here.")
                            .with_color(Color::Blue),
                    );
                }
                let count = rest.len();
                for (i, span) in rest.into_iter().enumerate() {
                    report = report.with_label(
                        Label::new(span.clone())
                            .with_message("Type also declared here")
                            .with_color(Color::Blue),
                    );
                    if i >= 2 {
                        report = report.with_note(format!(
                            "{} additional type declaration locations omitted",
                            count - i
                        ));
                        break;
                    }
                }
                let t_count = template_locations.len();
                for (i, location) in template_locations.iter().cloned().enumerate() {
                    report = report.with_label(
                        Label::new(location)
                            .with_message("Matching template defined here")
                            .with_color(Color::Blue),
                    );
                    if i >= 2 {
                        report = report.with_note(format!(
                            "{} additional template declaration locations omitted",
                            t_count - i
                        ));
                        break;
                    }
                }
                ctx.add_fatal_report(report.finish());
            }
            if ty.cpp_stack_owned.is_some()
                && matches!(
                    ty.layout,
                    Some(LayoutPolicy::HeapAllocated | LayoutPolicy::OnlyByRef)
                )
            {
                let stack_owned_spans =
                    ctx.fetch_spans_global(&PartialSpanKey::cpp_stack_owned(ty.ty.clone()));
                let layout_spans = ctx.fetch_spans_global(&PartialSpanKey::layout(ty.ty.clone()));
                let report_span = stack_owned_spans
                    .last()
                    .copied()
                    .cloned()
                    .or_else(|| layout_spans.last().copied().cloned())
                    .unwrap_or((0, 0usize..0));

                let layout_name = match ty.layout {
                    Some(LayoutPolicy::HeapAllocated) => "#heap_allocated",
                    Some(LayoutPolicy::OnlyByRef) => "#only_by_ref",
                    _ => unreachable!(),
                };

                let mut report = Report::build(ReportKind::Error, report_span)
                    .with_message(format!(
                        "`#cpp_stack_owned` cannot be used with `{layout_name}` for type {}.",
                        ty.ty
                    ))
                    .with_note(
                        "`#cpp_stack_owned` types must use `#layout` or `#layout_conservative`.",
                    );

                if let Some(span) = stack_owned_spans.last() {
                    report = report.with_label(
                        Label::new((*span).clone())
                            .with_message("`#cpp_stack_owned` declared here")
                            .with_color(Color::Yellow),
                    );
                }
                if let Some(span) = layout_spans.last() {
                    report = report.with_label(
                        Label::new((*span).clone())
                            .with_message(format!("`{layout_name}` declared here"))
                            .with_color(Color::Yellow),
                    );
                }

                ctx.add_fatal_report(report.finish());
            } else if matches!(ty.ty, RustType::Cpp(_))
                && (matches!(ty.layout, Some(LayoutPolicy::OnlyByRef))
                    || (matches!(ty.layout, Some(LayoutPolicy::HeapAllocated))
                        && ty.cpp_heap_allocated.is_none()))
            {
                let layout_spans = ctx.fetch_spans_global(&PartialSpanKey::layout(ty.ty.clone()));
                let report_span = layout_spans
                    .last()
                    .copied()
                    .cloned()
                    .or_else(|| template_locations.last().cloned())
                    .or_else(|| ctx.fetch_spans_global(ty).first().copied().cloned())
                    .unwrap_or((0, 0usize..0));
                let msg = "`#heap_allocated` and `#only_by_ref` are for Rust types; a `c++::` \
                           type without a layout is already reference-only";
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, report_span.clone())
                        .with_message(msg)
                        .with_label(
                            Label::new(report_span)
                                .with_message(msg)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
            }
            if let Some(directive) = ty.trivially_relocatable.directive() {
                let tr_spans = ctx.fetch_spans_global(&PartialSpanKey::trivially_relocatable(
                    ty.ty.clone(),
                    ty.trivially_relocatable,
                ));
                let tr_span = tr_spans
                    .last()
                    .copied()
                    .cloned()
                    .or_else(|| template_locations.last().cloned())
                    .or_else(|| ctx.fetch_spans_global(ty).first().copied().cloned())
                    .unwrap_or((0, 0usize..0));
                if !matches!(ty.ty, RustType::Cpp(_)) {
                    let msg = format!(
                        "`{directive}` is only for `c++::` types (Rust types are always trivially \
                         relocatable)"
                    );
                    ctx.add_fatal_report(
                        Report::build(ReportKind::Error, tr_span.clone())
                            .with_message(&msg)
                            .with_label(
                                Label::new(tr_span)
                                    .with_message(&msg)
                                    .with_color(Color::Red),
                            )
                            .finish(),
                    );
                } else if !cpp_directives.is_empty() {
                    let msg = format!(
                        "`{directive}` cannot be combined with `#cpp_stack_owned`, `#cpp_ref`, or \
                         `#cpp_heap_allocated`"
                    );
                    let mut report = Report::build(ReportKind::Error, tr_span.clone())
                        .with_message(&msg)
                        .with_label(
                            Label::new(tr_span)
                                .with_message(format!("`{directive}` declared here"))
                                .with_color(Color::Red),
                        );
                    for (dir_name, dir_key) in &cpp_directives {
                        if let Some(span) = ctx.fetch_spans_global(dir_key).last() {
                            report = report.with_label(
                                Label::new((*span).clone())
                                    .with_message(format!("`{dir_name}` declared here"))
                                    .with_color(Color::Yellow),
                            );
                        }
                    }
                    ctx.add_fatal_report(report.finish());
                } else if ty.cpp_by_value_layout().is_none() {
                    let msg = format!(
                        "`{directive}` needs a `#layout(...)` or `#layout_conservative(...)`, \
                         which Rust needs to hold the C++ type by value"
                    );
                    ctx.add_fatal_report(
                        Report::build(ReportKind::Error, tr_span.clone())
                            .with_message(&msg)
                            .with_label(
                                Label::new(tr_span)
                                    .with_message(&msg)
                                    .with_color(Color::Red),
                            )
                            .finish(),
                    );
                }
            }
            if matches!(ty.ty, RustType::Cpp(_))
                && let Some(ctor) = &ty.constructor
            {
                let c_key = ctor.clone().into_span_key_with(ty.ty.clone());
                let c_span = ctx
                    .fetch_spans_global(&c_key)
                    .first()
                    .copied()
                    .cloned()
                    .or_else(|| template_locations.last().cloned())
                    .or_else(|| ctx.fetch_spans_global(ty).first().copied().cloned())
                    .unwrap_or((0, 0usize..0));
                let msg = "C++ constructors start with `safe` or `unsafe` and need a Rust name \
                           with `as <name>` (e.g. `safe constructor(...) as new;`)";
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, c_span.clone())
                        .with_message(msg)
                        .with_label(Label::new(c_span).with_message(msg).with_color(Color::Red))
                        .finish(),
                );
            }
            if matches!(ty.ty, RustType::Cpp(_)) && !ty.methods.is_empty() {
                for rust_method in &ty.methods {
                    let m_key = rust_method.data.clone().into_span_key_with(ty.ty.clone());
                    let m_span = ctx
                        .fetch_spans_global(&m_key)
                        .first()
                        .copied()
                        .cloned()
                        .or_else(|| template_locations.last().cloned())
                        .or_else(|| ctx.fetch_spans_global(ty).first().copied().cloned())
                        .unwrap_or((0, 0usize..0));
                    let msg = if rust_method.data.is_safe.is_none() {
                        "C++ member functions must be marked `safe` or `unsafe`"
                    } else if rust_method.use_path.is_some() || rust_method.deref.is_some() {
                        "C++ member functions do not support `use` or `deref` (those are for \
                         Rust methods; implement Rust traits on `c++::` types via \
                         `extern \"C++\" { impl Trait for c++::T { ... } }`)"
                    } else if !rust_method.data.generics.is_empty() {
                        "C++ member functions do not support generic type arguments"
                    } else {
                        "the receiver of a C++ member function is a reference to a C++ object, \
                         which says whether it's `const`: `&'a const self` or `&'a unique var self`"
                    };
                    ctx.add_fatal_report(
                        Report::build(ReportKind::Error, m_span.clone())
                            .with_message(msg)
                            .with_label(Label::new(m_span).with_message(msg).with_color(Color::Red))
                            .finish(),
                    );
                }
            }
            let fallback_span = template_locations
                .last()
                .cloned()
                .or_else(|| ctx.fetch_spans_global(ty).first().copied().cloned());
            let mut check_methods = true;
            if let Some(first_method) = ty.cpp_methods.first() {
                let method_key = first_method.into_span_key_with(ty.ty.clone());
                let method_span = || {
                    ctx.fetch_spans_global(&method_key)
                        .first()
                        .copied()
                        .cloned()
                        .or_else(|| fallback_span.clone())
                        .unwrap_or((0, 0usize..0))
                };
                // Checked after merging, since C++ member functions may be declared in a different
                // file than the type's layout or `#cpp_heap_allocated` directive, or via a
                // `#unstable(template_types)` template.
                if !matches!(ty.ty, RustType::Cpp(_)) {
                    let span = method_span();
                    let msg = "C++ member functions (`safe fn` / `unsafe fn`) can only be \
                               declared on `c++::` types";
                    ctx.add_fatal_report(
                        Report::build(ReportKind::Error, span.clone())
                            .with_message(msg)
                            .with_label(Label::new(span).with_message(msg).with_color(Color::Red))
                            .finish(),
                    );
                    check_methods = false;
                } else if ty.cpp_heap_allocated.is_some() {
                    let span = method_span();
                    let msg = "C++ member functions (`safe fn` / `unsafe fn`) cannot be declared \
                               on `#cpp_heap_allocated` types";
                    ctx.add_fatal_report(
                        Report::build(ReportKind::Error, span.clone())
                            .with_message(msg)
                            .with_label(Label::new(span).with_message(msg).with_color(Color::Red))
                            .finish(),
                    );
                    check_methods = false;
                }
            }
            let mut check_constructors = true;
            if let Some(first_ctor) = ty.cpp_constructors.first()
                && (!matches!(ty.ty, RustType::Cpp(_))
                    || !ty.trivially_relocatable.is_relocatable()
                    || ty.cpp_by_value_layout().is_none())
            {
                let ctor_key =
                    PartialSpanKey::cpp_constructor(ty.ty.clone(), first_ctor.rust_name.clone());
                let span = ctx
                    .fetch_spans_global(&ctor_key)
                    .first()
                    .copied()
                    .cloned()
                    .or_else(|| fallback_span.clone())
                    .unwrap_or((0, 0usize..0));
                let msg = "C++ constructors can only be declared on `c++::` types that Rust \
                           holds by value (with a layout and `#trivially_relocatable`)";
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, span.clone())
                        .with_message(msg)
                        .with_label(Label::new(span).with_message(msg).with_color(Color::Red))
                        .finish(),
                );
                check_constructors = false;
            }
            if let Some(first_rust_field) = ty.fields.first()
                && matches!(ty.ty, RustType::Cpp(_))
            {
                let field_key = first_rust_field.into_span_key_with_variant(ty.ty.clone(), None);
                let span = ctx
                    .fetch_spans_global(&field_key)
                    .first()
                    .copied()
                    .cloned()
                    .or_else(|| fallback_span.clone())
                    .unwrap_or((0, 0usize..0));
                let name = &first_rust_field.name;
                let msg = format!(
                    "C++ field accessors omit `offset` and say what they return (e.g. \
                     `field &const {name} (type = ...);` or `field var {name} (type = ...);`)"
                );
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, span.clone())
                        .with_message(&msg)
                        .with_label(Label::new(span).with_message(&msg).with_color(Color::Red))
                        .finish(),
                );
            }
            let mut check_fields = true;
            if let Some(first_field) = ty.cpp_fields.first() {
                let field_key = first_field.into_span_key_with(ty.ty.clone());
                let field_span = || {
                    ctx.fetch_spans_global(&field_key)
                        .first()
                        .copied()
                        .cloned()
                        .or_else(|| fallback_span.clone())
                        .unwrap_or((0, 0usize..0))
                };
                if !matches!(ty.ty, RustType::Cpp(_)) {
                    let span = field_span();
                    let msg = "C++ field accessors (`field name (type = T)`) can only be declared \
                               on `c++::` types";
                    ctx.add_fatal_report(
                        Report::build(ReportKind::Error, span.clone())
                            .with_message(msg)
                            .with_label(Label::new(span).with_message(msg).with_color(Color::Red))
                            .finish(),
                    );
                    check_fields = false;
                } else if ty.cpp_heap_allocated.is_some() {
                    let span = field_span();
                    let msg = "C++ field accessors (`field name (type = T)`) cannot be declared \
                               on `#cpp_heap_allocated` types";
                    ctx.add_fatal_report(
                        Report::build(ReportKind::Error, span.clone())
                            .with_message(msg)
                            .with_label(Label::new(span).with_message(msg).with_color(Color::Red))
                            .finish(),
                    );
                    check_fields = false;
                }
            }
            check_cpp_members(
                ty,
                fallback_span,
                check_methods,
                check_constructors,
                check_fields,
                &CppMemberCheckEnv {
                    known_cpp_types: &known_cpp_types,
                    known_cpp_paths: &known_cpp_paths,
                    by_value_cpp_types: &by_value_cpp_types,
                    check_undeclared_templates: !had_incomplete_imports,
                    heap_info: template_locations.is_empty().then_some(&heap_info),
                    template_member_lints: Some(template_member_lints),
                    instantiation_allowed,
                    from_matched_template: !template_locations.is_empty(),
                },
                &mut reported_deferred,
                ctx,
            );
        }
        for func in &spec.cpp_funcs {
            let f_span = ctx
                .fetch_spans_global(func)
                .first()
                .copied()
                .cloned()
                .unwrap_or((0, 0usize..0));
            if let Some(len) = (1..=func.namespace.len())
                .find(|&len| known_cpp_paths.contains(&func.namespace[..len]))
            {
                let prefix = func.namespace[..len].join("::");
                let msg = if len == func.namespace.len() {
                    format!(
                        "`c++::{prefix}` is a C++ type, not a namespace \
                         (declare static member functions inside `type c++::{prefix} {{ ... }}`)"
                    )
                } else {
                    format!("`c++::{prefix}` is a C++ type, not a namespace")
                };
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, f_span.clone())
                        .with_message(&msg)
                        .with_label(Label::new(f_span).with_message(&msg).with_color(Color::Red))
                        .finish(),
                );
                continue;
            }
            if let Err(msg) = func.inputs.iter().chain([&func.output]).try_for_each(|t| {
                check_cpp_signature_type(
                    t,
                    false,
                    &known_cpp_types,
                    &known_cpp_paths,
                    &by_value_cpp_types,
                    !had_incomplete_imports,
                )
            }) {
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, f_span.clone())
                        .with_message(&msg)
                        .with_label(Label::new(f_span).with_message(&msg).with_color(Color::Red))
                        .finish(),
                );
            }
        }
        // Check that imported `c++::` types and templates do not collide with local plain
        // `c++::` declarations or with incompatible `c++::` imports from other crates.
        let mut seen_reexports: HashSet<(String, Vec<String>, bool)> = HashSet::new();
        let mut imported_owners: std::collections::HashMap<
            Vec<String>,
            (String, bool, ReportSpan),
        > = std::collections::HashMap::new();
        for (module, import_span) in spec.imported_modules.iter().zip(&module_import_spans) {
            let Some(crate_name) = &module.crate_name else {
                continue;
            };
            let norm_crate = crate_name.replace('-', "_");
            if let Some(own_crate) = &spec.crate_name
                && own_crate.replace('-', "_") == norm_crate
            {
                let msg = format!(
                    "imported module `{}` has the same `#crate` (`{crate_name}`) as this module; \
                     use `merge \"{}\";` to split `.zng` files within a single crate",
                    module.path.display(),
                    module.path.display(),
                );
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, import_span.clone())
                        .with_message(&msg)
                        .with_label(
                            Label::new(import_span.clone())
                                .with_message(&msg)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
                continue;
            }
            for (segs, is_plain) in module
                .cpp_types
                .iter()
                .map(|s| (s, true))
                .chain(module.cpp_templates.iter().map(|s| (s, false)))
            {
                if !seen_reexports.insert((norm_crate.clone(), segs.clone(), is_plain)) {
                    continue;
                }
                let local_ty = RustType::Cpp(segs.clone().into());
                if spec.types.iter().any(|t| t.ty == local_ty) {
                    let local_span = ctx
                        .fetch_spans_global(&local_ty)
                        .first()
                        .copied()
                        .cloned()
                        .unwrap_or_else(|| import_span.clone());
                    let msg = format!(
                        "`c++::{}` is already imported from `{}` (`#crate {crate_name}`), and \
                         cannot also be declared in this module",
                        segs.join("::"),
                        module.path.display(),
                    );
                    let report = Report::build(ReportKind::Error, local_span.clone())
                        .with_message(&msg)
                        .with_label(
                            Label::new(local_span)
                                .with_message("declared locally here")
                                .with_color(Color::Red),
                        )
                        .with_label(
                            Label::new(import_span.clone())
                                .with_message(format!(
                                    "imported from `{}` (`#crate {crate_name}`) here",
                                    module.path.display()
                                ))
                                .with_color(Color::Yellow),
                        )
                        .finish();
                    ctx.add_fatal_report(report);
                } else if let Some((prev_crate, prev_is_plain, prev_span)) =
                    imported_owners.get(segs)
                {
                    if *prev_is_plain != is_plain {
                        let (plain_crate, template_crate) = if *prev_is_plain {
                            (prev_crate.as_str(), norm_crate.as_str())
                        } else {
                            (norm_crate.as_str(), prev_crate.as_str())
                        };
                        let msg = format!(
                            "`c++::{}` is imported both as a plain C++ type (`{plain_crate}`) \
                             and as a C++ class template (`{template_crate}`)",
                            segs.join("::")
                        );
                        let report = Report::build(ReportKind::Error, import_span.clone())
                            .with_message(&msg)
                            .with_label(
                                Label::new(import_span.clone())
                                    .with_message(format!("also imported from `{norm_crate}` here"))
                                    .with_color(Color::Red),
                            )
                            .with_label(
                                Label::new(prev_span.clone())
                                    .with_message(format!(
                                        "first imported from `{prev_crate}` here"
                                    ))
                                    .with_color(Color::Yellow),
                            )
                            .finish();
                        ctx.add_fatal_report(report);
                    } else if is_plain && prev_crate != &norm_crate {
                        let msg = format!(
                            "`c++::{}` is imported from multiple crates (`{prev_crate}` and \
                             `{norm_crate}`)",
                            segs.join("::")
                        );
                        let report = Report::build(ReportKind::Error, import_span.clone())
                            .with_message(&msg)
                            .with_label(
                                Label::new(import_span.clone())
                                    .with_message(format!("also imported from `{norm_crate}` here"))
                                    .with_color(Color::Red),
                            )
                            .with_label(
                                Label::new(prev_span.clone())
                                    .with_message(format!(
                                        "first imported from `{prev_crate}` here"
                                    ))
                                    .with_color(Color::Yellow),
                            )
                            .finish();
                        ctx.add_fatal_report(report);
                    }
                } else {
                    imported_owners.insert(
                        segs.clone(),
                        (norm_crate.clone(), is_plain, import_span.clone()),
                    );
                }
            }
        }
        for (t_idx, template) in templates.iter().enumerate() {
            if !matches!(template.ty.ty, RustType::Cpp(_))
                || template.imported
                || matched_template_indices.contains(&t_idx)
            {
                continue;
            }
            let ty = &template.ty;
            let template_span = (template.source_id, template.span.into_range());
            let check_res = ty
                .methods
                .iter()
                .flat_map(|m| m.data.inputs.iter().chain([&m.data.output]))
                .chain(
                    ty.cpp_methods
                        .iter()
                        .flat_map(|m| m.inputs.iter().chain([&m.output])),
                )
                .chain(ty.cpp_constructors.iter().flat_map(|c| c.inputs.iter()))
                .try_for_each(|t| check_cpp_objects_inner(t, true, true, Some(&heap_info)))
                .and_then(|()| {
                    ty.methods
                        .iter()
                        .flat_map(|m| {
                            m.data
                                .generics
                                .iter()
                                .chain(m.deref.as_ref().map(|(d, _)| d))
                        })
                        .try_for_each(|t| {
                            check_cpp_objects_inner(t, false, false, Some(&heap_info))
                        })
                })
                .and_then(|()| {
                    ty.cpp_fields
                        .iter()
                        .try_for_each(|f| check_cpp_field(f, Some(&heap_info)))
                });
            if let Err(err) = check_res {
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, template_span.clone())
                        .with_message(&err)
                        .with_label(
                            Label::new(template_span.clone())
                                .with_message(&err)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
            }
            check_cpp_members(
                ty,
                Some(template_span),
                true,
                true,
                true,
                &CppMemberCheckEnv {
                    known_cpp_types: &known_cpp_types,
                    known_cpp_paths: &known_cpp_paths,
                    by_value_cpp_types: &by_value_cpp_types,
                    check_undeclared_templates: !had_incomplete_imports,
                    heap_info: None,
                    template_member_lints: None,
                    instantiation_allowed: None,
                    from_matched_template: false,
                },
                &mut reported_deferred,
                ctx,
            );
        }
        let local_cpp_templates: Vec<&TemplateDef> = templates
            .iter()
            .filter(|t| matches!(t.ty.ty, RustType::Cpp(_)) && !t.imported)
            .collect();
        let unmatched_local_cpp_templates: Vec<&TemplateDef> = templates
            .iter()
            .enumerate()
            .filter(|(idx, t)| {
                matches!(t.ty.ty, RustType::Cpp(_))
                    && !t.imported
                    && !matched_template_indices.contains(idx)
            })
            .map(|(_, t)| t)
            .collect();
        resolve_cpp_templates(
            &mut spec,
            &local_cpp_templates,
            &module_import_spans,
            &module_import_resolved_paths,
            ctx,
        );
        check_cpp_template_uses(
            &spec,
            &unmatched_local_cpp_templates,
            &all_template_locations,
            &module_import_resolved_paths,
            &known_cpp_types,
            &known_cpp_paths,
            &by_value_cpp_types,
            !had_incomplete_imports,
            ctx,
        );
        spec.cpp_templates = templates
            .into_iter()
            .filter(|t| matches!(t.ty.ty, RustType::Cpp(_)) && !t.imported)
            .map(|t| t.ty)
            .collect();
        spec
    }
}

struct CppMemberCheckEnv<'a> {
    known_cpp_types: &'a HashSet<CppTypePath>,
    known_cpp_paths: &'a HashSet<Vec<String>>,
    by_value_cpp_types: &'a HashSet<CppTypePath>,
    check_undeclared_templates: bool,
    heap_info: Option<&'a CppHeapAllocatedInfo<'a>>,
    template_member_lints: Option<&'a TemplateMemberLints>,
    instantiation_allowed: Option<&'a HashSet<String>>,
    from_matched_template: bool,
}

/// Validates C++ member signatures (`cpp_methods`, `cpp_constructors`, `cpp_fields`), copy-field
/// by-value requirements, and cross-kind member name collisions on `ty`.
#[allow(clippy::too_many_arguments)]
fn check_cpp_members(
    ty: &ZngurType,
    fallback_span: Option<ReportSpan>,
    check_methods: bool,
    check_constructors: bool,
    check_fields: bool,
    env: &CppMemberCheckEnv<'_>,
    reported_deferred: &mut HashSet<(ReportSpan, &'static str)>,
    ctx: &mut ParseContext,
) {
    let default_span = || fallback_span.clone().unwrap_or((0, 0usize..0));
    if check_methods {
        for method in &ty.cpp_methods {
            let m_key = method.into_span_key_with(ty.ty.clone());
            let direct_spans = ctx.fetch_spans_global(&m_key);
            let is_from_template = direct_spans.is_empty() && env.from_matched_template;
            let m_span = direct_spans
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(&default_span);
            if is_from_template {
                if is_rust_keyword(method.rust_name()) {
                    let msg = rust_keyword_fn_error(
                        "C++ function",
                        method.rust_name(),
                        method.rust_name.is_some(),
                    );
                    ctx.add_fatal_report(
                        Report::build(ReportKind::Error, m_span.clone())
                            .with_message(&msg)
                            .with_label(
                                Label::new(m_span.clone())
                                    .with_message(&msg)
                                    .with_color(Color::Red),
                            )
                            .finish(),
                    );
                } else if matches!(
                    method.rust_name(),
                    "as_const" | "as_const_mut" | "into_const"
                ) {
                    let hint = if method.rust_name.is_some() {
                        "choose a different name in `as <rust_name>`"
                    } else {
                        "rename it with `as <rust_name>`"
                    };
                    let msg = format!(
                        "`{}` collides with an inherent conversion method on `c++::` wrappers; \
                         {hint}",
                        method.rust_name()
                    );
                    ctx.add_fatal_report(
                        Report::build(ReportKind::Error, m_span.clone())
                            .with_message(&msg)
                            .with_label(
                                Label::new(m_span.clone())
                                    .with_message(&msg)
                                    .with_color(Color::Red),
                            )
                            .finish(),
                    );
                }
                if let Some(origin) = env
                    .template_member_lints
                    .and_then(|l| l.methods.get(method.rust_name()))
                {
                    let inst_span = ctx
                        .fetch_spans_global(ty)
                        .first()
                        .copied()
                        .cloned()
                        .unwrap_or_else(|| origin.template_span.clone());
                    let sub_elided_recv =
                        method.receiver_lifetime.as_ref().is_none_or(|l| l.0 == "_")
                            && method.raw_receiver.is_none()
                            && method.receiver != ZngurMethodReceiver::Static;
                    let sub_elided = sub_elided_recv
                        || method
                            .inputs
                            .iter()
                            .chain([&method.output])
                            .any(has_elided_ref);
                    let sub_raw_ptr = method.is_safe
                        && (method.raw_receiver.is_some()
                            || method.inputs.iter().any(contains_raw_ptr));
                    report_template_member_lint(
                        ctx,
                        origin,
                        &inst_span,
                        env.instantiation_allowed,
                        "elided_lifetimes",
                        origin.orig_had_elided_lifetimes,
                        sub_elided,
                        &elided_lifetimes_warning("member function"),
                        reported_deferred,
                    );
                    report_template_member_lint(
                        ctx,
                        origin,
                        &inst_span,
                        env.instantiation_allowed,
                        "safe_raw_pointer",
                        origin.orig_had_safe_raw_pointer,
                        sub_raw_ptr,
                        &safe_raw_pointer_warning("member function"),
                        reported_deferred,
                    );
                }
            }
            if let Err(msg) = method
                .inputs
                .iter()
                .chain([&method.output])
                .try_for_each(|t| {
                    check_cpp_signature_type(
                        t,
                        false,
                        env.known_cpp_types,
                        env.known_cpp_paths,
                        env.by_value_cpp_types,
                        env.check_undeclared_templates,
                    )
                })
            {
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, m_span.clone())
                        .with_message(&msg)
                        .with_label(Label::new(m_span).with_message(&msg).with_color(Color::Red))
                        .finish(),
                );
            }
        }
    }
    if check_constructors {
        for ctor in &ty.cpp_constructors {
            let c_key = PartialSpanKey::cpp_constructor(ty.ty.clone(), ctor.rust_name.clone());
            let direct_spans = ctx.fetch_spans_global(&c_key);
            let c_span = direct_spans
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(&default_span);
            if direct_spans.is_empty()
                && let Some(origin) = env
                    .template_member_lints
                    .and_then(|l| l.constructors.get(&ctor.rust_name))
            {
                let inst_span = ctx
                    .fetch_spans_global(ty)
                    .first()
                    .copied()
                    .cloned()
                    .unwrap_or_else(|| origin.template_span.clone());
                report_template_member_lint(
                    ctx,
                    origin,
                    &inst_span,
                    env.instantiation_allowed,
                    "elided_lifetimes",
                    origin.orig_had_elided_lifetimes,
                    ctor.inputs.iter().any(has_elided_ref),
                    &elided_lifetimes_warning("constructor"),
                    reported_deferred,
                );
                report_template_member_lint(
                    ctx,
                    origin,
                    &inst_span,
                    env.instantiation_allowed,
                    "safe_raw_pointer",
                    origin.orig_had_safe_raw_pointer,
                    ctor.is_safe && ctor.inputs.iter().any(contains_raw_ptr),
                    &safe_raw_pointer_warning("constructor"),
                    reported_deferred,
                );
            }
            if let Err(msg) = ctor.inputs.iter().try_for_each(|t| {
                check_cpp_signature_type(
                    t,
                    false,
                    env.known_cpp_types,
                    env.known_cpp_paths,
                    env.by_value_cpp_types,
                    env.check_undeclared_templates,
                )
            }) {
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, c_span.clone())
                        .with_message(&msg)
                        .with_label(Label::new(c_span).with_message(&msg).with_color(Color::Red))
                        .finish(),
                );
            }
        }
    }
    if check_fields {
        for field in &ty.cpp_fields {
            let f_key = field.into_span_key_with(ty.ty.clone());
            let direct_spans = ctx.fetch_spans_global(&f_key);
            let f_span = direct_spans
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(&default_span);
            let inner_ty = match &field.ty {
                RustType::CppConst(inner) => inner.as_ref(),
                other => other,
            };
            let check = if let Some(heap_info) = env.heap_info {
                check_cpp_field(field, Some(heap_info))
            } else {
                Ok(())
            }
            .and_then(|()| {
                check_cpp_signature_type(
                    inner_ty,
                    true,
                    env.known_cpp_types,
                    env.known_cpp_paths,
                    env.by_value_cpp_types,
                    env.check_undeclared_templates,
                )
            })
            // Copies of fields of C++ class types are constructed in Rust's memory, so Rust must
            // hold them by value.
            .and_then(|()| {
                if matches!(field.access, CppFieldAccess::Copy { .. })
                    && let RustType::Cpp(erased_path) = inner_ty.with_erased_lifetimes()
                    && env.known_cpp_types.contains(&erased_path)
                    && !env.by_value_cpp_types.contains(&erased_path)
                {
                    Err(format!(
                        "the accessor `{}` returns a copy of the field `{}`, but Rust doesn't \
                         hold `{}` by value (it needs a layout and `#trivially_relocatable`); \
                         return a reference instead (`field &const {} ...`)",
                        field.rust_name(),
                        field.name,
                        zng_spelling(inner_ty),
                        field.name,
                    ))
                } else {
                    Ok(())
                }
            });
            if let Err(msg) = check {
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, f_span.clone())
                        .with_message(&msg)
                        .with_label(Label::new(f_span).with_message(&msg).with_color(Color::Red))
                        .finish(),
                );
            } else if direct_spans.is_empty()
                && let Some(origin) = env
                    .template_member_lints
                    .and_then(|l| l.fields.get(field.rust_name()))
            {
                let inst_span = ctx
                    .fetch_spans_global(ty)
                    .first()
                    .copied()
                    .cloned()
                    .unwrap_or_else(|| origin.template_span.clone());
                report_template_member_lint(
                    ctx,
                    origin,
                    &inst_span,
                    env.instantiation_allowed,
                    "safe_raw_pointer",
                    origin.orig_had_safe_raw_pointer,
                    is_mutable_raw_ptr_field(field),
                    SAFE_RAW_POINTER_FIELD_WARNING,
                    reported_deferred,
                );
            }
        }
    }
    // Members of the same kind are merged by their Rust name, and duplicates in a declaration
    // were reported there, so this finds C++ members of different kinds with the same Rust name
    // (or a C++ member colliding with a Rust `fn` method).
    let mut member_names: std::collections::HashMap<&str, ReportSpan> =
        std::collections::HashMap::new();
    for (name, key) in ty
        .cpp_constructors
        .iter()
        .map(|c| {
            (
                c.rust_name.as_str(),
                PartialSpanKey::cpp_constructor(ty.ty.clone(), c.rust_name.clone()),
            )
        })
        .chain(
            ty.cpp_methods
                .iter()
                .map(|m| (m.rust_name(), m.into_span_key_with(ty.ty.clone()))),
        )
        .chain(
            ty.cpp_fields
                .iter()
                .map(|f| (f.rust_name(), f.into_span_key_with(ty.ty.clone()))),
        )
    {
        let spans = ctx.fetch_spans_global(&key);
        let prev_span = member_names.get(name).cloned().or_else(|| {
            ty.methods
                .iter()
                .find(|m| m.data.name == name)
                .and_then(|rust_method| {
                    let other_key = rust_method.data.clone().into_span_key_with(ty.ty.clone());
                    ctx.fetch_spans_global(&other_key).first().copied().cloned()
                })
        });
        if let Some(prev_span) = prev_span {
            let span = spans
                .iter()
                .copied()
                .find(|s| *s != &prev_span)
                .or_else(|| spans.first().copied())
                .cloned()
                .unwrap_or_else(&default_span);
            let msg = duplicate_member_name(name);
            let mut report = Report::build(ReportKind::Error, span.clone())
                .with_message(&msg)
                .with_label(
                    Label::new(span.clone())
                        .with_message(&msg)
                        .with_color(Color::Red),
                );
            if prev_span != span && prev_span != (0, 0usize..0) {
                report = report.with_label(
                    Label::new(prev_span)
                        .with_message("also declared here")
                        .with_color(Color::Yellow),
                );
            }
            ctx.add_fatal_report(report.finish());
        } else if let Some(first_span) = spans
            .first()
            .copied()
            .cloned()
            .or_else(|| fallback_span.clone())
        {
            member_names.insert(name, first_span);
        }
    }
}

/// Reports `message` as an error at the declaration of `ty`.
fn type_error(ctx: &mut ParseContext, ty: &RustType, message: &str) {
    let span = ctx
        .fetch_spans_global(ty)
        .first()
        .copied()
        .cloned()
        .unwrap_or((0, 0usize..0));
    ctx.add_fatal_report(
        Report::build(ReportKind::Error, span.clone())
            .with_message(message)
            .with_label(
                Label::new(span)
                    .with_message(message)
                    .with_color(Color::Red),
            )
            .finish(),
    );
}

/// Works out the homes of the C++ class templates that this module instantiates or declares a
/// template for (`spec.cpp_template_homes`), and checks their instantiations: each C++ class
/// template is a Rust alias (in its home) that names one Rust struct per instantiation, which
/// limits where instantiations can be declared and what they can be.
fn resolve_cpp_templates(
    spec: &mut ZngurSpec,
    local_templates: &[&TemplateDef],
    module_import_spans: &[ReportSpan],
    module_import_resolved_paths: &[Option<std::path::PathBuf>],
    ctx: &mut ParseContext,
) {
    #[derive(Clone)]
    struct ImportedHomeEntry {
        import_index: usize,
        home: CppTemplateHome,
        is_direct: bool,
        owner_path: Option<std::path::PathBuf>,
        owner_crate: Option<String>,
    }

    let display = |path: &[String]| format!("c++::{}", path.join("::"));
    let same_owner = |a: &ImportedHomeEntry, b: &ImportedHomeEntry| {
        if let (Some(pa), Some(pb)) = (&a.owner_path, &b.owner_path)
            && pa == pb
        {
            return true;
        }
        if let (Some(ca), Some(cb)) = (&a.owner_crate, &b.owner_crate)
            && ca.replace('-', "_") == cb.replace('-', "_")
        {
            return true;
        }
        false
    };
    // Template path -> the distinct home modules brought in by direct or transitive imports.
    let mut imported_homes: Vec<(Vec<String>, Vec<ImportedHomeEntry>)> = vec![];
    let record_imported_home = |imported_homes: &mut Vec<(Vec<String>, Vec<ImportedHomeEntry>)>,
                                entry: ImportedHomeEntry| {
        match imported_homes
            .iter_mut()
            .find(|(p, _)| *p == entry.home.path)
        {
            Some((_, homes)) => {
                if let Some(existing) = homes.iter_mut().find(|prev| same_owner(prev, &entry)) {
                    if entry.is_direct && !existing.is_direct {
                        *existing = entry;
                    }
                } else {
                    homes.push(entry);
                }
            }
            None => imported_homes.push((entry.home.path.clone(), vec![entry])),
        }
    };
    for (index, module) in spec.imported_modules.iter().enumerate() {
        let resolved_path = module_import_resolved_paths.get(index).cloned().flatten();
        for home in &module.cpp_template_homes {
            record_imported_home(
                &mut imported_homes,
                ImportedHomeEntry {
                    import_index: index,
                    home: home.clone(),
                    is_direct: true,
                    owner_path: resolved_path.clone(),
                    owner_crate: module.crate_name.clone(),
                },
            );
        }
        if let Some(norm_path) = &resolved_path
            && let Some(cached) = ctx.parsed_module_cache.get(norm_path)
        {
            for ind in &cached.indirect_cpp_template_homes {
                record_imported_home(
                    &mut imported_homes,
                    ImportedHomeEntry {
                        import_index: index,
                        home: ind.home.clone(),
                        is_direct: false,
                        owner_path: Some(ind.home_module_path.clone()),
                        owner_crate: ind.home_crate_name.clone(),
                    },
                );
            }
        }
    }
    let import_name = |index: usize| spec.imported_modules[index].path.display().to_string();
    for (path, homes) in &imported_homes {
        if let [first, second, ..] = homes.as_slice() {
            let msg = if first.is_direct && second.is_direct {
                format!(
                    "imported modules `{}` and `{}` are both the home of `{}`; a C++ class \
                     template can only have one home (declare `type<T> {}<...>` in a module that \
                     both import)",
                    import_name(first.import_index),
                    import_name(second.import_index),
                    display(path),
                    display(path),
                )
            } else {
                let describe_side = |e: &ImportedHomeEntry| match (e.is_direct, &e.owner_crate) {
                    (true, Some(c)) => {
                        format!("`{}` (home `#crate {c}`)", import_name(e.import_index))
                    }
                    (false, Some(c)) => {
                        format!(
                            "`{}` (using home `#crate {c}`)",
                            import_name(e.import_index)
                        )
                    }
                    (_, None) => format!("`{}`", import_name(e.import_index)),
                };
                format!(
                    "imported modules {} and {} use different homes for `{}`; a C++ class \
                     template can only have one home (declare `type<T> {}<...>` in a module that \
                     both import)",
                    describe_side(first),
                    describe_side(second),
                    display(path),
                    display(path),
                )
            };
            let span = module_import_spans
                .get(second.import_index)
                .cloned()
                .unwrap_or((0, 0usize..0));
            ctx.add_fatal_report(
                Report::build(ReportKind::Error, span.clone())
                    .with_message(&msg)
                    .with_label(Label::new(span).with_message(&msg).with_color(Color::Red))
                    .finish(),
            );
        }
    }
    let imported_home = |path: &[String]| {
        imported_homes
            .iter()
            .find(|(p, _)| p.as_slice() == path)
            .map(|(_, homes)| &homes[0])
    };
    let imported_plain_cpp = |path: &[String]| {
        spec.imported_modules
            .iter()
            .enumerate()
            .find(|(_, m)| m.cpp_types.iter().any(|p| p == path))
            .map(|(index, _)| index)
    };
    let plain_local_cpp: HashSet<&[String]> = spec
        .types
        .iter()
        .filter_map(|t| match &t.ty {
            RustType::Cpp(p) if p.generics.is_empty() => Some(p.path.as_slice()),
            _ => None,
        })
        .collect();
    let own_crate_norm = spec.crate_name.as_ref().map(|c| c.replace('-', "_"));
    for (path, homes) in &imported_homes {
        let entry = &homes[0];
        let checked_by_direct_template_import = spec.imported_modules.iter().any(|m| {
            m.crate_name
                .as_ref()
                .is_some_and(|c| own_crate_norm.as_deref() != Some(&c.replace('-', "_")))
                && m.cpp_templates.contains(path)
        });
        if !checked_by_direct_template_import && plain_local_cpp.contains(path.as_slice()) {
            let msg = if entry.is_direct {
                format!(
                    "`{}` is already imported from `{}` as a C++ class template, and cannot also \
                     be declared as a plain C++ type in this module",
                    display(path),
                    import_name(entry.import_index),
                )
            } else {
                let via = match &entry.owner_crate {
                    Some(c) => format!(
                        "`#crate {c}`, imported via `{}`",
                        import_name(entry.import_index)
                    ),
                    None => format!("imported via `{}`", import_name(entry.import_index)),
                };
                format!(
                    "`{}` is already a C++ class template ({via}), and cannot also be declared as \
                     a plain C++ type in this module",
                    display(path),
                )
            };
            type_error(ctx, &RustType::Cpp(path.clone().into()), &msg);
        }
        if let Some(plain_idx) = imported_plain_cpp(path) {
            let plain_checked_by_owners = spec.imported_modules[plain_idx]
                .crate_name
                .as_ref()
                .is_some_and(|c| own_crate_norm.as_deref() != Some(&c.replace('-', "_")));
            if !(checked_by_direct_template_import && plain_checked_by_owners) {
                let plain_desc = match &spec.imported_modules[plain_idx].crate_name {
                    Some(c) => c.replace('-', "_"),
                    None => import_name(plain_idx),
                };
                let template_desc = match (entry.is_direct, &entry.owner_crate) {
                    (true, Some(c)) => c.replace('-', "_"),
                    (false, Some(c)) => {
                        format!(
                            "{} via `{}`",
                            c.replace('-', "_"),
                            import_name(entry.import_index)
                        )
                    }
                    (_, None) => import_name(entry.import_index),
                };
                let (first_idx, second_idx) = if plain_idx <= entry.import_index {
                    (plain_idx, entry.import_index)
                } else {
                    (entry.import_index, plain_idx)
                };
                let msg = format!(
                    "`{}` is imported both as a plain C++ type (`{plain_desc}`) and as a C++ \
                     class template (`{template_desc}`)",
                    display(path),
                );
                let span = module_import_spans
                    .get(second_idx)
                    .cloned()
                    .unwrap_or((0, 0usize..0));
                let mut report = Report::build(ReportKind::Error, span.clone())
                    .with_message(&msg)
                    .with_label(
                        Label::new(span.clone())
                            .with_message(&msg)
                            .with_color(Color::Red),
                    );
                if first_idx != second_idx
                    && let Some(prev_span) = module_import_spans.get(first_idx).cloned()
                {
                    report = report.with_label(
                        Label::new(prev_span)
                            .with_message(format!(
                                "first imported from `{}` here",
                                import_name(first_idx)
                            ))
                            .with_color(Color::Yellow),
                    );
                }
                ctx.add_fatal_report(report.finish());
            }
        }
    }

    let mut homes: Vec<CppTemplateHome> = vec![];
    for template in local_templates {
        let RustType::Cpp(path) = &template.ty.ty else {
            continue;
        };
        if plain_local_cpp.contains(path.path.as_slice()) {
            let span = (template.source_id, template.span.into_range());
            let message = format!(
                "`{}` cannot be declared both as a plain C++ type and as a C++ class template",
                display(&path.path)
            );
            ctx.add_fatal_report(
                Report::build(ReportKind::Error, span.clone())
                    .with_message(&message)
                    .with_label(
                        Label::new(span)
                            .with_message(&message)
                            .with_color(Color::Red),
                    )
                    .finish(),
            );
            continue;
        }
        if let Some(index) = imported_plain_cpp(&path.path) {
            let span = (template.source_id, template.span.into_range());
            let message = format!(
                "`{}` is already imported as a plain C++ type from `{}`",
                display(&path.path),
                import_name(index),
            );
            ctx.add_fatal_report(
                Report::build(ReportKind::Error, span.clone())
                    .with_message(&message)
                    .with_label(
                        Label::new(span)
                            .with_message(&message)
                            .with_color(Color::Red),
                    )
                    .finish(),
            );
            continue;
        }
        if let Some(existing) = homes.iter().find(|h| h.path == path.path) {
            if existing.arity != path.generics.len() {
                let span = (template.source_id, template.span.into_range());
                let message = format!(
                    "every `type<...>` declaration of `{}` must have the same number of template \
                     arguments ({})",
                    display(&path.path),
                    existing.arity,
                );
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, span.clone())
                        .with_message(&message)
                        .with_label(
                            Label::new(span)
                                .with_message(&message)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
            }
            continue;
        }
        if let Some(entry) = imported_home(&path.path) {
            let span = (template.source_id, template.span.into_range());
            let message = if entry.is_direct {
                format!(
                    "`{}` already has a home: imported module `{}`",
                    display(&path.path),
                    import_name(entry.import_index)
                )
            } else {
                let via = match &entry.owner_crate {
                    Some(c) => format!(
                        "`#crate {c}`, imported via `{}`",
                        import_name(entry.import_index)
                    ),
                    None => format!("imported via `{}`", import_name(entry.import_index)),
                };
                format!(
                    "`{}` already has a home ({via}); `import` its home `.zng` file directly \
                     instead of declaring `type<...>` again",
                    display(&path.path),
                )
            };
            ctx.add_fatal_report(
                Report::build(ReportKind::Error, span.clone())
                    .with_message(&message)
                    .with_label(
                        Label::new(span)
                            .with_message(&message)
                            .with_color(Color::Red),
                    )
                    .finish(),
            );
            continue;
        }
        homes.push(CppTemplateHome {
            path: path.path.clone(),
            arity: path.generics.len(),
            explicit: true,
        });
    }

    // Template path -> number of template arguments and Rust types of its instantiations so far.
    let mut instantiations: Vec<(&[String], usize, Vec<String>)> = vec![];
    for ty_def in &spec.types {
        let RustType::Cpp(path) = &ty_def.ty else {
            continue;
        };
        if path.generics.is_empty() {
            continue;
        }
        if plain_local_cpp.contains(path.path.as_slice()) {
            type_error(
                ctx,
                &ty_def.ty,
                &format!(
                    "`{}` cannot be declared both as a plain C++ type and as a C++ class template",
                    display(&path.path)
                ),
            );
            continue;
        }
        if let Some(index) = imported_plain_cpp(&path.path) {
            type_error(
                ctx,
                &ty_def.ty,
                &format!(
                    "`{}` is already imported as a plain C++ type from `{}`",
                    display(&path.path),
                    import_name(index),
                ),
            );
            continue;
        }
        if ty_def.cpp_heap_allocated.is_some() {
            type_error(
                ctx,
                &ty_def.ty,
                "instantiations of C++ class templates can't be `#cpp_heap_allocated`",
            );
            continue;
        }
        let home = homes
            .iter()
            .find(|h| h.path == path.path)
            .or_else(|| imported_home(&path.path).map(|e| &e.home));
        let first_arity = instantiations
            .iter()
            .find(|(p, _, _)| *p == path.path.as_slice())
            .map(|(_, arity, _)| *arity);
        let arity = home
            .map(|h| h.arity)
            .or(first_arity)
            .unwrap_or(path.generics.len());
        if arity != path.generics.len() {
            type_error(
                ctx,
                &ty_def.ty,
                &format!(
                    "every instantiation of `{}` must have the same number of template \
                     arguments as its template and its other instantiations, which have {arity}",
                    display(&path.path)
                ),
            );
            continue;
        }
        // Each instantiation must be its own Rust type, for its own Rust struct.
        let rust_type = ty_def.ty.to_string();
        match instantiations
            .iter_mut()
            .find(|(p, _, _)| *p == path.path.as_slice())
        {
            Some((_, _, seen)) if seen.contains(&rust_type) => {
                type_error(
                    ctx,
                    &ty_def.ty,
                    "another instantiation of this C++ class template is the same Rust type",
                );
                continue;
            }
            Some((_, _, seen)) => seen.push(rust_type),
            None => instantiations.push((&path.path, arity, vec![rust_type])),
        }
        if homes.iter().any(|h| h.path == path.path) {
            continue;
        }
        match imported_home(&path.path) {
            Some(entry) if !entry.is_direct => {
                let via = match &entry.owner_crate {
                    Some(c) => format!(
                        "`#crate {c}`, imported via `{}`",
                        import_name(entry.import_index)
                    ),
                    None => format!("imported via `{}`", import_name(entry.import_index)),
                };
                type_error(
                    ctx,
                    &ty_def.ty,
                    &format!(
                        "`{}` already has a home ({via}), which is not directly imported here; \
                         `import` the home `.zng` file of `{}` directly to declare instantiations \
                         of it",
                        display(&path.path),
                        display(&path.path),
                    ),
                );
            }
            // The instantiation's struct implements the home's trait, so (like Rust's orphan
            // rule) one of the template arguments must be a local plain `c++::` struct (including
            // `const`-qualified ones, which are `Local::<Const>`).
            Some(entry) if entry.home.explicit => {
                let has_local_argument = path.generics.iter().any(|arg| {
                    let inner = match arg {
                        RustType::CppConst(inner) | RustType::CppVar(inner) => inner.as_ref(),
                        other => other,
                    };
                    matches!(inner, RustType::Cpp(p) if p.generics.is_empty())
                        && spec.types.iter().any(|t| &t.ty == inner)
                });
                if !has_local_argument {
                    type_error(
                        ctx,
                        &ty_def.ty,
                        &format!(
                            "this instantiation of `{}`, whose home is imported module `{}`, can \
                             only be declared there or in a module that declares one of its \
                             template arguments (like Rust's orphan rule)",
                            display(&path.path),
                            import_name(entry.import_index)
                        ),
                    );
                }
            }
            Some(entry) => type_error(
                ctx,
                &ty_def.ty,
                &format!(
                    "`{}` is also instantiated in imported module `{}`; to share a C++ class \
                     template between modules, declare `type<T> {}<...>` in one of them (its \
                     home)",
                    display(&path.path),
                    import_name(entry.import_index),
                    display(&path.path),
                ),
            ),
            // No home in sight: this module is the (implicit) home.
            None => homes.push(CppTemplateHome {
                path: path.path.clone(),
                arity,
                explicit: false,
            }),
        }
    }
    spec.cpp_template_homes = homes;
}

/// Validates every `RustType::Cpp` reference across `spec` (nested template arguments, type
/// members, function/trait signatures, and `extern "C++"` blocks) against declared plain `c++::`
/// types and C++ class template instantiations.
#[allow(clippy::too_many_arguments)]
fn check_cpp_template_uses(
    spec: &ZngurSpec,
    unmatched_local_cpp_templates: &[&TemplateDef],
    all_template_locations: &[Vec<ReportSpan>],
    module_import_resolved_paths: &[Option<std::path::PathBuf>],
    known_cpp_types: &HashSet<CppTypePath>,
    known_cpp_paths: &HashSet<Vec<String>>,
    by_value_cpp_types: &HashSet<CppTypePath>,
    check_undeclared_templates: bool,
    ctx: &mut ParseContext,
) {
    let mut known_arities: std::collections::HashMap<Vec<String>, (usize, HashSet<CppTypePath>)> =
        std::collections::HashMap::new();
    let mut invalid_paths: HashSet<Vec<String>> = HashSet::new();
    let mut invalid_type_indices: HashSet<usize> = HashSet::new();
    let mut all_imported_instantiations: HashSet<CppTypePath> = HashSet::new();
    let mut indirect_instantiations: std::collections::HashMap<CppTypePath, std::path::PathBuf> =
        std::collections::HashMap::new();

    for (index, module) in spec.imported_modules.iter().enumerate() {
        for segs in &module.cpp_types {
            let (arity, _) = known_arities
                .entry(segs.clone())
                .or_insert_with(|| (0, HashSet::new()));
            if *arity != 0 {
                invalid_paths.insert(segs.clone());
            }
        }
        for home in &module.cpp_template_homes {
            let (arity, _) = known_arities
                .entry(home.path.clone())
                .or_insert_with(|| (home.arity, HashSet::new()));
            if *arity != home.arity {
                invalid_paths.insert(home.path.clone());
            }
        }
        all_imported_instantiations.extend(module.cpp_template_instantiations.iter().cloned());
        if let Some(Some(norm_path)) = module_import_resolved_paths.get(index)
            && let Some(cached) = ctx.parsed_module_cache.get(norm_path)
        {
            for ind in &cached.indirect_cpp_template_homes {
                let (arity, _) = known_arities
                    .entry(ind.home.path.clone())
                    .or_insert_with(|| (ind.home.arity, HashSet::new()));
                if *arity != ind.home.arity {
                    invalid_paths.insert(ind.home.path.clone());
                }
            }
            for (inst, owner_path) in &cached.indirect_cpp_template_instantiations {
                indirect_instantiations
                    .entry(inst.clone())
                    .or_insert_with(|| owner_path.clone());
            }
        }
    }

    for home in &spec.cpp_template_homes {
        let (arity, _) = known_arities
            .entry(home.path.clone())
            .or_insert_with(|| (home.arity, HashSet::new()));
        if *arity != home.arity {
            invalid_paths.insert(home.path.clone());
        }
    }

    for (idx, ty_def) in spec.types.iter().enumerate() {
        let RustType::Cpp(path) = &ty_def.ty else {
            continue;
        };
        if !path.generics.is_empty() && ty_def.cpp_heap_allocated.is_some() {
            invalid_paths.insert(path.path.clone());
            invalid_type_indices.insert(idx);
            continue;
        }
        let (arity, instantiations) = known_arities
            .entry(path.path.clone())
            .or_insert_with(|| (path.generics.len(), HashSet::new()));
        if *arity != path.generics.len() {
            invalid_paths.insert(path.path.clone());
            invalid_type_indices.insert(idx);
            continue;
        }
        if !path.generics.is_empty() {
            let RustType::Cpp(erased_path) = ty_def.ty.with_erased_lifetimes() else {
                unreachable!()
            };
            instantiations.insert(erased_path);
        }
    }

    for inst in all_imported_instantiations {
        if let Some((arity, instantiations)) = known_arities.get_mut(&inst.path)
            && *arity == inst.generics.len()
        {
            instantiations.insert(inst);
        }
    }

    fn check_ty(
        ty: &RustType,
        known_arities: &std::collections::HashMap<Vec<String>, (usize, HashSet<CppTypePath>)>,
        indirect_instantiations: &std::collections::HashMap<CppTypePath, std::path::PathBuf>,
        invalid_paths: &HashSet<Vec<String>>,
        check_undeclared_templates: bool,
    ) -> Result<(), String> {
        let all = |tys: &[RustType]| {
            tys.iter().try_for_each(|t| {
                check_ty(
                    t,
                    known_arities,
                    indirect_instantiations,
                    invalid_paths,
                    check_undeclared_templates,
                )
            })
        };
        match ty {
            RustType::Cpp(path) => {
                all(&path.generics)?;
                if invalid_paths.contains(&path.path) {
                    return Ok(());
                }
                let RustType::Cpp(erased_path) = ty.with_erased_lifetimes() else {
                    unreachable!()
                };
                if let Some((arity, instantiations)) = known_arities.get(&path.path) {
                    if *arity == 0 && !path.generics.is_empty() {
                        return Err(format!(
                            "`c++::{}` is declared as a plain C++ type, so it cannot take \
                             template arguments",
                            path.path.join("::")
                        ));
                    }
                    if *arity > 0 && path.generics.is_empty() {
                        let noun = if *arity == 1 { "argument" } else { "arguments" };
                        return Err(format!(
                            "`c++::{}` is a C++ class template and needs {arity} template {noun}",
                            path.path.join("::")
                        ));
                    }
                    if *arity > 0 && path.generics.len() != *arity {
                        let noun = if *arity == 1 { "argument" } else { "arguments" };
                        return Err(format!(
                            "`c++::{}` takes {arity} template {noun}, not {}",
                            path.path.join("::"),
                            path.generics.len()
                        ));
                    }
                    if *arity > 0
                        && check_undeclared_templates
                        && ty.type_vars().is_empty()
                        && !instantiations.contains(&erased_path)
                    {
                        let spelled = zng_spelling(ty);
                        if let Some(ind_path) = indirect_instantiations.get(&erased_path) {
                            return Err(format!(
                                "instantiation `{spelled}` of C++ class template `c++::{}` is \
                                 declared in `{}` (imported transitively), which is not directly \
                                 imported here; `import` that `.zng` file directly",
                                path.path.join("::"),
                                ind_path.display()
                            ));
                        }
                        return Err(format!(
                            "instantiation `{spelled}` of C++ class template `c++::{}` is not \
                             declared; add `type {spelled} {{ ... }}`",
                            path.path.join("::")
                        ));
                    }
                } else if !path.generics.is_empty() && check_undeclared_templates {
                    if !ty.type_vars().is_empty() {
                        return Err(format!(
                            "C++ class template `c++::{}` is not declared or imported",
                            path.path.join("::")
                        ));
                    }
                    let spelled = zng_spelling(ty);
                    return Err(format!(
                        "instantiation `{spelled}` of C++ class template `c++::{}` is not \
                         declared; add `type {spelled} {{ ... }}`",
                        path.path.join("::")
                    ));
                }
                Ok(())
            }
            RustType::Ref(_, _, inner)
            | RustType::Raw(_, inner)
            | RustType::Boxed(inner)
            | RustType::Slice(inner)
            | RustType::CppPtr(_, inner)
            | RustType::CppConst(inner)
            | RustType::CppVar(inner) => check_ty(
                inner,
                known_arities,
                indirect_instantiations,
                invalid_paths,
                check_undeclared_templates,
            ),
            RustType::Tuple(tys) => all(tys),
            RustType::Adt(pg) => {
                all(&pg.generics)?;
                pg.named_generics.iter().try_for_each(|(_, t)| {
                    check_ty(
                        t,
                        known_arities,
                        indirect_instantiations,
                        invalid_paths,
                        check_undeclared_templates,
                    )
                })
            }
            RustType::Dyn(tr, _) | RustType::Impl(tr, _) => match tr {
                RustTrait::Normal(pg) => {
                    all(&pg.generics)?;
                    pg.named_generics.iter().try_for_each(|(_, t)| {
                        check_ty(
                            t,
                            known_arities,
                            indirect_instantiations,
                            invalid_paths,
                            check_undeclared_templates,
                        )
                    })
                }
                RustTrait::Fn { inputs, output, .. } => {
                    all(inputs)?;
                    check_ty(
                        output,
                        known_arities,
                        indirect_instantiations,
                        invalid_paths,
                        check_undeclared_templates,
                    )
                }
            },
            RustType::Primitive(_) | RustType::TypeVar(_) => Ok(()),
        }
    }

    let check_at = |ctx: &mut ParseContext, span: ReportSpan, tys: &[&RustType]| {
        if let Err(msg) = tys.iter().try_for_each(|t| {
            check_ty(
                t,
                &known_arities,
                &indirect_instantiations,
                &invalid_paths,
                check_undeclared_templates,
            )
        }) {
            ctx.add_fatal_report(
                Report::build(ReportKind::Error, span.clone())
                    .with_message(&msg)
                    .with_label(Label::new(span).with_message(&msg).with_color(Color::Red))
                    .finish(),
            );
        }
    };

    for template in unmatched_local_cpp_templates {
        let ty_def = &template.ty;
        let ty_span = (template.source_id, template.span.into_range());
        if let RustType::Cpp(path) = &ty_def.ty
            && !invalid_paths.contains(&path.path)
        {
            let args: Vec<&RustType> = path.generics.iter().collect();
            check_at(ctx, ty_span.clone(), &args);
            if let Err(msg) = path.generics.iter().try_for_each(|g| {
                check_cpp_signature_type(
                    g,
                    true,
                    known_cpp_types,
                    known_cpp_paths,
                    by_value_cpp_types,
                    check_undeclared_templates,
                )
            }) {
                ctx.add_fatal_report(
                    Report::build(ReportKind::Error, ty_span.clone())
                        .with_message(&msg)
                        .with_label(
                            Label::new(ty_span.clone())
                                .with_message(&msg)
                                .with_color(Color::Red),
                        )
                        .finish(),
                );
            }
        }
        for method in &ty_def.methods {
            let key = method.data.clone().into_span_key_with(ty_def.ty.clone());
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| ty_span.clone());
            let tys: Vec<&RustType> = method
                .data
                .inputs
                .iter()
                .chain([&method.data.output])
                .chain(&method.data.generics)
                .chain(method.deref.as_ref().map(|(d, _)| d))
                .collect();
            check_at(ctx, span, &tys);
        }
        for method in &ty_def.cpp_methods {
            let key = method.into_span_key_with(ty_def.ty.clone());
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| ty_span.clone());
            let tys: Vec<&RustType> = method.inputs.iter().chain([&method.output]).collect();
            check_at(ctx, span, &tys);
        }
        for ctor in &ty_def.cpp_constructors {
            let key = PartialSpanKey::cpp_constructor(ty_def.ty.clone(), ctor.rust_name.clone());
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| ty_span.clone());
            let tys: Vec<&RustType> = ctor.inputs.iter().collect();
            check_at(ctx, span, &tys);
        }
        for field in &ty_def.cpp_fields {
            let key = field.into_span_key_with(ty_def.ty.clone());
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| ty_span.clone());
            check_at(ctx, span, &[&field.ty]);
        }
    }

    for (idx, ty_def) in spec.types.iter().enumerate() {
        let template_locations = all_template_locations
            .get(idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let ty_span = ctx
            .fetch_spans_global(ty_def)
            .first()
            .copied()
            .cloned()
            .or_else(|| template_locations.last().cloned())
            .unwrap_or((0, 0usize..0));

        if !invalid_type_indices.contains(&idx) {
            match &ty_def.ty {
                RustType::Cpp(path) => {
                    let args: Vec<&RustType> = path.generics.iter().collect();
                    check_at(ctx, ty_span.clone(), &args);
                    if let Err(msg) = path.generics.iter().try_for_each(|g| {
                        check_cpp_signature_type(
                            g,
                            true,
                            known_cpp_types,
                            known_cpp_paths,
                            by_value_cpp_types,
                            check_undeclared_templates,
                        )
                    }) {
                        ctx.add_fatal_report(
                            Report::build(ReportKind::Error, ty_span.clone())
                                .with_message(&msg)
                                .with_label(
                                    Label::new(ty_span.clone())
                                        .with_message(&msg)
                                        .with_color(Color::Red),
                                )
                                .finish(),
                        );
                    }
                }
                other => check_at(ctx, ty_span.clone(), &[other]),
            }
        }
        if let Some(ctor) = &ty_def.constructor {
            let key = ctor.clone().into_span_key_with(ty_def.ty.clone());
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| ty_span.clone());
            let tys: Vec<&RustType> = ctor.inputs.iter().map(|(_, t)| t).collect();
            check_at(ctx, span, &tys);
        }
        for field in &ty_def.fields {
            let key = PartialSpanKey::field(ty_def.ty.clone(), None, field.name.clone());
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| ty_span.clone());
            check_at(ctx, span, &[&field.ty]);
        }
        for variant in &ty_def.variants {
            for field in &variant.fields {
                let key = PartialSpanKey::field(
                    ty_def.ty.clone(),
                    Some(variant.name.clone()),
                    field.name.clone(),
                );
                let span = ctx
                    .fetch_spans_global(&key)
                    .first()
                    .copied()
                    .cloned()
                    .unwrap_or_else(|| ty_span.clone());
                check_at(ctx, span, &[&field.ty]);
            }
        }
        for method in &ty_def.methods {
            let key = method.data.clone().into_span_key_with(ty_def.ty.clone());
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| ty_span.clone());
            let tys: Vec<&RustType> = method
                .data
                .inputs
                .iter()
                .chain([&method.data.output])
                .chain(&method.data.generics)
                .chain(method.deref.as_ref().map(|(d, _)| d))
                .collect();
            check_at(ctx, span, &tys);
        }
        for method in &ty_def.cpp_methods {
            let key = method.into_span_key_with(ty_def.ty.clone());
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| ty_span.clone());
            let tys: Vec<&RustType> = method.inputs.iter().chain([&method.output]).collect();
            check_at(ctx, span, &tys);
        }
        for ctor in &ty_def.cpp_constructors {
            let key = PartialSpanKey::cpp_constructor(ty_def.ty.clone(), ctor.rust_name.clone());
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| ty_span.clone());
            let tys: Vec<&RustType> = ctor.inputs.iter().collect();
            check_at(ctx, span, &tys);
        }
        for field in &ty_def.cpp_fields {
            let key = field.into_span_key_with(ty_def.ty.clone());
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| ty_span.clone());
            check_at(ctx, span, &[&field.ty]);
        }
    }

    for (tr, tr_def) in &spec.traits {
        let tr_span = ctx
            .fetch_spans_global(tr_def)
            .first()
            .copied()
            .cloned()
            .unwrap_or((0, 0usize..0));
        let tr_ty = RustType::Dyn(tr.clone(), vec![]);
        check_at(ctx, tr_span.clone(), &[&tr_ty]);
        for method in &tr_def.methods {
            let key = PartialSpanKey::trait_method(tr.clone(), method.name.clone());
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| tr_span.clone());
            let tys: Vec<&RustType> = method
                .inputs
                .iter()
                .chain([&method.output])
                .chain(&method.generics)
                .collect();
            check_at(ctx, span, &tys);
        }
    }

    for func in &spec.funcs {
        let span = ctx
            .fetch_spans_global(func)
            .first()
            .copied()
            .cloned()
            .unwrap_or((0, 0usize..0));
        let tys: Vec<&RustType> = func
            .path
            .generics
            .iter()
            .chain(&func.inputs)
            .chain([&func.output])
            .collect();
        check_at(ctx, span, &tys);
    }

    for func in &spec.cpp_funcs {
        let span = ctx
            .fetch_spans_global(func)
            .first()
            .copied()
            .cloned()
            .unwrap_or((0, 0usize..0));
        let tys: Vec<&RustType> = func.inputs.iter().chain([&func.output]).collect();
        check_at(ctx, span, &tys);
    }

    for func in &spec.extern_cpp_funcs {
        let span = ctx
            .fetch_spans_global(func)
            .first()
            .copied()
            .cloned()
            .unwrap_or((0, 0usize..0));
        let tys: Vec<&RustType> = func.inputs.iter().chain([&func.output]).collect();
        check_at(ctx, span, &tys);
    }

    for imp in &spec.extern_cpp_impls {
        let imp_span = ctx
            .fetch_spans_global(imp)
            .first()
            .copied()
            .cloned()
            .unwrap_or((0, 0usize..0));
        let tr_ty = imp.tr.as_ref().map(|tr| RustType::Dyn(tr.clone(), vec![]));
        let mut header_tys: Vec<&RustType> = vec![&imp.ty];
        if let Some(t) = &tr_ty {
            header_tys.push(t);
        }
        check_at(ctx, imp_span.clone(), &header_tys);
        for method in &imp.methods {
            let key = PartialSpanKey::extern_cpp_method(
                imp.tr.clone(),
                imp.ty.clone(),
                method.name.clone(),
            );
            let span = ctx
                .fetch_spans_global(&key)
                .first()
                .copied()
                .cloned()
                .unwrap_or_else(|| imp_span.clone());
            let tys: Vec<&RustType> = method
                .inputs
                .iter()
                .chain([&method.output])
                .chain(&method.generics)
                .collect();
            check_at(ctx, span, &tys);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Token<'a> {
    Arrow,
    ArrowArm,
    AngleOpen,
    AngleClose,
    BracketOpen,
    BracketClose,
    Colon,
    ColonColon,
    ParenOpen,
    ParenClose,
    BraceOpen,
    BraceClose,
    And,
    Star,
    Sharp,
    Plus,
    Eq,
    Question,
    Comma,
    Semicolon,
    Pipe,
    Underscore,
    Dot,
    Bang,
    KwAs,
    KwAsync,
    KwDyn,
    KwUse,
    KwFor,
    KwMod,
    KwCrate,
    KwType,
    KwTrait,
    KwFn,
    KwMut,
    KwConst,
    KwExtern,
    KwImpl,
    KwImport,
    KwMerge,
    KwIf,
    KwElse,
    KwMatch,
    KwSafe,
    KwUnsafe,
    CppPathStart,
    /// `'a`, without the `'`.
    Lifetime(&'a str),
    Ident(&'a str),
    Str(&'a str),
    RawStr(usize, &'a str),
    Number(usize),
}

impl<'a> Token<'a> {
    fn ident_or_kw(ident: &'a str) -> Self {
        match ident {
            "as" => Token::KwAs,
            "async" => Token::KwAsync,
            "dyn" => Token::KwDyn,
            "mod" => Token::KwMod,
            "type" => Token::KwType,
            "trait" => Token::KwTrait,
            "crate" => Token::KwCrate,
            "fn" => Token::KwFn,
            "mut" => Token::KwMut,
            "const" => Token::KwConst,
            "use" => Token::KwUse,
            "for" => Token::KwFor,
            "extern" => Token::KwExtern,
            "impl" => Token::KwImpl,
            "import" => Token::KwImport,
            "merge" => Token::KwMerge,
            "if" => Token::KwIf,
            "else" => Token::KwElse,
            "match" => Token::KwMatch,
            "safe" => Token::KwSafe,
            "unsafe" => Token::KwUnsafe,
            x => Token::Ident(x),
        }
    }
}

impl Display for Token<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Arrow => write!(f, "->"),
            Token::ArrowArm => write!(f, "=>"),
            Token::AngleOpen => write!(f, "<"),
            Token::AngleClose => write!(f, ">"),
            Token::BracketOpen => write!(f, "["),
            Token::BracketClose => write!(f, "]"),
            Token::ParenOpen => write!(f, "("),
            Token::ParenClose => write!(f, ")"),
            Token::BraceOpen => write!(f, "{{"),
            Token::BraceClose => write!(f, "}}"),
            Token::Colon => write!(f, ":"),
            Token::ColonColon => write!(f, "::"),
            Token::And => write!(f, "&"),
            Token::Star => write!(f, "*"),
            Token::Sharp => write!(f, "#"),
            Token::Plus => write!(f, "+"),
            Token::Eq => write!(f, "="),
            Token::Question => write!(f, "?"),
            Token::Comma => write!(f, ","),
            Token::Semicolon => write!(f, ";"),
            Token::Pipe => write!(f, "|"),
            Token::Underscore => write!(f, "_"),
            Token::Dot => write!(f, "."),
            Token::Bang => write!(f, "!"),
            Token::KwAs => write!(f, "as"),
            Token::KwAsync => write!(f, "async"),
            Token::KwDyn => write!(f, "dyn"),
            Token::KwUse => write!(f, "use"),
            Token::KwFor => write!(f, "for"),
            Token::KwMod => write!(f, "mod"),
            Token::KwCrate => write!(f, "crate"),
            Token::KwType => write!(f, "type"),
            Token::KwTrait => write!(f, "trait"),
            Token::KwFn => write!(f, "fn"),
            Token::KwMut => write!(f, "mut"),
            Token::KwConst => write!(f, "const"),
            Token::KwExtern => write!(f, "extern"),
            Token::KwImpl => write!(f, "impl"),
            Token::KwImport => write!(f, "import"),
            Token::KwMerge => write!(f, "merge"),
            Token::KwIf => write!(f, "if"),
            Token::KwElse => write!(f, "else"),
            Token::KwMatch => write!(f, "match"),
            Token::KwSafe => write!(f, "safe"),
            Token::KwUnsafe => write!(f, "unsafe"),
            Token::CppPathStart => write!(f, "c++"),
            Token::Lifetime(l) => write!(f, "'{l}"),
            Token::Ident(i) => write!(f, "{i}"),
            Token::Number(n) => write!(f, "{n}"),
            Token::Str(s) => write!(f, r#""{s}""#),
            Token::RawStr(hashes, s) => {
                let h = "#".repeat(*hashes);
                write!(f, r#"r{h}"{s}"{h}"#)
            }
        }
    }
}

fn lexer<'src>()
-> impl Parser<'src, &'src str, Vec<(Token<'src>, Span)>, extra::Err<Rich<'src, char, Span>>> {
    let plain_string = just('"')
        .ignore_then(none_of('"').repeated().to_slice().map(Token::Str))
        .then_ignore(just('"'));

    let raw_string_start = just('r')
        .ignore_then(just('#').repeated().count())
        .then_ignore(just('"'));
    let raw_string_end =
        just('"').then(just('#').repeated().configure(|cfg, ctx| cfg.exactly(*ctx)));
    let raw_string = raw_string_start
        .then_with_ctx(
            any()
                .and_is(raw_string_end.not())
                .repeated()
                .to_slice()
                .then_ignore(raw_string_end),
        )
        .map(|(h, s)| Token::RawStr(h, s));

    let token = choice((
        raw_string,
        plain_string,
        just("c++").to(Token::CppPathStart),
        just('\'').ignore_then(text::ident()).map(Token::Lifetime),
        text::ident()
            .filter(|ident: &&str| *ident != "_")
            .map(Token::ident_or_kw),
        choice([
            just("->").to(Token::Arrow),
            just("=>").to(Token::ArrowArm),
            just("<").to(Token::AngleOpen),
            just(">").to(Token::AngleClose),
            just("[").to(Token::BracketOpen),
            just("]").to(Token::BracketClose),
            just("(").to(Token::ParenOpen),
            just(")").to(Token::ParenClose),
            just("{").to(Token::BraceOpen),
            just("}").to(Token::BraceClose),
            just("::").to(Token::ColonColon),
            just(":").to(Token::Colon),
            just("&").to(Token::And),
            just("*").to(Token::Star),
            just("#").to(Token::Sharp),
            just("+").to(Token::Plus),
            just("=").to(Token::Eq),
            just("?").to(Token::Question),
            just(",").to(Token::Comma),
            just(";").to(Token::Semicolon),
            just("|").to(Token::Pipe),
            just("_").to(Token::Underscore),
            just(".").to(Token::Dot),
            just("!").to(Token::Bang),
        ]),
        text::int(10).map(|x: &str| Token::Number(x.parse().unwrap())),
    ));

    let comment = just("//")
        .then(any().and_is(just('\n').not()).repeated())
        .padded();
    let padding = comment.repeated().padded();

    padding
        .clone()
        .ignore_then(
            token
                .map_with(|tok, extra| (tok, extra.span()))
                .then_ignore(padding)
                .repeated()
                .collect(),
        )
        .boxed()
}

fn alias<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    just(Token::KwUse)
        .ignore_then(path())
        .then_ignore(just(Token::KwAs))
        .then(select! {
            Token::Ident(c) => c,
        })
        .then_ignore(just(Token::Semicolon))
        .map_with(|(path, name), extra| {
            ParsedItem::Alias(ParsedAlias {
                name,
                path,
                span: extra.span(),
            })
        })
        .boxed()
}

fn file_parser<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedZngFile<'a>, ZngParserExtra<'a>> + Clone {
    item()
        .repeated()
        .collect::<Vec<_>>()
        .map(ParsedZngFile)
        .boxed()
}

fn rust_type<'a>() -> Boxed<'a, 'a, ParserInput<'a>, ParsedRustType<'a>, ZngParserExtra<'a>> {
    let as_scalar = |s: &str, head: char| -> Option<u32> {
        let s = s.strip_prefix(head)?;
        s.parse().ok()
    };

    let scalar = select! {
        Token::Ident("bool") => PrimitiveRustType::Bool,
        Token::Ident("str") => PrimitiveRustType::Str,
        Token::Ident("char") => PrimitiveRustType::Char,
        Token::Ident("usize") => PrimitiveRustType::Usize,
        Token::Ident(c) if as_scalar(c, 'u').is_some() => PrimitiveRustType::Uint(as_scalar(c, 'u').unwrap()),
        Token::Ident(c) if as_scalar(c, 'i').is_some() => PrimitiveRustType::Int(as_scalar(c, 'i').unwrap()),
        Token::Ident(c) if as_scalar(c, 'f').is_some() => PrimitiveRustType::Float(as_scalar(c, 'f').unwrap()),
    }.map(ParsedRustType::Primitive);

    recursive(|parser| {
        let parser = parser.boxed();
        let pg = rust_path_and_generics(parser.clone());
        let adt = pg.clone().map(ParsedRustType::Adt);

        let trait_bound = just(Token::Plus)
            .ignore_then(path())
            .try_map_with(|path, extra| {
                let qualified = path.segments.len() > 1 || path.start != ParsedPathStart::Relative;
                let valid_qualified_path = matches!(
                    (path.start, path.segments.as_slice()),
                    (
                        ParsedPathStart::Relative | ParsedPathStart::Absolute,
                        ["std" | "core", "marker", _]
                    )
                );
                if qualified && !valid_qualified_path {
                    return Err(Rich::custom(
                        extra.span(),
                        "qualified trait bounds must use `std::marker` or `core::marker`",
                    ));
                }
                path.segments
                    .last()
                    .copied()
                    .ok_or_else(|| Rich::custom(extra.span(), "expected a trait bound path"))
            });

        let dyn_trait = just(Token::KwDyn)
            .or(just(Token::KwImpl))
            .then(rust_trait(parser.clone()))
            .then(trait_bound.repeated().collect::<Vec<_>>().boxed())
            .map(|((token, first), rest)| match token {
                Token::KwDyn => ParsedRustType::Dyn(first, rest),
                Token::KwImpl => ParsedRustType::Impl(first, rest),
                _ => unreachable!(),
            });
        let boxed = just(Token::Ident("Box"))
            .then(rust_generics(parser.clone()))
            .try_map(|(_, x), span| match <[_; 1]>::try_from(x) {
                Ok([ParsedGenericArg::Type(t)]) => Ok(ParsedRustType::Boxed(Box::new(t))),
                _ => Err(Rich::custom(span, "`Box` takes exactly one type argument")),
            });
        let unit = just(Token::ParenOpen)
            .then(just(Token::ParenClose))
            .map(|_| ParsedRustType::Tuple(vec![]));
        let tuple = parser
            .clone()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
            .map(|xs| ParsedRustType::Tuple(xs));
        let slice = parser
            .clone()
            .map(|x| ParsedRustType::Slice(Box::new(x)))
            .delimited_by(just(Token::BracketOpen), just(Token::BracketClose));
        // References: `&'a T` / `&'a mut T` (to Rust values), and `&'a [unique] (const | var) T`
        // (to C++ objects). `unique` and `var` are only keywords right after `&` / `&'a`.
        let constness = choice((
            just(Token::KwConst).to(CppConstness::Const),
            just(Token::Ident("var")).to(CppConstness::Var),
        ));
        let unique = just(Token::Ident("unique"));
        let reference = just(Token::And)
            .ignore_then(select! { Token::Lifetime(l) => l }.or_not())
            .then(choice((
                unique
                    .clone()
                    .ignore_then(constness.clone())
                    .then(parser.clone())
                    .map(|(c, x)| {
                        let kind = ParsedRefKind::Cpp {
                            unique: true,
                            constness: Some(c),
                        };
                        (kind, x)
                    })
                    .boxed(),
                constness
                    .then(parser.clone())
                    .map(|(c, x)| {
                        let kind = ParsedRefKind::Cpp {
                            unique: false,
                            constness: Some(c),
                        };
                        (kind, x)
                    })
                    .boxed(),
                // An error (reported by `check_cpp_const`).
                unique
                    .ignore_then(parser.clone())
                    .map(|x| {
                        let kind = ParsedRefKind::Cpp {
                            unique: true,
                            constness: None,
                        };
                        (kind, x)
                    })
                    .boxed(),
                just(Token::KwMut)
                    .ignore_then(parser.clone())
                    .map(|x| (ParsedRefKind::Rust(Mutability::Mut), x))
                    .boxed(),
                parser
                    .clone()
                    .map(|x| (ParsedRefKind::Rust(Mutability::Not), x))
                    .boxed(),
            )))
            .map(|(l, (kind, x))| ParsedRustType::Ref(kind, l, Box::new(x)));
        let raw_ptr = just(Token::Star)
            .ignore_then(
                just(Token::KwMut)
                    .to(Mutability::Mut)
                    .or(just(Token::KwConst).to(Mutability::Not)),
            )
            .then(parser)
            .map(|(m, x)| ParsedRustType::Raw(m, Box::new(x)));
        let non_cpp_ptr = choice((
            scalar.boxed(),
            boxed.boxed(),
            unit.boxed(),
            tuple.boxed(),
            slice.boxed(),
            adt.boxed(),
            reference.boxed(),
            raw_ptr.boxed(),
            dyn_trait.boxed(),
        ))
        .boxed();
        // `const T` or `T const`, which is only valid as the pointee of a C++ pointer or reference
        // or as a C++ template argument (checked by `check_cpp_const`).
        let cpp_const = just(Token::KwConst)
            .ignore_then(non_cpp_ptr.clone())
            .map(|x| ParsedRustType::CppConst(Box::new(x)));
        let non_cpp_ptr_maybe_east_const =
            non_cpp_ptr
                .then(just(Token::KwConst).or_not())
                .map(|(x, post_const)| {
                    if post_const.is_some() {
                        ParsedRustType::CppConst(Box::new(x))
                    } else {
                        x
                    }
                });
        // C++ pointers and references: `const T*`, `T const*`, `T*`, `const T&`, `T&`, `T**`,
        // `T* const*`, ...
        let cpp_indirection = choice((
            just(Token::Star).to(CppIndirection::Pointer),
            just(Token::And).to(CppIndirection::Reference),
        ))
        .then(just(Token::KwConst).or_not());
        choice((cpp_const, non_cpp_ptr_maybe_east_const))
            .foldl(cpp_indirection.repeated(), |x, (kind, post_const)| {
                let ptr = ParsedRustType::CppPtr(kind, Box::new(x));
                if post_const.is_some() {
                    ParsedRustType::CppConst(Box::new(ptr))
                } else {
                    ptr
                }
            })
            .boxed()
    })
    .try_map(|ty, span| match check_cpp_const(&ty, false) {
        Ok(()) => Ok(ty),
        Err(message) => Err(Rich::custom(span, message)),
    })
    .boxed()
}

/// Checks that `const T` only appears where it's valid: as the pointee of a C++ pointer or
/// reference or as a C++ template argument (`allowed`), with a `T` that is a C++ type, a builtin,
/// or a C++ pointer. Also checks the spelling of references to C++ objects (`&'a unique T` needs
/// `const` or `var`, and `&'a mut const T` is `&'a unique const T`). Whether `T` is a C++ object
/// at all is checked once the paths are resolved (see `check_cpp_objects_inner`).
fn check_cpp_const(ty: &ParsedRustType<'_>, allowed: bool) -> Result<(), String> {
    let all = |tys: &[ParsedRustType<'_>]| tys.iter().try_for_each(|t| check_cpp_const(t, false));
    match ty {
        ParsedRustType::CppConst(inner) => {
            if matches!(
                **inner,
                ParsedRustType::CppPtr(CppIndirection::Reference, _)
            ) {
                return Err("a C++ reference cannot be `const`-qualified (`T& const`)".to_owned());
            }
            if !allowed {
                return Err(
                    "`const T` can only be the type of a C++ pointer or reference \
                     (`const T&`, `const T*`, `&'a const T`) or a C++ template argument"
                        .to_owned(),
                );
            }
            match &**inner {
                ParsedRustType::Primitive(
                    PrimitiveRustType::Int(8 | 16 | 32 | 64)
                    | PrimitiveRustType::Uint(8 | 16 | 32 | 64)
                    | PrimitiveRustType::Float(32 | 64)
                    | PrimitiveRustType::Usize
                    | PrimitiveRustType::Bool,
                ) => Ok(()),
                ParsedRustType::Primitive(
                    PrimitiveRustType::Int(_)
                    | PrimitiveRustType::Uint(_)
                    | PrimitiveRustType::Float(_),
                ) => Err(
                    "only `bool`, 8/16/32/64-bit integers, `usize`, `f32`, and `f64` are C++ \
                     builtin scalars"
                        .to_owned(),
                ),
                ParsedRustType::Primitive(PrimitiveRustType::Str | PrimitiveRustType::Char) => {
                    Err("only C++ types and builtins can be `const`".to_owned())
                }
                ParsedRustType::Adt(_) | ParsedRustType::CppPtr(CppIndirection::Pointer, _) => {
                    check_cpp_const(inner, false)
                }
                _ => Err("only C++ types and builtins can be `const`".to_owned()),
            }
        }
        ParsedRustType::CppPtr(_, inner) => check_cpp_const(inner, true),
        ParsedRustType::Primitive(_) => Ok(()),
        ParsedRustType::Ref(
            ParsedRefKind::Cpp {
                unique: true,
                constness: None,
            },
            l,
            _,
        ) => {
            let l = l.map(|l| format!("'{l} ")).unwrap_or_default();
            Err(format!(
                "a `unique` reference is a reference to a C++ object, which says whether it's \
                 `const`: `&{l}unique var T` (a C++ `T&` that Rust uses uniquely, `&mut T` in \
                 Rust) or `&{l}unique const T`; a reference to a Rust value is `&{l}mut T`"
            ))
        }
        ParsedRustType::Ref(ParsedRefKind::Rust(Mutability::Mut), l, t)
            if matches!(**t, ParsedRustType::CppConst(_)) =>
        {
            let l = l.map(|l| format!("'{l} ")).unwrap_or_default();
            Err(format!(
                "references to C++ objects say `unique` rather than `mut`: write \
                 `&{l}unique const T` (or `&{l}const T` for a shared reference)"
            ))
        }
        ParsedRustType::Ref(_, _, t)
        | ParsedRustType::Raw(_, t)
        | ParsedRustType::Boxed(t)
        | ParsedRustType::Slice(t) => check_cpp_const(t, false),
        ParsedRustType::Tuple(tys) => all(tys),
        ParsedRustType::Dyn(tr, _) | ParsedRustType::Impl(tr, _) => match tr {
            ParsedRustTrait::Normal(pg) => {
                pg.generics
                    .iter()
                    .try_for_each(|t| check_cpp_const(t, false))?;
                pg.named_generics
                    .iter()
                    .try_for_each(|(_, t)| check_cpp_const(t, false))
            }
            ParsedRustTrait::Fn { inputs, output, .. } => {
                all(inputs)?;
                check_cpp_const(output, false)
            }
        },
        ParsedRustType::Adt(pg) => {
            // Whether `pg.path` resolves to `c++::` or a Rust ADT is checked once `Scope` resolves
            // `use` aliases and `mod c++::` scopes (see `ParsedRustPathAndGenerics::to_zngur`).
            pg.generics
                .iter()
                .try_for_each(|t| check_cpp_const(t, true))?;
            pg.named_generics
                .iter()
                .try_for_each(|(_, t)| check_cpp_const(t, false))
        }
    }
}

/// A generic argument of a Rust path: `'a`, `T` or `Name = T`.
#[derive(Debug, Clone)]
enum ParsedGenericArg<'a> {
    Lifetime(&'a str),
    Type(ParsedRustType<'a>),
    Named(&'a str, ParsedRustType<'a>),
}

fn rust_generics<'a>(
    rust_type: Boxed<'a, 'a, ParserInput<'a>, ParsedRustType<'a>, ZngParserExtra<'a>>,
) -> impl Parser<'a, ParserInput<'a>, Vec<ParsedGenericArg<'a>>, ZngParserExtra<'a>> + Clone {
    let named_generic = select! {
        Token::Ident(c) => c,
    }
    .then_ignore(just(Token::Eq))
    .then(rust_type.clone())
    .map(|(name, ty)| ParsedGenericArg::Named(name, ty));
    let lifetime = select! { Token::Lifetime(l) => ParsedGenericArg::Lifetime(l) };
    just(Token::ColonColon)
        .repeated()
        .at_most(1)
        .ignore_then(
            choice((
                lifetime,
                named_generic,
                rust_type.clone().map(ParsedGenericArg::Type),
            ))
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::AngleOpen), just(Token::AngleClose))
            .boxed(),
        )
        .boxed()
}

fn rust_path_and_generics<'a>(
    rust_type: Boxed<'a, 'a, ParserInput<'a>, ParsedRustType<'a>, ZngParserExtra<'a>>,
) -> impl Parser<'a, ParserInput<'a>, ParsedRustPathAndGenerics<'a>, ZngParserExtra<'a>> + Clone {
    let generics = rust_generics(rust_type.clone());
    path()
        .then(generics.clone().repeated().at_most(1).collect::<Vec<_>>())
        .try_map(|x, span| {
            let has_explicit_angle_brackets = !x.1.is_empty();
            let mut lifetimes = vec![];
            let mut generics = vec![];
            let mut named_generics = vec![];
            for arg in x.1.into_iter().next().unwrap_or_default() {
                match arg {
                    ParsedGenericArg::Lifetime(l)
                        if generics.is_empty() && named_generics.is_empty() =>
                    {
                        lifetimes.push(l)
                    }
                    ParsedGenericArg::Lifetime(_) => {
                        return Err(Rich::custom(
                            span,
                            "lifetime arguments must come before type arguments",
                        ));
                    }
                    ParsedGenericArg::Type(t) => generics.push(t),
                    ParsedGenericArg::Named(n, t) => named_generics.push((n, t)),
                }
            }
            Ok(ParsedRustPathAndGenerics {
                path: x.0,
                lifetimes,
                generics,
                named_generics,
                has_explicit_angle_brackets,
                span,
            })
        })
        .boxed()
}

/// Required lifetime parameters, `<'a, 'b>` (without the `'`).
fn required_lifetime_params<'a>()
-> impl Parser<'a, ParserInput<'a>, Vec<&'a str>, ZngParserExtra<'a>> + Clone {
    select! { Token::Lifetime(l) => l }
        .separated_by(just(Token::Comma))
        .at_least(1)
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::AngleOpen), just(Token::AngleClose))
        .boxed()
}

/// Optional lifetime parameters, `<'a, 'b>` (without the `'`), or nothing.
fn lifetime_params<'a>()
-> impl Parser<'a, ParserInput<'a>, Vec<&'a str>, ZngParserExtra<'a>> + Clone {
    required_lifetime_params()
        .or_not()
        .map(Option::unwrap_or_default)
        .boxed()
}

fn fn_args<'a>(
    rust_type: Boxed<'a, 'a, ParserInput<'a>, ParsedRustType<'a>, ZngParserExtra<'a>>,
) -> impl Parser<'a, ParserInput<'a>, (Vec<ParsedRustType<'a>>, ParsedRustType<'a>), ZngParserExtra<'a>>
+ Clone {
    rust_type
        .clone()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
        .then(
            just(Token::Arrow)
                .ignore_then(rust_type)
                .or(empty().to(ParsedRustType::Tuple(vec![]))),
        )
        .boxed()
}

fn spanned<'a, T>(
    parser: impl Parser<'a, ParserInput<'a>, T, ZngParserExtra<'a>> + Clone,
) -> impl Parser<'a, ParserInput<'a>, Spanned<T>, ZngParserExtra<'a>> + Clone {
    parser.map_with(|inner, extra| Spanned {
        inner,
        span: extra.span(),
    })
}

fn rust_trait<'a>(
    rust_type: Boxed<'a, 'a, ParserInput<'a>, ParsedRustType<'a>, ZngParserExtra<'a>>,
) -> impl Parser<'a, ParserInput<'a>, ParsedRustTrait<'a>, ZngParserExtra<'a>> + Clone {
    let for_binder = just(Token::KwFor)
        .ignore_then(required_lifetime_params())
        .or_not()
        .map(Option::unwrap_or_default);
    let fn_trait = for_binder
        .then(select! {
            Token::Ident(c) => c,
        })
        .then(fn_args(rust_type.clone()))
        .map(
            |((lifetimes, name), (inputs, output))| ParsedRustTrait::Fn {
                name,
                lifetimes,
                inputs,
                output: Box::new(output),
            },
        );

    let rust_trait = fn_trait.or(rust_path_and_generics(rust_type).map(ParsedRustTrait::Normal));
    rust_trait.boxed()
}

fn fn_generic_params<'a>()
-> impl Parser<'a, ParserInput<'a>, (Vec<&'a str>, Vec<ParsedRustType<'a>>), ZngParserExtra<'a>> + Clone
{
    select! { Token::Lifetime(l) => Either::Left(l) }
        .or(rust_type().map(Either::Right))
        .separated_by(just(Token::Comma))
        .at_least(1)
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::AngleOpen), just(Token::AngleClose))
        .or_not()
        .map(Option::unwrap_or_default)
        .try_map(|params, span| {
            let lifetimes: Vec<&str> = params
                .iter()
                .map_while(|p| p.as_ref().left().copied())
                .collect();
            let generics: Vec<ParsedRustType<'_>> = params
                .into_iter()
                .skip(lifetimes.len())
                .map(|p| {
                    p.right().ok_or_else(|| {
                        Rich::custom(span, "lifetime parameters must come before type arguments")
                    })
                })
                .collect::<Result<_, _>>()?;
            Ok((lifetimes, generics))
        })
        .boxed()
}

fn method<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedMethod<'a>, ZngParserExtra<'a>> + Clone {
    choice((
        just(Token::KwSafe).to(true),
        just(Token::KwUnsafe).to(false),
    ))
    .or_not()
    .then(method_signature())
    .map(|(is_safe, mut method)| {
        method.is_safe = is_safe;
        method
    })
    .boxed()
}

fn method_signature<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedMethod<'a>, ZngParserExtra<'a>> + Clone {
    spanned(just(Token::KwAsync))
        .or_not()
        .then_ignore(just(Token::KwFn))
        .then(select! {
            Token::Ident(c) => c,
        })
        .then(fn_generic_params())
        .then(fn_args(rust_type()))
        .try_map(|(((opt_async, name), (lifetimes, generics)), args), span| {
            parsed_method_from_signature(
                name,
                opt_async.map(|x| x.span),
                lifetimes,
                generics,
                args,
                span,
            )
        })
        .boxed()
}

fn parsed_method_from_signature<'a>(
    name: &'a str,
    async_span: Option<Span>,
    lifetimes: Vec<&'a str>,
    generics: Vec<ParsedRustType<'a>>,
    args: (Vec<ParsedRustType<'a>>, ParsedRustType<'a>),
    span: Span,
) -> Result<ParsedMethod<'a>, Rich<'a, Token<'a>, Span>> {
    let is_self = |ty: &ParsedRustType<'_>| {
        if let ParsedRustType::Adt(ty) = ty {
            ty.path.start == ParsedPathStart::Relative
                && &ty.path.segments == &["self"]
                && ty.lifetimes.is_empty()
                && ty.generics.is_empty()
                && ty.named_generics.is_empty()
        } else {
            false
        }
    };
    let (inputs, receiver, receiver_lifetime) = match args.0.first() {
        Some(ty) if is_self(ty) => (args.0[1..].to_vec(), ZngurMethodReceiver::Move, None),
        Some(ParsedRustType::Ref(ParsedRefKind::Rust(mutability), l, ty)) if is_self(ty) => (
            args.0[1..].to_vec(),
            ZngurMethodReceiver::Ref(*mutability),
            *l,
        ),
        Some(ParsedRustType::Ref(ParsedRefKind::Cpp { .. }, _, ty)) if is_self(ty) => {
            return Err(Rich::custom(
                span,
                "methods of Rust types and `extern \"C++\"` impls take `&self` or \
                 `&mut self`, not `&const self` / `&var self`",
            ));
        }
        _ => (args.0, ZngurMethodReceiver::Static, None),
    };
    let mut output = args.1;
    if let Some(async_span) = async_span {
        output = ParsedRustType::Impl(
            ParsedRustTrait::Normal(ParsedRustPathAndGenerics {
                path: ParsedPath {
                    start: ParsedPathStart::Absolute,
                    segments: vec!["std", "future", "Future"],
                    span: async_span,
                },
                lifetimes: vec![],
                generics: vec![],
                named_generics: vec![("Output", output)],
                has_explicit_angle_brackets: true,
                span: async_span,
            }),
            vec![],
        )
    }
    Ok(ParsedMethod {
        name,
        lifetimes,
        is_safe: None,
        receiver,
        receiver_lifetime,
        generics,
        inputs,
        output,
    })
}

fn inner_type_item<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedTypeItem<'a>, ZngParserExtra<'a>> + Clone {
    let property_item = (spanned(select! {
        Token::Ident(c) => c,
    }))
    .then_ignore(just(Token::Eq))
    .then(select! {
        Token::Number(c) => c,
    });
    let layout = just([Token::Sharp, Token::Ident("layout")])
        .ignore_then(
            property_item
                .clone()
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .boxed(),
        )
        .map(ParsedLayoutPolicy::StackAllocated)
        .or(just([Token::Sharp, Token::Ident("layout_conservative")])
            .ignore_then(
                property_item
                    .clone()
                    .separated_by(just(Token::Comma))
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                    .boxed(),
            )
            .map(ParsedLayoutPolicy::Conservative))
        .or(just([Token::Sharp, Token::Ident("only_by_ref")]).to(ParsedLayoutPolicy::OnlyByRef))
        .or(just([Token::Sharp, Token::Ident("heap_allocated")])
            .to(ParsedLayoutPolicy::HeapAllocated))
        .map_with(|x, extra| ParsedTypeItem::Layout(extra.span(), x))
        .boxed();
    let trait_item = select! {
        Token::Ident("Debug") => ZngurWellknownTrait::Debug,
        Token::Ident("Copy") => ZngurWellknownTrait::Copy,
    }
    .or(just(Token::Question)
        .then(just(Token::Ident("Sized")))
        .to(ZngurWellknownTrait::Unsized));
    let traits = just(Token::Ident("wellknown_traits"))
        .ignore_then(
            spanned(trait_item)
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .boxed(),
        )
        .map(ParsedTypeItem::Traits)
        .boxed();
    let non_exhaustive =
        just(Token::Ident("non_exhaustive")).map(|_| ParsedTypeItem::NonExhaustive);
    let constructor_args = rust_type()
        .separated_by(just(Token::Comma))
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
        .map(ParsedConstructorArgs::Tuple)
        .or((select! {
            Token::Ident(c) => c,
        })
        .boxed()
        .then_ignore(just(Token::Colon))
        .then(rust_type())
        .separated_by(just(Token::Comma))
        .collect::<Vec<_>>()
        .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
        .map(ParsedConstructorArgs::Named))
        .or(empty().to(ParsedConstructorArgs::Unit))
        .boxed();
    let constructor = just(Token::Ident("constructor"))
        .ignore_then(lifetime_params())
        .then(constructor_args)
        .then(just(Token::KwAs).ignore_then(cpp_name()).or_not())
        .map(
            |((lifetimes, args), rust_name)| ParsedTypeItem::Constructor {
                lifetimes,
                args,
                rust_name,
            },
        );
    let field = just(Token::Ident("field")).ignore_then(
        (select! {
            Token::Ident(c) => c.to_owned(),
            Token::Number(c) => c.to_string(),
        })
        .then(
            just(Token::Ident("offset"))
                .then(just(Token::Eq))
                .ignore_then(select! {
                    Token::Number(c) => Some(c),
                    Token::Ident("auto") => None,
                })
                .then(
                    just(Token::Comma)
                        .then(just(Token::KwType))
                        .then(just(Token::Eq))
                        .ignore_then(rust_type()),
                )
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .boxed(),
        )
        .map(|(name, (offset, ty))| ParsedTypeItem::Field { name, ty, offset }),
    );
    // Unlike fields of Rust types, accessors for fields of C++ types have no offset (C++ knows
    // it), so they're told apart by `(type = ...)`.
    let cpp_field = just(Token::Ident("field"))
        .ignore_then(just(Token::And).or_not().map(|a| a.is_some()))
        .then(just(Token::Ident("unique")).or_not().map(|u| u.is_some()))
        .then(
            choice((
                just(Token::KwConst).to(CppConstness::Const),
                just(Token::Ident("var")).to(CppConstness::Var),
            ))
            .or_not(),
        )
        .then(cpp_name())
        .then(
            just(Token::KwType)
                .then(just(Token::Eq))
                .ignore_then(rust_type())
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose)),
        )
        .then(just(Token::KwAs).ignore_then(cpp_name()).or_not())
        .map(
            |(((((is_ref, unique), constness), name), ty), rust_name)| ParsedTypeItem::CppField {
                is_safe: true,
                is_ref,
                unique,
                constness,
                name,
                ty,
                rust_name,
            },
        );
    let cpp_value = just(Token::Sharp)
        .then(just(Token::Ident("cpp_value")))
        .ignore_then(select! {
            Token::Str(c) => c,
        })
        .then(select! {
            Token::Str(c) => c,
        })
        .map(|x| ParsedTypeItem::CppValue {
            field: x.0,
            cpp_type: x.1,
        });
    let cpp_heap_allocated = just(Token::Sharp)
        .then(just(Token::Ident("cpp_heap_allocated")))
        .ignore_then(
            select! {
                Token::Str(c) => c,
            }
            .or_not(),
        )
        .map(|cpp_type| ParsedTypeItem::CppHeapAllocated { cpp_type });
    let cpp_ref = just(Token::Sharp)
        .then(just(Token::Ident("cpp_ref")))
        .ignore_then(
            select! {
                Token::Str(c) => c,
            }
            .or_not(),
        )
        .map(|x| ParsedTypeItem::CppRef { cpp_type: x });
    let cpp_stack_owned = just(Token::Sharp)
        .then(just(Token::Ident("cpp_stack_owned")))
        .ignore_then(
            select! {
                Token::Str(c) => c,
            }
            .or_not(),
        )
        .then(
            property_item
                .clone()
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .or_not()
                .boxed(),
        )
        .map(|(cpp_type, props)| ParsedTypeItem::CppStackOwned { cpp_type, props });
    let trivially_relocatable = choice((
        just([Token::Sharp, Token::Ident("trivially_relocatable")]).to(false),
        just([
            Token::Sharp,
            Token::KwUnsafe,
            Token::ParenOpen,
            Token::Ident("trivially_relocatable"),
            Token::ParenClose,
        ])
        .to(true),
    ))
    .map(|unchecked| ParsedTypeItem::TriviallyRelocatable { unchecked });

    let variant = just(Token::Ident("variant"))
        .ignore_then(select! { Token::Ident(c) => c })
        .then(
            spanned(
                choice((non_exhaustive.clone(), field.clone())).then_ignore(just(Token::Semicolon)),
            )
            .repeated()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::BraceOpen), just(Token::BraceClose)),
        )
        .map(|(name, items)| ParsedTypeItem::Variant { name, items });

    recursive(|item| {
        // Functions and C++ constructors inside `type` blocks.
        let fn_params = rust_type()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::ParenOpen), just(Token::ParenClose));
        let cpp_constructor = just(Token::Ident("constructor"))
            .ignore_then(lifetime_params())
            .then(fn_params.clone())
            .then(just(Token::KwAs).ignore_then(cpp_name()).or_not())
            .map(
                |((lifetimes, inputs), rust_name)| ParsedTypeItem::CppConstructor {
                    is_safe: true,
                    lifetimes,
                    inputs,
                    rust_name,
                },
            );
        let safe_or_unsafe = choice((
            just(Token::KwSafe).to(true),
            just(Token::KwUnsafe).to(false),
        ))
        .then(choice((cpp_constructor, cpp_field.clone())))
        .map(|(safe, mut item)| {
            match &mut item {
                ParsedTypeItem::CppConstructor { is_safe, .. }
                | ParsedTypeItem::CppField { is_safe, .. } => *is_safe = safe,
                _ => unreachable!(),
            }
            item
        });
        let safety_and_static = choice((
            just(Token::KwSafe).to(true),
            just(Token::KwUnsafe).to(false),
        ))
        .then(just(Token::Ident("static")).or_not().map(|s| s.is_some()))
        .map(|(is_safe, is_static)| (Some(is_safe), is_static))
        .or(empty().to((None, false)));
        let type_fn_sig = safety_and_static
            .then(spanned(just(Token::KwAsync)).or_not())
            .then_ignore(just(Token::KwFn))
            .then(cpp_name_with_ident_flag())
            .then(fn_generic_params())
            .then(fn_params)
            .then(just(Token::KwConst).or_not().map(|c| c.is_some()))
            .then(just(Token::Arrow).ignore_then(rust_type()).or_not())
            .map_with(|sig, extra| (sig, extra.span()));
        let type_fn = type_fn_sig
            .then(
                just(Token::KwUse)
                    .ignore_then(path())
                    .map(Some)
                    .or(empty().to(None))
                    .boxed(),
            )
            .then(
                just(Token::Ident("deref"))
                    .ignore_then(rust_type())
                    .map(Some)
                    .or(empty().to(None))
                    .boxed(),
            )
            .then(
                just(Token::KwAs)
                    .ignore_then(cpp_name_with_ident_flag())
                    .or_not()
                    .boxed(),
            )
            .map(|((((sig, sig_span), use_path), deref), alias)| {
                let (
                    (
                        (
                            (
                                (((is_safe, is_static), opt_async), (name, name_is_ident)),
                                (lifetimes, generics),
                            ),
                            inputs,
                        ),
                        is_const,
                    ),
                    output,
                ) = sig;
                ParsedTypeItem::Fn {
                    is_safe,
                    is_static,
                    async_span: opt_async.map(|x| x.span),
                    name,
                    name_is_ident,
                    lifetimes,
                    generics,
                    inputs,
                    is_const,
                    output: output.unwrap_or(ParsedRustType::Tuple(vec![])),
                    sig_span,
                    use_path,
                    deref,
                    alias,
                }
            });
        let inner_item = choice((
            layout.boxed(),
            traits.boxed(),
            non_exhaustive.boxed(),
            constructor.boxed(),
            field.boxed(),
            cpp_field.boxed(),
            cpp_value.boxed(),
            cpp_heap_allocated.boxed(),
            cpp_ref.boxed(),
            cpp_stack_owned.boxed(),
            trivially_relocatable.boxed(),
            safe_or_unsafe.boxed(),
            type_fn.boxed(),
        ));

        let match_stmt = conditional_item::<_, CfgConditional<'a>, NItems>(item)
            .map(ParsedTypeItem::MatchOnCfg)
            .boxed();

        choice((
            match_stmt,
            variant,
            inner_item.then_ignore(just(Token::Semicolon)).boxed(),
        ))
    })
    .boxed()
}

fn type_item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    // `type<'a, T>`: lifetime parameters, then type variables (which make it a template).
    let params = select! { Token::Lifetime(l) => Either::Left(l) }
        .or(select! { Token::Ident(c) => Either::Right(ParsedTypeVar(c)) })
        .separated_by(just(Token::Comma))
        .at_least(1)
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::AngleOpen), just(Token::AngleClose))
        .or_not()
        .try_map(|params, span| {
            let params = params.unwrap_or_default();
            let lifetimes: Vec<&str> = params
                .iter()
                .map_while(|p| p.as_ref().left().copied())
                .collect();
            let mut type_vars = HashSet::new();
            for p in params.into_iter().skip(lifetimes.len()) {
                let tv = p.right().ok_or_else(|| {
                    Rich::custom(span, "lifetime parameters must come before type variables")
                })?;
                let name = tv.0;
                if !type_vars.insert(tv) {
                    return Err(Rich::custom(
                        span,
                        format!("type parameter `{name}` is declared twice"),
                    ));
                }
            }
            Ok((lifetimes, (!type_vars.is_empty()).then_some(type_vars)))
        });
    just(Token::KwType)
        .ignore_then(params)
        .then(spanned(rust_type()))
        // Templates of C++ class templates (`type<T> c++::std::unique_ptr<T>`) are stable;
        // templates of Rust types aren't yet (checked after `mod`/`use` scope resolution).
        .map_with(
            |((lifetimes, type_vars), ty), e: &mut MapExtra<_, ZngParserExtra>| {
                (
                    lifetimes,
                    type_vars,
                    ty,
                    e.state().unstable_features.template_types,
                )
            },
        )
        .then(
            spanned(inner_type_item())
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
                .boxed(),
        )
        .map(
            |((lifetimes, type_vars, ty, unstable_template_types), items)| ParsedItem::Type {
                ty,
                items,
                lifetimes,
                type_vars,
                unstable_template_types,
            },
        )
        .boxed()
}

fn trait_item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone
{
    just(Token::KwTrait)
        .ignore_then(spanned(rust_trait(rust_type())))
        .then(
            spanned(method())
                .then_ignore(just(Token::Semicolon))
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
                .boxed(),
        )
        .map(|(tr, methods)| ParsedItem::Trait { tr, methods })
        .boxed()
}

fn fn_item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    let safety = choice((
        just(Token::KwSafe).to(true),
        just(Token::KwUnsafe).to(false),
    ))
    .or_not();
    let start = choice((
        just(Token::ColonColon).to(ParsedPathStart::Absolute),
        just([Token::KwCrate, Token::ColonColon]).to(ParsedPathStart::Crate),
        just([Token::CppPathStart, Token::ColonColon]).to(ParsedPathStart::Cpp),
        empty().to(ParsedPathStart::Relative),
    ));
    let fn_path = start
        .then(
            cpp_name_with_ident_flag()
                .separated_by(just(Token::ColonColon))
                .at_least(1)
                .collect::<Vec<_>>()
                .boxed(),
        )
        .try_map_with(|(start, segs), extra| {
            for &(seg, is_ident) in &segs[..segs.len() - 1] {
                if !is_ident {
                    return Err(Rich::custom(
                        extra.span(),
                        format!("expected identifier in path, found keyword `{seg}`"),
                    ));
                }
            }
            let name_is_ident = segs.last().expect("at_least(1)").1;
            let segments = segs.into_iter().map(|(s, _)| s).collect();
            Ok((
                ParsedPath {
                    start,
                    segments,
                    span: extra.span(),
                },
                name_is_ident,
            ))
        });
    let sig = spanned(
        spanned(just(Token::KwAsync))
            .or_not()
            .then_ignore(just(Token::KwFn))
            .then(fn_path)
            .then(fn_generic_params())
            .then(fn_args(rust_type()))
            .then(
                just(Token::KwAs)
                    .ignore_then(cpp_name_with_ident_flag())
                    .or_not(),
            ),
    );
    spanned(safety.then(sig))
        .then_ignore(just(Token::Semicolon))
        .map(|outer| {
            let (is_safe, sig) = outer.inner;
            let sig_span = sig.span;
            let (
                (((opt_async, (path, name_is_ident)), (lifetimes, generics)), (inputs, output)),
                alias,
            ) = sig.inner;
            ParsedItem::Fn(Spanned {
                inner: ParsedFunction {
                    is_safe,
                    async_span: opt_async.map(|x| x.span),
                    path,
                    name_is_ident,
                    lifetimes,
                    generics,
                    inputs,
                    output,
                    alias,
                    sig_span,
                },
                span: outer.span,
            })
        })
        .boxed()
}

fn word_token<'a>() -> impl Parser<'a, ParserInput<'a>, &'a str, ZngParserExtra<'a>> + Clone {
    select! {
        Token::Ident(x) => x,
        Token::KwAs => "as",
        Token::KwAsync => "async",
        Token::KwDyn => "dyn",
        Token::KwMod => "mod",
        Token::KwType => "type",
        Token::KwTrait => "trait",
        Token::KwCrate => "crate",
        Token::KwFn => "fn",
        Token::KwMut => "mut",
        Token::KwConst => "const",
        Token::KwUse => "use",
        Token::KwFor => "for",
        Token::KwExtern => "extern",
        Token::KwImpl => "impl",
        Token::KwImport => "import",
        Token::KwMerge => "merge",
        Token::KwIf => "if",
        Token::KwElse => "else",
        Token::KwMatch => "match",
        Token::KwSafe => "safe",
        Token::KwUnsafe => "unsafe",
    }
}

/// The name of a function or member function: an identifier (`true`), or a word that is a zngur
/// keyword (`false`) which can be a C++ name (e.g. a member function named `type`).
fn cpp_name_with_ident_flag<'a>()
-> impl Parser<'a, ParserInput<'a>, (&'a str, bool), ZngParserExtra<'a>> + Clone {
    select! {
        Token::Ident(c) => (c, true),
        Token::KwAs => ("as", false),
        Token::KwAsync => ("async", false),
        Token::KwCrate => ("crate", false),
        Token::KwDyn => ("dyn", false),
        Token::KwFn => ("fn", false),
        Token::KwImpl => ("impl", false),
        Token::KwImport => ("import", false),
        Token::KwMatch => ("match", false),
        Token::KwMerge => ("merge", false),
        Token::KwMod => ("mod", false),
        Token::KwMut => ("mut", false),
        Token::KwSafe => ("safe", false),
        Token::KwTrait => ("trait", false),
        Token::KwType => ("type", false),
        Token::KwUnsafe => ("unsafe", false),
        Token::KwUse => ("use", false),
    }
}

fn cpp_name<'a>() -> impl Parser<'a, ParserInput<'a>, &'a str, ZngParserExtra<'a>> + Clone {
    cpp_name_with_ident_flag().map(|(s, _)| s)
}

fn additional_include_item<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    just(Token::Sharp)
        .ignore_then(choice((
            just(Token::Ident("cpp_additional_includes"))
                .ignore_then(select! {
                    Token::Str(c) => ParsedItem::CppAdditionalInclude(c),
                    Token::RawStr(_, c) => ParsedItem::CppAdditionalInclude(c),
                })
                .boxed(),
            just(Token::Ident("convert_panic_to_exception"))
                .map_with(|_, extra| ParsedItem::ConvertPanicToException(extra.span()))
                .boxed(),
            just(Token::KwCrate)
                .ignore_then(word_token())
                .map_with(|name, extra| ParsedItem::CrateName(name, extra.span()))
                .boxed(),
        )))
        .boxed()
}

fn extern_cpp_item<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    let safety = choice((
        just(Token::KwSafe).to(true),
        just(Token::KwUnsafe).to(false),
    ));
    let function = safety
        .clone()
        .then(spanned(method_signature()))
        .then_ignore(just(Token::Semicolon))
        .map(|(is_safe, method)| ParsedExternCppItem::Function { is_safe, method });
    let impl_block = just(Token::KwImpl)
        .ignore_then(lifetime_params())
        .then(
            rust_trait(rust_type())
                .then_ignore(just(Token::KwFor))
                .map(Some)
                .or(empty().to(None))
                .then(spanned(rust_type()))
                .boxed(),
        )
        .then(
            safety
                .then(spanned(method_signature()))
                .then_ignore(just(Token::Semicolon))
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
                .boxed(),
        )
        .map(
            |((lifetimes, (tr, ty)), methods)| ParsedExternCppItem::Impl {
                lifetimes,
                tr,
                ty,
                methods,
            },
        );
    just(Token::KwExtern)
        .then(just(Token::Str("C++")))
        .ignore_then(
            function
                .or(impl_block)
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
                .boxed(),
        )
        .map(ParsedItem::ExternCpp)
        .boxed()
}

/// The warnings that `#allow(...)` can silence.
const KNOWN_LINTS: &[&str] = &["elided_lifetimes", "safe_raw_pointer"];

fn allow_item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone
{
    just([Token::Sharp, Token::Ident("allow")])
        .ignore_then(
            select! { Token::Ident(lint) => lint }
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose)),
        )
        .map_with(|lint, extra| ParsedItem::Allow(lint, extra.span()))
        .boxed()
}

fn unstable_feature<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    just([Token::Sharp, Token::Ident("unstable")])
        .ignore_then(
            select! { Token::Ident(feat) => feat }
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .try_map_with(|feat, e| match feat {
                    "cfg_match" => {
                        let ctx: &mut extra::SimpleState<ZngParserState> = e.state();
                        ctx.unstable_features.cfg_match = true;
                        Ok(ParsedItem::UnstableFeature("cfg_match"))
                    }
                    "cfg_if" => {
                        let ctx: &mut extra::SimpleState<ZngParserState> = e.state();
                        ctx.unstable_features.cfg_if = true;
                        Ok(ParsedItem::UnstableFeature("cfg_if"))
                    }
                    "template_types" => {
                        let ctx: &mut extra::SimpleState<ZngParserState> = e.state();
                        ctx.unstable_features.template_types = true;
                        Ok(ParsedItem::UnstableFeature("template_types"))
                    }
                    _ => Err(Rich::custom(
                        e.span(),
                        format!("unknown unstable feature '{feat}'"),
                    )),
                }),
        )
        .boxed()
}

fn item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    recursive(|item| {
        choice((
            unstable_feature(),
            allow_item(),
            just(Token::KwMod)
                .ignore_then(mod_path())
                .then(
                    item.clone()
                        .repeated()
                        .collect::<Vec<_>>()
                        .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
                        .boxed(),
                )
                .map(|(path, items)| ParsedItem::Mod { path, items })
                .boxed(),
            type_item(),
            trait_item(),
            extern_cpp_item(),
            fn_item(),
            additional_include_item(),
            import_item(),
            module_import_item(),
            alias(),
            conditional_item::<_, CfgConditional<'a>, NItems>(item).map(ParsedItem::MatchOnCfg),
        ))
    })
    .boxed()
}

fn import_item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone
{
    just(Token::KwMerge)
        .ignore_then(select! {
            Token::Str(path) => path,
        })
        .then_ignore(just(Token::Semicolon))
        .map_with(|path, extra| {
            ParsedItem::Import(ParsedImportPath {
                path: std::path::PathBuf::from(path),
                span: extra.span(),
            })
        })
        .boxed()
}

fn module_import_item<'a>()
-> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone {
    just(Token::KwImport)
        .ignore_then(select! { Token::Str(path) => path })
        .then_ignore(just(Token::Semicolon))
        .map_with(|path, extra| ParsedItem::ModuleImport {
            path: std::path::PathBuf::from(path),
            span: extra.span(),
        })
        .boxed()
}

fn mod_path<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedPath<'a>, ZngParserExtra<'a>> + Clone {
    path()
        .or(just(Token::CppPathStart).map_with(|_, extra| ParsedPath {
            start: ParsedPathStart::Cpp,
            segments: vec![],
            span: extra.span(),
        }))
        .boxed()
}

fn path<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedPath<'a>, ZngParserExtra<'a>> + Clone {
    let start = choice((
        just(Token::ColonColon).to(ParsedPathStart::Absolute),
        just(Token::KwCrate)
            .then(just(Token::ColonColon))
            .to(ParsedPathStart::Crate),
        just(Token::CppPathStart)
            .then(just(Token::ColonColon))
            .to(ParsedPathStart::Cpp),
        empty().to(ParsedPathStart::Relative),
    ));

    start
        .then(
            (select! {
                Token::Ident(c) => c,
            })
            .separated_by(just(Token::ColonColon))
            .at_least(1)
            .collect::<Vec<_>>()
            .boxed(),
        )
        .or(just(Token::KwCrate).to((ParsedPathStart::Crate, vec![])))
        .map_with(|(start, segments), extra| ParsedPath {
            start,
            segments,
            span: extra.span(),
        })
        .boxed()
}

impl<'a> conditional::BodyItem for crate::ParsedTypeItem<'a> {
    type Processed = Self;

    fn process(self, _ctx: &mut ParseContext) -> Self::Processed {
        self
    }
}

impl<'a> conditional::BodyItem for crate::ParsedItem<'a> {
    type Processed = ProcessedItemOrAlias<'a>;

    fn process(self, ctx: &mut ParseContext) -> Self::Processed {
        crate::process_parsed_item(self, ctx)
    }
}
