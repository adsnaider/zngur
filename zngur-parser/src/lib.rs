use std::{
    collections::HashSet,
    fmt::Display,
    ops::{Deref, DerefMut},
    path::Component,
};

use ariadne::{Color, Label, Report, ReportKind};
use chumsky::{input::MapExtra, prelude::*};
use itertools::{Either, Itertools};

use zngur_def::{
    AdditionalIncludes, ConflictSource, ConvertPanicToException, CppIndirection, CppTemplateHome,
    CppTypePath, Import, LayoutPolicy, Merge, MergeFailure, ModuleImport, Mutability,
    PrimitiveRustType, RustPathAndGenerics, RustTrait, RustType, TriviallyRelocatable, TypeVar,
    ZngurConstructor, ZngurCppConstructor, ZngurCppField, ZngurCppFn, ZngurCppMethod,
    ZngurExternCppFn, ZngurExternCppImpl, ZngurField, ZngurFn, ZngurMethod, ZngurMethodDetails,
    ZngurMethodReceiver, ZngurSpec, ZngurTrait, ZngurType, ZngurVariant, ZngurWellknownTrait,
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
    template_types::try_match_template,
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
}

#[derive(Debug, Clone)]
struct Scope<'a> {
    aliases: Vec<ParsedAlias<'a>>,
    base: EntityPath,
    type_vars: HashSet<ParsedTypeVar<'a>>,
}

impl<'a> Scope<'a> {
    /// Create a new root scope containing the specified aliases.
    fn new_root(aliases: Vec<ParsedAlias<'a>>) -> Scope<'a> {
        Scope {
            aliases,
            base: EntityPath::Rust(Default::default()),
            type_vars: Default::default(),
        }
    }

    /// Resolve a path according to the current scope.
    fn resolve_path(&self, path: ParsedPath<'a>) -> EntityPath {
        // Check to see if the path refers to an alias:
        if let Some(expanded_alias) = self
            .aliases
            .iter()
            .find_map(|alias| alias.expand(&path, &self.base))
        {
            expanded_alias
        } else {
            path.to_zngur(&self.base)
        }
    }

    /// Return rust source text that references `target` from within `self`.
    fn reference_to(&self, target: &EntityPath) -> Option<String> {
        match (target, &self.base) {
            // Rust paths don't use self.base
            (EntityPath::Rust(v), _) => Some(
                v.iter()
                    .map(|s| {
                        if s == "crate" {
                            s.clone()
                        } else {
                            format!("::{s}")
                        }
                    })
                    .collect(),
            ),
            // Cannot reference a Cpp path from a Rust path because we don't
            // know where the generated rust code will end up.
            (EntityPath::Cpp(_), EntityPath::Rust(_)) => None,
            // Cpp paths can reference other Cpp paths, but super may be needed.
            (EntityPath::Cpp(target_segs), EntityPath::Cpp(self_segs)) => {
                let common = self_segs
                    .iter()
                    .zip(target_segs)
                    .take_while(|(a, b)| a == b)
                    .count();
                let ups = std::iter::repeat("super").take(self_segs.len() - common);
                let downs = target_segs[common..].iter().map(String::as_str);
                Some(ups.chain(downs).join("::"))
            }
        }
    }

    fn sub_scope(&self, new_aliases: &[ParsedAlias<'a>], nested_path: ParsedPath<'a>) -> Scope<'_> {
        let base = nested_path.to_zngur(&self.base);
        let mut mod_aliases = new_aliases.to_vec();
        mod_aliases.extend_from_slice(&self.aliases);

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
            generics,
            named_generics,
        } = ty
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

    fn matches_alias(&self, alias: &ParsedAlias<'_>) -> bool {
        self.start == ParsedPathStart::Relative
            && self
                .segments
                .first()
                .is_some_and(|part| *part == alias.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAlias<'a> {
    name: &'a str,
    path: ParsedPath<'a>,
    span: Span,
}

impl ParsedAlias<'_> {
    fn expand(&self, path: &ParsedPath<'_>, base: &EntityPath) -> Option<EntityPath> {
        if path.matches_alias(self) {
            let extra = path.segments.iter().skip(1).copied();
            Some(self.path.clone().to_zngur(base).join(extra))
        } else {
            None
        }
    }
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
    Mod {
        path: ParsedPath<'a>,
        items: Vec<ParsedItem<'a>>,
    },
    Type {
        ty: Spanned<ParsedRustType<'a>>,
        items: Vec<Spanned<ParsedTypeItem<'a>>>,
        type_vars: Option<HashSet<ParsedTypeVar<'a>>>,
    },
    Trait {
        tr: Spanned<ParsedRustTrait<'a>>,
        methods: Vec<ParsedMethod<'a>>,
    },
    Fn(Spanned<ParsedMethod<'a>>),
    CppFn(Spanned<ParsedCppFn<'a>>),
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
        type_vars: Option<HashSet<ParsedTypeVar<'a>>>,
    },
    Trait {
        tr: Spanned<ParsedRustTrait<'a>>,
        methods: Vec<ParsedMethod<'a>>,
    },
    Fn(Spanned<ParsedMethod<'a>>),
    CppFn(Spanned<ParsedCppFn<'a>>),
    ExternCpp(Vec<ParsedExternCppItem<'a>>),
    Import(ParsedImportPath),
    ModuleImport {
        path: std::path::PathBuf,
        span: Span,
    },
}

/// A free C++ function, declared in a `c++::` scope:
/// `(safe | unsafe) fn name[<types...>](types...) [-> type] [as rust_name];`
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedCppFn<'a> {
    is_safe: bool,
    name: &'a str,
    template_args: Vec<ParsedRustType<'a>>,
    inputs: Vec<ParsedRustType<'a>>,
    output: ParsedRustType<'a>,
    rust_name: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedExternCppItem<'a> {
    Function {
        is_safe: bool,
        method: Spanned<ParsedMethod<'a>>,
    },
    Impl {
        tr: Option<ParsedRustTrait<'a>>,
        ty: Spanned<ParsedRustType<'a>>,
        methods: Vec<(bool, ParsedMethod<'a>)>,
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
        args: ParsedConstructorArgs<'a>,
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
    Method {
        data: ParsedMethod<'a>,
        use_path: Option<ParsedPath<'a>>,
        deref: Option<ParsedRustType<'a>>,
        cpp_name: Option<&'a str>,
    },
    /// A C++ member function, declared C++-style:
    /// `(safe | unsafe) [static] fn name[<types...>](types...) [const] [-> type] [as rust_name];`
    CppMethod {
        is_safe: bool,
        is_static: bool,
        name: &'a str,
        template_args: Vec<ParsedRustType<'a>>,
        inputs: Vec<ParsedRustType<'a>>,
        is_const: bool,
        output: ParsedRustType<'a>,
        rust_name: Option<&'a str>,
    },
    /// A C++ constructor: `(safe | unsafe) constructor(types...);`
    CppConstructor {
        is_safe: bool,
        inputs: Vec<ParsedRustType<'a>>,
    },
    /// An accessor for a field of a C++ type: `field [const] name (type = T) [as rust_name];`
    CppField {
        is_const: bool,
        name: &'a str,
        ty: ParsedRustType<'a>,
        rust_name: Option<&'a str>,
    },
    /// `#trivially_relocatable`, or `#unsafe(trivially_relocatable)` if `unchecked`.
    TriviallyRelocatable {
        unchecked: bool,
    },
    /// One of the removed `#cpp_*` directives (`name` is e.g. `cpp_stack_owned`).
    RemovedCppDirective {
        name: &'a str,
    },
    MatchOnCfg(Condition<CfgConditional<'a>, ParsedTypeItem<'a>, NItems>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ParsedTypeVar<'a>(&'a str);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedMethod<'a> {
    name: &'a str,
    receiver: ZngurMethodReceiver,
    generics: Vec<ParsedRustType<'a>>,
    inputs: Vec<ParsedRustType<'a>>,
    output: ParsedRustType<'a>,
}

impl ParsedMethod<'_> {
    fn to_zngur(self, scope: &Scope<'_>) -> ZngurMethod {
        ZngurMethod {
            name: self.name.to_owned(),
            generics: self
                .generics
                .into_iter()
                .map(|x| x.to_zngur(scope))
                .collect(),
            receiver: self.receiver,
            inputs: self.inputs.into_iter().map(|x| x.to_zngur(scope)).collect(),
            output: self.output.to_zngur(scope),
            is_safe: true,
        }
    }
}

struct MergeContext {
    outer_ty: RustType,
    variant: Option<String>,
}

/// Whether `ty` is or contains a C++ pointer (which may be null or invalid, unlike a C++
/// reference).
fn contains_cpp_ptr(ty: &RustType) -> bool {
    match ty {
        RustType::CppPtr(CppIndirection::Pointer, _) => true,
        RustType::CppPtr(CppIndirection::Reference, t)
        | RustType::CppConst(t)
        | RustType::Ref(_, t)
        | RustType::Raw(_, t)
        | RustType::Boxed(t)
        | RustType::Slice(t) => contains_cpp_ptr(t),
        RustType::Tuple(tys) => tys.iter().any(contains_cpp_ptr),
        RustType::Adt(pg) => pg
            .generics
            .iter()
            .chain(pg.named_generics.iter().map(|(_, t)| t))
            .any(contains_cpp_ptr),
        RustType::Cpp(path) => path.generics.iter().any(contains_cpp_ptr),
        _ => false,
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
    let (src_span, dst_span, first_span, rest) = if src_conflict == dst_conflict {
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
                    ParsedPathStart::Cpp => Some("c++::"),
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
                    let sub_scope = scope.sub_scope(&aliases, path);
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
                r.spec.imported_modules.push(ModuleImport {
                    path: path.clone(),
                    crate_name: None,
                    cpp_types: vec![],
                    cpp_by_value_types: vec![],
                    cpp_template_homes: vec![],
                });
                r.module_import_spans.push(span);
            }
            ProcessedItem::Type {
                ty,
                items,
                type_vars,
            } => {
                if ty.inner == ParsedRustType::Tuple(vec![]) {
                    // We add unit type implicitly.
                    ctx.add_error_str(
                        "Unit type is declared implicitly. Remove this entirely.",
                        ty.span,
                    );
                }

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
                            "align" => align = Some(value),
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
                let rust_ty = ty.inner.to_zngur(scope);
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
                                            for (msg, span) in errs {
                                                ctx.add_error_str(msg, span);
                                            }
                                            continue;
                                        }
                                    }
                                }
                                ParsedLayoutPolicy::Conservative(p) => match check_size_align(p) {
                                    Ok((size, align)) => LayoutPolicy::Conservative { size, align },
                                    Err(errs) => {
                                        for (msg, span) in errs {
                                            ctx.add_error_str(msg, span);
                                        }
                                        continue;
                                    }
                                },
                                ParsedLayoutPolicy::HeapAllocated => LayoutPolicy::HeapAllocated,
                                ParsedLayoutPolicy::OnlyByRef => LayoutPolicy::OnlyByRef,
                            };
                            if matches!(rust_ty, RustType::Cpp(_))
                                && matches!(
                                    l,
                                    LayoutPolicy::HeapAllocated | LayoutPolicy::OnlyByRef
                                )
                            {
                                ctx.add_error_str(
                                    "`#heap_allocated` and `#only_by_ref` are for Rust types; a \
                                     `c++::` type without a layout is already reference-only",
                                    span,
                                );
                                continue;
                            }
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
                        ParsedTypeItem::Constructor { args } => {
                            if constructor.is_some() {
                                ctx.add_error_str("Duplicate constructor found", item_span);
                            }
                            let c = ZngurConstructor {
                                inputs: match args {
                                    ParsedConstructorArgs::Unit => vec![],
                                    ParsedConstructorArgs::Tuple(t) => t
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, t)| (i.to_string(), t.to_zngur(scope)))
                                        .collect(),
                                    ParsedConstructorArgs::Named(t) => t
                                        .into_iter()
                                        .map(|(i, t)| (i.to_owned(), t.to_zngur(scope)))
                                        .collect(),
                                },
                            };
                            ctx.record_span(
                                &c.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            constructor = Some(c);
                        }
                        ParsedTypeItem::Variant { name, items } => {
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
                                            ty: ty.to_zngur(scope),
                                            offset,
                                        };
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
                            let field = ZngurField {
                                name: name.to_owned(),
                                ty: ty.to_zngur(scope),
                                offset,
                            };
                            ctx.record_span(
                                &field.into_span_key_with_variant(rust_ty.clone(), None),
                                item_span.into_range(),
                            );
                            fields.push(field);
                        }
                        ParsedTypeItem::CppMethod {
                            is_safe,
                            is_static,
                            name,
                            template_args,
                            inputs,
                            is_const,
                            output,
                            rust_name,
                        } => {
                            let receiver = match (is_static, is_const) {
                                (true, true) => {
                                    ctx.add_error_str(
                                        "a static member function can't be `const`",
                                        item_span,
                                    );
                                    continue;
                                }
                                (true, false) => ZngurMethodReceiver::Static,
                                (false, true) => ZngurMethodReceiver::Ref(Mutability::Not),
                                (false, false) => ZngurMethodReceiver::Ref(Mutability::Mut),
                            };
                            let inputs: Vec<RustType> =
                                inputs.into_iter().map(|x| x.to_zngur(scope)).collect();
                            if is_safe && inputs.iter().any(contains_cpp_ptr) {
                                ctx.add_error_str(
                                    "this C++ member function must be `unsafe`, because it takes a \
                                     C++ pointer, which may be null or invalid",
                                    item_span,
                                );
                                continue;
                            }
                            let method = ZngurCppMethod {
                                cpp_name: name.to_owned(),
                                template_args: template_args
                                    .into_iter()
                                    .map(|x| x.to_zngur(scope))
                                    .collect(),
                                rust_name: rust_name.map(|n| n.to_owned()),
                                receiver,
                                inputs,
                                output: output.to_zngur(scope),
                                is_safe,
                            };
                            ctx.record_span(
                                &method.as_rust_method().into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_methods.push(method);
                        }
                        ParsedTypeItem::CppConstructor { is_safe, inputs } => {
                            let inputs: Vec<RustType> =
                                inputs.into_iter().map(|x| x.to_zngur(scope)).collect();
                            if is_safe && inputs.iter().any(contains_cpp_ptr) {
                                ctx.add_error_str(
                                    "this C++ constructor must be `unsafe`, because it takes a C++ \
                                     pointer, which may be null or invalid",
                                    item_span,
                                );
                                continue;
                            }
                            // They'd implement the same `ZngCppConstruct<Args>`.
                            if cpp_constructors
                                .iter()
                                .any(|c: &ZngurCppConstructor| c.inputs == inputs)
                            {
                                ctx.add_error_str(
                                    "another C++ constructor has the same argument types",
                                    item_span,
                                );
                                continue;
                            }
                            let constructor = ZngurCppConstructor { inputs, is_safe };
                            ctx.record_span(
                                &PartialSpanKey::CppConstructor(
                                    rust_ty.clone(),
                                    constructor.inputs.clone(),
                                ),
                                item_span.into_range(),
                            );
                            cpp_constructors.push(constructor);
                        }
                        ParsedTypeItem::CppField {
                            is_const,
                            name,
                            ty,
                            rust_name,
                        } => {
                            let field = ZngurCppField {
                                name: name.to_owned(),
                                rust_name: rust_name.map(|n| n.to_owned()),
                                ty: ty.to_zngur(scope),
                                mutability: if is_const {
                                    Mutability::Not
                                } else {
                                    Mutability::Mut
                                },
                            };
                            ctx.record_span(
                                &field
                                    .as_cpp_method()
                                    .as_rust_method()
                                    .into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            cpp_fields.push(field);
                        }
                        ParsedTypeItem::Method {
                            data,
                            use_path,
                            deref,
                            cpp_name,
                        } => {
                            let deref = deref.and_then(|x| {
                                let deref_type = x.to_zngur(scope);
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
                                let span = x.span;
                                let target = scope.resolve_path(x);
                                let reference = scope.reference_to(&target);
                                if reference.is_none() {
                                    ctx.add_error_str(
                                        "cannot reference a c++::-only path from a Rust-only scope",
                                        span,
                                    );
                                }
                                reference
                            });
                            let method = ZngurMethodDetails {
                                data: data.to_zngur(scope),
                                use_path,
                                deref,
                                cpp_name: cpp_name.map(|s| s.to_owned()),
                            };
                            ctx.record_span(
                                &method.data.into_span_key_with(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            methods.push(method);
                        }
                        ParsedTypeItem::TriviallyRelocatable { unchecked } => {
                            if !matches!(rust_ty, RustType::Cpp(_)) {
                                ctx.add_error_str(
                                    "`#trivially_relocatable` is only for `c++::` types (Rust types \
                                     are always trivially relocatable)",
                                    item_span,
                                );
                                continue;
                            }
                            ctx.record_span(
                                &PartialSpanKey::TriviallyRelocatable(rust_ty.clone()),
                                item_span.into_range(),
                            );
                            trivially_relocatable = trivially_relocatable.max(if unchecked {
                                TriviallyRelocatable::Unchecked
                            } else {
                                TriviallyRelocatable::Checked
                            });
                        }
                        ParsedTypeItem::RemovedCppDirective { name } => {
                            let help = match name {
                                "cpp_heap_allocated" | "cpp_value" => {
                                    "declare the type as `type c++::ns::T { ... }` with no \
                                     layout, and own it through a C++ smart pointer, e.g. \
                                     `type c++::std::unique_ptr<c++::ns::T> { #layout(size = 8, \
                                     align = 8); #trivially_relocatable; }`"
                                }
                                "cpp_ref" => {
                                    "declare the type as `type c++::ns::T { ... }` with no \
                                     layout; Rust can then only refer to it"
                                }
                                _ => {
                                    "declare the type as `type c++::ns::T { #layout(size = X, \
                                     align = Y); #trivially_relocatable; }`; Rust holds C++ types \
                                     by value when they have a layout and are trivially \
                                     relocatable"
                                }
                            };
                            let report_span = ctx.report_span_for_range(item_span.into_range());
                            let report = Report::build(ReportKind::Error, report_span.clone())
                                .with_message(format!(
                                    "`#{name}` is no longer supported: C++ types are `c++::` \
                                     types, whose C++ name is their path"
                                ))
                                .with_label(
                                    Label::new(report_span)
                                        .with_message("no longer supported")
                                        .with_color(Color::Red),
                                )
                                .with_help(help)
                                .finish();
                            ctx.add_fatal_report(report);
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
                    trivially_relocatable,
                };
                if is_template {
                    r.templates.push(TemplateDef {
                        ty: zngur_type,
                        source_id: ctx.source_id(),
                        span: ty.span,
                        imported: false,
                    });
                } else {
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
                let trt = ZngurTrait {
                    tr: tr.inner.to_zngur(scope),
                    methods: methods.into_iter().map(|m| m.to_zngur(scope)).collect(),
                };
                ctx.record_span(&trt, tr.span.into_range());
                checked_merge(trt, &mut r.spec, tr.span, ctx, None);
            }
            ProcessedItem::Fn(f) => {
                let method = f.inner.to_zngur(scope);
                let path = match scope.base.child(&method.name) {
                    EntityPath::Rust(v) => v,
                    EntityPath::Cpp(_) => {
                        ctx.add_error_str(
                            "a free function cannot be declared inside a c++:: scope (declare C++ \
                             functions as `safe fn` or `unsafe fn`)",
                            f.span,
                        );
                        return;
                    }
                };
                let func = ZngurFn {
                    path: RustPathAndGenerics {
                        path,
                        generics: method.generics,
                        named_generics: vec![],
                    },
                    inputs: method.inputs,
                    output: method.output,
                };
                ctx.record_span(&func, f.span.into_range());
                checked_merge(func, &mut r.spec, f.span, ctx, None);
            }
            ProcessedItem::CppFn(f) => {
                let span = f.span;
                let f = f.inner;
                let namespace = match scope.base.child(f.name) {
                    EntityPath::Cpp(mut segments) => {
                        segments.pop();
                        segments
                    }
                    EntityPath::Rust(_) => {
                        ctx.add_error_str(
                            "C++ functions (`safe fn` / `unsafe fn`) can only be declared inside a \
                             `mod c++::...` scope, which is their C++ namespace",
                            span,
                        );
                        return;
                    }
                };
                if namespace.is_empty() {
                    ctx.add_error_str(
                        "C++ functions in the global namespace aren't supported",
                        span,
                    );
                    return;
                }
                let inputs: Vec<RustType> =
                    f.inputs.into_iter().map(|x| x.to_zngur(scope)).collect();
                if f.is_safe && inputs.iter().any(contains_cpp_ptr) {
                    ctx.add_error_str(
                        "this C++ function must be `unsafe`, because it takes a C++ pointer, \
                         which may be null or invalid",
                        span,
                    );
                    return;
                }
                let func = ZngurCppFn {
                    namespace,
                    cpp_name: f.name.to_owned(),
                    template_args: f
                        .template_args
                        .into_iter()
                        .map(|x| x.to_zngur(scope))
                        .collect(),
                    rust_name: f.rust_name.map(|n| n.to_owned()),
                    inputs,
                    output: f.output.to_zngur(scope),
                    is_safe: f.is_safe,
                };
                checked_merge(func, &mut r.spec, span, ctx, None);
            }
            ProcessedItem::ExternCpp(items) => {
                for item in items {
                    match item {
                        ParsedExternCppItem::Function { is_safe, method } => {
                            let span = method.span;
                            let method = method.inner.to_zngur(scope);
                            checked_merge(
                                ZngurExternCppFn {
                                    name: method.name.to_string(),
                                    inputs: method.inputs,
                                    output: method.output,
                                    is_safe,
                                },
                                &mut r.spec,
                                span,
                                ctx,
                                None,
                            );
                        }
                        ParsedExternCppItem::Impl { tr, ty, methods } => {
                            checked_merge(
                                ZngurExternCppImpl {
                                    tr: tr.map(|x| x.to_zngur(scope)),
                                    ty: ty.inner.to_zngur(scope),
                                    methods: methods
                                        .into_iter()
                                        .map(|(is_safe, x)| {
                                            let mut m = x.to_zngur(scope);
                                            m.is_safe = is_safe;
                                            m
                                        })
                                        .collect(),
                                },
                                &mut r.spec,
                                ty.span,
                                ctx,
                                None,
                            );
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
                if ctx.depth > 0 {
                    ctx.add_error_str(
                        "`#crate` can only be used in the top-level file of a zngur module, not in merged files",
                        span,
                    );
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedRustType<'a> {
    Primitive(PrimitiveRustType),
    Ref(Mutability, Box<ParsedRustType<'a>>),
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
    fn to_zngur(self, scope: &Scope<'_>) -> RustType {
        match self {
            ParsedRustType::Primitive(s) => RustType::Primitive(s),
            ParsedRustType::CppPtr(kind, s) => RustType::CppPtr(kind, Box::new(s.to_zngur(scope))),
            ParsedRustType::CppConst(s) => RustType::CppConst(Box::new(s.to_zngur(scope))),
            ParsedRustType::Ref(m, s) => RustType::Ref(m, Box::new(s.to_zngur(scope))),
            ParsedRustType::Raw(m, s) => RustType::Raw(m, Box::new(s.to_zngur(scope))),
            ParsedRustType::Boxed(s) => RustType::Boxed(Box::new(s.to_zngur(scope))),
            ParsedRustType::Slice(s) => RustType::Slice(Box::new(s.to_zngur(scope))),
            ParsedRustType::Dyn(tr, bounds) => RustType::Dyn(
                tr.to_zngur(scope),
                bounds.into_iter().map(|x| x.to_owned()).collect(),
            ),
            ParsedRustType::Impl(tr, bounds) => RustType::Impl(
                tr.to_zngur(scope),
                bounds.into_iter().map(|x| x.to_owned()).collect(),
            ),
            ParsedRustType::Tuple(v) => {
                RustType::Tuple(v.into_iter().map(|s| s.to_zngur(scope)).collect())
            }
            ParsedRustType::Adt(s) => match scope.as_type_var(&s) {
                Some(v) => RustType::TypeVar(v),
                None => match scope.resolve_path(s.path.clone()) {
                    // Named generics on `c++::` paths are rejected by the grammar.
                    EntityPath::Cpp(path) => RustType::Cpp(CppTypePath {
                        path,
                        generics: s.generics.into_iter().map(|x| x.to_zngur(scope)).collect(),
                    }),
                    EntityPath::Rust(_) => RustType::Adt(s.to_zngur(scope)),
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
        inputs: Vec<ParsedRustType<'a>>,
        output: Box<ParsedRustType<'a>>,
    },
}

impl ParsedRustTrait<'_> {
    fn to_zngur(self, scope: &Scope<'_>) -> RustTrait {
        match self {
            ParsedRustTrait::Normal(s) => RustTrait::Normal(s.to_zngur(scope)),
            ParsedRustTrait::Fn {
                name,
                inputs,
                output,
            } => RustTrait::Fn {
                name: name.to_owned(),
                inputs: inputs.into_iter().map(|s| s.to_zngur(scope)).collect(),
                output: Box::new(output.to_zngur(scope)),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedRustPathAndGenerics<'a> {
    path: ParsedPath<'a>,
    generics: Vec<ParsedRustType<'a>>,
    named_generics: Vec<(&'a str, ParsedRustType<'a>)>,
}

impl ParsedRustPathAndGenerics<'_> {
    fn to_zngur(self, scope: &Scope<'_>) -> RustPathAndGenerics {
        RustPathAndGenerics {
            path: match scope.resolve_path(self.path) {
                EntityPath::Rust(v) => v,
                EntityPath::Cpp(_) => todo!(
                    "a c++::-only path was used somewhere that can't support it (e.g. as a trait); needs a proper user-facing error instead of this panic"
                ),
            },
            generics: self
                .generics
                .into_iter()
                .map(|x| x.to_zngur(scope))
                .collect(),
            named_generics: self
                .named_generics
                .into_iter()
                .map(|(name, x)| (name.to_owned(), x.to_zngur(scope)))
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
    /// a `#trivially_relocatable` directive
    /// outer_ty
    TriviallyRelocatable(RustType),
    /// a [`Method`](zngur_def::ZngurMethod) on a
    /// [`Type`](zngur_def::ZngurType) or [`Trait`](zngur_def::ZngurTrait)
    /// outer_ty, method
    Method(RustType, ZngurMethod),
    /// a [`CppConstructor`](zngur_def::ZngurCppConstructor)
    /// outer_ty, inputs
    CppConstructor(RustType, Vec<RustType>),
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
        PartialSpanKey::Ty(self.ty.clone())
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
            zngur_def::ConflictSource::Layout => PartialSpanKey::Layout(outer_ty),
            zngur_def::ConflictSource::Constructor(sig) => {
                PartialSpanKey::Constructor(outer_ty, sig.clone())
            }
            zngur_def::ConflictSource::Method(method) => {
                PartialSpanKey::Method(outer_ty, method.clone())
            }
            zngur_def::ConflictSource::CppConstructor(inputs) => {
                PartialSpanKey::CppConstructor(outer_ty, inputs.clone())
            }
            zngur_def::ConflictSource::Field(name) => {
                PartialSpanKey::Field(outer_ty, variant, name.clone())
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
        PartialSpanKey::Field(outer_ty, variant, self.name.clone())
    }
}

impl QualifiedSpanKeyExt for zngur_def::LayoutPolicy {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::Layout(outer_ty)
    }
}

impl QualifiedSpanKeyExt for zngur_def::ZngurMethod {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::Method(outer_ty, self.clone())
    }
}

impl QualifiedSpanKeyExt for zngur_def::ZngurConstructor {
    fn into_span_key_with(&self, outer_ty: RustType) -> PartialSpanKey {
        PartialSpanKey::Constructor(outer_ty, Some(self.inputs.clone()))
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

#[derive(Debug, Default)]
struct ReportCounter {
    pub errors: usize,
    pub warnings: usize,
}

/// How many levels of `import`s are parsed: the module's imports, to find their `#crate`, `c++::`
/// types and the C++ class templates they're the home of, and their imports, to find the explicit
/// homes (`type<T> c++::...`) those modules see.
const MAX_MODULE_IMPORT_DEPTH: usize = 2;

/// The parse context. Holds references to the current source path and text and tracks
/// errors and declaration spans. Also holds the [configuration provider](RustCfgProvider)
/// and [report sink](ReportSink) used for this parse.
struct ParseContext<'this, 'source, 'cfg> {
    path: &'source std::path::Path,
    source: &'source str,
    source_id: SourceId,
    depth: usize,
    /// How many levels of `import`ed modules are still parsed (see `MAX_MODULE_IMPORT_DEPTH`).
    module_import_depth: usize,
    cfg_provider: &'cfg dyn RustCfgProvider,
    report_sink: &'cfg mut dyn ReportSink,
    /// All .zng files processed during parsing (main file + imports)
    processed_files: OwnedRefMut<'this, Vec<std::path::PathBuf>>,
    report_counter: OwnedRefMut<'this, ReportCounter>,
    sources: OwnedRefMut<'this, indexmap::IndexMap<std::path::PathBuf, ariadne::Source<String>>>,
    recorded_spans: OwnedRefMut<'this, indexmap::IndexMap<SpanKey, Vec<ReportSpan>>>,
}

impl<'this, 'source, 'cfg> ParseContext<'this, 'source, 'cfg> {
    fn new(
        path: &'source std::path::Path,
        source: &'source str,
        cfg: &'cfg dyn RustCfgProvider,
        report_sink: &'cfg mut dyn ReportSink,
    ) -> Self {
        let processed_files = OwnedRefMut::Owned(vec![path.to_path_buf()]);
        let mut sources = OwnedRefMut::Owned(indexmap::IndexMap::default());
        let (source_id, _) = sources.insert_full(
            path.to_path_buf(),
            ariadne::Source::from(source.to_string()),
        );
        Self {
            path,
            source_id,
            source,
            depth: 0,
            module_import_depth: MAX_MODULE_IMPORT_DEPTH,
            cfg_provider: cfg,
            report_sink,
            processed_files,
            report_counter: Default::default(),
            sources,
            recorded_spans: Default::default(),
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
            source_id,
            source,
            depth: self.depth + 1,
            module_import_depth: self.module_import_depth,
            cfg_provider: self.cfg_provider,
            report_sink: self.report_sink,
            processed_files: self.processed_files.into_borrowed(),
            report_counter: self.report_counter.into_borrowed(),
            sources: self.sources.into_borrowed(),
            recorded_spans: self.recorded_spans.into_borrowed(),
        }
    }

    /// build a parse context for the top-level file of an `import`ed module, which is parsed
    /// into its own spec (it's the top level of that module)
    fn nested_module<'borrowed, 'src>(
        &'borrowed mut self,
        path: &'src std::path::Path,
        source: &'src str,
    ) -> ParseContext<'borrowed, 'src, 'borrowed> {
        let module_import_depth = self.module_import_depth.saturating_sub(1);
        let mut ctx = self.nested(path, source);
        ctx.depth = 0;
        ctx.module_import_depth = module_import_depth;
        ctx
    }

    /// the [`SourceId`] of the current source which can be fetched from the cache
    /// retrieved by calling [`source_cache`](Self::source_cache)
    fn source_id(&self) -> SourceId {
        self.source_id
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
    #[allow(dead_code)] // No warnings at the moment.
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

impl ImportResolver for DefaultImportResolver {
    fn resolve_import(
        &self,
        cwd: &std::path::Path,
        relpath: &std::path::Path,
    ) -> Result<String, String> {
        let path = cwd
            .join(relpath)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        std::fs::read_to_string(path).map_err(|e| e.to_string())
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

impl<'a> ParsedZngFile<'a> {
    /// Parses the modules that the current file `import`s (from `first` on), to find their
    /// `#crate` and the `c++::` types they declare, which this module can then refer to.
    fn resolve_module_imports(
        zngur: &mut ZngurSpecBuilder,
        first: usize,
        ctx: &mut ParseContext,
        resolver: &impl ImportResolver,
    ) {
        let Some(dirname) = ctx.path.to_owned().parent().map(|d| d.to_owned()) else {
            return;
        };
        for index in first..zngur.spec.imported_modules.len() {
            let import_path = zngur.spec.imported_modules[index].path.clone();
            let span = zngur.module_import_spans[index];
            let Ok(text) = resolver.resolve_import(&dirname, &import_path) else {
                // Not an error: an `import` only has to name the generated header (found via
                // the C++ include path), so the `.zng` file may not be at this relative path.
                // Its `c++::` types just can't be referred to from this module then.
                continue;
            };
            let path = dirname.join(&import_path);
            let (imported, imported_templates) = {
                let mut module_ctx = ctx.nested_module(&path, &text);
                let mut module = ZngurSpecBuilder::default();
                Self::parse_into(&mut module, &mut module_ctx, resolver);
                // Templates of C++ class templates (`type<T> c++::...`) declared in the imported
                // module also apply to this module's instantiations. (Templates of Rust types
                // stay local to their module.)
                let cpp_templates: Vec<TemplateDef> = module
                    .templates
                    .iter()
                    .filter(|t| matches!(t.ty.ty, RustType::Cpp(_)) && !t.imported)
                    .cloned()
                    .collect();
                (module.to_zngur(&mut module_ctx), cpp_templates)
            };
            let cpp_types: Vec<Vec<String>> = imported
                .types
                .iter()
                .filter_map(|ty| match &ty.ty {
                    RustType::Cpp(path) if path.generics.is_empty() => Some(path.path.clone()),
                    _ => None,
                })
                .collect();
            let cpp_by_value_types: Vec<(RustType, String)> = imported
                .types
                .iter()
                .filter_map(|ty| {
                    ty.cpp_by_value_layout()?;
                    Some((ty.ty.clone(), ty.cpp_name()?))
                })
                .collect();
            if (!cpp_types.is_empty() || !imported.cpp_template_homes.is_empty())
                && imported.crate_name.is_none()
            {
                ctx.add_error_str(
                    &format!(
                        "imported module `{}` declares `c++::` types, so it needs a `#crate <name>` \
                         directive for this module to find their Rust wrappers \
                         (at `::<name>::generated::...`)",
                        import_path.display()
                    ),
                    span,
                );
            }
            zngur
                .templates
                .extend(imported_templates.into_iter().map(|t| TemplateDef {
                    imported: true,
                    ..t
                }));
            let module_import = &mut zngur.spec.imported_modules[index];
            module_import.crate_name = imported.crate_name;
            module_import.cpp_types = cpp_types;
            module_import.cpp_by_value_types = cpp_by_value_types;
            module_import.cpp_template_homes = imported.cpp_template_homes;
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

        let (aliases, items) = partition_parsed_items(
            ast.0
                .0
                .into_iter()
                .map(|item| process_parsed_item(item, ctx)),
        );
        let first_new_module_import = zngur.spec.imported_modules.len();
        ProcessedZngFile::new(aliases, items).into_zngur_spec(zngur, ctx);
        if ctx.reports().errors > 0 {
            return;
        }
        if ctx.module_import_depth > 0 {
            Self::resolve_module_imports(zngur, first_new_module_import, ctx, resolver);
        }

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
            type_vars,
        } => Ret::Processed(ProcessedItem::Type {
            ty,
            items,
            type_vars,
        }),
        ParsedItem::Trait { tr, methods } => Ret::Processed(ProcessedItem::Trait { tr, methods }),
        ParsedItem::Fn(method) => Ret::Processed(ProcessedItem::Fn(method)),
        ParsedItem::CppFn(f) => Ret::Processed(ProcessedItem::CppFn(f)),
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
        let root_scope = Scope::new_root(self.aliases);

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
    /// Whether the template comes from an imported module (only templates of C++ class templates
    /// do), rather than from this one.
    imported: bool,
}

#[derive(Default)]
struct ZngurSpecBuilder {
    spec: ZngurSpec,
    templates: Vec<TemplateDef>,
    imports: Vec<Import>,
    /// The span of each entry of `spec.imported_modules`, in the file that imports it.
    module_import_spans: Vec<Span>,
}

impl ZngurSpecBuilder {
    fn to_zngur(self, ctx: &mut ParseContext) -> ZngurSpec {
        let ZngurSpecBuilder {
            mut spec,
            templates,
            imports: _,
            module_import_spans,
        } = self;
        for ty in &mut spec.types {
            let mut template_locations = Vec::new();
            for template in &templates {
                if let Some(template_match) = try_match_template(&ty.ty, &template.ty) {
                    let location = (template.source_id, template.span.into_range());
                    if let Err(e) = template_match.merge(ty) {
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
                        template_locations.push(location);
                    }
                }
            }
            if !ty.wellknown_traits.iter().any(|wkt| {
                matches!(
                    wkt,
                    ZngurWellknownTrait::Copy | ZngurWellknownTrait::Unsized
                )
            }) {
                ty.wellknown_traits.push(ZngurWellknownTrait::Drop);
            }
            if ty.layout.is_none() && !matches!(ty.ty, RustType::Cpp(_)) {
                let spans = ctx.fetch_spans_global(ty);
                let (first, rest) = {
                    let mut it = spans.into_iter();
                    let first = it.next().cloned();
                    (first, it.collect::<Vec<_>>())
                };
                let mut report = Report::build(ReportKind::Error, (0, 0usize..0)).with_message(format!(
                    "No layout policy found for type {}.",
                    ty.ty
                )).with_note("Use one of `#layout(size = X, align = Y)`, `#heap_allocated` or `#only_by_ref`.");

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
                for (i, location) in template_locations.into_iter().enumerate() {
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
            let is_cpp = matches!(ty.ty, RustType::Cpp(_));
            if ty.trivially_relocatable.is_relocatable() && ty.cpp_by_value_layout().is_none() {
                type_error(
                    ctx,
                    &ty.ty,
                    "`#trivially_relocatable` needs a `#layout(...)` or `#layout_conservative(...)`, \
                     which Rust needs to hold the C++ type by value",
                );
            }
            // Checked after merging, since the C++ member functions may be declared in a
            // different file than the type's layout.
            if !ty.cpp_methods.is_empty() && !is_cpp {
                type_error(
                    ctx,
                    &ty.ty,
                    "C++ member functions (`safe fn` / `unsafe fn`) can only be declared on `c++::` \
                     types",
                );
            }
            if !ty.cpp_constructors.is_empty() && ty.cpp_by_value_layout().is_none() {
                type_error(
                    ctx,
                    &ty.ty,
                    "C++ constructors can only be declared on `c++::` types that Rust holds by value \
                     (with a layout and `#trivially_relocatable`)",
                );
            }
            if !ty.cpp_fields.is_empty() && !is_cpp {
                type_error(
                    ctx,
                    &ty.ty,
                    "C++ field accessors (`field name (type = T)`) can only be declared on `c++::` \
                     types",
                );
            }
            if is_cpp && let Err(e) = ty.ty.cpp_spelling() {
                type_error(ctx, &ty.ty, &e);
            }
        }
        let local_cpp_templates: Vec<&TemplateDef> = templates
            .iter()
            .filter(|t| matches!(t.ty.ty, RustType::Cpp(_)) && !t.imported)
            .collect();
        resolve_cpp_templates(&mut spec, &local_cpp_templates, &module_import_spans, ctx);
        spec.cpp_templates = templates
            .into_iter()
            .filter(|t| matches!(t.ty.ty, RustType::Cpp(_)) && !t.imported)
            .map(|t| t.ty)
            .collect();
        spec
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
            .with_label(Label::new(span).with_message(message))
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
    module_import_spans: &[Span],
    ctx: &mut ParseContext,
) {
    let display = |path: &[String]| format!("c++::{}", path.join("::"));
    // Template path -> the imported modules that are its home.
    let mut imported_homes: Vec<(&[String], Vec<(usize, &CppTemplateHome)>)> = vec![];
    for (index, module) in spec.imported_modules.iter().enumerate() {
        for home in &module.cpp_template_homes {
            match imported_homes
                .iter_mut()
                .find(|(p, _)| *p == home.path.as_slice())
            {
                Some((_, homes)) => homes.push((index, home)),
                None => imported_homes.push((&home.path, vec![(index, home)])),
            }
        }
    }
    let import_name = |index: usize| spec.imported_modules[index].path.display().to_string();
    for (path, homes) in &imported_homes {
        if let [_, (second, _), ..] = homes.as_slice() {
            ctx.add_error_str(
                &format!(
                    "imported modules `{}` and `{}` are both the home of `{}`; a C++ class \
                     template can only have one home (declare `type<T> {}<...>` in a module that \
                     both import)",
                    import_name(homes[0].0),
                    import_name(*second),
                    display(path),
                    display(path),
                ),
                module_import_spans
                    .get(*second)
                    .copied()
                    .unwrap_or_else(|| (0..0).into()),
            );
        }
    }
    let imported_home = |path: &[String]| {
        imported_homes
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, homes)| homes[0])
    };

    let mut homes = vec![];
    for template in local_templates {
        let RustType::Cpp(path) = &template.ty.ty else {
            continue;
        };
        if homes.iter().any(|h: &CppTemplateHome| h.path == path.path) {
            continue;
        }
        if let Some((index, _)) = imported_home(&path.path) {
            let span = (template.source_id, template.span.into_range());
            let message = format!(
                "`{}` already has a home: imported module `{}`",
                display(&path.path),
                import_name(index)
            );
            ctx.add_fatal_report(
                Report::build(ReportKind::Error, span.clone())
                    .with_message(&message)
                    .with_label(Label::new(span).with_message(&message))
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
        let home = homes
            .iter()
            .find(|h| h.path == path.path)
            .or_else(|| imported_home(&path.path).map(|(_, h)| h));
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
            // The instantiation's struct implements the home's trait, so (like Rust's orphan
            // rule) one of the template arguments must be declared in this module.
            Some((index, home)) if home.explicit => {
                // Only `c++::` types declared here are local structs (builtins aren't).
                let has_local_argument = path.generics.iter().any(|arg| {
                    matches!(arg, RustType::Cpp(_)) && spec.types.iter().any(|t| &t.ty == arg)
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
                            import_name(index)
                        ),
                    );
                }
            }
            Some((index, _)) => type_error(
                ctx,
                &ty_def.ty,
                &format!(
                    "`{}` is also instantiated in imported module `{}`; to share a C++ class \
                     template between modules, declare `type<T> {}<...>` in one of them (its \
                     home)",
                    display(&path.path),
                    import_name(index),
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
        choice([
            just("c++").to(Token::CppPathStart),
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
        raw_string,
        plain_string,
        text::ident().map(Token::ident_or_kw),
        text::int(10).map(|x: &str| Token::Number(x.parse().unwrap())),
    ));

    let comment = just("//")
        .then(any().and_is(just('\n').not()).repeated())
        .padded();

    token
        .map_with(|tok, extra| (tok, extra.span()))
        .padded_by(comment.clone().repeated())
        .padded()
        .repeated()
        .collect()
        // Comments after the last token, or in a file without tokens.
        .then_ignore(comment.repeated())
        .padded()
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
        let adt = pg
            .clone()
            .try_map_with(|pg, extra| {
                if pg.path.start == ParsedPathStart::Cpp && !pg.named_generics.is_empty() {
                    Err(Rich::custom(
                        extra.span(),
                        "C++ template arguments are positional; `Name = T` is only for Rust types",
                    ))
                } else {
                    Ok(pg)
                }
            })
            .map(ParsedRustType::Adt);

        let dyn_trait = just(Token::KwDyn)
            .or(just(Token::KwImpl))
            .then(rust_trait(parser.clone()))
            .then(
                just(Token::Plus)
                    .ignore_then(select! {
                        Token::Ident(c) => c,
                    })
                    .repeated()
                    .collect::<Vec<_>>()
                    .boxed(),
            )
            .map(|((token, first), rest)| match token {
                Token::KwDyn => ParsedRustType::Dyn(first, rest),
                Token::KwImpl => ParsedRustType::Impl(first, rest),
                _ => unreachable!(),
            });
        let boxed = just(Token::Ident("Box"))
            .then(rust_generics(parser.clone()))
            .map(|(_, x)| {
                assert_eq!(x.len(), 1);
                ParsedRustType::Boxed(Box::new(x.into_iter().next().unwrap().right().unwrap()))
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
        let reference = just(Token::And)
            .ignore_then(
                just(Token::KwMut)
                    .to(Mutability::Mut)
                    .or(empty().to(Mutability::Not)),
            )
            .then(parser.clone())
            .map(|(m, x)| ParsedRustType::Ref(m, Box::new(x)));
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
        // `const T`, which is only valid as the pointee of a C++ pointer or reference, or as a C++
        // template argument (checked by `check_cpp_const`).
        let cpp_const = just(Token::KwConst)
            .ignore_then(non_cpp_ptr.clone())
            .map(|x| ParsedRustType::CppConst(Box::new(x)));
        // C++ pointers and references: `const T*`, `T*`, `const T&` and `T&`.
        let cpp_ptr = choice((cpp_const.clone(), non_cpp_ptr.clone()))
            .then(choice((
                just(Token::Star).to(CppIndirection::Pointer),
                just(Token::And).to(CppIndirection::Reference),
            )))
            .map(|(x, kind)| ParsedRustType::CppPtr(kind, Box::new(x)));
        choice((cpp_ptr.boxed(), cpp_const.boxed(), non_cpp_ptr))
    })
    .try_map(|ty, span| match check_cpp_const(&ty, false) {
        Ok(()) => Ok(ty),
        Err(message) => Err(Rich::custom(span, message)),
    })
    .boxed()
}

/// Checks that `const T` only appears where it's valid: as the pointee of a C++ pointer or
/// reference, or as a C++ template argument (`allowed`), with a `T` that is a C++ type or a
/// builtin.
fn check_cpp_const(ty: &ParsedRustType<'_>, allowed: bool) -> Result<(), String> {
    let all = |tys: &[ParsedRustType<'_>]| tys.iter().try_for_each(|t| check_cpp_const(t, false));
    match ty {
        ParsedRustType::CppConst(inner) => {
            if !allowed {
                return Err(
                    "`const T` can only be the type of a C++ pointer or reference \
                            (`const T&`, `const T*`) or a C++ template argument"
                        .to_owned(),
                );
            }
            match &**inner {
                ParsedRustType::Primitive(PrimitiveRustType::Str | PrimitiveRustType::Char) => {
                    Err("only C++ types and builtins can be `const`".to_owned())
                }
                ParsedRustType::Primitive(_) => Ok(()),
                ParsedRustType::Adt(_) => check_cpp_const(inner, false),
                _ => Err("only C++ types and builtins can be `const`".to_owned()),
            }
        }
        ParsedRustType::CppPtr(_, inner) => check_cpp_const(inner, true),
        ParsedRustType::Primitive(_) => Ok(()),
        ParsedRustType::Ref(_, t)
        | ParsedRustType::Raw(_, t)
        | ParsedRustType::Boxed(t)
        | ParsedRustType::Slice(t) => check_cpp_const(t, false),
        ParsedRustType::Tuple(tys) => all(tys),
        ParsedRustType::Dyn(..) | ParsedRustType::Impl(..) => Ok(()),
        ParsedRustType::Adt(pg) => {
            let is_cpp = pg.path.start == ParsedPathStart::Cpp;
            pg.generics
                .iter()
                .try_for_each(|t| check_cpp_const(t, is_cpp))?;
            pg.named_generics
                .iter()
                .try_for_each(|(_, t)| check_cpp_const(t, false))
        }
    }
}

fn rust_generics<'a>(
    rust_type: Boxed<'a, 'a, ParserInput<'a>, ParsedRustType<'a>, ZngParserExtra<'a>>,
) -> impl Parser<
    'a,
    ParserInput<'a>,
    Vec<Either<(&'a str, ParsedRustType<'a>), ParsedRustType<'a>>>,
    ZngParserExtra<'a>,
> + Clone {
    let named_generic = select! {
        Token::Ident(c) => c,
    }
    .then_ignore(just(Token::Eq))
    .then(rust_type.clone())
    .map(Either::Left);
    just(Token::ColonColon)
        .repeated()
        .at_most(1)
        .ignore_then(
            named_generic
                .or(rust_type.clone().map(Either::Right))
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
        .map(|x| {
            let generics = x.1.into_iter().next().unwrap_or_default();
            let (named_generics, generics) = generics.into_iter().partition_map(|x| x);
            ParsedRustPathAndGenerics {
                path: x.0,
                generics,
                named_generics,
            }
        })
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
    let fn_trait = select! {
        Token::Ident(c) => c,
    }
    .then(fn_args(rust_type.clone()))
    .map(|x| ParsedRustTrait::Fn {
        name: x.0,
        inputs: x.1.0,
        output: Box::new(x.1.1),
    });

    let rust_trait = fn_trait.or(rust_path_and_generics(rust_type)
        .try_map_with(|pg, extra| {
            if pg.path.start == ParsedPathStart::Cpp {
                Err(Rich::custom(
                    extra.span(),
                    "`c++::` paths cannot be used as a trait",
                ))
            } else {
                Ok(pg)
            }
        })
        .map(ParsedRustTrait::Normal));
    rust_trait.boxed()
}

fn method<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedMethod<'a>, ZngParserExtra<'a>> + Clone {
    spanned(just(Token::KwAsync))
        .or_not()
        .then_ignore(just(Token::KwFn))
        .then(select! {
            Token::Ident(c) => c,
        })
        .then(
            rust_type()
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::AngleOpen), just(Token::AngleClose))
                .or(empty().to(vec![]))
                .boxed(),
        )
        .then(fn_args(rust_type()))
        .map(|(((opt_async, name), generics), args)| {
            let is_self = |c: &ParsedRustType<'_>| {
                if let ParsedRustType::Adt(c) = c {
                    c.path.start == ParsedPathStart::Relative
                        && &c.path.segments == &["self"]
                        && c.generics.is_empty()
                } else {
                    false
                }
            };
            let (inputs, receiver) = match args.0.get(0) {
                Some(x) if is_self(&x) => (args.0[1..].to_vec(), ZngurMethodReceiver::Move),
                Some(ParsedRustType::Ref(m, x)) if is_self(&x) => {
                    (args.0[1..].to_vec(), ZngurMethodReceiver::Ref(*m))
                }
                _ => (args.0, ZngurMethodReceiver::Static),
            };
            let mut output = args.1;
            if let Some(async_kw) = opt_async {
                output = ParsedRustType::Impl(
                    ParsedRustTrait::Normal(ParsedRustPathAndGenerics {
                        path: ParsedPath {
                            start: ParsedPathStart::Absolute,
                            segments: vec!["std", "future", "Future"],
                            span: async_kw.span,
                        },
                        generics: vec![],
                        named_generics: vec![("Output", output)],
                    }),
                    vec![],
                )
            }
            ParsedMethod {
                name,
                receiver,
                generics,
                inputs,
                output,
            }
        })
        .boxed()
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
        .ignore_then(constructor_args)
        .map(|args| ParsedTypeItem::Constructor { args });
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
        .ignore_then(just(Token::KwConst).or_not().map(|c| c.is_some()))
        .then(cpp_name())
        .then(
            just(Token::KwType)
                .then(just(Token::Eq))
                .ignore_then(rust_type())
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose)),
        )
        .then(
            just(Token::KwAs)
                .ignore_then(select! { Token::Ident(c) => c })
                .or_not(),
        )
        .map(
            |(((is_const, name), ty), rust_name)| ParsedTypeItem::CppField {
                is_const,
                name,
                ty,
                rust_name,
            },
        );
    // The `#cpp_*` directives were removed: a `c++::` type's C++ name is its path, and whether
    // Rust can hold it by value follows from `#layout` + `#trivially_relocatable`. They're still
    // parsed, to explain the new syntax.
    let removed_cpp_directive = just(Token::Sharp)
        .ignore_then(select! {
            Token::Ident(c) if matches!(c, "cpp_value" | "cpp_heap_allocated" | "cpp_ref" | "cpp_stack_owned") => c,
        })
        .then_ignore(
            select! {
                Token::Str(_) => (),
                Token::RawStr(_, _) => (),
            }
            .repeated(),
        )
        .then_ignore(
            property_item
                .clone()
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
                .or_not(),
        )
        .map(|name| ParsedTypeItem::RemovedCppDirective { name });
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
        // C++ member functions and constructors. Unlike methods of Rust types, they always start
        // with `safe` or `unsafe`, and member functions take `this` implicitly (`const` after the
        // parameters).
        let cpp_params = rust_type()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::ParenOpen), just(Token::ParenClose));
        let cpp_method = just(Token::Ident("static"))
            .or_not()
            .map(|s| s.is_some())
            .then_ignore(just(Token::KwFn))
            .then(cpp_name())
            .then(cpp_template_args())
            .then(cpp_params.clone())
            .then(just(Token::KwConst).or_not().map(|c| c.is_some()))
            .then(just(Token::Arrow).ignore_then(rust_type()).or_not())
            .then(
                just(Token::KwAs)
                    .ignore_then(select! { Token::Ident(c) => c })
                    .or_not(),
            )
            .map(
                |(
                    (((((is_static, name), template_args), inputs), is_const), output),
                    rust_name,
                )| {
                    ParsedTypeItem::CppMethod {
                        is_safe: false, // Set below.
                        is_static,
                        name,
                        template_args,
                        inputs,
                        is_const,
                        output: output.unwrap_or(ParsedRustType::Tuple(vec![])),
                        rust_name,
                    }
                },
            );
        let cpp_constructor = just(Token::Ident("constructor"))
            .ignore_then(cpp_params)
            .map(|inputs| ParsedTypeItem::CppConstructor {
                is_safe: false, // Set below.
                inputs,
            });
        let cpp_method = choice((
            just(Token::KwSafe).to(true),
            just(Token::KwUnsafe).to(false),
        ))
        .then(choice((cpp_constructor, cpp_method)))
        .map(|(safe, mut item)| {
            match &mut item {
                ParsedTypeItem::CppMethod { is_safe, .. }
                | ParsedTypeItem::CppConstructor { is_safe, .. } => *is_safe = safe,
                _ => unreachable!(),
            }
            item
        });
        let inner_item = choice((
            layout.boxed(),
            traits.boxed(),
            non_exhaustive.boxed(),
            constructor.boxed(),
            field.boxed(),
            cpp_field.boxed(),
            removed_cpp_directive.boxed(),
            trivially_relocatable.boxed(),
            cpp_method.boxed(),
            method()
                .then(
                    just(Token::KwUse)
                        .ignore_then(non_cpp_path("a method's `use` path"))
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
                        .ignore_then(select! { Token::Ident(c) => Some(c), })
                        .or(empty().to(None))
                        .boxed(),
                )
                .map(
                    |(((data, use_path), deref), cpp_name)| ParsedTypeItem::Method {
                        deref,
                        use_path,
                        data,
                        cpp_name,
                    },
                )
                .boxed(),
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
    just(Token::KwType)
        .ignore_then(
            (select! { Token::Ident(c) => c })
                .map(ParsedTypeVar)
                .separated_by(just(Token::Comma))
                .at_least(1)
                .allow_trailing()
                .collect()
                .delimited_by(just(Token::AngleOpen), just(Token::AngleClose))
                .or_not(),
        )
        .then(spanned(rust_type()))
        // Templates of C++ class templates (`type<T> c++::std::unique_ptr<T>`) are stable;
        // templates of Rust types aren't yet.
        .try_map_with(|(type_vars, ty), e: &mut MapExtra<_, ZngParserExtra>| {
            let is_cpp = matches!(&ty.inner, ParsedRustType::Adt(pg) if pg.path.start == ParsedPathStart::Cpp);
            if type_vars.is_some() && !is_cpp && !e.state().unstable_features.template_types {
                Err(Rich::custom(e.span(), "Template types are unstable. Enable them by using `#unstable(template_types)` at the top of the file."))
            } else {
                Ok((type_vars, ty))
            }
        })
        .then(
            spanned(inner_type_item())
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::BraceOpen), just(Token::BraceClose)).boxed(),
        )
        .map(|((type_vars, ty), items)| ParsedItem::Type {
            ty,
            items,
            type_vars,
        })
        .boxed()
}

fn trait_item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone
{
    just(Token::KwTrait)
        .ignore_then(spanned(rust_trait(rust_type())))
        .then(
            method()
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
    spanned(method())
        .then_ignore(just(Token::Semicolon))
        .map(ParsedItem::Fn)
        .boxed()
}

/// The name of a C++ declaration: an identifier, or a word that is a zngur keyword but can be a
/// C++ name (e.g. a field named `type`).
fn cpp_name<'a>() -> impl Parser<'a, ParserInput<'a>, &'a str, ZngParserExtra<'a>> + Clone {
    select! {
        Token::Ident(c) => c,
        Token::KwAs => "as",
        Token::KwAsync => "async",
        Token::KwCrate => "crate",
        Token::KwDyn => "dyn",
        Token::KwFn => "fn",
        Token::KwImpl => "impl",
        Token::KwImport => "import",
        Token::KwMatch => "match",
        Token::KwMerge => "merge",
        Token::KwMod => "mod",
        Token::KwMut => "mut",
        Token::KwSafe => "safe",
        Token::KwTrait => "trait",
        Token::KwType => "type",
        Token::KwUnsafe => "unsafe",
        Token::KwUse => "use",
    }
}

/// Explicit template arguments of a C++ function template: `<types...>`, or nothing.
fn cpp_template_args<'a>()
-> impl Parser<'a, ParserInput<'a>, Vec<ParsedRustType<'a>>, ZngParserExtra<'a>> + Clone {
    rust_type()
        .separated_by(just(Token::Comma))
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(just(Token::AngleOpen), just(Token::AngleClose))
        .or_not()
        .map(Option::unwrap_or_default)
}

fn cpp_fn_item<'a>() -> impl Parser<'a, ParserInput<'a>, ParsedItem<'a>, ZngParserExtra<'a>> + Clone
{
    let cpp_fn = choice((
        just(Token::KwSafe).to(true),
        just(Token::KwUnsafe).to(false),
    ))
    .then_ignore(just(Token::KwFn))
    .then(cpp_name())
    .then(cpp_template_args())
    .then(
        rust_type()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::ParenOpen), just(Token::ParenClose)),
    )
    .then(just(Token::Arrow).ignore_then(rust_type()).or_not())
    .then(
        just(Token::KwAs)
            .ignore_then(select! { Token::Ident(c) => c })
            .or_not(),
    )
    .map(
        |(((((is_safe, name), template_args), inputs), output), rust_name)| ParsedCppFn {
            is_safe,
            name,
            template_args,
            inputs,
            output: output.unwrap_or(ParsedRustType::Tuple(vec![])),
            rust_name,
        },
    );
    spanned(cpp_fn)
        .then_ignore(just(Token::Semicolon))
        .map(ParsedItem::CppFn)
        .boxed()
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
                .ignore_then(select! { Token::Ident(name) => name })
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
        .then(spanned(method()))
        .then_ignore(just(Token::Semicolon))
        .map(|(is_safe, method)| ParsedExternCppItem::Function { is_safe, method });
    let impl_block = just(Token::KwImpl)
        .ignore_then(
            rust_trait(rust_type())
                .then_ignore(just(Token::KwFor))
                .map(Some)
                .or(empty().to(None))
                .then(spanned(rust_type()))
                .boxed(),
        )
        .then(
            safety
                .then(method())
                .then_ignore(just(Token::Semicolon))
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
                .boxed(),
        )
        .map(|((tr, ty), methods)| ParsedExternCppItem::Impl { tr, ty, methods });
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
            just(Token::KwMod)
                .ignore_then(path())
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
            cpp_fn_item(),
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

fn non_cpp_path<'a>(
    context: &'static str,
) -> impl Parser<'a, ParserInput<'a>, ParsedPath<'a>, ZngParserExtra<'a>> + Clone {
    path().try_map_with(move |p, extra| {
        if p.start == ParsedPathStart::Cpp {
            Err(Rich::custom(
                extra.span(),
                format!("`c++::` paths cannot be used in {context}"),
            ))
        } else {
            Ok(p)
        }
    })
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
