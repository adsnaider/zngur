use std::panic::catch_unwind;

use expect_test::{Expect, expect};
use zngur_def::{
    CppIndirection, CppTemplateHome, CppTypePath, LayoutPolicy, Lifetime, Mutability,
    PrimitiveRustType, RustPathAndGenerics, RustType, TriviallyRelocatable, ZngurSpec,
};

use crate::{
    DefaultImportResolver, EntityPath, ImportResolver, ParsedZngFile, ReportSink,
    cfg::{InMemoryRustCfgProvider, NullCfg, RustCfgProvider},
};

/// Parse a .zng file from a string.
fn parse_str(
    text: &str,
    cfg: impl RustCfgProvider + 'static,
    warning_sink: impl FnMut(&str),
) -> crate::ParseResult {
    let mut error_sink = TestErrorSink::new(warning_sink);
    ParsedZngFile::parse_str_with_resolver(
        text,
        "test.zng",
        cfg,
        &DefaultImportResolver,
        &mut error_sink,
    )
}

fn parse_str_with_resolver(
    text: &str,
    cfg: impl RustCfgProvider + 'static,
    resolver: &impl ImportResolver,
    warning_sink: impl FnMut(&str),
) -> crate::ParseResult {
    let mut report_sink = TestErrorSink::new(warning_sink);
    ParsedZngFile::parse_str_with_resolver(text, "test.zng", cfg, resolver, &mut report_sink)
}

#[derive(Debug)]
struct TestErrorSink<F>
where
    F: FnMut(&str),
{
    error_buffer: Vec<u8>,
    warning_sink: F,
}

impl<F> TestErrorSink<F>
where
    F: FnMut(&str),
{
    fn new(f: F) -> Self {
        Self {
            error_buffer: Default::default(),
            warning_sink: f,
        }
    }
}

impl<F> ReportSink for TestErrorSink<F>
where
    F: FnMut(&str),
{
    fn sink_report(&mut self, report: &crate::ReportEntry, source_cache: crate::SourceCache) {
        if report.fatal {
            report
                .report
                .write(source_cache, &mut self.error_buffer)
                .unwrap();
        } else {
            let mut buf = Vec::<u8>::new();
            report.report.write(source_cache, &mut buf).unwrap();
            (self.warning_sink)(&String::from_utf8(strip_ansi_escapes::strip(buf)).unwrap());
        };
    }
}

/// unwind with the full error on drop if not empty
impl<F> Drop for TestErrorSink<F>
where
    F: FnMut(&str),
{
    fn drop(&mut self) {
        if !self.error_buffer.is_empty() {
            std::panic::resume_unwind(Box::new(ErrorText(
                String::from_utf8(strip_ansi_escapes::strip(&self.error_buffer)).unwrap(),
            )))
        }
    }
}

fn check_success(zng: &str) {
    let _ = parse_str(zng, NullCfg, |msg| {
        unreachable!("an unexpected warning was emitted: {msg}");
    });
}

fn check_warnings(zng: &str, expected: Expect) {
    let mut warnings = String::new();
    let _ = parse_str(zng, NullCfg, |msg| {
        warnings.push_str(msg);
    });
    expected.assert_eq(&warnings);
}

pub struct ErrorText(pub String);

fn check_fail(zng: &str, error: Expect) {
    let r = catch_unwind(|| {
        let _ = parse_str(zng, NullCfg, |_| {});
    });
    match r {
        Ok(_) => panic!("Parsing succeeded but we expected fail"),
        Err(e) => match e.downcast::<ErrorText>() {
            Ok(t) => error.assert_eq(&t.0),
            Err(e) => std::panic::resume_unwind(e),
        },
    }
}

fn check_import_fail(zng: &str, error: Expect, resolver: &MockFilesystem) {
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(zng, NullCfg, resolver, |_| {});
    });

    match r {
        Ok(_) => panic!("Parsing succeeded but we expected fail"),
        Err(e) => match e.downcast::<ErrorText>() {
            Ok(t) => error.assert_eq(&t.0),
            Err(e) => std::panic::resume_unwind(e),
        },
    }
}

// useful for debugging a test that should succeeded on parse
fn catch_parse_fail(
    zng: &str,
    cfg: impl RustCfgProvider + std::panic::UnwindSafe + 'static,
) -> crate::ParseResult {
    let r = catch_unwind(move || parse_str(zng, cfg, |_| {}));

    match r {
        Ok(r) => r,
        Err(e) => match e.downcast::<ErrorText>() {
            Ok(t) => {
                eprintln!("{}", t.0);
                crate::ParseResult {
                    spec: ZngurSpec::default(),
                    processed_files: Vec::new(),
                    errors: 1,
                    warnings: 0,
                }
            }
            Err(e) => std::panic::resume_unwind(e),
        },
    }
}

#[test]
fn parse_tuple() {
    check_success(
        r#"
type (i8, u8) {
    #layout(size = 0, align = 1);
}
    "#,
    );
}

#[test]
fn layout_invalid_alignment_power_of_two() {
    check_fail(
        r#"
type crate::Way {
    #layout(size = 16, align = 0);
}
    "#,
        expect![[r#"
            Error: Alignment must be a power of two
               ╭─[ test.zng:3:24 ]
               │
             3 │     #layout(size = 16, align = 0);
               │                        ──┬──  
               │                          ╰──── Alignment must be a power of two
            ───╯
        "#]],
    );
    check_fail(
        r#"
type crate::Way {
    #layout_conservative(size = 16, align = 3);
}
    "#,
        expect![[r#"
            Error: Alignment must be a power of two
               ╭─[ test.zng:3:37 ]
               │
             3 │     #layout_conservative(size = 16, align = 3);
               │                                     ──┬──  
               │                                       ╰──── Alignment must be a power of two
            ───╯
        "#]],
    );
}

#[test]
fn warning_in_merged_file_renders_child_source_span() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
mod c++::ns {
    safe fn f(i32*);
}
"#,
    )]);

    let mut warnings = Vec::new();
    let _ = parse_str_with_resolver(
        r#"
merge "./a.zng";
"#,
        NullCfg,
        &resolver,
        &mut |w: &str| warnings.push(w.to_owned()),
    );
    // The `safe fn f(i32*);` declaration that triggered this warning lives in the
    // merged `a.zng`, not the root file, so the rendered warning must show `a.zng`'s
    // own filename and source snippet.
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("a.zng"), "got:\n{}", warnings[0]);
    assert!(
        warnings[0].contains("safe fn f(i32*);"),
        "got:\n{}",
        warnings[0]
    );
}

macro_rules! assert_ty_path {
    ($path_expected:expr, $ty:expr) => {{
        let RustType::Adt(RustPathAndGenerics { path: p, .. }) = $ty else {
            panic!("type `{:?}` is not a path", $ty);
        };
        assert_eq!(p.as_slice(), $path_expected);
    }};
}

#[test]
fn alias_expands_correctly() {
    let parsed = parse_str(
        r#"
use ::std::string::String as MyString;
type MyString {
    #layout(size = 24, align = 8);
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let RustType::Adt(RustPathAndGenerics { path: p, .. }) = &ty.ty else {
        panic!("no match?");
    };
    assert_eq!(p.as_slice(), ["std", "string", "String"]);
}

#[test]
fn alias_expands_nearest_scope_first() {
    let parsed = parse_str(
        r#"
use ::std::string::String as MyString;
mod crate {
    use MyLocalString as MyString;
    type MyString {
        #layout(size = 24, align = 8);
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let RustType::Adt(RustPathAndGenerics { path: p, .. }) = &ty.ty else {
        panic!("no match?");
    };
    assert_eq!(p.as_slice(), ["crate", "MyLocalString"]);
}

#[test]
fn parse_variants() {
    println!("meow");

    let parsed = parse_str(
        r#"
type Option<i32> {
    #layout(size = 8, align = 4);
    variant None { }
    variant Some {
        field 0 (offset = auto, type = i32);
    }
}
        "#,
        NullCfg,
        |_| {},
    );

    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_eq!(ty.variants.len(), 2, "should have two variants");

    assert_eq!(ty.variants[0].name, "None");
    assert_eq!(ty.variants[0].fields.len(), 0);

    assert_eq!(ty.variants[1].name, "Some");
    assert_eq!(ty.variants[1].fields.len(), 1);
    assert_eq!(ty.variants[1].fields[0].name, "0");
    assert_eq!(
        ty.variants[1].fields[0].ty,
        RustType::Primitive(zngur_def::PrimitiveRustType::Int(32))
    );
    assert_eq!(ty.variants[1].fields[0].offset, None);
}

struct MockFilesystem {
    files: std::collections::HashMap<std::path::PathBuf, String>,
}

impl MockFilesystem {
    fn new(
        files: impl IntoIterator<Item = (impl Into<std::path::PathBuf>, impl Into<String>)>,
    ) -> Self {
        Self {
            files: files
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }
}

impl ImportResolver for MockFilesystem {
    fn resolve_import(
        &self,
        cwd: &std::path::Path,
        relpath: &std::path::Path,
    ) -> Result<String, String> {
        let path = cwd.join(relpath);
        if let Some(content) = self.files.get(&path) {
            return Ok(content.clone());
        }
        let norm = crate::normalize_lexical_path(&path);
        self.files
            .iter()
            .find_map(|(k, v)| (crate::normalize_lexical_path(k) == norm).then(|| v.clone()))
            .ok_or_else(|| format!("File not found: {}", path.display()))
    }
}

#[test]
fn import_parser_test() {
    let resolver = MockFilesystem::new(vec![(
        "./relative/path.zng",
        "type Imported { #layout(size = 1, align = 1); }",
    )]);

    let parsed = parse_str_with_resolver(
        r#"
merge "./relative/path.zng";
type Example {
    #layout(size = 1, align = 1);
}
    "#,
        NullCfg,
        &resolver,
        |_| {},
    );
    assert_eq!(parsed.spec.types.len(), 2);
}

#[test]
fn convert_panic_to_exception_in_main_file_succeeds() {
    check_success(
        r#"
#convert_panic_to_exception
type A {
    #layout(size = 1, align = 1);
}
        "#,
    );
}

// Tests for processed_files tracking (depfile support)

#[test]
fn processed_files_single_file() {
    let parsed = parse_str(
        r#"
type A {
    #layout(size = 1, align = 1);
}
        "#,
        NullCfg,
        |_| {},
    );
    // Should have exactly one file (test.zng)
    assert_eq!(parsed.processed_files.len(), 1);
    assert_eq!(
        parsed.processed_files[0]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap(),
        "test.zng"
    );
}

#[test]
fn processed_files_with_import() {
    let resolver = MockFilesystem::new(vec![(
        "./imported.zng",
        "type Imported { #layout(size = 1, align = 1); }",
    )]);

    let parsed = parse_str_with_resolver(
        r#"
merge "./imported.zng";
type Main {
    #layout(size = 1, align = 1);
}
        "#,
        NullCfg,
        &resolver,
        |_| {},
    );
    // Should have two files: main (test.zng) + imported
    assert_eq!(parsed.processed_files.len(), 2);
    let file_names: Vec<_> = parsed
        .processed_files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(file_names.contains(&"test.zng"));
    assert!(file_names.contains(&"imported.zng"));
}

#[test]
fn processed_files_with_nested_imports() {
    let resolver = MockFilesystem::new(vec![
        (
            "./a.zng",
            r#"merge "./b.zng"; type A { #layout(size = 1, align = 1); }"#,
        ),
        (
            "./b.zng",
            r#"merge "./c.zng"; type B { #layout(size = 1, align = 1); }"#,
        ),
        ("./c.zng", "type C { #layout(size = 1, align = 1); }"),
    ]);

    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
type Main {
    #layout(size = 1, align = 1);
}
        "#,
        NullCfg,
        &resolver,
        |_| {},
    );
    // Should have four files: main + a + b + c
    assert_eq!(parsed.processed_files.len(), 4);
    let file_names: Vec<_> = parsed
        .processed_files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(file_names.contains(&"test.zng"));
    assert!(file_names.contains(&"a.zng"));
    assert!(file_names.contains(&"b.zng"));
    assert!(file_names.contains(&"c.zng"));
}

fn assert_layout(wanted_size: usize, wanted_align: usize, layout: &Option<LayoutPolicy>) {
    if !matches!(layout, Some(LayoutPolicy::StackAllocated { size, align }) if *size == wanted_size && *align == wanted_align)
    {
        panic!(
            "no match: StackAllocated {{ size: {wanted_size}, align: {wanted_align} }} != {:?} ",
            layout
        );
    };
}

static EMPTY_CFG: [(&str, &[&str]); 0] = [];

#[test]
fn test_if_conditional_type_item() {
    let source = r#"
#unstable(cfg_if)

type ::std::string::String {
    #if cfg!(target_pointer_width = "64") {
        #layout(size = 24, align = 8);
    } #else if cfg!(target_pointer_width = "32") {
        #layout(size = 12, align = 4);
    } #else {
        // silly size for testing
        #layout(size = 48, align = 16);
    }
}
    "#;
    let parsed = catch_parse_fail(
        source,
        InMemoryRustCfgProvider::default().with_values([("target_pointer_width", &["64"])]),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_layout(24, 8, &ty.layout);
    let parsed = catch_parse_fail(
        source,
        InMemoryRustCfgProvider::default().with_values([("target_pointer_width", &["32"])]),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_layout(12, 4, &ty.layout);
    let parsed = catch_parse_fail(source, NullCfg);
    let ty = parsed.spec.types.first().expect("no type parsed");

    assert_layout(48, 16, &ty.layout);
}

#[test]
fn test_match_conditional_type_item() {
    let source = r#"
#unstable(cfg_match)

type ::std::string::String {
    #match cfg!(target_pointer_width) {
        // single item arm
        "64" => #layout(size = 24, align = 8);
        // match usize numbers
        32 => {
            #layout(size = 12, align = 4);
        },

        _ => {
            // silly size for testing
            #layout(size = 48, align = 16);
        }
    }
}
    "#;
    let parsed = catch_parse_fail(
        source,
        InMemoryRustCfgProvider::default().with_values([("target_pointer_width", &["64"])]),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_layout(24, 8, &ty.layout);
    let parsed = catch_parse_fail(
        source,
        InMemoryRustCfgProvider::default().with_values([("target_pointer_width", &["32"])]),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_layout(12, 4, &ty.layout);
    let parsed = catch_parse_fail(source, NullCfg);
    let ty = parsed.spec.types.first().expect("no type parsed");

    assert_layout(48, 16, &ty.layout);
}

macro_rules! test_paths_with_cfg {
    ($src:expr, $features_path_pairs:expr) => {
        for (cfg, path) in $features_path_pairs.iter().map(|(cfg, path)| {
            (
                cfg.into_iter().copied().collect::<Vec<_>>(),
                path.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            )
        }) {
            let parsed =
                catch_parse_fail($src, InMemoryRustCfgProvider::default().with_values(cfg));
            let ty = parsed.spec.types.first().expect("no type parsed");
            assert_ty_path!(path, &ty.ty);
        }
    };
}

type CfgPathPairs<'a> = &'a [(&'a [(&'a str, &'a [&'a str])], &'a [&'a str])];

#[test]
fn conditional_if_spec_item() {
    let source = r#"
#unstable(cfg_if)

#if cfg!(feature = "foo") {
    type crate::Foo {
        #layout(size = 1, align = 1);
    }
} #else {
    type crate::Bar {
        #layout(size = 1, align = 1);
    }
}
    "#;

    let pairs: CfgPathPairs = &[
        (&[("feature", &["foo"])], &["crate", "Foo"]),
        (&EMPTY_CFG, &["crate", "Bar"]),
    ];
    test_paths_with_cfg!(source, pairs);
}

#[test]
fn conditional_match_spec_item() {
    let source = r#"
#unstable(cfg_match)

#match cfg!(feature) {
    "foo" => type crate::Foo {
        #layout(size = 1, align = 1);
    }
    _ => {
        type crate::Bar {
            #layout(size = 1, align = 1);
        }
    }
}
    "#;
    let pairs: CfgPathPairs = &[
        (&[("feature", &["foo"])], &["crate", "Foo"]),
        (&EMPTY_CFG, &["crate", "Bar"]),
    ];
    test_paths_with_cfg!(source, pairs);
}

#[test]
fn match_pattern_single_cfg() {
    let source = r#"
#unstable(cfg_match)

#match cfg!(feature) {
    "bar" | "zigza" => type crate::BarZigZa {
        #layout(size = 1, align = 1);
    }
    // match two values from a cfg value as a set
    "foo" & "baz" => type crate::FooBaz {
        #layout(size = 1, align = 1);
    }
    // negative matching (no feature baz)
    "foo" & !"baz" => type crate::FooNoBaz {
        #layout(size = 1, align = 1);
    }
    _ => {
        type crate::Zoop {
            #layout(size = 1, align = 1);
        }
    }
}
    "#;
    let pairs: CfgPathPairs = &[
        (&EMPTY_CFG, &["crate", "Zoop"]),
        (&[("feature", &["foo"])], &["crate", "FooNoBaz"]),
        (&[("feature", &["bar"])], &["crate", "BarZigZa"]),
        (&[("feature", &["zigza"])], &["crate", "BarZigZa"]),
        (&[("feature", &["foo", "baz"])], &["crate", "FooBaz"]),
    ];
    test_paths_with_cfg!(source, pairs);
}

#[test]
fn if_pattern_multi_cfg() {
    let source = r#"
#unstable(cfg_if)
// match two cfg keys as a set
#if cfg!(feature.foo) && cfg!(target_pointer_width = 32) {
    type crate::Foo32 {
        #layout(size = 1, align = 1);
    }
} #else if cfg!(feature.foo = None) && cfg!(target_pointer_width = 64) {
    type crate::NoFoo64 {
        #layout(size = 1, align = 1);
    }
} #else if (cfg!(feature.foo = Some) && cfg!(target_pointer_width = 64)) || cfg!(feature.baz) {
    type crate::Foo64_OrBaz {
        #layout(size = 1, align = 1);
    }
} #else {
    type crate::SpecialFoo {
        #layout(size = 1, align = 1);
    }
}
    "#;
    let pairs: CfgPathPairs = &[
        (
            &[("target_pointer_width", &["32"]), ("feature", &["foo"])],
            &["crate", "Foo32"],
        ),
        (
            &[("target_pointer_width", &["64"]), ("feature", &["bar"])],
            &["crate", "NoFoo64"],
        ),
        (
            &[("target_pointer_width", &["64"]), ("feature", &["foo"])],
            &["crate", "Foo64_OrBaz"],
        ),
        (
            &[("target_pointer_width", &["32"]), ("feature", &["baz"])],
            &["crate", "Foo64_OrBaz"],
        ),
        (&[("feature", &["foo"])], &["crate", "SpecialFoo"]),
        (&EMPTY_CFG, &["crate", "SpecialFoo"]),
    ];
    test_paths_with_cfg!(source, pairs);
}

#[test]
fn match_pattern_multi_cfg() {
    let source = r#"
#unstable(cfg_match)

#match (cfg!(feature.foo), cfg!(target_pointer_width)) {
    // match two cfg keys as a set
    (Some, "32") => type crate::Foo32 {
        #layout(size = 1, align = 1);
    }
    (None, 64) => type crate::NoFoo64 {
        #layout(size = 1, align = 1);
    }
    _ => {
        type crate::SpecialFoo {
            #layout(size = 1, align = 1);
        }
    }
}
    "#;
    let pairs: CfgPathPairs = &[
        (
            &[("target_pointer_width", &["32"]), ("feature", &["foo"])],
            &["crate", "Foo32"],
        ),
        (
            &[("target_pointer_width", &["64"]), ("feature", &["bar"])],
            &["crate", "NoFoo64"],
        ),
        (&[("feature", &["foo"])], &["crate", "SpecialFoo"]),
        (&EMPTY_CFG, &["crate", "SpecialFoo"]),
    ];
    test_paths_with_cfg!(source, pairs);
}

#[test]
fn module_import_parser_test() {
    let parsed = parse_str(
        r#"
import "module.zng";
"#,
        NullCfg,
        |_| {},
    );
    assert_eq!(parsed.spec.imported_modules.len(), 1);
    assert_eq!(
        parsed.spec.imported_modules[0].path.to_str().unwrap(),
        "module.zng"
    );
}

#[test]
fn cpp_additional_includes() {
    let parsed = parse_str(
        r#"
#cpp_additional_includes "
    // comment
    stuff
"
    "#,
        NullCfg,
        |_| {},
    );
    assert_eq!(
        parsed.spec.additional_includes.0,
        "\n    // comment\n    stuff\n"
    );

    let parsed = parse_str(
        r##"
#cpp_additional_includes r#"
    // comment
    "stuff"
"#
    "##,
        NullCfg,
        |_| {},
    );
    assert_eq!(
        parsed.spec.additional_includes.0,
        "\n    // comment\n    \"stuff\"\n"
    );
}

// Tests for `c++::a::b::Name` paths.

fn assert_cpp(expected: &[&str], ty: &RustType) {
    let RustType::Cpp(segs) = ty else {
        panic!("type `{:?}` is not a c++ type", ty);
    };
    assert!(segs.generics.is_empty(), "{segs:?}");
    assert_eq!(segs.path.as_slice(), expected);
}

#[test]
fn mod_cpp_shorthand_matches_explicit_cpp_path() {
    let direct = parse_str(
        r#"
type c++::a::b::Name {
    #layout(size = 16, align = 8);
}
    "#,
        NullCfg,
        |_| {},
    );
    let via_mod = parse_str(
        r#"
mod c++::a::b {
    type Name {
        #layout(size = 16, align = 8);
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let direct_ty = &direct.spec.types.first().expect("no type parsed").ty;
    let via_mod_ty = &via_mod.spec.types.first().expect("no type parsed").ty;
    assert_cpp(&["a", "b", "Name"], direct_ty);
    assert_eq!(direct_ty, via_mod_ty);
}

#[test]
fn alias_can_target_a_cpp_path() {
    // Unlike a method's `use` path or a trait bound, a `use ... as` alias
    // target can legitimately be a `c++::` path -- referencing the alias
    // elsewhere should resolve to exactly that `c++::` path.
    let parsed = parse_str(
        r#"
use c++::a::Foo as MyFoo;

type MyFoo {
    #layout(size = 16, align = 8);
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_cpp(&["a", "Foo"], &ty.ty);
}

#[test]
fn cpp_path_rejected_in_method_use_path() {
    check_fail(
        r#"
type crate::Foo {
    #layout(size = 1, align = 1);
    fn bar(self) -> usize use c++::a::Bar;
    fn baz(self) -> usize use c++::a::Bar as cpp_baz;
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used in a method's `use` path
               ╭─[ test.zng:4:31 ]
               │
             4 │     fn bar(self) -> usize use c++::a::Bar;
               │                               ─────┬─────  
               │                                    ╰─────── `c++::` paths cannot be used in a method's `use` path
            ───╯
            Error: `c++::` paths cannot be used in a method's `use` path
               ╭─[ test.zng:5:31 ]
               │
             5 │     fn baz(self) -> usize use c++::a::Bar as cpp_baz;
               │                               ─────┬─────  
               │                                    ╰─────── `c++::` paths cannot be used in a method's `use` path
            ───╯
        "#]],
    );
}

#[test]
fn cpp_path_rejected_as_trait() {
    check_fail(
        r#"
trait c++::Foo {
}

extern "C++" {
    impl c++::Foo for crate::X {
    }
    safe fn take(&dyn c++::Foo);
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used as a trait
               ╭─[ test.zng:2:7 ]
               │
             2 │ trait c++::Foo {
               │       ────┬───  
               │           ╰───── `c++::` paths cannot be used as a trait
            ───╯
            Error: `c++::` paths cannot be used as a trait
               ╭─[ test.zng:6:10 ]
               │
             6 │     impl c++::Foo for crate::X {
               │          ────┬───  
               │              ╰───── `c++::` paths cannot be used as a trait
            ───╯
            Error: `c++::` paths cannot be used as a trait
               ╭─[ test.zng:8:23 ]
               │
             8 │     safe fn take(&dyn c++::Foo);
               │                       ────┬───  
               │                           ╰───── `c++::` paths cannot be used as a trait
            ───╯
        "#]],
    );
}

#[test]
fn cpp_path_via_alias_indirection_rejected_as_trait() {
    check_fail(
        r#"
use c++::Foo as MyTrait;

extern "C++" {
    impl MyTrait for crate::X {
    }
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used as a trait
               ╭─[ test.zng:5:10 ]
               │
             5 │     impl MyTrait for crate::X {
               │          ───┬───  
               │             ╰───── `c++::` paths cannot be used as a trait
            ───╯
        "#]],
    );
}

#[test]
fn relative_trait_inside_cpp_mod_rejected_with_help() {
    check_fail(
        r#"
mod c++::ns {
    extern "C++" {
        impl std::fmt::Debug for Inventory {
        }
    }
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used as a trait
               ╭─[ test.zng:4:14 ]
               │
             4 │         impl std::fmt::Debug for Inventory {
               │              ───────┬───────  
               │                     ╰───────── `c++::` paths cannot be used as a trait
               │ 
               │ Help: inside a `c++::` module, prefix Rust traits with `::` or `crate::`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_path_alias_and_relative_use_rejected_in_method_use_path() {
    check_fail(
        r#"
use c++::a::Bar as MyBar;

type crate::Foo {
    #layout(size = 1, align = 1);
    fn bar(self) -> usize use MyBar;
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used in a method's `use` path
               ╭─[ test.zng:6:31 ]
               │
             6 │     fn bar(self) -> usize use MyBar;
               │                               ──┬──  
               │                                 ╰──── `c++::` paths cannot be used in a method's `use` path
            ───╯
        "#]],
    );
    check_fail(
        r#"
mod c++::a {
    type crate::Foo {
        #layout(size = 1, align = 1);
        fn bar(&self) use std::ops::Deref;
    }
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used in a method's `use` path
               ╭─[ test.zng:5:27 ]
               │
             5 │         fn bar(&self) use std::ops::Deref;
               │                           ───────┬───────  
               │                                  ╰───────── `c++::` paths cannot be used in a method's `use` path
               │ 
               │ Help: inside a `c++::` module, prefix Rust paths with `::` or `crate::`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_path_with_lifetime_arguments_rejected() {
    check_fail(
        r#"
extern "C++" {
    safe fn make<'a>() -> c++::Vec<'a, i32>;
}
    "#,
        expect![[r#"
            Error: `c++::` types don't take lifetime arguments
               ╭─[ test.zng:3:27 ]
               │
             3 │     safe fn make<'a>() -> c++::Vec<'a, i32>;
               │                           ────────┬────────  
               │                                   ╰────────── `c++::` types don't take lifetime arguments
            ───╯
        "#]],
    );
    check_fail(
        r#"
mod c++::ns {
    extern "C++" {
        safe fn make<'a>() -> Option<'a, i32>;
    }
}
    "#,
        expect![[r#"
            Error: `c++::` types don't take lifetime arguments
               ╭─[ test.zng:4:31 ]
               │
             4 │         safe fn make<'a>() -> Option<'a, i32>;
               │                               ───────┬───────  
               │                                      ╰───────── `c++::` types don't take lifetime arguments
               │ 
               │ Help: inside a `c++::` module, prefix Rust paths with `::` or `crate::`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_path_allowed_as_impl_target() {
    check_success(
        r#"
extern "C++" {
    impl c++::Foo {
    }
}
    "#,
    );
}

#[test]
fn cpp_path_allowed_as_impl_for_trait_target() {
    check_success(
        r#"
extern "C++" {
    impl crate::SomeTrait for c++::Foo {
    }
}
    "#,
    );
}

#[test]
fn cpp_path_lexer_tolerates_whitespace_between_tokens() {
    // `c++` is lexed as a single, indivisible token (like `->` or `::`), so
    // whitespace *inside* it (`c ++`) does not lex as `Token::CppPathStart` --
    // it falls back to `Ident("c")` + `Plus` + `Plus`, same as any other
    // unrecognized punctuation sequence would. But, like every other token in
    // this grammar (e.g. `crate ::`), ordinary whitespace *between* the
    // `c++` token and the following `::`/segments is fine.
    let normal = parse_str(
        r#"
type c++::Name {
    #layout(size = 16, align = 8);
}
    "#,
        NullCfg,
        |_| {},
    );
    let spaced = parse_str(
        r#"
type c++ :: Name {
    #layout(size = 16, align = 8);
}
    "#,
        NullCfg,
        |_| {},
    );
    let normal_ty = &normal.spec.types.first().expect("no type parsed").ty;
    let spaced_ty = &spaced.spec.types.first().expect("no type parsed").ty;
    assert_cpp(&["Name"], normal_ty);
    assert_eq!(normal_ty, spaced_ty);
}

#[test]
fn cpp_path_with_space_inside_token_is_a_clean_syntax_error_not_a_panic() {
    // `c ++ :: Name` (space between `c` and `++`) does NOT lex as the `c++`
    // path-start token -- it lexes as `Ident("c")` followed by `Plus`, `Plus`,
    // which is a plain syntax error (not a panic).
    check_fail(
        r#"
type c ++ :: Name {
    #layout(size = 16, align = 8);
}
    "#,
        expect![[r#"
            Error: found '+' expected '::', '<', 'const', '*', '&', or '{'
               ╭─[ test.zng:2:8 ]
               │
             2 │ type c ++ :: Name {
               │        ┬  
               │        ╰── found '+' expected '::', '<', 'const', '*', '&', or '{'
            ───╯
        "#]],
    );
}

#[test]
fn ordinary_path_starting_with_c_is_unaffected() {
    let parsed = parse_str(
        r#"
type crate::config::Foo {
    #layout(size = 1, align = 1);
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_ty_path!(["crate", "config", "Foo"], &ty.ty);
}

#[test]
fn cpp_mod_rejected_when_nested_inside_another_module() {
    check_fail(
        r#"
mod crate::foo {
    mod c++::a::b {
        type Name {
            #layout(size = 16, align = 8);
        }
    }
}
    "#,
        expect![[r#"
            Error: `c++::` modules can only appear at the top level of a file, not nested inside another module
               ╭─[ test.zng:3:9 ]
               │
             3 │     mod c++::a::b {
               │         ────┬────  
               │             ╰────── `c++::` modules can only appear at the top level of a file, not nested inside another module
            ───╯
        "#]],
    );
}

#[test]
fn free_fn_rejected_inside_cpp_scope() {
    // There's no real Rust function living in the generated c++::-only
    // module tree, so a bare `fn` declaration inside `mod c++::a { ... }`
    // must be rejected (requiring `safe`/`unsafe` for C++ functions, or
    // `::`/`crate::` for Rust functions), even if an unrelated `use ... as foo`
    // alias is in scope.
    check_fail(
        r#"
use crate::other as foo;
mod c++::a {
    fn foo(i32) -> bool;
}
    "#,
        expect![[r#"
            Error: C++ functions must be marked `safe` or `unsafe`
               ╭─[ test.zng:4:5 ]
               │
             4 │     fn foo(i32) -> bool;
               │     ─────────┬─────────  
               │              ╰─────────── C++ functions must be marked `safe` or `unsafe`
               │ 
               │ Help: inside a `c++::` module, prefix Rust functions with `::` or `crate::`
            ───╯
        "#]],
    );
}

#[test]
fn crate_mod_rejected_when_nested_inside_another_module() {
    check_fail(
        r#"
mod crate::foo {
    mod crate::bar {
        type Name {
            #layout(size = 16, align = 8);
        }
    }
}
    "#,
        expect![[r#"
            Error: `crate::` modules can only appear at the top level of a file, not nested inside another module
               ╭─[ test.zng:3:9 ]
               │
             3 │     mod crate::bar {
               │         ─────┬────  
               │              ╰────── `crate::` modules can only appear at the top level of a file, not nested inside another module
            ───╯
        "#]],
    );
}

#[test]
fn absolute_mod_rejected_when_nested_inside_another_module() {
    check_fail(
        r#"
mod crate::foo {
    mod ::std::bar {
        type Name {
            #layout(size = 16, align = 8);
        }
    }
}
    "#,
        expect![[r#"
            Error: `::` modules can only appear at the top level of a file, not nested inside another module
               ╭─[ test.zng:3:9 ]
               │
             3 │     mod ::std::bar {
               │         ─────┬────  
               │              ╰────── `::` modules can only appear at the top level of a file, not nested inside another module
            ───╯
        "#]],
    );
}

#[test]
fn relative_mod_nested_in_cpp_mod_extends_the_cpp_prefix() {
    // `mod c++::a { mod b { type Name { ... } } }` is `c++::a::b::Name` --
    // a plain relative `mod` nested inside a `c++::` scope composes onto the
    // same `c++::` prefix rather than starting a fresh Rust module path.
    let parsed = parse_str(
        r#"
mod c++::a {
    mod b {
        type Name {
            #layout(size = 16, align = 8);
        }
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_cpp(&["a", "b", "Name"], &ty.ty);
}

#[test]
fn aliased_type_referenced_inside_cpp_mod_still_resolves_via_the_alias() {
    // A bare relative name that matches an alias must still expand via that
    // alias, even when referenced from inside a `c++::` scope -- it must
    // NOT get the `c++::` prefix composed onto it instead.
    let parsed = parse_str(
        r#"
use ::std::string::String as MyString;

mod c++::a {
    type Name {
        safe fn take<'a>(&'a const self, MyString);
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    // The type's own declaration still composes onto the c++:: prefix:
    assert_cpp(&["a", "Name"], &ty.ty);
    // But the aliased argument type resolves as the real Rust path, not
    // `Cpp(["a", "MyString"])`:
    let method = ty.cpp_methods.first().expect("no method parsed");
    assert_ty_path!(["std", "string", "String"], &method.inputs[0]);
}

#[test]
fn unprefixed_root_alias_referenced_inside_cpp_mod_keeps_rust_target() {
    let parsed = parse_str(
        r#"
use std::string::String as MyString;

mod c++::a {
    type Name {
        safe fn take<'a>(&'a const self, MyString);
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_cpp(&["a", "Name"], &ty.ty);
    let method = ty.cpp_methods.first().expect("no method parsed");
    assert_ty_path!(["std", "string", "String"], &method.inputs[0]);
}

#[test]
fn alias_in_cpp_mod_inherited_into_nested_mod_resolves_against_declaring_mod() {
    let parsed = parse_str(
        r#"
mod c++::a {
    use Foo as MyFoo;
    mod b {
        extern "C++" {
            safe fn make() -> MyFoo;
        }
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let func = parsed
        .spec
        .extern_cpp_funcs
        .first()
        .expect("no extern C++ fn parsed");
    assert_cpp(&["a", "Foo"], &func.output);
}

#[test]
fn same_module_alias_chains_are_order_independent_and_shadow_parent() {
    let parsed = parse_str(
        r#"
use c++::outer as sub;
use ns::Top as MyTop;
use c++::root_ns as ns;

mod c++::a {
    use sub::Foo as MyFoo;
    use b as sub;

    extern "C++" {
        safe fn make_foo() -> MyFoo;
        safe fn make_top() -> MyTop;
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    assert_eq!(parsed.spec.extern_cpp_funcs.len(), 2);
    assert_cpp(&["a", "b", "Foo"], &parsed.spec.extern_cpp_funcs[0].output);
    assert_cpp(&["root_ns", "Top"], &parsed.spec.extern_cpp_funcs[1].output);

    // Cyclic and duplicate aliases in the same module emit parse errors cleanly without hanging.
    check_fail(
        r#"
use b as a;
use a as b;
extern "C++" {
    safe fn f(&a);
}
"#,
        expect![[r#"
            Error: cyclic `use` alias definition `a`
               ╭─[ test.zng:2:1 ]
               │
             2 │ use b as a;
               │ ─────┬─────  
               │      ╰─────── cyclic `use` alias definition `a`
            ───╯
        "#]],
    );

    check_fail(
        r#"
use c++::outer as sub;
mod a {
    use sub::Inner as sub;
}
"#,
        expect![[r#"
            Error: cyclic `use` alias definition `sub`
               ╭─[ test.zng:4:5 ]
               │
             4 │     use sub::Inner as sub;
               │     ───────────┬──────────  
               │                ╰──────────── cyclic `use` alias definition `sub`
            ───╯
        "#]],
    );

    check_fail(
        r#"
use c++::x as Foo;
use c++::y as Foo;
"#,
        expect![[r#"
            Error: duplicate `use` alias `Foo` in the same module
               ╭─[ test.zng:3:1 ]
               │
             3 │ use c++::y as Foo;
               │ ─────────┬────────  
               │          ╰────────── duplicate `use` alias `Foo` in the same module
               │
               ├─[ test.zng:3:1 ]
               │
             2 │ use c++::x as Foo;
               │ ─────────┬────────  
               │          ╰────────── previously defined here
            ───╯
        "#]],
    );
}

#[test]
fn entity_path_rust_reference_formats_rust_paths_and_rejects_empty_or_cpp() {
    assert_eq!(
        EntityPath::crate_relative(["foo", "Bar"]).rust_reference(),
        Some("crate::foo::Bar".to_owned()),
    );
    assert_eq!(
        EntityPath::rust(["foo", "Bar"]).rust_reference(),
        Some("::foo::Bar".to_owned()),
    );
    assert_eq!(EntityPath::Rust(Vec::new()).rust_reference(), None);
    assert_eq!(EntityPath::Cpp(Vec::new()).rust_reference(), None);
    assert_eq!(EntityPath::cpp(["a", "Foo"]).rust_reference(), None);
}

#[test]
fn files_with_only_comments_parse() {
    for text in [
        "",
        "   \n\t\n",
        "// Only a comment.",
        "// Only a comment, with a final newline.\n",
        "\n// Two comments,\n// and no items, with an apostrophe: isn't it?\n\n",
    ] {
        let parsed = parse_str(text, NullCfg, |e| {
            panic!("unexpected diagnostic for {text:?}: {e}")
        });
        assert_eq!(parsed.errors, 0, "{text:?}");
        assert_eq!(parsed.warnings, 0, "{text:?}");
        assert!(parsed.spec.types.is_empty(), "{text:?}");
    }
}

#[test]
fn comments_after_the_last_item_parse() {
    for text in [
        "type c++::a::B {\n}\n// A trailing comment.\n",
        "type c++::a::B {\n}\n// A trailing comment.",
    ] {
        let parsed = parse_str(text, NullCfg, |e| {
            panic!("unexpected diagnostic for {text:?}: {e}")
        });
        assert_eq!(parsed.errors, 0, "{text:?}");
        assert_eq!(parsed.warnings, 0, "{text:?}");
        assert_eq!(parsed.spec.types.len(), 1, "{text:?}");
    }
}

#[test]
fn cpp_type_capabilities_follow_from_layout_and_trivially_relocatable() {
    let parsed = parse_str(
        r#"
#allow(layout_without_relocatable)

type c++::a::ByValue {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}

type c++::a::Conservative {
    #layout_conservative(size = 32, align = 16);
    #trivially_relocatable;
}

// Promised, rather than checked in C++.
type c++::a::Unchecked {
    #layout(size = 8, align = 8);
    #unsafe(trivially_relocatable);
}

// A layout without `#trivially_relocatable` is only checked in C++.
type c++::a::NotRelocatable {
    #layout(size = 8, align = 4);
}

// No layout: Rust only refers to it.
type c++::a::Opaque {
}
"#,
        NullCfg,
        |e| panic!("unexpected diagnostic: {e}"),
    );
    let ty = |name: &str| {
        parsed
            .spec
            .types
            .iter()
            .find(|t| matches!(&t.ty, RustType::Cpp(segs) if segs.path.last().unwrap() == name))
            .unwrap_or_else(|| panic!("no type {name}"))
    };
    assert_eq!(ty("ByValue").cpp_name().as_deref(), Some("::a::ByValue"));
    assert_eq!(
        ty("ByValue").cpp_by_value_layout(),
        Some(LayoutPolicy::StackAllocated { size: 8, align: 4 })
    );
    assert_eq!(
        ty("Conservative").cpp_by_value_layout(),
        Some(LayoutPolicy::Conservative {
            size: 32,
            align: 16
        })
    );
    assert_eq!(
        ty("Unchecked").cpp_by_value_layout(),
        Some(LayoutPolicy::StackAllocated { size: 8, align: 8 })
    );
    assert_eq!(
        ty("Unchecked").trivially_relocatable,
        TriviallyRelocatable::Unchecked
    );
    assert_eq!(
        ty("ByValue").trivially_relocatable,
        TriviallyRelocatable::Checked
    );
    assert_eq!(ty("NotRelocatable").cpp_by_value_layout(), None);
    assert_eq!(
        ty("NotRelocatable").layout,
        Some(LayoutPolicy::StackAllocated { size: 8, align: 4 })
    );
    assert_eq!(ty("Opaque").cpp_by_value_layout(), None);
    assert_eq!(ty("Opaque").layout, None);
}

#[test]
fn trivially_relocatable_can_be_declared_in_another_file() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type c++::B {
    #layout(size = 8, align = 4);
}
"#,
    )]);
    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
type c++::B {
    #trivially_relocatable;
}
"#,
        NullCfg,
        &resolver,
        |e| panic!("unexpected diagnostic: {e}"),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert!(ty.cpp_by_value_layout().is_some());
    assert_eq!(ty.trivially_relocatable, TriviallyRelocatable::Checked);
}

#[test]
fn trivially_relocatable_merge_precedence() {
    // Direction 1: `Unchecked` in root + `Checked` in merged file -> `Unchecked`.
    let resolver1 = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
"#,
    )]);
    let parsed1 = parse_str_with_resolver(
        r#"
merge "./a.zng";
type c++::B {
    #unsafe(trivially_relocatable);
}
"#,
        NullCfg,
        &resolver1,
        |e| panic!("unexpected diagnostic: {e}"),
    );
    let ty1 = parsed1.spec.types.first().expect("no type parsed");
    assert!(ty1.cpp_by_value_layout().is_some());
    assert_eq!(ty1.trivially_relocatable, TriviallyRelocatable::Unchecked);

    // Direction 2: `Checked` in root + `Unchecked` in merged file -> `Unchecked`.
    let resolver2 = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #unsafe(trivially_relocatable);
}
"#,
    )]);
    let parsed2 = parse_str_with_resolver(
        r#"
merge "./a.zng";
type c++::B {
    #trivially_relocatable;
}
"#,
        NullCfg,
        &resolver2,
        |e| panic!("unexpected diagnostic: {e}"),
    );
    let ty2 = parsed2.spec.types.first().expect("no type parsed");
    assert!(ty2.cpp_by_value_layout().is_some());
    assert_eq!(ty2.trivially_relocatable, TriviallyRelocatable::Unchecked);
}

#[test]
fn trivially_relocatable_without_layout_is_rejected() {
    check_fail(
        r#"
type c++::B {
    #trivially_relocatable;
}

type c++::C {
    #unsafe(trivially_relocatable);
}
"#,
        expect![[r#"
            Error: `#trivially_relocatable` needs a `#layout(...)` or `#layout_conservative(...)`, which Rust needs to hold the C++ type by value
               ╭─[ test.zng:3:5 ]
               │
             3 │     #trivially_relocatable;
               │     ───────────┬───────────  
               │                ╰───────────── `#trivially_relocatable` needs a `#layout(...)` or `#layout_conservative(...)`, which Rust needs to hold the C++ type by value
            ───╯
            Error: `#unsafe(trivially_relocatable)` needs a `#layout(...)` or `#layout_conservative(...)`, which Rust needs to hold the C++ type by value
               ╭─[ test.zng:7:5 ]
               │
             7 │     #unsafe(trivially_relocatable);
               │     ───────────────┬───────────────  
               │                    ╰───────────────── `#unsafe(trivially_relocatable)` needs a `#layout(...)` or `#layout_conservative(...)`, which Rust needs to hold the C++ type by value
            ───╯
        "#]],
    );
}

#[test]
fn trivially_relocatable_on_rust_type_is_rejected() {
    check_fail(
        r#"
type crate::Foo {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}

type crate::Bar {
    #layout(size = 8, align = 4);
    #unsafe(trivially_relocatable);
}
"#,
        expect![[r#"
            Error: `#trivially_relocatable` is only for `c++::` types (Rust types are always trivially relocatable)
               ╭─[ test.zng:4:5 ]
               │
             4 │     #trivially_relocatable;
               │     ───────────┬───────────  
               │                ╰───────────── `#trivially_relocatable` is only for `c++::` types (Rust types are always trivially relocatable)
            ───╯
            Error: `#unsafe(trivially_relocatable)` is only for `c++::` types (Rust types are always trivially relocatable)
               ╭─[ test.zng:9:5 ]
               │
             9 │     #unsafe(trivially_relocatable);
               │     ───────────────┬───────────────  
               │                    ╰───────────────── `#unsafe(trivially_relocatable)` is only for `c++::` types (Rust types are always trivially relocatable)
            ───╯
        "#]],
    );
}

#[test]
fn removed_cpp_directives_explain_the_new_syntax() {
    check_fail(
        r#"
type crate::A {
    #cpp_stack_owned "::A" (size = 8, align = 4);
}

type crate::B {
    #cpp_ref "::B";
}

type c++::C {
    #cpp_ref;
}

type crate::D {
    #layout(size = 8, align = 8);
    #cpp_heap_allocated "::D";
}

type crate::E {
    #layout(size = 8, align = 8);
    #cpp_value "0" "::E";
}
"#,
        expect![[r#"
            Error: `#cpp_stack_owned` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
               ╭─[ test.zng:3:5 ]
               │
             3 │     #cpp_stack_owned "::A" (size = 8, align = 4);
               │     ──────────────────────┬──────────────────────  
               │                           ╰──────────────────────── no longer supported
               │ 
               │ Help: declare the type as `type c++::ns::T { #layout(size = X, align = Y); #trivially_relocatable; }`; Rust holds C++ types by value when they have a layout and are trivially relocatable
            ───╯
            Error: `#cpp_ref` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
               ╭─[ test.zng:7:5 ]
               │
             7 │     #cpp_ref "::B";
               │     ───────┬───────  
               │            ╰───────── no longer supported
               │ 
               │ Help: declare the type as `type c++::ns::T { ... }` with no layout; Rust can then only refer to it
            ───╯
            Error: `#cpp_ref` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
                ╭─[ test.zng:11:5 ]
                │
             11 │     #cpp_ref;
                │     ────┬────  
                │         ╰────── no longer supported
                │ 
                │ Help: declare the type as `type c++::ns::T { ... }` with no layout; Rust can then only refer to it
            ────╯
            Error: `#cpp_heap_allocated` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
                ╭─[ test.zng:16:5 ]
                │
             16 │     #cpp_heap_allocated "::D";
                │     ─────────────┬────────────  
                │                  ╰────────────── no longer supported
                │ 
                │ Help: declare the type as `type c++::ns::T { ... }` with no layout, and own it through a C++ smart pointer, e.g. `type c++::std::unique_ptr<c++::ns::T> { #layout(size = 8, align = 8); #trivially_relocatable; }`
            ────╯
            Error: `#cpp_value` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
                ╭─[ test.zng:21:5 ]
                │
             21 │     #cpp_value "0" "::E";
                │     ──────────┬──────────  
                │               ╰──────────── no longer supported
                │ 
                │ Help: declare the type as `type c++::ns::T { ... }` with no layout, and own it through a C++ smart pointer, e.g. `type c++::std::unique_ptr<c++::ns::T> { #layout(size = 8, align = 8); #trivially_relocatable; }`
            ────╯
        "#]],
    );

    // Splitting a legacy `#cpp_*` directive from `#layout` across `merge`d files, reopened `type`
    // blocks, or `type<T>` templates must not emit cascading layout conflict or missing-layout
    // errors.
    let resolver = MockFilesystem::new(vec![(
        "./layout.zng",
        r#"
type crate::Split {
    #layout(size = 8, align = 4);
}
"#,
    )]);
    let err = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            r#"
merge "./layout.zng";

type crate::Split {
    #cpp_stack_owned "::Split";
}

type crate::Reopened {
    #layout(size = 8, align = 4);
}
type crate::Reopened {
    #cpp_ref "::Reopened";
}

type<T> crate::Templated<T> {
    #layout(size = 8, align = 8);
}
type crate::Templated<i32> {
    #cpp_stack_owned "::Templated";
}
"#,
            NullCfg,
            &resolver,
            |_| {},
        );
    })
    .unwrap_err()
    .downcast::<ErrorText>()
    .expect("expected ErrorText")
    .0;
    assert!(
        err.contains("`#cpp_stack_owned` is no longer supported"),
        "{err}"
    );
    assert!(err.contains("`#cpp_ref` is no longer supported"), "{err}");
    assert!(
        !err.contains("Conflicting layout policy"),
        "unexpected layout conflict error:\n{err}"
    );
    assert!(
        !err.contains("No layout policy"),
        "unexpected missing layout error:\n{err}"
    );
}

#[test]
fn heap_allocated_only_by_ref_and_unsized_on_cpp_type_are_rejected() {
    check_fail(
        r#"
type c++::A {
    #heap_allocated;
}

type c++::B {
    #only_by_ref;
}

type c++::C {
    wellknown_traits(?Sized);
}
"#,
        expect![[r#"
            Error: `#heap_allocated` and `#only_by_ref` are for Rust types; a `c++::` type without a layout is already reference-only
               ╭─[ test.zng:3:5 ]
               │
             3 │     #heap_allocated;
               │     ───────┬───────  
               │            ╰───────── `#heap_allocated` and `#only_by_ref` are for Rust types; a `c++::` type without a layout is already reference-only
            ───╯
            Error: `#heap_allocated` and `#only_by_ref` are for Rust types; a `c++::` type without a layout is already reference-only
               ╭─[ test.zng:7:5 ]
               │
             7 │     #only_by_ref;
               │     ──────┬─────  
               │           ╰─────── `#heap_allocated` and `#only_by_ref` are for Rust types; a `c++::` type without a layout is already reference-only
            ───╯
            Error: `?Sized` is not supported on C++ types; a `c++::` type without a layout is already reference-only
                ╭─[ test.zng:11:22 ]
                │
             11 │     wellknown_traits(?Sized);
                │                      ───┬──  
                │                         ╰──── `?Sized` is not supported on C++ types; a `c++::` type without a layout is already reference-only
            ────╯
        "#]],
    );

    // Also rejected when inherited from `#unstable(template_types)`.
    let r = catch_unwind(|| {
        let _ = parse_str(
            "#unstable(template_types)\ntype<T> T { #only_by_ref; }\ntype c++::Foo {}\n",
            NullCfg,
            |_| {},
        );
    });
    let err = r
        .expect_err("expected failure")
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("`#heap_allocated` and `#only_by_ref` are for Rust types"),
        "unexpected error:\n{err}"
    );
}

#[test]
fn processed_files_includes_imported_module_and_its_merged_files() {
    let resolver = MockFilesystem::new(vec![
        (
            "./bar.zng",
            "#crate bar_bindings\nmerge \"./bar_part.zng\";\ntype c++::ns::Bar {}\n",
        ),
        ("./bar_part.zng", "type c++::ns::BarPart {}\n"),
    ]);
    let parsed = parse_str_with_resolver("import \"./bar.zng\";\n", NullCfg, &resolver, |_| {});
    let file_names: Vec<_> = parsed
        .processed_files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(file_names.len(), 3, "got: {file_names:?}");
    assert!(file_names.contains(&"test.zng"));
    assert!(file_names.contains(&"bar.zng"));
    assert!(file_names.contains(&"bar_part.zng"));
}

#[test]
fn crate_directive_sets_the_crate_name_and_allows_identical_duplicates() {
    let parsed = parse_str(
        r#"
#crate my_bindings
#crate my_bindings
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.crate_name.as_deref(), Some("my_bindings"));

    // Valid Rust crate names that happen to be Zngur-only keywords (`merge`, `import`, `safe`).
    for kw_crate in ["merge", "import", "safe"] {
        let parsed = parse_str(&format!("#crate {kw_crate}\n"), NullCfg, |w| {
            panic!("unexpected warning: {w}")
        });
        assert_eq!(parsed.spec.crate_name.as_deref(), Some(kw_crate));
    }
}

#[test]
fn crate_directive_rejects_self_and_super() {
    check_fail(
        r#"
#crate self
#crate super
#crate Self
#crate struct
#crate crate
#crate mod
mod foo {
    #crate bar
}
"#,
        expect![[r#"
            Error: `self` cannot be used as a `#crate` name
               ╭─[ test.zng:2:2 ]
               │
             2 │ #crate self
               │  ─────┬────  
               │       ╰────── `self` cannot be used as a `#crate` name
            ───╯
            Error: `super` cannot be used as a `#crate` name
               ╭─[ test.zng:3:2 ]
               │
             3 │ #crate super
               │  ─────┬─────  
               │       ╰─────── `super` cannot be used as a `#crate` name
            ───╯
            Error: `Self` cannot be used as a `#crate` name
               ╭─[ test.zng:4:2 ]
               │
             4 │ #crate Self
               │  ─────┬────  
               │       ╰────── `Self` cannot be used as a `#crate` name
            ───╯
            Error: `struct` cannot be used as a `#crate` name
               ╭─[ test.zng:5:2 ]
               │
             5 │ #crate struct
               │  ──────┬─────  
               │        ╰─────── `struct` cannot be used as a `#crate` name
            ───╯
            Error: `crate` cannot be used as a `#crate` name
               ╭─[ test.zng:6:2 ]
               │
             6 │ #crate crate
               │  ─────┬─────  
               │       ╰─────── `crate` cannot be used as a `#crate` name
            ───╯
            Error: `mod` cannot be used as a `#crate` name
               ╭─[ test.zng:7:2 ]
               │
             7 │ #crate mod
               │  ────┬────  
               │      ╰────── `mod` cannot be used as a `#crate` name
            ───╯
            Error: `#crate` can only be used at the top level of a file, not inside a `mod` block
               ╭─[ test.zng:9:6 ]
               │
             9 │     #crate bar
               │      ────┬────  
               │          ╰────── `#crate` can only be used at the top level of a file, not inside a `mod` block
            ───╯
        "#]],
    );
}

#[test]
fn crate_directive_not_allowed_in_merged_files() {
    let resolver = MockFilesystem::new(vec![("./merged.zng", "#crate other")]);
    check_import_fail(
        r#"
merge "./merged.zng";
"#,
        expect![[r#"
            Error: `#crate` can only be used in the top-level file of a zngur module, not in merged files
               ╭─[ ./merged.zng:1:2 ]
               │
             1 │ #crate other
               │  ─────┬─────  
               │       ╰─────── `#crate` can only be used in the top-level file of a zngur module, not in merged files
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn conflicting_crate_directives_are_rejected() {
    check_fail(
        r#"
#crate first
#crate second
"#,
        expect![[r#"
            Error: conflicting `#crate`: already set to `first`
               ╭─[ test.zng:3:2 ]
               │
             3 │ #crate second
               │  ──────┬─────  
               │        ╰─────── conflicting `#crate`: already set to `first`
            ───╯
        "#]],
    );
}

#[test]
fn imported_module_cpp_types_and_crate_are_resolved_including_merged_files() {
    let resolver = MockFilesystem::new(vec![
        (
            "./bar.zng",
            r#"
#crate bar_bindings
merge "./bar_extra.zng";

type c++::ns::Bar {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}

type crate::RustOnly {
    #layout(size = 1, align = 1);
}
"#,
        ),
        (
            "./bar_extra.zng",
            r#"
type c++::ns::BarExtra {
}
"#,
        ),
    ]);
    let parsed = parse_str_with_resolver(
        r#"
import "./bar.zng";

mod crate {
    fn use_bar(&const c++::ns::Bar, &const c++::ns::BarExtra);
}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    let [module] = parsed.spec.imported_modules.as_slice() else {
        panic!("expected one imported module");
    };
    assert_eq!(module.crate_name.as_deref(), Some("bar_bindings"));
    assert_eq!(
        module.cpp_types,
        vec![
            vec!["ns".to_owned(), "Bar".to_owned()],
            vec!["ns".to_owned(), "BarExtra".to_owned()],
        ]
    );
    assert_eq!(
        module.cpp_by_value_types,
        vec![(
            RustType::Cpp(vec!["ns".to_owned(), "Bar".to_owned()].into()),
            "::ns::Bar".to_owned(),
        )]
    );
    // The imported module's types aren't part of this module's spec.
    assert!(parsed.spec.types.is_empty());
}

#[test]
fn duplicate_module_import_across_merged_files_succeeds() {
    let resolver = MockFilesystem::new(vec![
        ("./bar.zng", "#crate bar_bindings\ntype c++::ns::Bar {}\n"),
        ("./a.zng", "import \"./bar.zng\";\n"),
        ("./b.zng", "import \"./bar.zng\";\n"),
    ]);
    let parsed = parse_str_with_resolver(
        "merge \"./a.zng\";\nmerge \"./b.zng\";\n",
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.imported_modules.len(), 1);
    let bar_count = parsed
        .processed_files
        .iter()
        .filter(|p| p.file_name().and_then(|s| s.to_str()) == Some("bar.zng"))
        .count();
    assert_eq!(
        bar_count, 1,
        "processed_files: {:?}",
        parsed.processed_files
    );
}

#[test]
fn colliding_cpp_types_across_local_and_imported_modules_are_rejected() {
    // 1. Local `c++::` type colliding with imported `c++::` type.
    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        "#crate bar_bindings\ntype c++::ns::Bar {}\n",
    )]);
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            "import \"./bar.zng\";\ntype c++::ns::Bar {}\n",
            NullCfg,
            &resolver,
            |_| {},
        );
    });
    let err = r
        .expect_err("expected failure")
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("`c++::ns::Bar` is already imported from `./bar.zng`"),
        "unexpected error:\n{err}"
    );

    // 2. Two imported modules from different crates declaring the same `c++::` type.
    let resolver2 = MockFilesystem::new(vec![
        (
            "./one.zng",
            "#crate one_bindings\ntype c++::ns::Shared {}\n",
        ),
        (
            "./two.zng",
            "#crate two_bindings\ntype c++::ns::Shared {}\n",
        ),
    ]);
    let r2 = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            "import \"./one.zng\";\nimport \"./two.zng\";\n",
            NullCfg,
            &resolver2,
            |_| {},
        );
    });
    let err2 = r2
        .expect_err("expected failure")
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err2.contains(
            "`c++::ns::Shared` is imported from multiple crates (`one_bindings` and `two_bindings`)"
        ),
        "unexpected error:\n{err2}"
    );
}

#[test]
fn imported_module_spans_do_not_pollute_importing_module_diagnostics() {
    // `./bar.zng` declares `crate::Foo` (valid). `test.zng` declares `crate::Foo` WITHOUT a
    // layout: the missing-layout error must point to `test.zng`, never `./bar.zng`.
    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        "#crate bar_bindings\ntype crate::Foo { #layout(size = 1, align = 1); }\n",
    )]);
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            "import \"./bar.zng\";\ntype crate::Foo {}\n",
            NullCfg,
            &resolver,
            |_| {},
        );
    });
    let err = r
        .expect_err("expected failure")
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(err.contains("test.zng"), "missing `test.zng` in:\n{err}");
    assert!(
        !err.contains("bar.zng"),
        "imported module `./bar.zng` polluted spans in:\n{err}"
    );
}

#[test]
fn error_in_imported_module_does_not_cascade_or_abort_sibling_merges() {
    // `./broken.zng` mistakenly puts `#crate` in a merged file `./broken_sub.zng`.
    // 1. We should NOT get a second misleading "needs a `#crate <name>`" error.
    // 2. Sibling `merge "./sibling.zng";` must still be parsed (so an error in `./sibling.zng`
    //    is still reported).
    let resolver = MockFilesystem::new(vec![
        (
            "./broken.zng",
            "merge \"./broken_sub.zng\";\ntype c++::ns::Bar {}\n",
        ),
        ("./broken_sub.zng", "#crate bar_bindings\n"),
        ("./sibling.zng", "type crate::MissingLayout {}\n"),
    ]);
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            "import \"./broken.zng\";\nmerge \"./sibling.zng\";\n",
            NullCfg,
            &resolver,
            |_| {},
        );
    });
    let err = r
        .expect_err("expected failure")
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("`#crate` can only be used in the top-level file"),
        "missing `#crate` merged-file error in:\n{err}"
    );
    assert!(
        !err.contains("needs a `#crate <name>` directive"),
        "unexpected cascading missing-`#crate` error in:\n{err}"
    );
    assert!(
        err.contains("sibling.zng"),
        "sibling merge `./sibling.zng` was skipped:\n{err}"
    );
}

#[test]
fn imported_module_with_cpp_types_needs_a_crate() {
    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        r#"
type c++::ns::Bar {
}
"#,
    )]);
    check_import_fail(
        r#"
import "./bar.zng";
"#,
        expect![[r#"
            Error: imported module `./bar.zng` declares `c++::` types, so it needs a `#crate <name>` directive for this module to find their Rust wrappers (at `::<name>::generated::...`)
               ╭─[ test.zng:2:1 ]
               │
             2 │ import "./bar.zng";
               │ ─────────┬─────────  
               │          ╰─────────── imported module `./bar.zng` declares `c++::` types, so it needs a `#crate <name>` directive for this module to find their Rust wrappers (at `::<name>::generated::...`)
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn relative_module_imports_inside_merged_subdirectory_are_rebased_onto_root() {
    let resolver = MockFilesystem::new(vec![
        (
            "./sub/part.zng",
            "import \"./bar.zng\";\nimport \"../other/baz.zng\";\nimport \"../../outside/qux.zng\";\nimport \"header_only.zng\";\n",
        ),
        (
            "./sub/bar.zng",
            "#crate bar_bindings\ntype c++::ns::Bar {}\n",
        ),
        (
            "./other/baz.zng",
            "#crate baz_bindings\ntype c++::ns::Baz {}\n",
        ),
        (
            "../outside/qux.zng",
            "#crate qux_bindings\ntype c++::ns::Qux {}\n",
        ),
    ]);
    let parsed = parse_str_with_resolver("merge \"./sub/part.zng\";\n", NullCfg, &resolver, |w| {
        panic!("unexpected warning: {w}")
    });
    assert_eq!(parsed.spec.imported_modules.len(), 4);
    assert_eq!(
        parsed.spec.imported_modules[0].path.to_str(),
        Some("./sub/bar.zng")
    );
    assert_eq!(
        parsed.spec.imported_modules[0].crate_name.as_deref(),
        Some("bar_bindings")
    );
    assert_eq!(
        parsed.spec.imported_modules[0].cpp_types,
        vec![vec!["ns".to_owned(), "Bar".to_owned()]]
    );
    assert_eq!(
        parsed.spec.imported_modules[1].path.to_str(),
        Some("./other/baz.zng")
    );
    assert_eq!(
        parsed.spec.imported_modules[1].crate_name.as_deref(),
        Some("baz_bindings")
    );
    assert_eq!(
        parsed.spec.imported_modules[1].cpp_types,
        vec![vec!["ns".to_owned(), "Baz".to_owned()]]
    );
    assert_eq!(
        parsed.spec.imported_modules[2].path.to_str(),
        Some("../outside/qux.zng")
    );
    assert_eq!(
        parsed.spec.imported_modules[2].crate_name.as_deref(),
        Some("qux_bindings")
    );
    assert_eq!(
        parsed.spec.imported_modules[2].cpp_types,
        vec![vec!["ns".to_owned(), "Qux".to_owned()]]
    );
    // Bare C++ include-search paths without `./` or `../` are preserved as-is.
    assert_eq!(
        parsed.spec.imported_modules[3].path,
        std::path::PathBuf::from("header_only.zng")
    );
    assert_eq!(parsed.spec.imported_modules[3].crate_name, None);
}

#[test]
fn importing_module_with_same_crate_directive_suggests_merge() {
    let resolver = MockFilesystem::new(vec![(
        "./sub.zng",
        "#crate my_crate\ntype c++::ns::Bar {}\n",
    )]);
    check_import_fail(
        r#"
#crate my_crate
import "./sub.zng";
"#,
        expect![[r#"
            Error: imported module `./sub.zng` has the same `#crate` (`my_crate`) as this module; use `merge "./sub.zng";` to split `.zng` files within a single crate
               ╭─[ test.zng:3:1 ]
               │
             3 │ import "./sub.zng";
               │ ─────────┬─────────  
               │          ╰─────────── imported module `./sub.zng` has the same `#crate` (`my_crate`) as this module; use `merge "./sub.zng";` to split `.zng` files within a single crate
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn unreadable_imported_module_is_not_an_error() {
    // `import` only has to name the generated header, which may be found via the C++ include
    // path even if the `.zng` file isn't at the relative path.
    let resolver = MockFilesystem::new(vec![] as Vec<(&str, &str)>);
    let parsed = parse_str_with_resolver(
        r#"
import "elsewhere.zng";
"#,
        NullCfg,
        &resolver,
        |_| {},
    );
    assert_eq!(parsed.spec.imported_modules[0].crate_name, None);
}

#[test]
fn lifetimes_parse() {
    use zngur_def::{Lifetime, PrimitiveRustType, RustTrait};

    let parsed = parse_str(
        r#"
type<'a> crate::Parser<'a> {
    #layout(size = 24, align = 8);
    fn rest(&self) -> &'a str;
    fn peek<'b>(&'b mut self) -> &'b str;
    fn elided(&self, &'_ str) -> &'_ str;
}

type Box<dyn for<'a> Fn(&'a i32) -> &'a i32> {
    #layout(size = 16, align = 8);
}

extern "C++" {
    safe fn pick<'a>(&'a i32, &'static i32) -> &'a i32;
    impl<'a> crate::Parser<'a> {
        safe fn first(&self) -> &'a str;
    }
}
"#,
        NullCfg,
        |e| panic!("unexpected warning: {e}"),
    );
    let a = || Lifetime("a".to_owned());
    let anon = || Lifetime("_".to_owned());
    let i32 = || Box::new(RustType::Primitive(PrimitiveRustType::Int(32)));
    let str_ty = || Box::new(RustType::Primitive(PrimitiveRustType::Str));
    let parser_ty = &parsed.spec.types[0];
    assert_eq!(parser_ty.lifetimes, vec![a()]);
    assert_eq!(
        parser_ty.ty,
        RustType::Adt(RustPathAndGenerics {
            path: vec!["crate".to_owned(), "Parser".to_owned()],
            lifetimes: vec![a()],
            ..Default::default()
        })
    );
    let peek = &parser_ty.methods[1].data;
    assert_eq!(peek.lifetimes, vec![Lifetime("b".to_owned())]);
    assert_eq!(peek.receiver_lifetime, Some(Lifetime("b".to_owned())));
    let elided = &parser_ty.methods[2].data;
    assert!(elided.lifetimes.is_empty());
    assert_eq!(
        elided.inputs,
        vec![RustType::Ref(Mutability::Not, Some(anon()), str_ty())]
    );
    assert_eq!(
        elided.output,
        RustType::Ref(Mutability::Not, Some(anon()), str_ty())
    );
    let RustType::Boxed(dyn_fn) = &parsed.spec.types[1].ty else {
        panic!("not a box");
    };
    let RustType::Dyn(RustTrait::Fn { lifetimes, .. }, _) = &**dyn_fn else {
        panic!("not a dyn Fn");
    };
    assert_eq!(lifetimes, &vec![a()]);
    let pick = &parsed.spec.extern_cpp_funcs[0];
    assert_eq!(pick.lifetimes, vec![a()]);
    assert_eq!(
        pick.inputs,
        vec![
            RustType::Ref(Mutability::Not, Some(a()), i32()),
            RustType::Ref(Mutability::Not, Some(Lifetime("static".to_owned())), i32()),
        ]
    );
    assert_eq!(parsed.spec.extern_cpp_impls[0].lifetimes, vec![a()]);
}

#[test]
fn undeclared_lifetime_is_rejected() {
    check_fail(
        r#"
extern "C++" {
    safe fn pick<'a>(&'a i32, &'b i32) -> &'a i32;
}
"#,
        expect![[r#"
            Error: undeclared lifetime `'b`: declare it on the function, `impl` or `type` (e.g. `fn f<'b>(...)`)
               ╭─[ test.zng:3:10 ]
               │
             3 │     safe fn pick<'a>(&'a i32, &'b i32) -> &'a i32;
               │          ────────────────────┬───────────────────  
               │                              ╰───────────────────── undeclared lifetime `'b`: declare it on the function, `impl` or `type` (e.g. `fn f<'b>(...)`)
            ───╯
        "#]],
    );
}

#[test]
fn undeclared_type_and_trait_lifetimes_are_rejected() {
    // Higher-ranked `Fn(...)` inside a trait header is allowed (its references are bound by the
    // `Fn` trait binder rather than the `trait` header itself).
    let _ = parse_str(
        "trait crate::FnAssoc<F = Box<dyn Fn(&str) -> &str>> {}\n",
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );

    check_fail(
        r#"
type crate::Parser<'a> {
    #layout(size = 24, align = 8);
}

type<'a> crate::Unconstrained {
    #layout(size = 8, align = 8);
    fn f(&self, &'a str);
}

trait crate::MyTrait<'a> {
    fn f(&self);
}

trait crate::AnonTrait<'_> {
    fn f(&self);
}

trait ::std::iter::Iterator<Item = &'a str> {
    fn next(&mut self) -> ::std::option::Option<&'static str>;
}

trait ::std::iter::Iterator<Item = &'_ str> {
    fn next(&mut self) -> ::std::option::Option<&'static str>;
}

trait ::std::iter::Iterator<Item = &str> {
    fn next(&mut self) -> ::std::option::Option<&'static str>;
}

trait crate::Chooser {
    fn choose(&self, &'b str) -> &'b str;
}

extern "C++" {
    impl crate::Foo {
        safe fn bar(&self, &'c str);
    }
}
"#,
        expect![[r#"
            Error: undeclared lifetime `'a`: declare it on the function, `impl` or `type` (e.g. `fn f<'a>(...)`)
               ╭─[ test.zng:2:6 ]
               │
             2 │ type crate::Parser<'a> {
               │      ────────┬────────  
               │              ╰────────── undeclared lifetime `'a`: declare it on the function, `impl` or `type` (e.g. `fn f<'a>(...)`)
            ───╯
            Error: unconstrained lifetime `'a` in `type`: `'a` must be used in `crate::Unconstrained` (if `'a` is only used by methods, declare it on each method instead, e.g. `fn f<'a>(...)`)
               ╭─[ test.zng:6:10 ]
               │
             6 │ type<'a> crate::Unconstrained {
               │          ──────────┬─────────  
               │                    ╰─────────── unconstrained lifetime `'a` in `type`: `'a` must be used in `crate::Unconstrained` (if `'a` is only used by methods, declare it on each method instead, e.g. `fn f<'a>(...)`)
            ───╯
            Error: undeclared lifetime `'a`: `trait` declarations don't take lifetime parameters (only `'static` can be used here)
                ╭─[ test.zng:11:7 ]
                │
             11 │ trait crate::MyTrait<'a> {
                │       ─────────┬────────  
                │                ╰────────── undeclared lifetime `'a`: `trait` declarations don't take lifetime parameters (only `'static` can be used here)
            ────╯
            Error: anonymous lifetime `'_` cannot be used in a `trait` header: `trait` declarations don't take lifetime parameters (only `'static` can be used here)
                ╭─[ test.zng:15:7 ]
                │
             15 │ trait crate::AnonTrait<'_> {
                │       ──────────┬─────────  
                │                 ╰─────────── anonymous lifetime `'_` cannot be used in a `trait` header: `trait` declarations don't take lifetime parameters (only `'static` can be used here)
            ────╯
            Error: undeclared lifetime `'a`: `trait` declarations don't take lifetime parameters (only `'static` can be used here)
                ╭─[ test.zng:19:7 ]
                │
             19 │ trait ::std::iter::Iterator<Item = &'a str> {
                │       ──────────────────┬──────────────────  
                │                         ╰──────────────────── undeclared lifetime `'a`: `trait` declarations don't take lifetime parameters (only `'static` can be used here)
            ────╯
            Error: anonymous lifetime `'_` cannot be used in a `trait` header: `trait` declarations don't take lifetime parameters (only `'static` can be used here)
                ╭─[ test.zng:23:7 ]
                │
             23 │ trait ::std::iter::Iterator<Item = &'_ str> {
                │       ──────────────────┬──────────────────  
                │                         ╰──────────────────── anonymous lifetime `'_` cannot be used in a `trait` header: `trait` declarations don't take lifetime parameters (only `'static` can be used here)
            ────╯
            Error: elided reference lifetime cannot be used in a `trait` header: `trait` declarations don't take lifetime parameters (use `&'static ...` if `'static` is intended)
                ╭─[ test.zng:27:7 ]
                │
             27 │ trait ::std::iter::Iterator<Item = &str> {
                │       ─────────────────┬────────────────  
                │                        ╰────────────────── elided reference lifetime cannot be used in a `trait` header: `trait` declarations don't take lifetime parameters (use `&'static ...` if `'static` is intended)
            ────╯
            Error: undeclared lifetime `'b`: declare it on the function, `impl` or `type` (e.g. `fn f<'b>(...)`)
                ╭─[ test.zng:32:5 ]
                │
             32 │     fn choose(&self, &'b str) -> &'b str;
                │     ──────────────────┬─────────────────  
                │                       ╰─────────────────── undeclared lifetime `'b`: declare it on the function, `impl` or `type` (e.g. `fn f<'b>(...)`)
            ────╯
            Error: undeclared lifetime `'c`: declare it on the function, `impl` or `type` (e.g. `fn f<'c>(...)`)
                ╭─[ test.zng:37:14 ]
                │
             37 │         safe fn bar(&self, &'c str);
                │              ───────────┬──────────  
                │                         ╰──────────── undeclared lifetime `'c`: declare it on the function, `impl` or `type` (e.g. `fn f<'c>(...)`)
            ────╯
        "#]],
    );
}

#[test]
fn duplicate_and_shadowed_lifetimes_are_rejected() {
    check_fail(
        r#"
fn dup<'a, 'a>(&'a str);

type<'a> crate::Parser<'a> {
    #layout(size = 24, align = 8);
    fn rest<'a>(&'a self) -> &'a str;
}

extern "C++" {
    impl<'a> crate::Parser<'a> {
        safe fn first<'a>(&'a self) -> &'a str;
    }
}

type Box<dyn for<'a, 'a> Fn(&'a i32)> {
    #layout(size = 16, align = 8);
}

fn shadow_bind<'a>(Box<dyn for<'a> Fn(&'a i32)>);
"#,
        expect![[r#"
            Error: lifetime `'a` is declared twice
               ╭─[ test.zng:2:1 ]
               │
             2 │ fn dup<'a, 'a>(&'a str);
               │ ───────────┬───────────  
               │            ╰───────────── lifetime `'a` is declared twice
            ───╯
            Error: lifetime `'a` is already declared by the `impl` or `type`
               ╭─[ test.zng:6:5 ]
               │
             6 │     fn rest<'a>(&'a self) -> &'a str;
               │     ────────────────┬────────────────  
               │                     ╰────────────────── lifetime `'a` is already declared by the `impl` or `type`
            ───╯
            Error: lifetime `'a` is already declared by the `impl` or `type`
                ╭─[ test.zng:11:14 ]
                │
             11 │         safe fn first<'a>(&'a self) -> &'a str;
                │              ────────────────┬────────────────  
                │                              ╰────────────────── lifetime `'a` is already declared by the `impl` or `type`
            ────╯
            Error: lifetime `'a` is declared twice
                ╭─[ test.zng:15:6 ]
                │
             15 │ type Box<dyn for<'a, 'a> Fn(&'a i32)> {
                │      ────────────────┬───────────────  
                │                      ╰───────────────── lifetime `'a` is declared twice
            ────╯
            Error: lifetime `'a` in `for<...>` is already declared in an outer scope
                ╭─[ test.zng:19:1 ]
                │
             19 │ fn shadow_bind<'a>(Box<dyn for<'a> Fn(&'a i32)>);
                │ ────────────────────────┬───────────────────────  
                │                         ╰───────────────────────── lifetime `'a` in `for<...>` is already declared in an outer scope
            ────╯
        "#]],
    );
}

#[test]
fn builtin_lifetimes_in_declarations_are_rejected() {
    check_fail(
        r#"
fn f<'static>(&'static str);
fn g<'_>(&'_ str);

type<'static> crate::Foo {
    #layout(size = 8, align = 8);
    fn bar(&self);
}

type Box<dyn for<'static> Fn(&'static str)> {
    #layout(size = 16, align = 8);
}

type Box<dyn for<'_> Fn(&'_ str)> {
    #layout(size = 16, align = 8);
}
"#,
        expect![[r#"
            Error: `'static` can't be declared
               ╭─[ test.zng:2:1 ]
               │
             2 │ fn f<'static>(&'static str);
               │ ─────────────┬─────────────  
               │              ╰─────────────── `'static` can't be declared
            ───╯
            Error: `'_` can't be declared
               ╭─[ test.zng:3:1 ]
               │
             3 │ fn g<'_>(&'_ str);
               │ ────────┬────────  
               │         ╰────────── `'_` can't be declared
            ───╯
            Error: `'static` can't be declared
               ╭─[ test.zng:5:15 ]
               │
             5 │ type<'static> crate::Foo {
               │               ─────┬────  
               │                    ╰────── `'static` can't be declared
            ───╯
            Error: `'static` can't be declared
                ╭─[ test.zng:10:6 ]
                │
             10 │ type Box<dyn for<'static> Fn(&'static str)> {
                │      ───────────────────┬──────────────────  
                │                         ╰──────────────────── `'static` can't be declared
            ────╯
            Error: `'_` can't be declared
                ╭─[ test.zng:14:6 ]
                │
             14 │ type Box<dyn for<'_> Fn(&'_ str)> {
                │      ──────────────┬─────────────  
                │                    ╰─────────────── `'_` can't be declared
            ────╯
        "#]],
    );
}

#[test]
fn fn_trait_syntax_and_outer_lifetime_errors() {
    // Bare `for Fn(...)` without `<...>` is rejected as a syntax error.
    let r = catch_unwind(|| {
        let _ = parse_str(
            "type Box<dyn for Fn(i32)> { #layout(size = 16, align = 8); }\n",
            NullCfg,
            |_| {},
        );
    });
    assert!(r.is_err(), "expected bare `dyn for Fn(i32)` to fail");

    check_fail(
        r#"
fn f<'a>(Box<dyn Fn(&'a str)>);

type Box<dyn Fn(&'z str)> {
    #layout(size = 16, align = 8);
}
"#,
        expect![[r#"
            Error: outer lifetime `'a` can't be used inside `Fn(...)`: use `for<'a> Fn(...)` for a higher-ranked function trait
               ╭─[ test.zng:2:1 ]
               │
             2 │ fn f<'a>(Box<dyn Fn(&'a str)>);
               │ ───────────────┬──────────────  
               │                ╰──────────────── outer lifetime `'a` can't be used inside `Fn(...)`: use `for<'a> Fn(...)` for a higher-ranked function trait
            ───╯
            Error: undeclared lifetime `'z`: declare it with `for<'z> Fn(...)`
               ╭─[ test.zng:4:6 ]
               │
             4 │ type Box<dyn Fn(&'z str)> {
               │      ──────────┬─────────  
               │                ╰─────────── undeclared lifetime `'z`: declare it with `for<'z> Fn(...)`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_type_lifetimes_and_parameter_ordering_are_rejected() {
    for src in [
        "type<'a> c++::Foo {}\n",
        "mod c++::ns { type<'a> Foo {} }\n",
        "fn f(c++::Foo<'static>);\n",
        "mod c++::ns { extern \"C++\" { safe fn f(Foo<'static>); } }\n",
        "type ::std::slice::Iter<i32, 'static> { #layout(size = 16, align = 8); }\n",
        "fn f<i32, 'a>(&'a str);\n",
        "#unstable(template_types)\ntype<T, 'a> crate::Foo<'a, T> { #layout(size = 8, align = 8); }\n",
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains("`c++::` types don't")
                || err.contains("lifetime arguments must come before type arguments")
                || err.contains("lifetime parameters must come before"),
            "unexpected error for `{src}`:\n{err}"
        );
    }
}

#[test]
fn lifetime_types_merge_up_to_alpha_equivalence_and_reject_collisions() {
    use zngur_def::{Lifetime, PrimitiveRustType};

    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type<'a> crate::Parser<'a> {
    #layout(size = 24, align = 8);
    fn rest<'b>(&'b self) -> &'a str;
}
"#,
    )]);
    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
type<'b> crate::Parser<'b> {
    fn new(&'b str) -> crate::Parser<'b>;
}
"#,
        NullCfg,
        &resolver,
        |e| panic!("unexpected warning: {e}"),
    );
    assert_eq!(parsed.spec.types.len(), 1);
    let parser_ty = &parsed.spec.types[0];
    assert_eq!(parser_ty.lifetimes, vec![Lifetime("b".to_owned())]);
    assert_eq!(parser_ty.methods.len(), 2);
    let rest = &parser_ty.methods[1].data;
    assert_eq!(rest.name, "rest");
    // `rest`'s own `'b` from `./a.zng` was renamed to avoid capturing `test.zng`'s outer `'b`.
    assert_eq!(rest.lifetimes, vec![Lifetime("b_0".to_owned())]);
    assert_eq!(rest.receiver_lifetime, Some(Lifetime("b_0".to_owned())));
    assert_eq!(
        rest.output,
        RustType::Ref(
            Mutability::Not,
            Some(Lifetime("b".to_owned())),
            Box::new(RustType::Primitive(PrimitiveRustType::Str)),
        )
    );

    // Distinct lifetime instantiations mapping to the same C++ wrapper are rejected as conflicts.
    for src in [
        r#"
type ::std::slice::Iter<'static, i32> {
    #layout(size = 16, align = 8);
}
type<'a> ::std::slice::Iter<'a, i32> {
    #layout(size = 16, align = 8);
}
"#,
        r#"
type Box<dyn Fn(&str) -> &str> {
    #layout(size = 16, align = 8);
}
type Box<dyn for<'a> Fn(&'a str) -> &'a str> {
    #layout(size = 16, align = 8);
}
"#,
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains("map to the same C++ type"),
            "unexpected error:\n{err}"
        );
    }
}

#[test]
fn template_types_with_lifetimes_match_concrete_instantiations() {
    use zngur_def::{Lifetime, PrimitiveRustType};

    let parsed = parse_str(
        r#"
#unstable(template_types)

type<'a, T> ::std::slice::Iter<'a, T> {
    #layout(size = 16, align = 8);
    fn next<'b>(&'b mut self) -> ::std::option::Option<&'a T>;
}

type<'b> ::std::slice::Iter<'b, i32> {
}

type ::std::slice::Iter<'static, u8> {
}
"#,
        NullCfg,
        |e| panic!("unexpected warning: {e}"),
    );
    assert_eq!(parsed.spec.types.len(), 2);

    let iter_i32 = &parsed.spec.types[0];
    assert_eq!(iter_i32.lifetimes, vec![Lifetime("b".to_owned())]);
    let next_i32 = &iter_i32.methods[0].data;
    assert_eq!(next_i32.lifetimes, vec![Lifetime("b_0".to_owned())]);
    assert_eq!(next_i32.receiver_lifetime, Some(Lifetime("b_0".to_owned())));
    assert_eq!(
        next_i32.output,
        RustType::Adt(RustPathAndGenerics {
            path: vec!["std".to_owned(), "option".to_owned(), "Option".to_owned()],
            generics: vec![RustType::Ref(
                Mutability::Not,
                Some(Lifetime("b".to_owned())),
                Box::new(RustType::Primitive(PrimitiveRustType::Int(32))),
            )],
            ..Default::default()
        })
    );

    let iter_u8 = &parsed.spec.types[1];
    assert!(iter_u8.lifetimes.is_empty());
    let next_u8 = &iter_u8.methods[0].data;
    assert_eq!(next_u8.lifetimes, vec![Lifetime("b".to_owned())]);
    assert_eq!(next_u8.receiver_lifetime, Some(Lifetime("b".to_owned())));
    assert_eq!(
        next_u8.output,
        RustType::Adt(RustPathAndGenerics {
            path: vec!["std".to_owned(), "option".to_owned(), "Option".to_owned()],
            generics: vec![RustType::Ref(
                Mutability::Not,
                Some(Lifetime("static".to_owned())),
                Box::new(RustType::Primitive(PrimitiveRustType::Uint(8))),
            )],
            ..Default::default()
        })
    );

    // Substituting a `TypeVar` containing `for<'b>` into a template with method lifetime `'b`
    // renames the method lifetime so the inner `for<'b>` does not shadow it.
    let parsed_capture = parse_str(
        r#"
#unstable(template_types)

type<'a, T> crate::Holder<'a, T> {
    #layout(size = 24, align = 8);
    fn invoke<'b, 'b_0>(&'b self, &'b_0 str, T) -> &'a str;
}

type<'b> crate::Holder<'b, Box<dyn for<'c> Fn(&'c str)>> {
}

type<T> crate::Runner<T> {
    #layout(size = 16, align = 8);
    fn call<'b>(&'b self, T);
}

type crate::Runner<Box<dyn for<'b> Fn(&'b str)>> {
}
"#,
        NullCfg,
        |e| panic!("unexpected warning: {e}"),
    );
    let holder = &parsed_capture.spec.types[0];
    assert_eq!(
        holder.methods[0].data.lifetimes,
        vec![Lifetime("b_1".to_owned()), Lifetime("b_0".to_owned())]
    );
    let runner = &parsed_capture.spec.types[1];
    assert_eq!(
        runner.methods[0].data.lifetimes,
        vec![Lifetime("b_0".to_owned())]
    );
}

#[test]
fn unconstrained_extern_cpp_impl_lifetimes_are_rejected() {
    check_fail(
        r#"
extern "C++" {
    impl<'a> crate::Foo {
        safe fn bar(&self, &'a str);
    }

    impl<'a, 'b> crate::Parser<'a> {
        safe fn skip(&'a self, &'b str);
    }

    impl<'a> ::std::iter::Iterator<Item = &'a str> for crate::Foo {
        safe fn next(&mut self) -> ::std::option::Option<&'a str>;
    }
}
"#,
        expect![[r#"
            Error: unconstrained lifetime `'a` in `impl`: `'a` must be used in the implemented trait or self type (if `'a` is only used by methods, declare it on each method instead, e.g. `fn f<'a>(...)`)
               ╭─[ test.zng:3:14 ]
               │
             3 │     impl<'a> crate::Foo {
               │              ─────┬────  
               │                   ╰────── unconstrained lifetime `'a` in `impl`: `'a` must be used in the implemented trait or self type (if `'a` is only used by methods, declare it on each method instead, e.g. `fn f<'a>(...)`)
            ───╯
            Error: unconstrained lifetime `'b` in `impl`: `'b` must be used in the implemented trait or self type (if `'b` is only used by methods, declare it on each method instead, e.g. `fn f<'b>(...)`)
               ╭─[ test.zng:7:18 ]
               │
             7 │     impl<'a, 'b> crate::Parser<'a> {
               │                  ────────┬────────  
               │                          ╰────────── unconstrained lifetime `'b` in `impl`: `'b` must be used in the implemented trait or self type (if `'b` is only used by methods, declare it on each method instead, e.g. `fn f<'b>(...)`)
            ───╯
            Error: unconstrained lifetime `'a` in `impl`: `'a` must be used in the implemented trait or self type (if `'a` is only used by methods, declare it on each method instead, e.g. `fn f<'a>(...)`)
                ╭─[ test.zng:11:56 ]
                │
             11 │     impl<'a> ::std::iter::Iterator<Item = &'a str> for crate::Foo {
                │                                                        ─────┬────  
                │                                                             ╰────── unconstrained lifetime `'a` in `impl`: `'a` must be used in the implemented trait or self type (if `'a` is only used by methods, declare it on each method instead, e.g. `fn f<'a>(...)`)
            ────╯
        "#]],
    );
}

#[test]
fn lifetime_items_merge_alpha_equivalently_and_reject_conflicts() {
    use zngur_def::Lifetime;

    let resolver = MockFilesystem::new(vec![(
        "./other.zng",
        r#"
type<'a> crate::Parser<'a> {
    #layout(size = 40, align = 8);
    constructor(&'a str, Box<dyn for<'c> Fn(&'c str)>);
    field callback (offset = 8, type = Box<dyn for<'b> Fn(&'b str) -> &'b str>);
    fn pair<'b, 'b_0>(&'b self, &'b_0 str) -> &'a str;
}

trait crate::Picker {
    fn pick<'a>(&self, &'a str, &'a str) -> &'a str;
}

mod crate {
    fn longest<'a>(&'a str, &'a str) -> &'a str;
    fn generic<i32>(i32) -> i32;
}

extern "C++" {
    safe fn shortest<'a>(&'a str, &'a str) -> &'a str;
    impl<'a> crate::Parser<'a> {
        safe fn first(&'a self) -> &'a str;
        safe fn slice<'b>(&'b self) -> &'a str;
    }
}
"#,
    )]);

    let parsed = parse_str_with_resolver(
        r#"
merge "./other.zng";

type<'b> crate::Parser<'b> {
    constructor(&'b str, Box<dyn for<'d> Fn(&'d str)>);
    field callback (offset = 8, type = Box<dyn for<'x> Fn(&'x str) -> &'x str>);
}

trait crate::Picker {
    fn pick<'b>(&self, &'b str, &'b str) -> &'b str;
}

mod crate {
    fn longest<'b>(&'b str, &'b str) -> &'b str;
    fn generic<f64>(f64) -> f64;
}

extern "C++" {
    safe fn shortest<'b>(&'b str, &'b str) -> &'b str;
    impl<'b> crate::Parser<'b> {
        safe fn first(&'b self) -> &'b str;
    }
}
"#,
        NullCfg,
        &resolver,
        |e| panic!("unexpected warning: {e}"),
    );

    assert_eq!(parsed.spec.types.len(), 1);
    let parser_ty = &parsed.spec.types[0];
    assert_eq!(
        parser_ty.methods[0].data.lifetimes,
        vec![Lifetime("b_1".to_owned()), Lifetime("b_0".to_owned())]
    );
    assert_eq!(parsed.spec.traits.len(), 1);
    assert_eq!(parsed.spec.traits[0].methods.len(), 1);
    // `longest<'a>` and `longest<'b>` deduplicate, while `generic<i32>` and `generic<f64>` stay separate.
    assert_eq!(parsed.spec.funcs.len(), 3);
    assert_eq!(parsed.spec.extern_cpp_funcs.len(), 1);
    assert_eq!(parsed.spec.extern_cpp_impls.len(), 1);
    assert_eq!(parsed.spec.extern_cpp_impls[0].methods.len(), 2);
    assert_eq!(
        parsed.spec.extern_cpp_impls[0].methods[1].lifetimes,
        vec![Lifetime("b_0".to_owned())]
    );

    // Conflicting lifetime signatures across declarations are rejected instead of silently
    // keeping both or panicking.
    for (src, needle) in [
        (
            r#"
mod crate {
    fn longest<'a>(&'a str, &'a str) -> &'a str;
    fn longest<'a, 'b>(&'a str, &'b str) -> &'a str;
}
"#,
            "Function `crate::longest` mismatch",
        ),
        (
            r#"
extern "C++" {
    safe fn shortest<'a>(&'a str, &'a str) -> &'a str;
    safe fn shortest<'a, 'b>(&'a str, &'b str) -> &'a str;
}
"#,
            "Extern C++ function `shortest` mismatch",
        ),
        (
            r#"
trait crate::Picker {
    fn pick<'a>(&self, &'a str, &'a str) -> &'a str;
}
trait crate::Picker {
    fn pick<'a, 'b>(&self, &'a str, &'b str) -> &'a str;
}
"#,
            "Method mismatch",
        ),
        (
            r#"
trait crate::Handler<Box<dyn Fn(&str)>> {
    fn run(&self);
}
trait crate::Handler<Box<dyn for<'a> Fn(&'a str)>> {
    fn run(&self);
}
"#,
            "map to the same C++ trait",
        ),
        (
            r#"
extern "C++" {
    impl<'a> crate::Parser<'a> {
        safe fn first(&'a self) -> &'a str;
    }
    impl crate::Parser<'static> {
        safe fn first(&'static self) -> &'static str;
    }
}
"#,
            "Conflicting lifetime declarations in `impl`",
        ),
        (
            r#"
extern "C++" {
    impl<'a> crate::Parser<'a> {
        safe fn first(&'a self) -> &'a str;
    }
    impl<'b> crate::Parser<'b> {
        safe fn first(&'b self) -> &'static str;
    }
}
"#,
            "Method mismatch",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(needle),
            "expected `{needle}` in error for:\n{src}\ngot:\n{err}"
        );
    }
}

#[test]
fn cpp_pointers_and_references_parse() {
    let parsed = parse_str(
        r#"
type c++::a::B {
}

extern "C++" {
    unsafe fn f(const c++::a::B&, c++::a::B&, const i32*, i32*) -> const c++::a::B*;
    unsafe fn multi(c++::a::B**, const i32**, i32* const*);
}
"#,
        NullCfg,
        |_| {},
    );
    let f = &parsed.spec.extern_cpp_funcs[0];
    let b = || RustType::Cpp(vec!["a".to_owned(), "B".to_owned()].into());
    let i32 = || RustType::Primitive(PrimitiveRustType::Int(32));
    assert_eq!(
        f.inputs,
        vec![
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Reference, b()),
            RustType::cpp_ptr(Mutability::Mut, CppIndirection::Reference, b()),
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, i32()),
            RustType::cpp_ptr(Mutability::Mut, CppIndirection::Pointer, i32()),
        ]
    );
    assert_eq!(
        f.output,
        RustType::CppPtr(
            CppIndirection::Pointer,
            Box::new(RustType::CppConst(Box::new(b())))
        )
    );

    let multi = &parsed.spec.extern_cpp_funcs[1];
    let mut_ptr = |ty| RustType::cpp_ptr(Mutability::Mut, CppIndirection::Pointer, ty);
    let const_ptr = |ty| RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, ty);
    assert_eq!(
        multi.inputs,
        vec![
            mut_ptr(mut_ptr(b())),
            mut_ptr(const_ptr(i32())),
            const_ptr(mut_ptr(i32())),
        ]
    );
}

#[test]
fn const_type_by_value_is_rejected() {
    check_fail(
        r#"
extern "C++" {
    safe fn f(const i32);
}
"#,
        expect![[r#"
            Error: `const T` can only be the type of a C++ pointer or reference (`const T&`, `const T*`, `&'a const T`) or a C++ template argument
               ╭─[ test.zng:3:15 ]
               │
             3 │     safe fn f(const i32);
               │               ────┬────  
               │                   ╰────── `const T` can only be the type of a C++ pointer or reference (`const T&`, `const T*`, `&'a const T`) or a C++ template argument
            ───╯
        "#]],
    );
}

#[test]
fn const_rust_generic_argument_is_rejected() {
    check_fail(
        r#"
type ::std::vec::Vec<const i32> {
    #layout(size = 24, align = 8);
}
"#,
        expect![[r#"
            Error: `const T` can only be the type of a C++ pointer or reference (`const T&`, `const T*`, `&'a const T`) or a C++ template argument
               ╭─[ test.zng:2:6 ]
               │
             2 │ type ::std::vec::Vec<const i32> {
               │      ─────────────┬────────────  
               │                   ╰────────────── `const T` can only be the type of a C++ pointer or reference (`const T&`, `const T*`, `&'a const T`) or a C++ template argument
            ───╯
        "#]],
    );
}

#[test]
fn const_non_cpp_type_is_rejected() {
    check_fail(
        r#"
extern "C++" {
    safe fn f(const (i32, i32)&);
}
"#,
        expect![[r#"
            Error: only C++ types and builtins can be `const`
               ╭─[ test.zng:3:15 ]
               │
             3 │     safe fn f(const (i32, i32)&);
               │               ────────┬────────  
               │                       ╰────────── only C++ types and builtins can be `const`
            ───╯
        "#]],
    );
}

#[test]
fn references_to_cpp_objects_parse() {
    use zngur_def::CppConstness;

    let parsed = parse_str(
        r#"
type c++::B {
}

extern "C++" {
    safe fn f<'a>(&'a const c++::B, &'a var c++::B, &'a unique var c++::B, &'a unique const c++::B);
    safe fn g<'a>(&'a const i32, &'a var i32, &'a unique var i32, &'a i32, &'a mut i32) -> &'a var bool;
    unsafe fn h<'a>(&'a const c++::B*, c++::B*, const i32*);
}
"#,
        NullCfg,
        |_| {},
    );
    let funcs = &parsed.spec.extern_cpp_funcs;
    let b = RustType::Cpp(vec!["B".to_owned()].into());
    let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
    let reference = |m, constness, ty: &RustType| {
        RustType::Ref(
            m,
            Some(zngur_def::Lifetime("a".to_owned())),
            Box::new(RustType::cpp_object(constness, ty.clone())),
        )
    };
    assert_eq!(
        funcs[0].inputs,
        [
            reference(Mutability::Not, CppConstness::Const, &b),
            reference(Mutability::Not, CppConstness::Var, &b),
            reference(Mutability::Mut, CppConstness::Var, &b),
            reference(Mutability::Mut, CppConstness::Const, &b),
        ]
    );
    let rust_ref = |m| {
        RustType::Ref(
            m,
            Some(zngur_def::Lifetime("a".to_owned())),
            Box::new(i32.clone()),
        )
    };
    assert_eq!(
        funcs[1].inputs,
        [
            reference(Mutability::Not, CppConstness::Const, &i32),
            reference(Mutability::Not, CppConstness::Var, &i32),
            reference(Mutability::Mut, CppConstness::Var, &i32),
            rust_ref(Mutability::Not),
            rust_ref(Mutability::Mut),
        ]
    );
    let b_ptr = RustType::cpp_ptr(Mutability::Mut, CppIndirection::Pointer, b.clone());
    assert_eq!(
        funcs[2].inputs,
        [
            // `&'a const c++::B*` is a reference to a `const` pointer (`B* const&`).
            reference(Mutability::Not, CppConstness::Const, &b_ptr),
            b_ptr.clone(),
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, i32.clone()),
        ]
    );
}

#[test]
fn reference_to_cpp_class_without_constness_is_rejected() {
    check_fail(
        r#"
extern "C++" {
    safe fn f<'a>(&'a c++::B);
}
"#,
        expect![[r#"
            Error: a reference to a C++ class must say whether the object is `const`: `&'a const c++::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&'a var c++::B` (a shared `T&`, `&T` in Rust)
               ╭─[ test.zng:3:10 ]
               │
             3 │     safe fn f<'a>(&'a c++::B);
               │          ──────────┬─────────  
               │                    ╰─────────── a reference to a C++ class must say whether the object is `const`: `&'a const c++::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&'a var c++::B` (a shared `T&`, `&T` in Rust)
            ───╯
        "#]],
    );
}

#[test]
fn mutable_reference_to_cpp_class_is_rejected() {
    check_fail(
        r#"
type c++::B {}
extern "C++" {
    safe fn e<'a>(&'a mut c++::B);
}
mod c++::ns {
    safe fn f<'a>(&'a mut c++::B);
}
"#,
        expect![[r#"
            Error: a reference to a C++ class must say whether the object is `const`: `&'a unique var c++::B` (a `T&` that Rust uses uniquely, `&mut T` in Rust)
               ╭─[ test.zng:4:10 ]
               │
             4 │     safe fn e<'a>(&'a mut c++::B);
               │          ────────────┬───────────  
               │                      ╰───────────── a reference to a C++ class must say whether the object is `const`: `&'a unique var c++::B` (a `T&` that Rust uses uniquely, `&mut T` in Rust)
            ───╯
            Error: a reference to a C++ class must say whether the object is `const`: `&'a unique var c++::B` (a `T&` that Rust uses uniquely, `&mut T` in Rust)
               ╭─[ test.zng:7:5 ]
               │
             7 │     safe fn f<'a>(&'a mut c++::B);
               │     ──────────────┬──────────────  
               │                   ╰──────────────── a reference to a C++ class must say whether the object is `const`: `&'a unique var c++::B` (a `T&` that Rust uses uniquely, `&mut T` in Rust)
            ───╯
        "#]],
    );
}

#[test]
fn reference_to_cpp_class_through_an_alias_without_constness_is_rejected() {
    check_fail(
        r#"
use c++::a::B as B;

extern "C++" {
    safe fn f<'a>(&'a B);
}
"#,
        expect![[r#"
            Error: a reference to a C++ class must say whether the object is `const`: `&'a const c++::a::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&'a var c++::a::B` (a shared `T&`, `&T` in Rust)
               ╭─[ test.zng:5:10 ]
               │
             5 │     safe fn f<'a>(&'a B);
               │          ───────┬───────  
               │                 ╰───────── a reference to a C++ class must say whether the object is `const`: `&'a const c++::a::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&'a var c++::a::B` (a shared `T&`, `&T` in Rust)
            ───╯
        "#]],
    );
}

#[test]
fn reference_to_cpp_class_in_a_rust_item_without_constness_is_rejected() {
    check_fail(
        r#"
type crate::Foo {
    #layout(size = 0, align = 1);
    fn f(&self, &c++::B);
}
"#,
        expect![[r#"
            Error: a reference to a C++ class must say whether the object is `const`: `&const c++::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&var c++::B` (a shared `T&`, `&T` in Rust)
               ╭─[ test.zng:4:5 ]
               │
             4 │     fn f(&self, &c++::B);
               │     ──────────┬──────────  
               │               ╰──────────── a reference to a C++ class must say whether the object is `const`: `&const c++::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&var c++::B` (a shared `T&`, `&T` in Rust)
            ───╯
        "#]],
    );
}

#[test]
fn const_reference_to_rust_type_is_rejected() {
    check_fail(
        r#"
extern "C++" {
    safe fn f<'a>(&'a const crate::Foo);
}
"#,
        expect![[r#"
            Error: only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`, and `crate::Foo` is a Rust type: use `&'a crate::Foo`
               ╭─[ test.zng:3:10 ]
               │
             3 │     safe fn f<'a>(&'a const crate::Foo);
               │          ───────────────┬──────────────  
               │                         ╰──────────────── only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`, and `crate::Foo` is a Rust type: use `&'a crate::Foo`
            ───╯
        "#]],
    );
}

#[test]
fn var_reference_to_str_is_rejected() {
    check_fail(
        r#"
extern "C++" {
    safe fn f<'a>(&'a unique var str);
}
"#,
        expect![[r#"
            Error: only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`, and `str` is a Rust type: use `&'a mut str`
               ╭─[ test.zng:3:10 ]
               │
             3 │     safe fn f<'a>(&'a unique var str);
               │          ──────────────┬─────────────  
               │                        ╰─────────────── only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`, and `str` is a Rust type: use `&'a mut str`
            ───╯
        "#]],
    );
}

#[test]
fn const_reference_to_char_is_rejected() {
    check_fail(
        r#"
extern "C++" {
    safe fn f<'a>() -> &'a const char;
}
"#,
        expect![[r#"
            Error: `char` can't be a C++ object (C++ can store invalid code points in it), so references to it can't be `const` or `var`: use `&'a char` (a Rust value)
               ╭─[ test.zng:3:10 ]
               │
             3 │     safe fn f<'a>() -> &'a const char;
               │          ──────────────┬─────────────  
               │                        ╰─────────────── `char` can't be a C++ object (C++ can store invalid code points in it), so references to it can't be `const` or `var`: use `&'a char` (a Rust value)
            ───╯
        "#]],
    );
}

#[test]
fn unique_reference_without_constness_is_rejected() {
    check_fail(
        r#"
extern "C++" {
    safe fn f<'a>(&'a unique c++::B);
}
"#,
        expect![[r#"
            Error: a `unique` reference is a reference to a C++ object, which says whether it's `const`: `&'a unique var T` (a C++ `T&` that Rust uses uniquely, `&mut T` in Rust) or `&'a unique const T`; a reference to a Rust value is `&'a mut T`
               ╭─[ test.zng:3:19 ]
               │
             3 │     safe fn f<'a>(&'a unique c++::B);
               │                   ────────┬────────  
               │                           ╰────────── a `unique` reference is a reference to a C++ object, which says whether it's `const`: `&'a unique var T` (a C++ `T&` that Rust uses uniquely, `&mut T` in Rust) or `&'a unique const T`; a reference to a Rust value is `&'a mut T`
            ───╯
        "#]],
    );
}

#[test]
fn mut_const_reference_is_rejected() {
    check_fail(
        r#"
extern "C++" {
    safe fn f<'a>(&'a mut const c++::B);
}
"#,
        expect![[r#"
            Error: references to C++ objects say `unique` rather than `mut`: write `&'a unique const T` (or `&'a const T` for a shared reference)
               ╭─[ test.zng:3:19 ]
               │
             3 │     safe fn f<'a>(&'a mut const c++::B);
               │                   ──────────┬─────────  
               │                             ╰─────────── references to C++ objects say `unique` rather than `mut`: write `&'a unique const T` (or `&'a const T` for a shared reference)
            ───╯
        "#]],
    );
}

#[test]
fn const_receiver_on_rust_method_is_rejected() {
    check_fail(
        r#"
type crate::Foo {
    #layout(size = 0, align = 1);
    fn f(&const self);
}
"#,
        expect![[r#"
            Error: methods of Rust types and `extern "C++"` impls take `&self` or `&mut self`, not `&const self` / `&var self`
               ╭─[ test.zng:4:5 ]
               │
             4 │     fn f(&const self);
               │     ────────┬────────  
               │             ╰────────── methods of Rust types and `extern "C++"` impls take `&self` or `&mut self`, not `&const self` / `&var self`
            ───╯
        "#]],
    );
}

#[test]
fn unique_and_var_are_only_keywords_after_a_reference() {
    check_success(
        r#"
type crate::var {
    #layout(size = 0, align = 1);
    fn unique(&self, &crate::var) -> crate::var;
}
"#,
    );
}

#[test]
fn invalid_cpp_pointer_and_reference_combinations_are_rejected() {
    for (src, expected) in [
        (
            "extern \"C++\" { unsafe fn f<'a>(&'a const i32&); }\n",
            "references to C++ references aren't valid",
        ),
        (
            "extern \"C++\" { unsafe fn f<'a>(&'a i32&); }\n",
            "references to C++ references aren't valid",
        ),
        (
            "extern \"C++\" { unsafe fn f(i32&*); }\n",
            "pointers to C++ references (`T&*`) aren't valid",
        ),
        (
            "extern \"C++\" { unsafe fn f(i32&&); }\n",
            "references to C++ references (`T&&`) aren't valid",
        ),
        (
            "extern \"C++\" { unsafe fn f(i32& const); }\n",
            "a C++ reference cannot be `const`-qualified (`T& const`)",
        ),
        (
            "extern \"C++\" { unsafe fn f(crate::Foo*); }\n",
            "C++ pointers and references (`T*`, `T&`) can only point to C++ objects",
        ),
        (
            "extern \"C++\" { unsafe fn f(char*); }\n",
            "`char` can't be a C++ object",
        ),
        (
            "type i32& { #layout(size = 8, align = 8); }\n",
            "C++ reference `i32&` can only be a function parameter, return type, or C++ template argument",
        ),
        (
            "type Box<dyn Fn(const i32)> { #layout(size = 16, align = 8); }\n",
            "`const T` can only be the type of a C++ pointer or reference",
        ),
        (
            "type c++::B {}\ntype Box<dyn for<'a> Fn(&'a unique c++::B)> { #layout(size = 16, align = 8); }\n",
            "a `unique` reference is a reference to a C++ object, which says whether it's `const`",
        ),
        (
            "type c++::B {}\ntype Box<dyn for<'a> Fn(&'a mut const c++::B)> { #layout(size = 16, align = 8); }\n",
            "references to C++ objects say `unique` rather than `mut`",
        ),
        (
            "mod crate { fn f() -> impl ::std::future::Future<Output = const i32>; }\n",
            "`const T` can only be the type of a C++ pointer or reference",
        ),
        (
            "extern \"C++\" { safe fn f(&const i128); }\n",
            "`i128` is not a C++ builtin scalar",
        ),
        (
            "extern \"C++\" { safe fn f(&var f16); }\n",
            "`f16` is not a C++ builtin scalar",
        ),
        (
            "extern \"C++\" { unsafe fn f(const u128*); }\n",
            "only `bool`, 8/16/32/64-bit integers, `usize`, `f32`, and `f64` are C++ builtin scalars",
        ),
        (
            "extern \"C++\" { unsafe fn f(f128*); }\n",
            "`f128` is not a C++ builtin scalar",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn template_types_check_cpp_objects_after_substitution() {
    for (src, expected) in [
        (
            r#"
#unstable(template_types)
type c++::B {}
type<T> crate::Holder<T> {
    #layout(size = 8, align = 8);
    fn f(&self, &T);
}
type crate::Holder<c++::B> {}
"#,
            "a reference to a C++ class must say whether the object is `const`",
        ),
        (
            r#"
#unstable(template_types)
type crate::Foo { #layout(size = 4, align = 4); }
type<T> crate::Holder<T> {
    #layout(size = 8, align = 8);
    fn f(&self, &const T);
}
type crate::Holder<crate::Foo> {}
"#,
            "only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`",
        ),
        (
            r#"
#unstable(template_types)
type crate::Foo { #layout(size = 4, align = 4); }
type<T> crate::Holder<T> {
    #layout(size = 8, align = 8);
    fn f(&self, T*);
}
type crate::Holder<crate::Foo> {}
"#,
            "C++ pointers and references (`T*`, `T&`) can only point to C++ objects",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn rust_raw_pointer_to_cpp_class_is_rejected() {
    check_fail(
        r#"
type c++::B {}
extern "C++" {
    unsafe fn f(*const c++::B, *mut c++::B);
}
"#,
        expect![[r#"
            Error: a raw pointer to a C++ class must say whether the object is `const`: use `const c++::B*`
               ╭─[ test.zng:4:12 ]
               │
             4 │     unsafe fn f(*const c++::B, *mut c++::B);
               │            ────────────────┬───────────────  
               │                            ╰───────────────── a raw pointer to a C++ class must say whether the object is `const`: use `const c++::B*`
            ───╯
        "#]],
    );
    check_fail(
        r#"
type c++::B {}
extern "C++" {
    unsafe fn g(*mut c++::B);
}
"#,
        expect![[r#"
            Error: a raw pointer to a C++ class must say whether the object is `const`: use `c++::B*`
               ╭─[ test.zng:4:12 ]
               │
             4 │     unsafe fn g(*mut c++::B);
               │            ────────┬────────  
               │                    ╰────────── a raw pointer to a C++ class must say whether the object is `const`: use `c++::B*`
            ───╯
        "#]],
    );
}

#[test]
fn fn_trait_rejects_bare_cpp_references_and_accepts_pointers_or_lifetime_refs() {
    check_success(
        r#"
type c++::B {}
type Box<dyn Fn(const c++::B*) -> i32> {
    #layout(size = 16, align = 8);
}
type Box<dyn for<'a> Fn(&'a const c++::B) -> &'a const c++::B> {
    #layout(size = 16, align = 8);
}
"#,
    );
    for src in [
        "type c++::B {}\ntype Box<dyn Fn(const c++::B&) -> i32> { #layout(size = 16, align = 8); }\n",
        "type c++::B {}\ntype Box<dyn Fn() -> const c++::B&> { #layout(size = 16, align = 8); }\n",
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(
                "C++ reference `const c++::B&` can only be a function parameter, return type, or C++ template argument"
            ),
            "unexpected error for `{src}`:\n{err}"
        );
    }
}

#[test]
fn nested_or_field_cpp_object_references_are_rejected() {
    for (src, expected) in [
        (
            r#"
type c++::B {}
type<'a> crate::S<'a> {
    #layout(size = 8, align = 8);
    field x (offset = 0, type = &'a const c++::B);
}
"#,
            "reference to a C++ object `&'a const c++::B` is a C++ reference in C++ and can only be a function, method, or constructor parameter or return type",
        ),
        (
            r#"
type<'a> crate::S<'a> {
    #layout(size = 8, align = 8);
    field x (offset = 0, type = &'a unique var i32);
}
"#,
            "reference to a C++ object `&'a unique var i32` is a C++ reference in C++ and can only be a function, method, or constructor parameter or return type",
        ),
        (
            r#"
type c++::B {}
type<'a> crate::E<'a> {
    #layout(size = 16, align = 8);
    variant V {
        field x (offset = auto, type = &'a const c++::B);
    }
}
"#,
            "reference to a C++ object `&'a const c++::B` is a C++ reference in C++",
        ),
        (
            "type c++::B {}\nextern \"C++\" { safe fn f<'a>((&'a const c++::B, i32)); }\n",
            "reference to a C++ object `&'a const c++::B` is a C++ reference in C++",
        ),
        (
            "type c++::B {}\nextern \"C++\" { safe fn f<'a>(&'a &'a const c++::B); }\n",
            "reference to a C++ object `&'a const c++::B` is a C++ reference in C++",
        ),
        (
            "type c++::B {}\nextern \"C++\" { safe fn f<'a>(&'a mut &'a const c++::B); }\n",
            "reference to a C++ object `&'a const c++::B` is a C++ reference in C++",
        ),
        (
            "type c++::B {}\nextern \"C++\" { unsafe fn f<'a>(*const &'a const c++::B); }\n",
            "reference to a C++ object `&'a const c++::B` is a C++ reference in C++",
        ),
        (
            "type c++::B {}\nextern \"C++\" { unsafe fn f<'a>(*mut &'a const c++::B); }\n",
            "reference to a C++ object `&'a const c++::B` is a C++ reference in C++",
        ),
        (
            "type c++::B {}\nextern \"C++\" { safe fn f<'a>(Box<&'a const c++::B>); }\n",
            "reference to a C++ object `&'a const c++::B` is a C++ reference in C++",
        ),
        (
            "type c++::B {}\nextern \"C++\" { safe fn f<'a>(&'a [&'a const c++::B]); }\n",
            "reference to a C++ object `&'a const c++::B` is a C++ reference in C++",
        ),
        (
            "type c++::B {}\nextern \"C++\" { safe fn f<'a>(::std::vec::Vec<&'a const c++::B>); }\n",
            "reference to a C++ object `&'a const c++::B` is a C++ reference in C++",
        ),
        (
            "type c++::B {}\nmod crate { fn f() -> impl ::std::future::Future<Output = &'static const c++::B>; }\n",
            "reference to a C++ object `&'static const c++::B` is a C++ reference in C++",
        ),
        (
            r#"
type c++::B {}
trait crate::MyTrait {
    fn m(&self);
}
extern "C++" {
    impl<'a> crate::MyTrait for &'a const c++::B {
        safe fn m(&self);
    }
}
"#,
            "reference to a C++ object `&'a const c++::B` is a C++ reference in C++",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn cpp_pointers_and_object_refs_cannot_be_declared_as_types_or_inherent_impls() {
    for (src, expected) in [
        (
            "type c++::B {}\ntype c++::B* { #layout(size = 8, align = 8); }\n",
            "C++ pointers and references (`c++::B*`) are built-in, not types to declare",
        ),
        (
            "type const i32* { #layout(size = 8, align = 8); }\n",
            "C++ pointers and references (`const i32*`) are built-in, not types to declare",
        ),
        (
            "type c++::B {}\ntype<'a> &'a const c++::B { #layout(size = 8, align = 8); }\n",
            "C++ pointers and references (`&'a const c++::B`) are built-in, not types to declare",
        ),
        (
            "type<'a> &'a unique var i32 { #layout(size = 8, align = 8); }\n",
            "C++ pointers and references (`&'a unique var i32`) are built-in, not types to declare",
        ),
        (
            "type c++::B {}\nextern \"C++\" { impl c++::B* { safe fn f(&self); } }\n",
            "inherent `extern \"C++\"` impls are not supported on C++ pointers or references (`c++::B*`)",
        ),
        (
            "type c++::B {}\nextern \"C++\" { impl<'a> &'a const c++::B { safe fn f(&self); } }\n",
            "inherent `extern \"C++\"` impls are not supported on C++ pointers or references (`&'a const c++::B`)",
        ),
        (
            "type c++::B {}\nextern \"C++\" { unsafe fn f((c++::B, i32)*); }\n",
            "C++ pointers and references (`T*`, `T&`) can only point to C++ objects (`c++::` types, builtins and C++ pointers), not `(c++::B, i32)`",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn nullable_references_to_cpp_objects_parse() {
    use zngur_def::{Lifetime, PrimitiveRustType};

    let parsed = parse_str(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
extern "C++" {
    safe fn f<'a>(
        ::std::option::Option<&'a const c++::B>,
        ::core::option::Option<&'a var c++::B>,
        ::std::option::Option<&'a unique var c++::B>,
        ::core::option::Option<&'a unique const i32>,
    ) -> ::std::option::Option<&'a const c++::B>;
}
"#,
        NullCfg,
        |msg| panic!("{msg}"),
    );
    let func = &parsed.spec.extern_cpp_funcs[0];
    let lt_a = Lifetime("a".to_owned());
    let b = RustType::Cpp(vec!["B".to_owned()].into());
    let i32_ty = RustType::Primitive(PrimitiveRustType::Int(32));
    assert_eq!(
        func.inputs[0].as_nullable_cpp_ref(),
        Some((
            Mutability::Not,
            Some(&lt_a),
            &RustType::CppConst(Box::new(b.clone()))
        ))
    );
    assert_eq!(
        func.inputs[1].as_nullable_cpp_ref(),
        Some((
            Mutability::Not,
            Some(&lt_a),
            &RustType::CppVar(Box::new(b.clone()))
        ))
    );
    assert_eq!(
        func.inputs[2].as_nullable_cpp_ref(),
        Some((
            Mutability::Mut,
            Some(&lt_a),
            &RustType::CppVar(Box::new(b.clone()))
        ))
    );
    assert_eq!(
        func.inputs[3].as_nullable_cpp_ref(),
        Some((
            Mutability::Mut,
            Some(&lt_a),
            &RustType::CppConst(Box::new(i32_ty))
        ))
    );
    assert_eq!(
        func.output.as_nullable_cpp_ref(),
        Some((
            Mutability::Not,
            Some(&lt_a),
            &RustType::CppConst(Box::new(b))
        ))
    );
}

#[test]
fn nullable_references_to_cpp_objects_cant_be_declared_as_types() {
    check_fail(
        r#"
type ::std::option::Option<&var c++::B> {}
"#,
        expect![[r#"
            Error: a nullable reference to a C++ object (`Option<&'a const T>`, `Option<&'a var T>`, ...) is a C++ pointer in C++ (here to `c++::B`), not a type to declare: use it in signatures directly
               ╭─[ test.zng:2:6 ]
               │
             2 │ type ::std::option::Option<&var c++::B> {}
               │      ─────────────────┬────────────────  
               │                       ╰────────────────── a nullable reference to a C++ object (`Option<&'a const T>`, `Option<&'a var T>`, ...) is a C++ pointer in C++ (here to `c++::B`), not a type to declare: use it in signatures directly
            ───╯
        "#]],
    );
    check_fail(
        r#"
type ::std::option::Option<&var crate::Foo> {}
"#,
        expect![[r#"
            Error: only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`, and `crate::Foo` is a Rust type: use `&crate::Foo`
               ╭─[ test.zng:2:6 ]
               │
             2 │ type ::std::option::Option<&var crate::Foo> {}
               │      ───────────────────┬──────────────────  
               │                         ╰──────────────────── only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`, and `crate::Foo` is a Rust type: use `&crate::Foo`
            ───╯
        "#]],
    );
}

#[test]
fn nullable_cpp_refs_are_allowed_in_fields_and_containers_but_rejected_in_inherent_impls() {
    check_success(
        r#"
type c++::B {}
type<'a> crate::Holder<'a> {
    #layout(size = 8, align = 8);
    field ptr (offset = 0, type = ::std::option::Option<&'a const c++::B>);
}
extern "C++" {
    safe fn tuple_and_vec<'a>(
        (::std::option::Option<&'a const c++::B>, ::core::option::Option<&'a unique var i32>),
        ::std::vec::Vec<::std::option::Option<&'a const c++::B>>,
    );
}
mod crate {
    fn async_ptr() -> impl ::std::future::Future<Output = ::std::option::Option<&'static const c++::B>>;
}
"#,
    );

    for (src, expected) in [
        (
            "type c++::B {}\nextern \"C++\" { impl<'a> ::std::option::Option<&'a const c++::B> { safe fn f(&self); } }\n",
            "inherent `extern \"C++\"` impls are not supported on C++ pointers or references (`::std::option::Option<&'a const c++::B>`)",
        ),
        (
            r#"
#unstable(template_types)
type c++::B {}
type<T> crate::Wrap<::std::option::Option<T>> {
    #layout(size = 8, align = 8);
    field x (offset = 0, type = T);
}
type<'a> crate::Wrap<::std::option::Option<&'a const c++::B>> {}
"#,
            "use `::std::option::Option<&'a const c++::B>` for a nullable C++ pointer",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn cpp_methods_parse() {
    use zngur_def::{CppIndirection, Mutability, PrimitiveRustType, ZngurMethodReceiver};

    let parsed = parse_str(
        r#"
mod c++::a {
    type B {
        #layout(size = 8, align = 4);
        #trivially_relocatable;

        unsafe fn value(&const self) -> i32;
        unsafe fn set(&unique var self, i32, const B&);
        safe fn make(i32) -> B;
        unsafe fn get(&const self, i32) -> const i32& as get_const;
    }
}
"#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let b = RustType::Cpp(vec!["a".to_owned(), "B".to_owned()].into());
    let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
    let methods = &ty.cpp_methods;
    assert_eq!(methods.len(), 4);

    assert_eq!(methods[0].cpp_name, "value");
    assert_eq!(methods[0].rust_name(), "value");
    assert_eq!(
        methods[0].receiver,
        ZngurMethodReceiver::Ref(Mutability::Not)
    );
    assert!(methods[0].inputs.is_empty());
    assert_eq!(methods[0].output, i32);
    assert!(!methods[0].is_safe);

    assert_eq!(
        methods[1].receiver,
        ZngurMethodReceiver::Ref(Mutability::Mut)
    );
    assert_eq!(
        methods[1].inputs,
        vec![
            i32.clone(),
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Reference, b.clone()),
        ]
    );
    assert_eq!(methods[1].output, RustType::UNIT);

    assert_eq!(methods[2].receiver, ZngurMethodReceiver::Static);
    assert_eq!(methods[2].output, b);
    assert!(methods[2].is_safe);

    assert_eq!(methods[3].cpp_name, "get");
    assert_eq!(methods[3].rust_name(), "get_const");
    assert_eq!(
        methods[3].output,
        RustType::cpp_ptr(Mutability::Not, CppIndirection::Reference, i32)
    );
}

#[test]
fn safe_cpp_methods_taking_references_are_accepted() {
    let parsed = parse_str(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn value<'a>(&'a const self) -> i32;
    safe fn set<'a>(&'a unique var self, &'a const c++::B);
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert!(ty.cpp_methods.iter().all(|m| m.is_safe));
}

#[test]
fn safe_cpp_method_with_cpp_ptr_warns() {
    check_warnings(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn set(&unique var self, const c++::B*);
}
"#,
        expect![[r#"
            Warning: this C++ member function has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `fn f<'a>(&'a const self) -> &'a const T`), or silence this with `#allow(elided_lifetimes)`
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe fn set(&unique var self, const c++::B*);
               │     ──────────────────────┬──────────────────────  
               │                           ╰──────────────────────── this C++ member function has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `fn f<'a>(&'a const self) -> &'a const T`), or silence this with `#allow(elided_lifetimes)`
            ───╯
            Warning: this C++ member function is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe fn set(&unique var self, const c++::B*);
               │     ──────────────────────┬──────────────────────  
               │                           ╰──────────────────────── this C++ member function is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
            ───╯
        "#]],
    );
}

#[test]
fn safe_static_cpp_method_with_cpp_ptr_warns() {
    check_warnings(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn sum(const c++::B*) -> i32;
}
"#,
        expect![[r#"
            Warning: this C++ member function is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe fn sum(const c++::B*) -> i32;
               │     ─────────────────┬────────────────  
               │                      ╰────────────────── this C++ member function is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
            ───╯
        "#]],
    );
}

#[test]
fn nullable_cpp_ref_in_safe_cpp_method_does_not_warn() {
    check_success(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn first<'a>(&'a const self, ::std::option::Option<&'a const c++::B>) -> i32;
}
"#,
    );
}

#[test]
fn static_cpp_method_with_trailing_const_is_rejected() {
    check_fail(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn make() const -> c++::B;
}
"#,
        expect![[r#"
            Error: C++ member functions take an explicit receiver instead of a trailing `const`: `&'a const self` for `const` member functions, and `&'a unique var self` (or `&'a var self`) for the others
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe fn make() const -> c++::B;
               │     ───────────────┬───────────────  
               │                    ╰───────────────── C++ member functions take an explicit receiver instead of a trailing `const`: `&'a const self` for `const` member functions, and `&'a unique var self` (or `&'a var self`) for the others
            ───╯
        "#]],
    );
}

#[test]
fn cpp_method_on_rust_type_is_rejected() {
    check_fail(
        r#"
type crate::Foo {
    #layout(size = 8, align = 4);
    unsafe fn value<'a>(&'a const self) -> i32;
}
"#,
        expect![[r#"
            Error: methods of Rust types and `extern "C++"` impls take `&self` or `&mut self`, not `&const self` / `&var self`
               ╭─[ test.zng:4:5 ]
               │
             4 │     unsafe fn value<'a>(&'a const self) -> i32;
               │     ─────────────────────┬────────────────────  
               │                          ╰────────────────────── methods of Rust types and `extern "C++"` impls take `&self` or `&mut self`, not `&const self` / `&var self`
            ───╯
        "#]],
    );
}

#[test]
fn safe_and_unsafe_rust_methods_on_rust_type_parse_as_rust_methods() {
    let parsed = parse_str(
        r#"
type crate::Foo {
    #layout(size = 8, align = 4);
    fn plain(&self) -> i32;
    safe fn checked(&self) -> i32;
    unsafe fn raw_op(&mut self, *const i32) -> i32;
    safe fn make(i32) -> crate::Foo;
    unsafe fn make_unchecked(i32) -> crate::Foo;
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert!(ty.cpp_methods.is_empty());
    assert_eq!(ty.methods.len(), 5);
    assert_eq!(ty.methods[0].data.is_safe, None);
    assert_eq!(ty.methods[1].data.is_safe, Some(true));
    assert_eq!(ty.methods[2].data.is_safe, Some(false));
    assert_eq!(ty.methods[3].data.is_safe, Some(true));
    assert_eq!(ty.methods[4].data.is_safe, Some(false));
}

#[test]
fn bare_fn_and_rust_method_features_on_cpp_type_are_rejected() {
    check_fail(
        r#"
type c++::B {
    fn value<'a>(&'a const self) -> i32;
    fn make() -> i32;
    safe async fn fut<'a>(&'a const self);
    safe fn tr<'a>(&'a const self) use crate::MyTrait;
    safe fn dr<'a>(&'a const self) deref i32;
}
"#,
        expect![[r#"
            Error: C++ member functions must be marked `safe` or `unsafe`
               ╭─[ test.zng:3:5 ]
               │
             3 │     fn value<'a>(&'a const self) -> i32;
               │     ──────────────────┬─────────────────  
               │                       ╰─────────────────── C++ member functions must be marked `safe` or `unsafe`
            ───╯
            Error: C++ member functions must be marked `safe` or `unsafe`
               ╭─[ test.zng:4:5 ]
               │
             4 │     fn make() -> i32;
               │     ────────┬────────  
               │             ╰────────── C++ member functions must be marked `safe` or `unsafe`
            ───╯
            Error: C++ member functions cannot be `async`
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe async fn fut<'a>(&'a const self);
               │     ───────────────────┬──────────────────  
               │                        ╰──────────────────── C++ member functions cannot be `async`
            ───╯
            Error: C++ member functions do not support `use` or `deref` (those are for Rust methods; implement Rust traits on `c++::` types via `extern "C++" { impl Trait for c++::T { ... } }`)
               ╭─[ test.zng:6:5 ]
               │
             6 │     safe fn tr<'a>(&'a const self) use crate::MyTrait;
               │     ─────────────────────────┬────────────────────────  
               │                              ╰────────────────────────── C++ member functions do not support `use` or `deref` (those are for Rust methods; implement Rust traits on `c++::` types via `extern "C++" { impl Trait for c++::T { ... } }`)
            ───╯
            Error: C++ member functions do not support `use` or `deref` (those are for Rust methods; implement Rust traits on `c++::` types via `extern "C++" { impl Trait for c++::T { ... } }`)
               ╭─[ test.zng:7:5 ]
               │
             7 │     safe fn dr<'a>(&'a const self) deref i32;
               │     ────────────────────┬────────────────────  
               │                         ╰────────────────────── C++ member functions do not support `use` or `deref` (those are for Rust methods; implement Rust traits on `c++::` types via `extern "C++" { impl Trait for c++::T { ... } }`)
            ───╯
        "#]],
    );
}

#[test]
fn invalid_self_in_cpp_method_is_rejected() {
    for src in [
        "type c++::B { safe fn consume(self); }\n",
        "type c++::B { unsafe fn f(*const self); }\n",
        "type c++::B { unsafe fn f(*mut self); }\n",
        "type c++::B { safe fn f<'a>(::std::option::Option<&'a const self>); }\n",
        "type c++::B { safe fn f<'a>(i32, &'a const self); }\n",
        "type c++::B { safe fn f<'a>(&'a const self) -> &'a const self; }\n",
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains("invalid C++ member function receiver"),
            "expected invalid receiver error for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn duplicate_cpp_method_name_is_rejected() {
    check_fail(
        r#"
type c++::B {
    safe fn get<'a>(&'a const self) -> i32;
    safe fn get<'a>(&'a unique var self) -> i32;
}
"#,
        expect![[r#"
            Error: `get` is already the Rust name of another member of this type (a constructor, member function or field accessor)
               ╭─[ test.zng:4:5 ]
               │
             4 │     safe fn get<'a>(&'a unique var self) -> i32;
               │     ──────────────────────┬─────────────────────  
               │                           ╰─────────────────────── `get` is already the Rust name of another member of this type (a constructor, member function or field accessor)
            ───╯
        "#]],
    );
    check_fail(
        r#"
type c++::B {
    safe fn get<'a>(&'a const self) -> i32;
    safe fn other<'a>(&'a const self) -> i32 as get;
}
"#,
        expect![[r#"
            Error: `get` is already the Rust name of another member of this type (a constructor, member function or field accessor)
               ╭─[ test.zng:4:5 ]
               │
             4 │     safe fn other<'a>(&'a const self) -> i32 as get;
               │     ────────────────────────┬───────────────────────  
               │                             ╰───────────────────────── `get` is already the Rust name of another member of this type (a constructor, member function or field accessor)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_method_keyword_and_reserved_names() {
    let parsed = parse_str(
        r#"
type c++::B {
    safe fn type<'a>(&'a const self) -> i32 as kind;
    safe fn move<'a>(&'a unique var self) as take;
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let methods = &parsed.spec.types[0].cpp_methods;
    assert_eq!(methods[0].cpp_name, "type");
    assert_eq!(methods[0].rust_name(), "kind");
    assert_eq!(methods[1].cpp_name, "move");
    assert_eq!(methods[1].rust_name(), "take");

    for (src, expected) in [
        (
            "type c++::B { safe fn type<'a>(&'a const self) -> i32; }\n",
            "`type` is a Rust keyword; give this C++ function a non-keyword Rust name with `as <rust_name>`",
        ),
        (
            "type c++::B { safe fn move<'a>(&'a const self) -> i32; }\n",
            "`move` is a Rust keyword; give this C++ function a non-keyword Rust name with `as <rust_name>`",
        ),
        (
            "type c++::B { safe fn get<'a>(&'a const self) -> i32 as type; }\n",
            "`type` is a Rust keyword and cannot be used as the Rust function name in `as type`",
        ),
        (
            "type c++::B { safe fn get<'a>(&'a const self) -> i32 as move; }\n",
            "`move` is a Rust keyword and cannot be used as the Rust function name in `as move`",
        ),
        (
            "type c++::B { safe fn as_const<'a>(&'a const self) -> i32; }\n",
            "`as_const` collides with an inherent conversion method on `c++::` wrappers; rename it with `as <rust_name>`",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn conflicting_cpp_methods_across_files() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    unsafe fn value(&const self) -> i32;
}
"#,
    )]);

    check_import_fail(
        r#"
merge "./a.zng";
type c++::B {
    unsafe fn value(&const self) -> i64;
}
"#,
        expect![[r#"
            Error: C++ method mismatch
               ╭─[ ./a.zng:2:6 ]
               │
             2 │ type c++::B {
               │      ───┬──  
               │         ╰──── C++ method mismatch
               │ 
             5 │     unsafe fn value(&const self) -> i32;
               │     ──────────────────┬─────────────────  
               │                       ╰─────────────────── declaration here ...
               │
               ├─[ test.zng:4:5 ]
               │
             4 │     unsafe fn value(&const self) -> i64;
               │     ──────────────────┬─────────────────  
               │                       ╰─────────────────── conflicts with the declaration here.
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn cpp_methods_can_be_declared_in_another_file() {
    // E.g. a hand-written file amending a generated one.
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    unsafe fn value(&const self) -> i32;
}
"#,
    )]);
    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
type c++::B {
    unsafe fn set(&unique var self, i32);
}
"#,
        NullCfg,
        &resolver,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let mut names: Vec<_> = ty.cpp_methods.iter().map(|m| m.rust_name()).collect();
    names.sort();
    assert_eq!(names, ["set", "value"]);
}

#[test]
fn lifetimes_in_cpp_member_functions_are_rejected() {
    check_fail(
        r#"
type c++::B {
    unsafe fn name(&const self, &'a const i32) -> i32;
}
"#,
        expect![[r#"
            Error: undeclared lifetime `'a`: declare it on the function, `impl` or `type` (e.g. `fn f<'a>(...)`)
               ╭─[ test.zng:3:5 ]
               │
             3 │     unsafe fn name(&const self, &'a const i32) -> i32;
               │     ─────────────────────────┬────────────────────────  
               │                              ╰────────────────────────── undeclared lifetime `'a`: declare it on the function, `impl` or `type` (e.g. `fn f<'a>(...)`)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_member_function_without_receiver_is_static() {
    use zngur_def::ZngurMethodReceiver;

    let parsed = parse_str(
        r#"
type c++::B {
    safe fn value() -> i32;
    safe fn get<'a>(&'a const self) -> i32;
}
"#,
        NullCfg,
        |_| {},
    );
    let methods = &parsed.spec.types[0].cpp_methods;
    assert_eq!(methods[0].receiver, ZngurMethodReceiver::Static);
    assert_eq!(
        methods[1].receiver,
        ZngurMethodReceiver::Ref(Mutability::Not)
    );
}

#[test]
fn static_keyword_on_cpp_member_function_is_rejected() {
    check_fail(
        r#"
type c++::B {
    safe static fn make() -> i32;
}
"#,
        expect![[r#"
            Error: C++ static member functions are declared without a `self` receiver; remove `static`
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe static fn make() -> i32;
               │     ──────────────┬──────────────  
               │                   ╰──────────────── C++ static member functions are declared without a `self` receiver; remove `static`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_member_function_with_trailing_const_is_rejected() {
    check_fail(
        r#"
type c++::B {
    safe fn value() const -> i32;
}
"#,
        expect![[r#"
            Error: C++ member functions take an explicit receiver instead of a trailing `const`: `&'a const self` for `const` member functions, and `&'a unique var self` (or `&'a var self`) for the others
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn value() const -> i32;
               │     ──────────────┬──────────────  
               │                   ╰──────────────── C++ member functions take an explicit receiver instead of a trailing `const`: `&'a const self` for `const` member functions, and `&'a unique var self` (or `&'a var self`) for the others
            ───╯
        "#]],
    );
}

#[test]
fn cpp_receivers_parse() {
    use zngur_def::{CppConstness, CppIndirection, Lifetime, ZngurMethodReceiver};

    let parsed = parse_str(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;

    safe fn get<'a>(&'a const self) -> &'a const i32;
    safe fn set(&unique var self, i32);
    unsafe fn raw_get(const self*) -> i32;
    unsafe fn raw_set(self&, i32);
    safe fn make() -> c++::B;
}
"#,
        NullCfg,
        |_| {},
    );
    let methods = &parsed.spec.types[0].cpp_methods;
    assert_eq!(
        methods[0].receiver,
        ZngurMethodReceiver::Ref(Mutability::Not)
    );
    assert_eq!(methods[0].receiver_lifetime, Some(Lifetime("a".to_owned())));
    assert_eq!(methods[0].lifetimes, vec![Lifetime("a".to_owned())]);
    assert_eq!(
        methods[1].receiver,
        ZngurMethodReceiver::Ref(Mutability::Mut)
    );
    assert_eq!(methods[1].raw_receiver, None);
    assert_eq!(methods[1].inputs.len(), 1);
    assert_eq!(
        methods[2].receiver,
        ZngurMethodReceiver::Ref(Mutability::Not)
    );
    assert_eq!(methods[2].raw_receiver, Some(CppIndirection::Pointer));
    assert_eq!(
        methods[3].receiver,
        ZngurMethodReceiver::Ref(Mutability::Mut)
    );
    assert_eq!(methods[3].raw_receiver, Some(CppIndirection::Reference));
    assert_eq!(methods[4].receiver, ZngurMethodReceiver::Static);
    let constnesses: Vec<CppConstness> = methods.iter().map(|m| m.receiver_constness).collect();
    assert_eq!(
        constnesses,
        [
            CppConstness::Const,
            CppConstness::Var,
            CppConstness::Const,
            CppConstness::Var,
            CppConstness::Var
        ]
    );
}

#[test]
fn shared_var_and_unique_const_receivers_parse() {
    use zngur_def::{CppConstness, ZngurMethodReceiver};

    let parsed = parse_str(
        r#"
type c++::B {
    safe fn bump<'a>(&'a var self);
    safe fn peek<'a>(&'a unique const self) -> i32;
}
"#,
        NullCfg,
        |_| {},
    );
    let methods = &parsed.spec.types[0].cpp_methods;
    assert_eq!(
        methods[0].receiver,
        ZngurMethodReceiver::Ref(Mutability::Not)
    );
    assert_eq!(methods[0].receiver_constness, CppConstness::Var);
    assert_eq!(
        methods[1].receiver,
        ZngurMethodReceiver::Ref(Mutability::Mut)
    );
    assert_eq!(methods[1].receiver_constness, CppConstness::Const);
}

#[test]
fn rust_style_cpp_receivers_are_rejected() {
    check_fail(
        r#"
type c++::B {
    safe fn get<'a>(&'a self) -> i32;
    safe fn set(&mut self, i32);
}
"#,
        expect![[r#"
            Error: the receiver of a C++ member function is a reference to a C++ object, which says whether it's `const`: `&'a const self` (a `const` member function)
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn get<'a>(&'a self) -> i32;
               │     ────────────────┬────────────────  
               │                     ╰────────────────── the receiver of a C++ member function is a reference to a C++ object, which says whether it's `const`: `&'a const self` (a `const` member function)
            ───╯
            Error: the receiver of a C++ member function is a reference to a C++ object, which says whether it's `const`: `&unique var self` (a non-`const` member function, `&mut self` in Rust)
               ╭─[ test.zng:4:5 ]
               │
             4 │     safe fn set(&mut self, i32);
               │     ──────────────┬─────────────  
               │                   ╰─────────────── the receiver of a C++ member function is a reference to a C++ object, which says whether it's `const`: `&unique var self` (a non-`const` member function, `&mut self` in Rust)
            ───╯
        "#]],
    );
}

#[test]
fn elided_lifetimes_warn() {
    check_warnings(
        r#"
type c++::B {
    safe fn get(&const self) -> &const i32;
}
"#,
        expect![[r#"
            Warning: this C++ member function has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `fn f<'a>(&'a const self) -> &'a const T`), or silence this with `#allow(elided_lifetimes)`
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn get(&const self) -> &const i32;
               │     ───────────────────┬───────────────────  
               │                        ╰───────────────────── this C++ member function has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `fn f<'a>(&'a const self) -> &'a const T`), or silence this with `#allow(elided_lifetimes)`
            ───╯
        "#]],
    );
}

#[test]
fn allow_silences_warnings() {
    check_success(
        r#"
#allow(elided_lifetimes)
#allow(safe_raw_pointer)

type c++::B {
    safe fn get(&const self) -> &const i32;
    safe fn set(&unique var self, const c++::B*);
}
"#,
    );
}

#[test]
fn allow_is_scoped_to_single_file() {
    let resolver = MockFilesystem::new(vec![
        (
            "./a.zng",
            r#"
#allow(elided_lifetimes)
type c++::B {
    safe fn get(&const self) -> &const i32;
}
"#,
        ),
        (
            "./b.zng",
            r#"
#unstable(template_types)

type c++::B {
    safe fn direct_elided(&const self) -> &const i32;
    safe fn direct_ptr(const c++::B*);
}

type<T> T {
    safe fn templated_elided(const T*) -> &const i32;
}
"#,
        ),
    ]);
    let mut warnings = String::new();
    let _ = parse_str_with_resolver(
        r#"
merge "./a.zng";
type c++::B {
    safe fn other(&const self) -> &const i32;
}
"#,
        NullCfg,
        &resolver,
        |w| warnings.push_str(w),
    );
    assert!(
        warnings.contains("test.zng:4:5") && !warnings.contains("a.zng"),
        "expected warning only in test.zng, got:\n{warnings}"
    );

    let mut reverse_warnings = String::new();
    let _ = parse_str_with_resolver(
        r#"
#allow(elided_lifetimes)
#allow(safe_raw_pointer)
merge "./b.zng";
type c++::B {
    safe fn root_ok(&const self, const c++::B*) -> &const i32;
}
"#,
        NullCfg,
        &resolver,
        |w| reverse_warnings.push_str(w),
    );
    assert!(
        !reverse_warnings.contains("test.zng"),
        "expected no warnings in root test.zng, got:\n{reverse_warnings}"
    );
    assert_eq!(
        reverse_warnings.matches("Warning:").count(),
        4,
        "expected 4 warnings in b.zng (2 direct + 2 from template static method), got:\n{reverse_warnings}"
    );
}

#[test]
fn unknown_allow_is_rejected() {
    check_fail(
        r#"
#allow(everything)
"#,
        expect![[r#"
            Error: unknown warning `everything` (known ones: elided_lifetimes, safe_raw_pointer, layout_without_relocatable)
               ╭─[ test.zng:2:1 ]
               │
             2 │ #allow(everything)
               │ ─────────┬────────  
               │          ╰────────── unknown warning `everything` (known ones: elided_lifetimes, safe_raw_pointer, layout_without_relocatable)
            ───╯
        "#]],
    );
}

#[test]
fn nested_allow_is_rejected() {
    check_fail(
        r#"
mod c++::a {
    #allow(elided_lifetimes)
}
"#,
        expect![[r#"
            Error: `#allow(...)` applies to the whole file and can only appear at the top level
               ╭─[ test.zng:3:5 ]
               │
             3 │     #allow(elided_lifetimes)
               │     ────────────┬───────────  
               │                 ╰───────────── `#allow(...)` applies to the whole file and can only appear at the top level
            ───╯
        "#]],
    );
}

#[test]
fn cpp_method_by_value_ref_only_or_undeclared_cpp_types_are_rejected() {
    check_fail(
        r#"
type c++::RefOnly {}

type c++::Owner {
    safe fn make() -> c++::RefOnly;
    safe fn take<'a>(&'a const self, c++::RefOnly);
    safe fn unknown<'a>(&'a const self, &'a const c++::Missing);
}
"#,
        expect![[r#"
            Error: `c++::RefOnly` cannot be passed or returned by value without `#layout(...)` and `#trivially_relocatable`
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe fn make() -> c++::RefOnly;
               │     ───────────────┬───────────────  
               │                    ╰───────────────── `c++::RefOnly` cannot be passed or returned by value without `#layout(...)` and `#trivially_relocatable`
            ───╯
            Error: `c++::RefOnly` cannot be passed or returned by value without `#layout(...)` and `#trivially_relocatable`
               ╭─[ test.zng:6:5 ]
               │
             6 │     safe fn take<'a>(&'a const self, c++::RefOnly);
               │     ───────────────────────┬───────────────────────  
               │                            ╰───────────────────────── `c++::RefOnly` cannot be passed or returned by value without `#layout(...)` and `#trivially_relocatable`
            ───╯
            Error: `c++::Missing` is not declared or imported
               ╭─[ test.zng:7:5 ]
               │
             7 │     safe fn unknown<'a>(&'a const self, &'a const c++::Missing);
               │     ──────────────────────────────┬─────────────────────────────  
               │                                   ╰─────────────────────────────── `c++::Missing` is not declared or imported
            ───╯
        "#]],
    );

    let resolver = MockFilesystem::new(vec![(
        "./dep.zng",
        r#"
#crate dep_crate
type c++::ImportedRefOnly {}
"#,
    )]);
    check_import_fail(
        r#"
import "./dep.zng";

type c++::Local {
    safe fn ok<'a>(&'a const self, &'a const c++::ImportedRefOnly, ::std::option::Option<&'a const c++::ImportedRefOnly>);
    safe fn bad<'a>(&'a const self, c++::ImportedRefOnly);
}
"#,
        expect![[r#"
            Error: `c++::ImportedRefOnly` cannot be passed or returned by value without `#layout(...)` and `#trivially_relocatable`
               ╭─[ test.zng:6:5 ]
               │
             6 │     safe fn bad<'a>(&'a const self, c++::ImportedRefOnly);
               │     ───────────────────────────┬──────────────────────────  
               │                                ╰──────────────────────────── `c++::ImportedRefOnly` cannot be passed or returned by value without `#layout(...)` and `#trivially_relocatable`
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn cpp_method_merge_with_alpha_equivalent_lifetimes_succeeds() {
    let resolver = MockFilesystem::new(vec![(
        "./other.zng",
        r#"
type c++::B {
    safe fn get<'b>(&'b const self) -> &'b const i32;
}
"#,
    )]);
    let parsed = parse_str_with_resolver(
        r#"
merge "./other.zng";

type c++::B {
    safe fn get<'a>(&'a const self) -> &'a const i32;
}
"#,
        NullCfg,
        &resolver,
        |e| panic!("unexpected diagnostic: {e}"),
    );
    assert_eq!(parsed.spec.types[0].cpp_methods.len(), 1);
}

#[test]
fn anonymous_lifetime_warns() {
    check_warnings(
        r#"
type c++::B {
    safe fn get(&'_ const self) -> &'_ const i32;
    safe fn arg<'a>(&'a const self, &'_ const i32);
}
"#,
        expect![[r#"
            Warning: this C++ member function has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `fn f<'a>(&'a const self) -> &'a const T`), or silence this with `#allow(elided_lifetimes)`
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn get(&'_ const self) -> &'_ const i32;
               │     ──────────────────────┬──────────────────────  
               │                           ╰──────────────────────── this C++ member function has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `fn f<'a>(&'a const self) -> &'a const T`), or silence this with `#allow(elided_lifetimes)`
            ───╯
            Warning: this C++ member function has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `fn f<'a>(&'a const self) -> &'a const T`), or silence this with `#allow(elided_lifetimes)`
               ╭─[ test.zng:4:5 ]
               │
             4 │     safe fn arg<'a>(&'a const self, &'_ const i32);
               │     ───────────────────────┬───────────────────────  
               │                            ╰───────────────────────── this C++ member function has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `fn f<'a>(&'a const self) -> &'a const T`), or silence this with `#allow(elided_lifetimes)`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_method_rejects_impl_trait_and_warns_on_dyn_raw_ptr() {
    check_fail(
        r#"
type c++::B {
    safe fn ret<'a>(&'a const self) -> impl crate::MyTrait;
    safe fn arg<'a>(&'a const self, impl crate::MyTrait);
}
"#,
        expect![[r#"
            Error: `impl Trait` cannot be used in C++ function signatures; use `Box<dyn Trait>` or `&dyn Trait` instead
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn ret<'a>(&'a const self) -> impl crate::MyTrait;
               │     ───────────────────────────┬───────────────────────────  
               │                                ╰───────────────────────────── `impl Trait` cannot be used in C++ function signatures; use `Box<dyn Trait>` or `&dyn Trait` instead
            ───╯
            Error: `impl Trait` cannot be used in C++ function signatures; use `Box<dyn Trait>` or `&dyn Trait` instead
               ╭─[ test.zng:4:5 ]
               │
             4 │     safe fn arg<'a>(&'a const self, impl crate::MyTrait);
               │     ──────────────────────────┬──────────────────────────  
               │                               ╰──────────────────────────── `impl Trait` cannot be used in C++ function signatures; use `Box<dyn Trait>` or `&dyn Trait` instead
            ───╯
        "#]],
    );

    check_warnings(
        r#"
type c++::B {
    safe fn with_cb<'a>(&'a const self, Box<dyn Fn(i32*)>);
}
"#,
        expect![[r#"
            Warning: this C++ member function is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn with_cb<'a>(&'a const self, Box<dyn Fn(i32*)>);
               │     ───────────────────────────┬───────────────────────────  
               │                                ╰───────────────────────────── this C++ member function is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
            ───╯
        "#]],
    );
}

#[test]
fn template_types_substitute_cpp_methods_and_reject_rust_targets() {
    let parsed = parse_str(
        r#"
#unstable(template_types)
type<T> T {
    safe fn clone_val<'a>(&'a const self, &'a const T) -> T;
    safe fn make_default() -> T;
}
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let ty = &parsed.spec.types[0];
    assert!(ty.methods.is_empty());
    assert_eq!(ty.cpp_methods.len(), 2);
    assert_eq!(ty.cpp_methods[0].cpp_name, "clone_val");
    assert_eq!(
        ty.cpp_methods[0].output,
        RustType::Cpp(vec!["B".to_owned()].into())
    );
    assert_eq!(ty.cpp_methods[1].cpp_name, "make_default");

    check_fail(
        r#"
#unstable(template_types)
type<T> crate::Vec<T> {
    unsafe fn value<'a>(&'a const self) -> i32;
}
"#,
        expect![[r#"
            Error: methods of Rust types and `extern "C++"` impls take `&self` or `&mut self`, not `&const self` / `&var self`
               ╭─[ test.zng:4:5 ]
               │
             4 │     unsafe fn value<'a>(&'a const self) -> i32;
               │     ─────────────────────┬────────────────────  
               │                          ╰────────────────────── methods of Rust types and `extern "C++"` impls take `&self` or `&mut self`, not `&const self` / `&var self`
            ───╯
        "#]],
    );

    let r = catch_unwind(|| {
        let _ = parse_str(
            r#"
#unstable(template_types)
type<T> T {
    unsafe fn value<'a>(&'a const self) -> i32;
}
type crate::Foo {
    #layout(size = 8, align = 4);
}
"#,
            NullCfg,
            |_| {},
        );
    });
    let err = r
        .unwrap_err()
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains(
            "C++ member functions (`safe fn` / `unsafe fn`) can only be declared on `c++::` types"
        ),
        "unexpected error:\n{err}"
    );

    // Static template methods instantiated on `c++::` targets normalize `as <same_name>` (so they
    // merge cleanly with direct declarations), reject Rust keyword names/aliases, and run
    // `elided_lifetimes` / `safe_raw_pointer` lints.
    let merged = parse_str(
        r#"
#unstable(template_types)
type<T> T {
    safe fn make() -> T as make;
}
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn make() -> c++::B;
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(merged.spec.types[0].cpp_methods.len(), 1);
    assert_eq!(merged.spec.types[0].cpp_methods[0].rust_name, None);

    for (src, expected) in [
        (
            r#"
#unstable(template_types)
type<T> T {
    safe fn move() -> T;
}
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
"#,
            "`move` is a Rust keyword; give this C++ function a non-keyword Rust name with `as <rust_name>`",
        ),
        (
            r#"
#unstable(template_types)
type<T> T {
    safe fn make() -> T as move;
}
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
"#,
            "`move` is a Rust keyword and cannot be used as the Rust function name in `as move`",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }

    let mut warnings = Vec::new();
    let _ = parse_str(
        r#"
#unstable(template_types)
type<T> T {
    safe fn from_ptr(const i32*) -> T;
    safe fn id(&const i32) -> &const i32;
}
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
"#,
        NullCfg,
        |w| warnings.push(w.to_owned()),
    );
    assert_eq!(warnings.len(), 2, "unexpected warnings: {warnings:?}");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("#allow(safe_raw_pointer)")),
        "missing safe_raw_pointer warning in: {warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("#allow(elided_lifetimes)")),
        "missing elided_lifetimes warning in: {warnings:?}"
    );
}

#[test]
fn cpp_constructors_parse() {
    use zngur_def::{CppIndirection, Mutability, PrimitiveRustType};

    let parsed = parse_str(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;

    safe constructor() as new;
    safe constructor(i32, i32) as from_xy;
    unsafe constructor(const c++::B&) as copy_from;
    safe constructor<'a>(::core::option::Option<&'a unique var c++::B>) as from_option;
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let b = RustType::Cpp(vec!["B".to_owned()].into());
    let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
    let constructors = &ty.cpp_constructors;
    let names: Vec<_> = constructors.iter().map(|c| c.rust_name.as_str()).collect();
    assert_eq!(names, ["new", "from_xy", "copy_from", "from_option"]);
    assert!(constructors[0].inputs.is_empty());
    assert!(constructors[0].is_safe);
    assert_eq!(constructors[1].inputs, vec![i32.clone(), i32]);
    assert_eq!(
        constructors[2].inputs,
        vec![RustType::cpp_ptr(
            Mutability::Not,
            CppIndirection::Reference,
            b
        )]
    );
    assert!(!constructors[2].is_safe);
    assert_eq!(constructors[3].lifetimes, vec![Lifetime("a".to_owned())]);
}

#[test]
fn safe_cpp_constructor_with_cpp_ptr_warns() {
    check_warnings(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor(const c++::B*) as from_ptr;
}
"#,
        expect![[r#"
            Warning: this C++ constructor is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe constructor(const c++::B*) as from_ptr;
               │     ──────────────────────┬─────────────────────  
               │                           ╰─────────────────────── this C++ constructor is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_constructor_without_a_rust_name_is_rejected() {
    check_fail(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor(i32);
}
"#,
        expect![[r#"
            Error: C++ constructors need a Rust name: `as <name>`
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe constructor(i32);
               │     ───────────┬──────────  
               │                ╰──────────── C++ constructors need a Rust name: `as <name>`
            ───╯
        "#]],
    );
}

#[test]
fn bare_constructor_without_safety_on_cpp_type_is_rejected() {
    check_fail(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    constructor(i32);
    constructor(i32) as new;
}
"#,
        expect![[r#"
            Error: C++ constructors start with `safe` or `unsafe` and need a Rust name with `as <name>` (e.g. `safe constructor(...) as new;`)
               ╭─[ test.zng:5:5 ]
               │
             5 │     constructor(i32);
               │     ────────┬────────  
               │             ╰────────── C++ constructors start with `safe` or `unsafe` and need a Rust name with `as <name>` (e.g. `safe constructor(...) as new;`)
            ───╯
            Error: C++ constructors start with `safe` or `unsafe` and need a Rust name with `as <name>` (e.g. `safe constructor(...) as new;`)
               ╭─[ test.zng:6:5 ]
               │
             6 │     constructor(i32) as new;
               │     ────────────┬───────────  
               │                 ╰───────────── C++ constructors start with `safe` or `unsafe` and need a Rust name with `as <name>` (e.g. `safe constructor(...) as new;`)
            ───╯
        "#]],
    );
}

#[test]
fn members_with_the_same_rust_name_are_rejected() {
    check_fail(
        r#"
#allow(elided_lifetimes)
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor(i32) as new;
    unsafe constructor(i64) as new;
    safe fn new(&const self) -> i32;
    safe fn value(&const self) -> i32;
    safe fn value(&const self) -> i64;
    field &const value (type = i32);
    safe constructor() as value;
}
"#,
        expect![[r#"
            Error: `new` is already the Rust name of another member of this type (a constructor, member function or field accessor)
               ╭─[ test.zng:7:5 ]
               │
             7 │     unsafe constructor(i64) as new;
               │     ───────────────┬───────────────  
               │                    ╰───────────────── `new` is already the Rust name of another member of this type (a constructor, member function or field accessor)
            ───╯
            Error: `new` is already the Rust name of another member of this type (a constructor, member function or field accessor)
               ╭─[ test.zng:8:5 ]
               │
             8 │     safe fn new(&const self) -> i32;
               │     ────────────────┬───────────────  
               │                     ╰───────────────── `new` is already the Rust name of another member of this type (a constructor, member function or field accessor)
            ───╯
            Error: `value` is already the Rust name of another member of this type (a constructor, member function or field accessor)
                ╭─[ test.zng:10:5 ]
                │
             10 │     safe fn value(&const self) -> i64;
                │     ─────────────────┬────────────────  
                │                      ╰────────────────── `value` is already the Rust name of another member of this type (a constructor, member function or field accessor)
            ────╯
            Error: `value` is already the Rust name of another member of this type (a constructor, member function or field accessor)
                ╭─[ test.zng:11:5 ]
                │
             11 │     field &const value (type = i32);
                │     ────────────────┬───────────────  
                │                     ╰───────────────── `value` is already the Rust name of another member of this type (a constructor, member function or field accessor)
            ────╯
            Error: `value` is already the Rust name of another member of this type (a constructor, member function or field accessor)
                ╭─[ test.zng:12:5 ]
                │
             12 │     safe constructor() as value;
                │     ──────────────┬─────────────  
                │                   ╰─────────────── `value` is already the Rust name of another member of this type (a constructor, member function or field accessor)
            ────╯
        "#]],
    );
}

#[test]
fn members_with_the_same_rust_name_in_different_files_are_rejected() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn make() -> c++::B;
    safe constructor(i32) as new;
}
"#,
    )]);

    check_import_fail(
        r#"
merge "./a.zng";
type c++::B {
    safe constructor() as make;
    safe constructor(i64) as new;
}
"#,
        expect![[r#"
            Error: C++ constructor mismatch
               ╭─[ ./a.zng:2:6 ]
               │
             2 │ type c++::B {
               │      ───┬──  
               │         ╰──── C++ constructor mismatch
               │ 
             6 │     safe constructor(i32) as new;
               │     ──────────────┬──────────────  
               │                   ╰──────────────── declaration here ...
               │
               ├─[ test.zng:5:5 ]
               │
             5 │     safe constructor(i64) as new;
               │     ──────────────┬──────────────  
               │                   ╰──────────────── conflicts with the declaration here.
            ───╯
            Error: `make` is already the Rust name of another member of this type (a constructor, member function or field accessor)
               ╭─[ ./a.zng:5:5 ]
               │
             5 │     safe fn make() -> c++::B;
               │     ────────────┬────────────  
               │                 ╰────────────── `make` is already the Rust name of another member of this type (a constructor, member function or field accessor)
               │
               ├─[ test.zng:4:5 ]
               │
             4 │     safe constructor() as make;
               │     ─────────────┬─────────────  
               │                  ╰─────────────── also declared here
            ───╯
        "#]],
        &resolver,
    );

    let safety_resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor(i32) as from_int;
}
"#,
    )]);
    check_import_fail(
        r#"
merge "./a.zng";
type c++::B {
    unsafe constructor(i32) as from_int;
}
"#,
        expect![[r#"
            Error: C++ constructor mismatch
               ╭─[ ./a.zng:2:6 ]
               │
             2 │ type c++::B {
               │      ───┬──  
               │         ╰──── C++ constructor mismatch
               │ 
             5 │     safe constructor(i32) as from_int;
               │     ─────────────────┬────────────────  
               │                      ╰────────────────── declaration here ...
               │
               ├─[ test.zng:4:5 ]
               │
             4 │     unsafe constructor(i32) as from_int;
               │     ──────────────────┬─────────────────  
               │                       ╰─────────────────── conflicts with the declaration here.
            ───╯
        "#]],
        &safety_resolver,
    );
}

#[test]
fn cpp_constructors_can_be_declared_and_deduplicated_across_files() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor() as new;
    safe constructor<'a>(&'a const c++::B) as copy;
}
"#,
    )]);
    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
type c++::B {
    safe constructor() as new;
    safe constructor<'b>(&'b const c++::B) as copy;
    safe constructor(i32) as from_i32;
}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let names: Vec<_> = ty
        .cpp_constructors
        .iter()
        .map(|c| c.rust_name.as_str())
        .collect();
    assert_eq!(names, ["new", "copy", "from_i32"]);
}

#[test]
fn cpp_constructors_with_the_same_argument_types_are_fine() {
    // Each one is its own Rust function, so nothing overlaps.
    let parsed = parse_str(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor(i32) as new;
    unsafe constructor(i32) as new_unchecked;
    safe constructor<'a>(i32, &'a const c++::B) as from_const;
    safe constructor<'b>(i32, &'b var c++::B) as from_var;
    safe constructor(const c++::B&) as from_ref;
    unsafe constructor(const c++::B*) as from_ptr;
}
"#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_eq!(ty.cpp_constructors.len(), 6);
}

#[test]
fn cpp_constructor_on_non_by_value_or_non_cpp_type_is_rejected() {
    for src in [
        // Reference-only `c++::` type without layout.
        "type c++::B { safe constructor(i32) as new; }\n",
        // `c++::` type with `#layout`, but without `#trivially_relocatable`.
        "type c++::B { #layout(size = 8, align = 4); safe constructor(i32) as new; }\n",
        // Ordinary Rust type.
        "type crate::Foo { #layout(size = 8, align = 4); safe constructor(i32) as new; }\n",
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(
                "C++ constructors can only be declared on `c++::` types that Rust holds by value \
                 (with a layout and `#trivially_relocatable`)"
            ),
            "expected non-by-value/non-cpp constructor error for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn cpp_constructor_lifetime_diagnostics() {
    check_fail(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor(&'a const c++::B) as copy;
}
"#,
        expect![[r#"
            Error: undeclared lifetime `'a`: declare it on the function, `impl` or `type` (e.g. `fn f<'a>(...)`)
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe constructor(&'a const c++::B) as copy;
               │     ─────────────────────┬─────────────────────  
               │                          ╰─────────────────────── undeclared lifetime `'a`: declare it on the function, `impl` or `type` (e.g. `fn f<'a>(...)`)
            ───╯
        "#]],
    );
    check_warnings(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor(&const c++::B) as copy;
}
"#,
        expect![[r#"
            Warning: this C++ constructor has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `safe constructor<'a>(&'a const c++::T) as from_ref`), or silence this with `#allow(elided_lifetimes)`
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe constructor(&const c++::B) as copy;
               │     ────────────────────┬───────────────────  
               │                         ╰───────────────────── this C++ constructor has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `safe constructor<'a>(&'a const c++::T) as from_ref`), or silence this with `#allow(elided_lifetimes)`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_constructor_with_self_in_parameter_is_rejected() {
    check_fail(
        r#"
type c++::Foo {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor<'a>(&'a const self) as copy;
}

mod c++::geo {
    type Point {
        #layout(size = 8, align = 4);
        #trivially_relocatable;
        safe constructor<'a>(&'a const self) as copy;
    }
}
"#,
        expect![[r#"
            Error: C++ constructors do not have a `self` receiver; spell out the C++ type path explicitly (e.g. `&'a const c++::ns::Foo`)
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe constructor<'a>(&'a const self) as copy;
               │     ──────────────────────┬──────────────────────  
               │                           ╰──────────────────────── C++ constructors do not have a `self` receiver; spell out the C++ type path explicitly (e.g. `&'a const c++::ns::Foo`)
            ───╯
            Error: C++ constructors do not have a `self` receiver; spell out the C++ type path explicitly (e.g. `&'a const c++::ns::Foo`)
                ╭─[ test.zng:12:9 ]
                │
             12 │         safe constructor<'a>(&'a const self) as copy;
                │         ──────────────────────┬──────────────────────  
                │                               ╰──────────────────────── C++ constructors do not have a `self` receiver; spell out the C++ type path explicitly (e.g. `&'a const c++::ns::Foo`)
            ────╯
        "#]],
    );
}

#[test]
fn cpp_constructor_additional_validation_paths() {
    // `constructor<'a>(...)` and `constructor(...) as foo` on a Rust type are rejected.
    check_fail(
        r#"
type crate::Foo {
    #layout(size = 4, align = 4);
    constructor<'a>(i32);
    constructor(i32) as foo;
}
"#,
        expect![[r#"
            Error: Rust tuple/struct `constructor` declarations don't take lifetime parameters or `as <name>` (C++ constructors start with `safe` or `unsafe` on `c++::` types)
               ╭─[ test.zng:4:5 ]
               │
             4 │     constructor<'a>(i32);
               │     ──────────┬──────────  
               │               ╰──────────── Rust tuple/struct `constructor` declarations don't take lifetime parameters or `as <name>` (C++ constructors start with `safe` or `unsafe` on `c++::` types)
            ───╯
            Error: Rust tuple/struct `constructor` declarations don't take lifetime parameters or `as <name>` (C++ constructors start with `safe` or `unsafe` on `c++::` types)
               ╭─[ test.zng:5:5 ]
               │
             5 │     constructor(i32) as foo;
               │     ────────────┬───────────  
               │                 ╰───────────── Rust tuple/struct `constructor` declarations don't take lifetime parameters or `as <name>` (C++ constructors start with `safe` or `unsafe` on `c++::` types)
            ───╯
        "#]],
    );

    // Rust keywords and inherent wrapper method names (`as_const`, `as_const_mut`, `into_const`)
    // are rejected on `CppConstructor` with a rename-aware hint.
    check_fail(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor() as type;
    safe constructor() as as_const;
    safe constructor() as as_const_mut;
    safe constructor() as into_const;
}
"#,
        expect![[r#"
            Error: `type` is a Rust keyword and cannot be used as the Rust function name in `as type`
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe constructor() as type;
               │     ─────────────┬─────────────  
               │                  ╰─────────────── `type` is a Rust keyword and cannot be used as the Rust function name in `as type`
            ───╯
            Error: `as_const` collides with an inherent conversion method on `c++::` wrappers; choose a different name in `as <rust_name>`
               ╭─[ test.zng:6:5 ]
               │
             6 │     safe constructor() as as_const;
               │     ───────────────┬───────────────  
               │                    ╰───────────────── `as_const` collides with an inherent conversion method on `c++::` wrappers; choose a different name in `as <rust_name>`
            ───╯
            Error: `as_const_mut` collides with an inherent conversion method on `c++::` wrappers; choose a different name in `as <rust_name>`
               ╭─[ test.zng:7:5 ]
               │
             7 │     safe constructor() as as_const_mut;
               │     ─────────────────┬─────────────────  
               │                      ╰─────────────────── `as_const_mut` collides with an inherent conversion method on `c++::` wrappers; choose a different name in `as <rust_name>`
            ───╯
            Error: `into_const` collides with an inherent conversion method on `c++::` wrappers; choose a different name in `as <rust_name>`
               ╭─[ test.zng:8:5 ]
               │
             8 │     safe constructor() as into_const;
               │     ────────────────┬────────────────  
               │                     ╰────────────────── `into_const` collides with an inherent conversion method on `c++::` wrappers; choose a different name in `as <rust_name>`
            ───╯
        "#]],
    );

    // Reference-only / undeclared `c++::` parameter by value in a constructor is rejected.
    check_fail(
        r#"
type c++::RefOnly {}
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor(c++::RefOnly) as from_ref_only;
}
"#,
        expect![[r#"
            Error: `c++::RefOnly` cannot be passed or returned by value without `#layout(...)` and `#trivially_relocatable`
               ╭─[ test.zng:6:5 ]
               │
             6 │     safe constructor(c++::RefOnly) as from_ref_only;
               │     ────────────────────────┬───────────────────────  
               │                             ╰───────────────────────── `c++::RefOnly` cannot be passed or returned by value without `#layout(...)` and `#trivially_relocatable`
            ───╯
        "#]],
    );

    // `#unsafe(trivially_relocatable)` allows C++ constructors just like `#trivially_relocatable`.
    let parsed = parse_str(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #unsafe(trivially_relocatable);
    safe constructor(i32) as new;
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.types[0].cpp_constructors.len(), 1);
}

#[test]
fn template_types_substitute_cpp_constructors_and_reject_invalid_targets() {
    use zngur_def::{Lifetime, Mutability, PrimitiveRustType, TypeVar, ZngurCppConstructor};

    let parsed = parse_str(
        r#"
#unstable(template_types)
type<T> T {
    safe constructor<'a>(&'a const T) as copy;
    safe constructor(i32) as from_int;
}
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let ty = &parsed.spec.types[0];
    assert_eq!(ty.cpp_constructors.len(), 2);
    assert_eq!(ty.cpp_constructors[0].rust_name, "copy");
    assert_eq!(ty.cpp_constructors[0].lifetimes, vec![Lifetime("a".into())]);
    assert_eq!(
        ty.cpp_constructors[0].inputs,
        vec![RustType::Ref(
            Mutability::Not,
            Some(Lifetime("a".into())),
            Box::new(RustType::CppConst(Box::new(RustType::Cpp(
                vec!["B".into()].into(),
            )))),
        )]
    );
    assert_eq!(ty.cpp_constructors[1].rust_name, "from_int");
    assert_eq!(
        ty.cpp_constructors[1].inputs,
        vec![RustType::Primitive(PrimitiveRustType::Int(32))]
    );

    // Colliding constructor lifetime parameters are alpha-renamed when `target_lts` /
    // `outer_in_scope` already contains the lifetime.
    let lt_a = Lifetime("a".into());
    let var_t = RustType::TypeVar(TypeVar("T".into()));
    let template = zngur_def::ZngurType {
        ty: var_t.clone(),
        lifetimes: vec![],
        layout: None,
        trivially_relocatable: zngur_def::TriviallyRelocatable::No,
        wellknown_traits: vec![],
        exhaustive: true,
        methods: vec![],
        cpp_methods: vec![],
        cpp_constructors: vec![ZngurCppConstructor {
            rust_name: "from_borrow".into(),
            lifetimes: vec![lt_a.clone()],
            inputs: vec![
                RustType::Ref(
                    Mutability::Not,
                    Some(lt_a.clone()),
                    Box::new(RustType::Primitive(PrimitiveRustType::Int(32))),
                ),
                var_t,
            ],
            is_safe: true,
        }],
        cpp_fields: vec![],
        constructor: None,
        variants: vec![],
        fields: vec![],
    };
    let target_ty = RustType::Adt(zngur_def::RustPathAndGenerics {
        path: vec!["crate".into(), "Foo".into()],
        lifetimes: vec![lt_a.clone()],
        generics: vec![],
        named_generics: vec![],
    });
    let target = zngur_def::ZngurType {
        ty: target_ty.clone(),
        lifetimes: vec![lt_a.clone()],
        cpp_constructors: vec![],
        ..template.clone()
    };
    let matched = crate::try_match_template(&target, &template)
        .expect("template should match")
        .0;
    let fresh_a = Lifetime("a_0".into());
    assert_eq!(matched.cpp_constructors[0].lifetimes, vec![fresh_a.clone()]);
    assert_eq!(
        matched.cpp_constructors[0].inputs,
        vec![
            RustType::Ref(
                Mutability::Not,
                Some(fresh_a),
                Box::new(RustType::Primitive(PrimitiveRustType::Int(32))),
            ),
            target_ty,
        ]
    );

    for src in [
        r#"
#unstable(template_types)
type<T> T {
    safe constructor(i32) as new;
}
type crate::Foo {
    #layout(size = 8, align = 4);
}
"#,
        r#"
#unstable(template_types)
type<T> T {
    safe constructor(i32) as new;
}
type c++::RefOnly {}
"#,
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(
                "C++ constructors can only be declared on `c++::` types that Rust holds by value \
                 (with a layout and `#trivially_relocatable`)"
            ),
            "unexpected error for `{src}`:\n{err}"
        );
    }
}

#[test]
fn cpp_functions_parse() {
    use zngur_def::{CppIndirection, Mutability, PrimitiveRustType};

    let parsed = parse_str(
        r#"
use c++::a::b as ab;

mod c++::a {
    type B {
        #layout(size = 8, align = 4);
        #trivially_relocatable;
    }

    mod b {
        unsafe fn f(const c++::a::B&) -> i32;
        safe fn g(i32) -> c++::a::B as make_b;
        safe fn h();
        safe fn type(i32) -> i32 as get_type;
    }

    safe fn b::sub_fn(i32) -> i32;
}

safe fn c++::a::b::top_fn();
safe fn ab::alias_fn();

safe fn crate::rust_root(i32);
mod crate::r {
    unsafe fn rust_unsafe(i32);
    fn rust_default(i32);
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let funcs = &parsed.spec.cpp_funcs;
    assert_eq!(funcs.len(), 7);
    let b = RustType::Cpp(vec!["a".to_owned(), "B".to_owned()].into());
    let i32 = RustType::Primitive(PrimitiveRustType::Int(32));

    assert_eq!(funcs[0].namespace, ["a", "b"]);
    assert_eq!(funcs[0].cpp_path(), "::a::b::f");
    assert_eq!(funcs[0].rust_name(), "f");
    assert_eq!(
        funcs[0].inputs,
        vec![RustType::cpp_ptr(
            Mutability::Not,
            CppIndirection::Reference,
            b.clone()
        )]
    );
    assert_eq!(funcs[0].output, i32);
    assert!(!funcs[0].is_safe);

    assert_eq!(funcs[1].cpp_name, "g");
    assert_eq!(funcs[1].rust_name(), "make_b");
    assert_eq!(funcs[1].output, b);
    assert!(funcs[1].is_safe);

    assert_eq!(funcs[2].output, RustType::UNIT);

    assert_eq!(funcs[3].cpp_name, "type");
    assert_eq!(funcs[3].rust_name(), "get_type");

    assert_eq!(funcs[4].namespace, ["a", "b"]);
    assert_eq!(funcs[4].cpp_path(), "::a::b::sub_fn");

    assert_eq!(funcs[5].namespace, ["a", "b"]);
    assert_eq!(funcs[5].cpp_path(), "::a::b::top_fn");

    assert_eq!(funcs[6].namespace, ["a", "b"]);
    assert_eq!(funcs[6].cpp_path(), "::a::b::alias_fn");

    let rust_funcs = &parsed.spec.funcs;
    assert_eq!(rust_funcs.len(), 3);
    assert_eq!(rust_funcs[0].is_safe, Some(true));
    assert_eq!(rust_funcs[1].is_safe, Some(false));
    assert_eq!(rust_funcs[2].is_safe, None);
}

#[test]
fn cpp_function_in_global_namespace_or_invalid_modifiers_are_rejected() {
    check_fail(
        r#"
mod c++ {
    safe fn global_fn(i32);
}
safe fn c++::global_fn2(i32);
fn c++::a::bare_fn(i32);
safe async fn c++::a::async_fn();
fn crate::a::f(i32) as g;
"#,
        expect![[r#"
            Error: C++ functions in the global namespace aren't supported
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn global_fn(i32);
               │     ───────────┬──────────  
               │                ╰──────────── C++ functions in the global namespace aren't supported
            ───╯
            Error: C++ functions in the global namespace aren't supported
               ╭─[ test.zng:5:1 ]
               │
             5 │ safe fn c++::global_fn2(i32);
               │ ──────────────┬─────────────  
               │               ╰─────────────── C++ functions in the global namespace aren't supported
            ───╯
            Error: C++ functions must be marked `safe` or `unsafe`
               ╭─[ test.zng:6:1 ]
               │
             6 │ fn c++::a::bare_fn(i32);
               │ ───────────┬───────────  
               │            ╰───────────── C++ functions must be marked `safe` or `unsafe`
            ───╯
            Error: C++ functions cannot be `async`
               ╭─[ test.zng:7:1 ]
               │
             7 │ safe async fn c++::a::async_fn();
               │ ────────────────┬───────────────  
               │                 ╰───────────────── C++ functions cannot be `async`
            ───╯
            Error: Rust free functions cannot be renamed with `as`
               ╭─[ test.zng:8:1 ]
               │
             8 │ fn crate::a::f(i32) as g;
               │ ────────────┬───────────  
               │             ╰───────────── Rust free functions cannot be renamed with `as`
            ───╯
        "#]],
    );
}

#[test]
fn safe_cpp_function_with_cpp_ptr_and_elided_lifetime_warn() {
    check_warnings(
        r#"
mod c++::a {
    safe fn f(i32*);
    safe fn g(&const i32) -> &const i32;
    safe fn h<'a>(::std::option::Option<&'a const i32>);
}
"#,
        expect![[r#"
            Warning: this C++ function is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn f(i32*);
               │     ───────┬───────  
               │            ╰───────── this C++ function is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
            ───╯
            Warning: this C++ function has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `fn f<'a>(&'a const T) -> &'a const U`), or silence this with `#allow(elided_lifetimes)`
               ╭─[ test.zng:4:5 ]
               │
             4 │     safe fn g(&const i32) -> &const i32;
               │     ─────────────────┬─────────────────  
               │                      ╰─────────────────── this C++ function has a reference without a lifetime, which can't be checked against C++; name the lifetimes (e.g. `fn f<'a>(&'a const T) -> &'a const U`), or silence this with `#allow(elided_lifetimes)`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_function_keyword_rust_names_are_rejected() {
    for (src, expected) in [
        (
            "mod c++::a { safe fn type(i32) -> i32; }\n",
            "`type` is a Rust keyword; give this C++ function a non-keyword Rust name with `as <rust_name>`",
        ),
        (
            "mod c++::a { safe fn f(i32) -> i32 as type; }\n",
            "`type` is a Rust keyword and cannot be used as the Rust function name in `as type`",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn duplicate_cpp_function_name_is_rejected() {
    check_fail(
        r#"
mod c++::geo {
    safe fn scale(f64) -> f64;
    safe fn scale(i32) -> i32;
    safe fn other() as scale;
}
"#,
        expect![[r#"
            Error: `scale` is already the Rust name of another C++ function in `c++::geo` (use `as <rust_name>` to give C++ overloads distinct Rust names)
               ╭─[ test.zng:4:5 ]
               │
             4 │     safe fn scale(i32) -> i32;
               │     ────────────┬────────────  
               │                 ╰────────────── `scale` is already the Rust name of another C++ function in `c++::geo` (use `as <rust_name>` to give C++ overloads distinct Rust names)
            ───╯
            Error: `scale` is already the Rust name of another C++ function in `c++::geo` (use `as <rust_name>` to give C++ overloads distinct Rust names)
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe fn other() as scale;
               │     ────────────┬───────────  
               │                 ╰───────────── `scale` is already the Rust name of another C++ function in `c++::geo` (use `as <rust_name>` to give C++ overloads distinct Rust names)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_functions_merge_and_conflict_across_files() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
mod c++::geo {
    safe fn origin() -> i32;
    safe fn scale(f64) -> f64;
}
"#,
    )]);
    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
mod c++::geo {
    safe fn origin() -> i32;
    safe fn scale(i32) -> i32 as scale_i32;
}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    let names: Vec<_> = parsed
        .spec
        .cpp_funcs
        .iter()
        .map(|f| f.rust_name())
        .collect();
    assert_eq!(names, ["origin", "scale_i32", "scale"]);

    check_import_fail(
        r#"
merge "./a.zng";
mod c++::geo {
    safe fn scale(i32) -> i32;
}
"#,
        expect![[r#"
            Error: C++ function mismatch
               ╭─[ ./a.zng:4:5 ]
               │
             4 │     safe fn scale(f64) -> f64;
               │     ────────────┬────────────  
               │                 ╰────────────── C++ function mismatch
               │                 │              
               │                 ╰────────────── declaration here ...
               │
               ├─[ test.zng:4:5 ]
               │
             4 │     safe fn scale(i32) -> i32;
               │     ────────────┬────────────  
               │                 ╰────────────── conflicts with the declaration here.
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn cpp_function_with_self_is_rejected() {
    check_fail(
        r#"
mod c++::ns {
    safe fn f<'a>(&'a const self);
    safe fn g() -> &const self;
}
"#,
        expect![[r#"
            Error: free C++ functions do not take a `self` receiver (declare member functions inside a `type c++::... { ... }` block)
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn f<'a>(&'a const self);
               │     ──────────────┬──────────────  
               │                   ╰──────────────── free C++ functions do not take a `self` receiver (declare member functions inside a `type c++::... { ... }` block)
            ───╯
            Error: free C++ functions do not take a `self` receiver (declare member functions inside a `type c++::... { ... }` block)
               ╭─[ test.zng:4:5 ]
               │
             4 │     safe fn g() -> &const self;
               │     ─────────────┬────────────  
               │                  ╰────────────── free C++ functions do not take a `self` receiver (declare member functions inside a `type c++::... { ... }` block)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_function_redundant_as_same_name_merges() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
mod c++::geo {
    safe fn scale(f64) -> f64;
    safe fn pick<'a>(&'a const i32) -> &'a const i32;
}
"#,
    )]);
    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
mod c++::geo {
    safe fn scale(f64) -> f64 as scale;
    safe fn pick<'b>(&'b const i32) -> &'b const i32;
}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.cpp_funcs.len(), 2);
    assert_eq!(parsed.spec.cpp_funcs[0].cpp_name, "scale");
    assert_eq!(parsed.spec.cpp_funcs[0].rust_name, None);
    assert_eq!(parsed.spec.cpp_funcs[0].rust_name(), "scale");
}

#[test]
fn cpp_function_undeclared_or_non_by_value_cpp_type_is_rejected() {
    check_fail(
        r#"
type c++::geo::RefOnly {
    #layout(size = 8, align = 4);
}
mod c++::geo {
    safe fn f(&const c++::geo::Missing);
    safe fn g(c++::geo::RefOnly);
}
"#,
        expect![[r#"
            Error: `c++::geo::Missing` is not declared or imported
               ╭─[ test.zng:6:5 ]
               │
             6 │     safe fn f(&const c++::geo::Missing);
               │     ─────────────────┬─────────────────  
               │                      ╰─────────────────── `c++::geo::Missing` is not declared or imported
            ───╯
            Error: `c++::geo::RefOnly` cannot be passed or returned by value without `#layout(...)` and `#trivially_relocatable`
               ╭─[ test.zng:7:5 ]
               │
             7 │     safe fn g(c++::geo::RefOnly);
               │     ──────────────┬─────────────  
               │                   ╰─────────────── `c++::geo::RefOnly` cannot be passed or returned by value without `#layout(...)` and `#trivially_relocatable`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_function_can_share_name_with_cpp_type_in_same_namespace() {
    let parsed = parse_str(
        r#"
mod c++::ns {
    type Foo {}
    safe fn Foo<'a>(&'a const Foo);
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.types.len(), 1);
    assert_eq!(parsed.spec.cpp_funcs.len(), 1);
    assert_eq!(parsed.spec.cpp_funcs[0].cpp_name, "Foo");
}

#[test]
fn cpp_function_with_cpp_type_as_namespace_is_rejected() {
    check_fail(
        r#"
type c++::ns::Foo {}
safe fn c++::ns::Foo::bar() -> i32;
safe fn c++::ns::Foo::detail::baz() -> i32;
mod c++::ns::Foo {
    safe fn qux();
}
"#,
        expect![[r#"
            Error: `c++::ns::Foo` is a C++ type, not a namespace (declare static member functions inside `type c++::ns::Foo { ... }`)
               ╭─[ test.zng:3:1 ]
               │
             3 │ safe fn c++::ns::Foo::bar() -> i32;
               │ ─────────────────┬────────────────  
               │                  ╰────────────────── `c++::ns::Foo` is a C++ type, not a namespace (declare static member functions inside `type c++::ns::Foo { ... }`)
            ───╯
            Error: `c++::ns::Foo` is a C++ type, not a namespace
               ╭─[ test.zng:4:1 ]
               │
             4 │ safe fn c++::ns::Foo::detail::baz() -> i32;
               │ ─────────────────────┬────────────────────  
               │                      ╰────────────────────── `c++::ns::Foo` is a C++ type, not a namespace
            ───╯
            Error: `c++::ns::Foo` is a C++ type, not a namespace (declare static member functions inside `type c++::ns::Foo { ... }`)
               ╭─[ test.zng:6:5 ]
               │
             6 │     safe fn qux();
               │     ──────┬──────  
               │           ╰──────── `c++::ns::Foo` is a C++ type, not a namespace (declare static member functions inside `type c++::ns::Foo { ... }`)
            ───╯
        "#]],
    );

    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        r#"
#crate bar_bindings
type c++::ext::Bar {}
"#,
    )]);
    check_import_fail(
        r#"
import "./bar.zng";
safe fn c++::ext::Bar::make() -> i32;
safe fn c++::ext::Bar::sub::helper() -> i32;
"#,
        expect![[r#"
            Error: `c++::ext::Bar` is a C++ type, not a namespace (declare static member functions inside `type c++::ext::Bar { ... }`)
               ╭─[ test.zng:3:1 ]
               │
             3 │ safe fn c++::ext::Bar::make() -> i32;
               │ ──────────────────┬─────────────────  
               │                   ╰─────────────────── `c++::ext::Bar` is a C++ type, not a namespace (declare static member functions inside `type c++::ext::Bar { ... }`)
            ───╯
            Error: `c++::ext::Bar` is a C++ type, not a namespace
               ╭─[ test.zng:4:1 ]
               │
             4 │ safe fn c++::ext::Bar::sub::helper() -> i32;
               │ ─────────────────────┬─────────────────────  
               │                      ╰─────────────────────── `c++::ext::Bar` is a C++ type, not a namespace
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn cpp_fields_parse() {
    use zngur_def::{CppConstness, CppFieldAccess, PrimitiveRustType};

    let parsed = parse_str(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;

    field &const x (type = i32);
    safe field &unique var x (type = i32) as x_mut;
    field &var x (type = i32) as x_var;
    field &unique const x (type = i32) as x_unique_const;
    field const x (type = i32) as x_const_copy;
    field var x (type = i32) as x_copy;
    field &const flag (type = bool);
    field &const flag_ptr (type = const bool*);
    unsafe field &var next (type = c++::B*) as next_var;
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
    let bool_ty = RustType::Primitive(PrimitiveRustType::Bool);
    let fields = &ty.cpp_fields;
    assert_eq!(fields.len(), 9);
    assert_eq!(fields[0].name, "x");
    assert_eq!(fields[0].rust_name(), "x");
    assert_eq!(fields[0].ty, i32);
    assert!(fields[0].is_safe);
    let reference = |unique, constness| CppFieldAccess::Ref { unique, constness };
    assert_eq!(fields[0].access, reference(false, CppConstness::Const));
    assert_eq!(fields[1].name, "x");
    assert_eq!(fields[1].rust_name(), "x_mut");
    assert_eq!(fields[1].access, reference(true, CppConstness::Var));
    assert!(fields[1].is_safe);
    assert_eq!(fields[2].access, reference(false, CppConstness::Var));
    assert_eq!(fields[3].access, reference(true, CppConstness::Const));
    assert_eq!(
        fields[4].access,
        CppFieldAccess::Copy {
            constness: CppConstness::Const
        }
    );
    assert_eq!(
        fields[5].access,
        CppFieldAccess::Copy {
            constness: CppConstness::Var
        }
    );
    assert_eq!(fields[6].ty, bool_ty);
    assert!(!fields[8].is_safe);
    // Fields of Rust types are unaffected.
    assert!(ty.fields.is_empty());
}

#[test]
fn cpp_field_without_constness_is_rejected() {
    check_fail(
        r#"
type c++::B {
    field x (type = i32);
}
"#,
        expect![[r#"
            Error: a C++ field accessor says what it returns: `field &const x ...` / `field &var x ...` (a reference to the field, `unique` if Rust uses it uniquely), or `field const x ...` / `field var x ...` (a copy of it)
               ╭─[ test.zng:3:5 ]
               │
             3 │     field x (type = i32);
               │     ──────────┬──────────  
               │               ╰──────────── a C++ field accessor says what it returns: `field &const x ...` / `field &var x ...` (a reference to the field, `unique` if Rust uses it uniquely), or `field const x ...` / `field var x ...` (a copy of it)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_field_unique_without_reference_is_rejected() {
    check_fail(
        r#"
type c++::B {
    field unique var x (type = i32);
}
"#,
        expect![[r#"
            Error: `unique` only applies to references: `field &unique var x ...` returns a unique reference to the field, and `field var x ...` a copy of it
               ╭─[ test.zng:3:5 ]
               │
             3 │     field unique var x (type = i32);
               │     ────────────────┬───────────────  
               │                     ╰───────────────── `unique` only applies to references: `field &unique var x ...` returns a unique reference to the field, and `field var x ...` a copy of it
            ───╯
        "#]],
    );
}

#[test]
fn cpp_field_copy_of_a_class_needs_a_by_value_type() {
    check_fail(
        r#"
type c++::A {}

type c++::B {
    field var a (type = c++::A);
}
"#,
        expect![[r#"
            Error: the accessor `a` returns a copy of the field `a`, but Rust doesn't hold `c++::A` by value (it needs a layout and `#trivially_relocatable`); return a reference instead (`field &const a ...`)
               ╭─[ test.zng:5:5 ]
               │
             5 │     field var a (type = c++::A);
               │     ──────────────┬─────────────  
               │                   ╰─────────────── the accessor `a` returns a copy of the field `a`, but Rust doesn't hold `c++::A` by value (it needs a layout and `#trivially_relocatable`); return a reference instead (`field &const a ...`)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_field_invalid_types_are_rejected() {
    for (decl, expected) in [
        (
            "field &const s (type = ::std::string::String);",
            "the field `s` has the type `::std::string::String`, but C++ fields are C++ objects",
        ),
        (
            "field &const r (type = i32&);",
            "the field `r` has the reference type `i32&`, and fields of reference type aren't supported",
        ),
        (
            "field &const r (type = const c++::B&);",
            "the field `r` has the reference type `const c++::B&`, and fields of reference type aren't supported",
        ),
        (
            "field &const p (type = char*);",
            "`char` can't be a C++ object",
        ),
        (
            "field &const p (type = str*);",
            "C++ pointers and references (`T*`, `T&`) can only point to C++ objects (`c++::` types, builtins and C++ pointers), not `str`",
        ),
        (
            "field &const p (type = crate::Foo*);",
            "C++ pointers and references (`T*`, `T&`) can only point to C++ objects (`c++::` types, builtins and C++ pointers), not `crate::Foo`",
        ),
        (
            "field &const x (type = &'a i32);",
            "lifetimes (`'a`) aren't supported in the signatures of C++ fields yet",
        ),
        (
            "field &const x (type = crate::Ref<'a>);",
            "lifetimes (`'a`) aren't supported in the signatures of C++ fields yet",
        ),
    ] {
        let src = format!(
            "type crate::Foo {{ #layout(size = 4, align = 4); }}\n\
             type<'a> crate::Ref<'a> {{ #layout(size = 8, align = 8); }}\n\
             type c++::B {{\n    {decl}\n}}\n"
        );
        let r = catch_unwind(|| {
            let _ = parse_str(&src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(err.contains(expected), "expected `{expected}`, got:\n{err}");
    }
}

#[test]
fn cpp_field_on_rust_type_is_rejected() {
    check_fail(
        r#"
type crate::Foo {
    #layout(size = 8, align = 4);
    field const x (type = i32);
}
"#,
        expect![[r#"
            Error: C++ field accessors (`field name (type = T)`) can only be declared on `c++::` types
               ╭─[ test.zng:4:5 ]
               │
             4 │     field const x (type = i32);
               │     ─────────────┬─────────────  
               │                  ╰─────────────── C++ field accessors (`field name (type = T)`) can only be declared on `c++::` types
            ───╯
        "#]],
    );
}

#[test]
fn safe_mutable_cpp_ptr_field_warns_unless_unsafe_or_allowed() {
    let mut warnings = vec![];
    let _ = parse_str(
        r#"
type c++::B {
    field &var next (type = c++::B*);
    field &unique var prev (type = i32*);
    field &unique const tail (type = c++::B*);
    // Read-only and copy accessors, and `unsafe field`, do not warn.
    field &const next_const (type = c++::B*);
    field var next_copy (type = c++::B*);
    unsafe field &var next_unsafe (type = c++::B*);
    unsafe field &unique const tail_unsafe (type = c++::B*);
}
"#,
        NullCfg,
        |w| warnings.push(w.to_owned()),
    );
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .all(|w| w.contains("exposes mutable access to a raw pointer")),
        "{warnings:?}"
    );

    let mut allowed_warnings = vec![];
    let _ = parse_str(
        r#"
#allow(safe_raw_pointer)
type c++::B {
    field &var next (type = c++::B*);
}
"#,
        NullCfg,
        |w| allowed_warnings.push(w.to_owned()),
    );
    assert!(allowed_warnings.is_empty(), "{allowed_warnings:?}");
}

#[test]
fn cpp_fields_merge_and_conflict_across_files() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type c++::B {
    field &const x (type = i32);
    field &const y (type = i32);
    safe fn clash<'a>(&'a const self) -> i32;
}
"#,
    )]);

    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
type c++::B {
    field &const x (type = i32);
    field &unique var x (type = i32) as x_mut;
}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    let names: Vec<_> = parsed.spec.types[0]
        .cpp_fields
        .iter()
        .map(|f| f.rust_name())
        .collect();
    assert_eq!(names, ["x", "x_mut", "y"]);

    check_import_fail(
        r#"
merge "./a.zng";
type c++::B {
    field &var y (type = i32);
    field &const z (type = i32) as clash;
}
"#,
        expect![[r#"
            Error: C++ field accessor mismatch
               ╭─[ ./a.zng:2:6 ]
               │
             2 │ type c++::B {
               │      ───┬──  
               │         ╰──── C++ field accessor mismatch
               │ 
             4 │     field &const y (type = i32);
               │     ──────────────┬─────────────  
               │                   ╰─────────────── declaration here ...
               │
               ├─[ test.zng:4:5 ]
               │
             4 │     field &var y (type = i32);
               │     ─────────────┬────────────  
               │                  ╰────────────── conflicts with the declaration here.
            ───╯
            Error: `clash` is already the Rust name of another member of this type (a constructor, member function or field accessor)
               ╭─[ test.zng:5:5 ]
               │
             5 │     field &const z (type = i32) as clash;
               │     ──────────────────┬──────────────────  
               │                       ╰──────────────────── `clash` is already the Rust name of another member of this type (a constructor, member function or field accessor)
               │
               ├─[ ./a.zng:5:5 ]
               │
             5 │     safe fn clash<'a>(&'a const self) -> i32;
               │     ────────────────────┬────────────────────  
               │                         ╰────────────────────── also declared here
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn template_types_substitute_and_check_cpp_fields() {
    use zngur_def::{CppIndirection, Mutability};

    let parsed = parse_str(
        r#"
#unstable(template_types)
type<T> T {
    unsafe field &var next (type = T*) as next_var;
}
type c++::Node {}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let node = &parsed.spec.types[0];
    assert_eq!(node.cpp_fields.len(), 1);
    assert_eq!(
        node.cpp_fields[0].ty,
        RustType::cpp_ptr(
            Mutability::Mut,
            CppIndirection::Pointer,
            RustType::Cpp(vec!["Node".to_owned()].into())
        )
    );

    for (inst, expected) in [
        (
            "char",
            "the field `v` has the type `char`, but C++ fields are C++ objects",
        ),
        (
            "::std::string::String",
            "the field `v` has the type `::std::string::String`, but C++ fields are C++ objects",
        ),
    ] {
        let src = format!(
            "#unstable(template_types)\n\
             type<T> crate::Box<T> {{ #layout(size = 8, align = 8); field &const v (type = T); }}\n\
             type crate::Box<{inst}> {{}}\n"
        );
        let r = catch_unwind(|| {
            let _ = parse_str(&src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(err.contains(expected), "expected `{expected}`, got:\n{err}");
        assert!(err.contains("in template instantiated here"), "got:\n{err}");
    }
}

#[test]
fn cpp_names_can_be_zngur_keywords() {
    let parsed = parse_str(
        r#"
type c++::a::B {
    field &const type (type = i32) as type_;
    unsafe fn match<'a>(&'a const self, i32) -> i32 as match_;
}

mod c++::a {
    safe fn use(i32) as use_;
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let ty = &parsed.spec.types[0];
    assert_eq!(ty.cpp_fields[0].name, "type");
    assert_eq!(ty.cpp_fields[0].rust_name(), "type_");
    assert_eq!(ty.cpp_methods[0].cpp_name, "match");
    assert_eq!(ty.cpp_methods[0].rust_name(), "match_");
    assert_eq!(parsed.spec.cpp_funcs[0].cpp_name, "use");
    assert_eq!(parsed.spec.cpp_funcs[0].rust_name(), "use_");

    check_fail(
        r#"
type c++::a::B {
    field &const type (type = i32);
    field &const x (type = i32) as match;
    field &const as_const (type = i32);
}
"#,
        expect![[r#"
            Error: `type` is a Rust keyword; give this C++ field accessor a non-keyword Rust name with `as <rust_name>`
               ╭─[ test.zng:3:5 ]
               │
             3 │     field &const type (type = i32);
               │     ───────────────┬───────────────  
               │                    ╰───────────────── `type` is a Rust keyword; give this C++ field accessor a non-keyword Rust name with `as <rust_name>`
            ───╯
            Error: `match` is a Rust keyword and cannot be used as the Rust function name in `as match`
               ╭─[ test.zng:4:5 ]
               │
             4 │     field &const x (type = i32) as match;
               │     ──────────────────┬──────────────────  
               │                       ╰──────────────────── `match` is a Rust keyword and cannot be used as the Rust function name in `as match`
            ───╯
            Error: `as_const` collides with an inherent conversion method on `c++::` wrappers; rename it with `as <rust_name>`
               ╭─[ test.zng:5:5 ]
               │
             5 │     field &const as_const (type = i32);
               │     ─────────────────┬─────────────────  
               │                      ╰─────────────────── `as_const` collides with an inherent conversion method on `c++::` wrappers; rename it with `as <rust_name>`
            ───╯
        "#]],
    );
}

#[test]
fn cpp_field_copy_with_imported_cpp_types() {
    let resolver = MockFilesystem::new(vec![(
        "./base.zng",
        r#"
#crate base_bindings
type c++::base::ByVal {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
type c++::base::RefOnly {}
"#,
    )]);

    let parsed = parse_str_with_resolver(
        r#"
import "./base.zng";
type c++::Owner {
    field var val (type = c++::base::ByVal);
    field const val_const (type = c++::base::ByVal);
    field &const ref_only (type = c++::base::RefOnly);
}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.types[0].cpp_fields.len(), 3);

    check_import_fail(
        r#"
import "./base.zng";
type c++::Owner {
    field var ref_only (type = c++::base::RefOnly);
}
"#,
        expect![[r#"
            Error: the accessor `ref_only` returns a copy of the field `ref_only`, but Rust doesn't hold `c++::base::RefOnly` by value (it needs a layout and `#trivially_relocatable`); return a reference instead (`field &const ref_only ...`)
               ╭─[ test.zng:4:5 ]
               │
             4 │     field var ref_only (type = c++::base::RefOnly);
               │     ───────────────────────┬───────────────────────  
               │                            ╰───────────────────────── the accessor `ref_only` returns a copy of the field `ref_only`, but Rust doesn't hold `c++::base::RefOnly` by value (it needs a layout and `#trivially_relocatable`); return a reference instead (`field &const ref_only ...`)
            ───╯
        "#]],
        &resolver,
    );

    // Incomplete imports (e.g. an imported module whose own `import`s aren't resolved when parsed
    // in isolation, or a file with an earlier import/merge error) do not falsely reject an
    // imported `field var` type as non-by-value.
    let transitive_resolver = MockFilesystem::new(vec![
        (
            "./base.zng",
            r#"
#crate base_bindings
type c++::base::ByVal {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
"#,
        ),
        (
            "./mid.zng",
            r#"
#crate mid_bindings
import "./base.zng";
type c++::mid::MidOwner {
    field var by_val (type = c++::base::ByVal);
}
"#,
        ),
    ]);
    let via_mid = parse_str_with_resolver(
        r#"
import "./mid.zng";
type c++::Top {
    field &const m (type = c++::mid::MidOwner);
}
"#,
        NullCfg,
        &transitive_resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(via_mid.spec.types[0].cpp_fields.len(), 1);

    // Undeclared `field var` types emit a single not-declared error rather than a duplicate
    // not-by-value error.
    check_fail(
        r#"
type c++::Owner {
    field var u (type = c++::Unknown);
}
"#,
        expect![[r#"
            Error: `c++::Unknown` is not declared or imported
               ╭─[ test.zng:3:5 ]
               │
             3 │     field var u (type = c++::Unknown);
               │     ─────────────────┬────────────────  
               │                      ╰────────────────── `c++::Unknown` is not declared or imported
            ───╯
        "#]],
    );
}

#[test]
fn rust_field_syntax_on_cpp_type_is_rejected() {
    check_fail(
        r#"
type c++::Point {
    field x (offset = 0, type = i32);
    field y (offset = auto, type = i32);
}
"#,
        expect![[r#"
            Error: C++ field accessors omit `offset` and say what they return (e.g. `field &const x (type = ...);` or `field var x (type = ...);`)
               ╭─[ test.zng:3:5 ]
               │
             3 │     field x (offset = 0, type = i32);
               │     ────────────────┬────────────────  
               │                     ╰────────────────── C++ field accessors omit `offset` and say what they return (e.g. `field &const x (type = ...);` or `field var x (type = ...);`)
            ───╯
            Error: C++ field accessors omit `offset` and say what they return (e.g. `field &const y (type = ...);` or `field var y (type = ...);`)
               ╭─[ test.zng:4:5 ]
               │
             4 │     field y (offset = auto, type = i32);
               │     ──────────────────┬─────────────────  
               │                       ╰─────────────────── C++ field accessors omit `offset` and say what they return (e.g. `field &const y (type = ...);` or `field var y (type = ...);`)
            ───╯
        "#]],
    );

    let r = catch_unwind(|| {
        let _ = parse_str(
            r#"
#unstable(template_types)
type<T> T {
    field x (offset = 0, type = i32);
}
type c++::Point {}
"#,
            NullCfg,
            |_| {},
        );
    });
    let err = r
        .unwrap_err()
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("C++ field accessors omit `offset` and say what they return"),
        "unexpected error:\n{err}"
    );
}

#[test]
fn cpp_template_instantiations_parse() {
    let parsed = parse_str(
        r#"
type c++::a::B {
}

type c++::std::unique_ptr<c++::a::B> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
}

type c++::std::map<i32, const c++::a::B*> {
}
"#,
        NullCfg,
        |_| {},
    );
    let b = RustType::Cpp(vec!["a".to_owned(), "B".to_owned()].into());
    let types = &parsed.spec.types;
    assert_eq!(
        types[1].ty,
        RustType::Cpp(CppTypePath {
            path: vec!["std".to_owned(), "unique_ptr".to_owned()],
            generics: vec![b.clone()],
        })
    );
    // The C++ name includes the template arguments, spelled the C++ way.
    assert_eq!(types[1].cpp_name().unwrap(), "::std::unique_ptr< ::a::B >");
    assert_eq!(
        types[2].ty,
        RustType::Cpp(CppTypePath {
            path: vec!["std".to_owned(), "map".to_owned()],
            generics: vec![
                RustType::Primitive(PrimitiveRustType::Int(32)),
                RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, b),
            ],
        })
    );
    assert_eq!(
        types[2].cpp_name().unwrap(),
        "::std::map< ::int32_t, const ::a::B* >"
    );
}

#[test]
fn cpp_template_with_named_generics_is_rejected() {
    check_fail(
        r#"
type c++::std::vector<T = i32> {
}
"#,
        expect![[r#"
            Error: C++ template arguments are positional; `Name = T` is only for Rust types
               ╭─[ test.zng:2:6 ]
               │
             2 │ type c++::std::vector<T = i32> {
               │      ────────────┬────────────  
               │                  ╰────────────── C++ template arguments are positional; `Name = T` is only for Rust types
            ───╯
        "#]],
    );
}

#[test]
fn cpp_template_with_rust_type_argument_is_rejected() {
    check_fail(
        r#"
type c++::std::vector<::std::string::String> {
}
"#,
        expect![[r#"
            Error: `::std::string::String` can't be a C++ template argument (only `c++::` types, builtins and C++ pointers and references can, `const` or not)
               ╭─[ test.zng:2:6 ]
               │
             2 │ type c++::std::vector<::std::string::String> {
               │      ───────────────────┬───────────────────  
               │                         ╰───────────────────── `::std::string::String` can't be a C++ template argument (only `c++::` types, builtins and C++ pointers and references can, `const` or not)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_template_with_unsupported_rust_primitive_is_rejected() {
    for (prim, expected) in [
        ("i128", "`i128` has no C++ equivalent"),
        ("u128", "`u128` has no C++ equivalent"),
        ("char", "`char` has no C++ equivalent"),
        ("str", "`str` has no C++ equivalent"),
        ("f16", "`f16` has no C++ equivalent"),
        ("isize", "`::isize` can't be a C++ template argument"),
    ] {
        for zng in [
            format!("type c++::Foo<{prim}> {{}}\n"),
            format!(
                "#unstable(template_types)\ntype<T> c++::Foo<T> {{}}\ntype c++::Foo<{prim}> {{}}\n"
            ),
        ] {
            let errs = match catch_unwind(|| {
                let _ = parse_str(&zng, NullCfg, |_| {});
            }) {
                Ok(_) => panic!("expected `{prim}` to fail parsing in:\n{zng}"),
                Err(e) => e.downcast::<ErrorText>().unwrap().0,
            };
            assert!(
                errs.contains(expected),
                "expected `{expected}` for `{prim}` in C++ class template args, got:\n{errs}"
            );
        }
    }
}

#[test]
fn cpp_template_with_different_arities_is_rejected() {
    check_fail(
        r#"
type c++::std::vector<i32> {
}

type c++::std::vector<i32, c++::MyAlloc> {
}
"#,
        expect![[r#"
            Error: every instantiation of `c++::std::vector` must have the same number of template arguments as its template and its other instantiations, which have 1
               ╭─[ test.zng:5:6 ]
               │
             5 │ type c++::std::vector<i32, c++::MyAlloc> {
               │      ─────────────────┬─────────────────  
               │                       ╰─────────────────── every instantiation of `c++::std::vector` must have the same number of template arguments as its template and its other instantiations, which have 1
            ───╯
        "#]],
    );
}

#[test]
fn cpp_template_instantiations_of_a_pointer_and_a_reference_are_the_same_rust_type() {
    // Both are `*const i32` in Rust.
    check_fail(
        r#"
type c++::std::pair<const i32&> {
}

type c++::std::pair<const i32*> {
}
"#,
        expect![[r#"
            Error: another instantiation of this C++ class template is the same Rust type
               ╭─[ test.zng:5:6 ]
               │
             5 │ type c++::std::pair<const i32*> {
               │      ─────────────┬────────────  
               │                   ╰────────────── another instantiation of this C++ class template is the same Rust type
            ───╯
        "#]],
    );
}

#[test]
fn cpp_template_implicitly_homed_in_an_import_cant_be_instantiated() {
    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        r#"
#crate bar_bindings

type c++::std::unique_ptr<i32> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
}
"#,
    )]);
    check_import_fail(
        r#"
import "./bar.zng";

type c++::std::unique_ptr<i64> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
}
"#,
        expect![[r#"
            Error: `c++::std::unique_ptr` is also instantiated in imported module `./bar.zng`; to share a C++ class template between modules, declare `type<T> c++::std::unique_ptr<...>` in one of them (its home)
               ╭─[ test.zng:4:6 ]
               │
             4 │ type c++::std::unique_ptr<i64> {
               │      ────────────┬────────────  
               │                  ╰────────────── `c++::std::unique_ptr` is also instantiated in imported module `./bar.zng`; to share a C++ class template between modules, declare `type<T> c++::std::unique_ptr<...>` in one of them (its home)
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn cpp_template_scope_resolution_and_validation() {
    // `const` C++ template arguments work inside `mod c++::...` and via `use c++::...`.
    let parsed = parse_str(
        r#"
use c++::std::span as Span;

mod c++::views {
    type View<const i32> {}
}

type Span<const i32> {}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(
        parsed.spec.types[0].cpp_name().as_deref(),
        Some("::views::View< const ::int32_t >")
    );
    assert_eq!(
        parsed.spec.types[1].cpp_name().as_deref(),
        Some("::std::span< const ::int32_t >")
    );

    // Named generics, lifetime arguments, and lifetime parameters are rejected on C++ types even
    // inside `mod c++::...` or via `use c++::...`, and `const T` is still rejected on aliased Rust
    // types.
    for (src, expected) in [
        (
            "mod c++::std { type vector<T = i32> {} }\n",
            "C++ template arguments are positional; `Name = T` is only for Rust types",
        ),
        (
            "mod c++::std { type vector<'static, i32> {} }\n",
            "`c++::` types don't take lifetime arguments",
        ),
        (
            "mod c++::std { type<'a> vector<i32> {} }\n",
            "`c++::` types don't have lifetime parameters",
        ),
        (
            "use c++::std::vector as Vec;\ntype Vec<T = i32> {}\n",
            "C++ template arguments are positional; `Name = T` is only for Rust types",
        ),
        (
            "use ::std::vec::Vec as RustVec;\ntype RustVec<const i32> { #layout(size = 24, align = 8); }\n",
            "`const T` can only be the type of a C++ pointer or reference (`const T&`, `const T*`, `&'a const T`) or a C++ template argument",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn cpp_template_arity_zero_and_import_collisions() {
    for src in [
        "type c++::a::Foo {}\ntype c++::a::Foo<i32> {}\n",
        "type c++::a::Foo<i32> {}\ntype c++::a::Foo {}\n",
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains("cannot be declared both as a plain C++ type and as a C++ class template"),
            "unexpected error for `{src}`:\n{err}"
        );
    }

    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        "#crate bar_bindings\ntype c++::a::Foo<i32> {}\n",
    )]);
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            "import \"./bar.zng\";\ntype c++::a::Foo {}\n",
            NullCfg,
            &resolver,
            |_| {},
        );
    });
    let err = r
        .unwrap_err()
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("`c++::a::Foo` is already imported from `./bar.zng`"),
        "unexpected error:\n{err}"
    );
}

#[test]
fn cpp_template_type_definitions_with_const_and_pointer_args() {
    use zngur_def::CppConstness;

    let parsed = parse_str(
        r#"
type<T> c++::Pair<T> {
    #layout(size = 16, align = 8);
    #trivially_relocatable;

    field &const first (type = T);
    field const first (type = T) as first_copy;
    unsafe fn get<'a>(&'a const self) -> &'a var T;
}

type c++::Pair<const i32> {}
type c++::Pair<const c++::Node*> {}
type c++::Node {}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let const_pair = &parsed.spec.types[0];
    assert_eq!(
        const_pair.cpp_fields[0].ty,
        RustType::Primitive(PrimitiveRustType::Int(32))
    );
    assert_eq!(
        const_pair.cpp_methods[0].output,
        RustType::Ref(
            Mutability::Not,
            Some(Lifetime("a".to_owned())),
            Box::new(RustType::cpp_object(
                CppConstness::Const,
                RustType::Primitive(PrimitiveRustType::Int(32))
            ))
        )
    );

    // Mutable field accessors and by-value `const T` methods/constructors on `const` template
    // instantiations are rejected cleanly.
    for (member, expected) in [
        (
            "field &unique var first (type = T);",
            "the field `first` has `const` type `const i32`, so its reference accessor must be `const`",
        ),
        (
            "field &unique const first (type = T);",
            "the field `first` has `const` type `const i32`, so `field &unique const` cannot be used",
        ),
        (
            "safe fn take(&const self) -> T;",
            "`const T` can only be the type of a C++ pointer or reference (`const T&`, `const T*`, `&'a const T`) or a C++ template argument",
        ),
    ] {
        let src = format!(
            "type<T> c++::Box<T> {{ #layout(size = 4, align = 4); #trivially_relocatable; {member} }}\n\
             type c++::Box<const i32> {{}}\n"
        );
        let r = catch_unwind(|| {
            let _ = parse_str(&src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(err.contains(expected), "expected `{expected}`, got:\n{err}");
    }
}

#[test]
fn cpp_template_signature_and_nested_uses_are_validated() {
    for (src, expected) in [
        (
            "type c++::Plain {}\ntype c++::Outer<c++::Plain<i32>> {}\n",
            "`c++::Plain` is declared as a plain C++ type, so it cannot take template arguments",
        ),
        (
            "type c++::Plain {}\nfn f(&const c++::Plain<i32>);\n",
            "`c++::Plain` is declared as a plain C++ type, so it cannot take template arguments",
        ),
        (
            "type c++::Foo<i32> {}\nfn f(&const c++::Foo<i32, u32>);\n",
            "`c++::Foo` takes 1 template argument, not 2",
        ),
        (
            "type c++::Foo<i32> {}\nmod c++::ns { safe fn f<'a>(&'a const c++::Foo); }\n",
            "`c++::Foo` is a C++ class template and needs 1 template argument",
        ),
        (
            "type c++::Foo<i32> {}\nextern \"C++\" { safe fn f<'a>(&'a const c++::Foo<i64>); }\n",
            "instantiation `c++::Foo<i64>` of C++ class template `c++::Foo` is not declared; add `type c++::Foo<i64> { ... }`",
        ),
        (
            "fn f(&const c++::Unknown<i32>);\n",
            "instantiation `c++::Unknown<i32>` of C++ class template `c++::Unknown` is not declared; add `type c++::Unknown<i32> { ... }`",
        ),
        (
            "type c++::std::unique_ptr<c++::Undeclared> {}\n",
            "`c++::Undeclared` is not declared or imported",
        ),
        (
            "type c++::View<const c++::Undeclared*> {}\n",
            "`c++::Undeclared` is not declared or imported",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn cpp_template_imported_instantiation_uses_and_errors() {
    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        r#"
#crate bar_bindings

type c++::Plain {
    #layout(size = 4, align = 4);
    #trivially_relocatable;
}

type c++::Box<i32> {
    #layout(size = 4, align = 4);
    #trivially_relocatable;
}
"#,
    )]);

    // Declared instantiations of an imported template can be used both by reference and by value.
    let parsed = parse_str_with_resolver(
        r#"
import "./bar.zng";

mod c++::ns {
    safe fn borrow<'a>(&'a const c++::Box<i32>) -> c++::Box<i32>;
}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.cpp_funcs.len(), 1);

    // Undeclared instantiations, wrong arity, missing template arguments, and template arguments
    // on imported plain C++ types are all rejected cleanly without secondary by-value noise.
    for (src, expected, unexpected) in [
        (
            "import \"./bar.zng\";\nmod c++::ns { safe fn f<'a>(&'a const c++::Box<i64>); }\n",
            "instantiation `c++::Box<i64>` of C++ class template `c++::Box` is not declared; add `type c++::Box<i64> { ... }`",
            Some("cannot be passed or returned by value"),
        ),
        (
            "import \"./bar.zng\";\nmod c++::ns { safe fn f() -> c++::Box<i64>; }\n",
            "instantiation `c++::Box<i64>` of C++ class template `c++::Box` is not declared; add `type c++::Box<i64> { ... }`",
            Some("cannot be passed or returned by value"),
        ),
        (
            "import \"./bar.zng\";\nfn f(&const c++::Box);\n",
            "`c++::Box` is a C++ class template and needs 1 template argument",
            None,
        ),
        (
            "import \"./bar.zng\";\nfn f(&const c++::Box<i32, u32>);\n",
            "`c++::Box` takes 1 template argument, not 2",
            None,
        ),
        (
            "import \"./bar.zng\";\nfn f(&const c++::Plain<i32>);\n",
            "`c++::Plain` is declared as a plain C++ type, so it cannot take template arguments",
            None,
        ),
        (
            "import \"./bar.zng\";\ntype c++::Plain<i32> {}\n",
            "`c++::Plain` is already imported as a plain C++ type from `./bar.zng`",
            None,
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str_with_resolver(src, NullCfg, &resolver, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
        if let Some(unexpected) = unexpected {
            assert!(
                !err.contains(unexpected),
                "did not expect `{unexpected}` for `{src}`, got:\n{err}"
            );
        }
    }
}

#[test]
fn cpp_template_merge_const_field_and_empty_angles_and_lifetimes() {
    // Merging a `type<T>` field `T` with an explicit `i32` field on a `const i32` instantiation
    // succeeds when the field accessor is `const`, while `&unique var` remains rejected.
    let must_parse = |src: &str| {
        catch_unwind(|| parse_str(src, NullCfg, |w| panic!("unexpected warning: {w}")))
            .unwrap_or_else(|e| {
                if let Ok(et) = e.downcast::<ErrorText>() {
                    panic!("unexpected ErrorText for:\n{src}\n---\n{}", et.0);
                }
                panic!("unexpected panic for:\n{src}");
            })
    };
    let parsed = must_parse(
        r#"
#unstable(template_types)

type<T> c++::Pair<T> {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    field &const first (type = T);
}

type c++::Pair<const i32> {
    field &const first (type = i32);
}
"#,
    );
    assert_eq!(parsed.spec.types.len(), 1);
    assert_eq!(parsed.spec.types[0].cpp_fields.len(), 1);

    let r = catch_unwind(|| {
        let _ = parse_str(
            r#"
#unstable(template_types)

type<T> c++::Pair<T> {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    field &unique var first (type = T);
}

type c++::Pair<const i32> {
    field &unique var first (type = i32);
}
"#,
            NullCfg,
            |_| {},
        );
    });
    let err = r
        .unwrap_err()
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains(
            "the field `first` has `const` type `const i32`, so its reference accessor must be `const`"
        ),
        "unexpected error:\n{err}"
    );

    // Empty `<>` on `c++::` paths is rejected rather than collapsing into a plain C++ type or a
    // `type<T>` variable.
    for src in [
        "type c++::Box<> {}\n",
        "fn f(&const c++::Box<>);\n",
        "#unstable(template_types)\nmod c++::ns { type<T> Holder<T> { unsafe fn f<'a>(&'a const self, &'a const T<>); } }\n",
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(
                "C++ class template instantiations in `.zng` must specify at least one template argument"
            ),
            "unexpected error for `{src}`:\n{err}"
        );
    }
    // And in Rust scope, `T<>` does not match the `type<T>` variable `T` either (resolving to
    // `crate::T` rather than substituting `i32`).
    let parsed_t_empty = must_parse(
        r#"
#unstable(template_types)
type<T> crate::Holder<T> { field x (offset = 0, type = T<>); }
type crate::Holder<i32> { #layout(size = 4, align = 4); }
"#,
    );
    assert_ne!(
        parsed_t_empty.spec.types[0].fields[0].ty,
        RustType::Primitive(PrimitiveRustType::Int(32))
    );

    // `RustType::Cpp` generic arguments participate in `all_lifetimes` and `for<'a>` binder renaming.
    let parsed_fn = must_parse("fn call(Box<dyn for<'a> Fn(&'a i32)>);\n");
    let fn_box_ty = parsed_fn.spec.funcs[0].inputs[0].clone();
    let cpp_with_binder = RustType::Cpp(zngur_def::CppTypePath {
        path: vec!["Box".to_owned()],
        generics: vec![fn_box_ty],
    });
    let cpp_with_lt = RustType::Cpp(zngur_def::CppTypePath {
        path: vec!["Ref".to_owned()],
        generics: vec![RustType::Ref(
            Mutability::Not,
            Some(Lifetime("a".to_owned())),
            Box::new(RustType::Primitive(PrimitiveRustType::Int(32))),
        )],
    });
    assert!(
        cpp_with_lt
            .all_lifetimes()
            .contains(&Lifetime("a".to_owned()))
    );
    let outer = std::collections::HashSet::from([Lifetime("a".to_owned())]);
    let mut avoid = outer.clone();
    let renamed = cpp_with_binder.rename_colliding_for_binders(&outer, &mut avoid);
    assert_ne!(renamed, cpp_with_binder);
    assert!(avoid.len() > 1);
}

#[test]
fn cpp_template_post_substitution_lints_and_scoped_allow() {
    use std::cell::RefCell;

    let collect_warnings = |src: &str, resolver: &MockFilesystem| -> Vec<String> {
        let warnings = RefCell::new(Vec::new());
        let res = parse_str_with_resolver(src, NullCfg, resolver, |w| {
            warnings.borrow_mut().push(w.to_owned());
        });
        assert_eq!(res.errors, 0);
        warnings.into_inner()
    };

    let empty_fs = MockFilesystem::new(Vec::<(&str, &str)>::new());

    // Substituting `i32*` into `T` triggers `safe_raw_pointer` on `safe fn`, `safe constructor`,
    // and mutable-ref `field`, whereas instantiating with `i32` produces zero warnings.
    let warnings = collect_warnings(
        r#"
#unstable(template_types)

type<T> c++::Box<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;

    safe constructor(T) as new;
    safe fn set<'a>(&'a unique var self, T);
    field &unique var val (type = T);
}

type c++::Box<i32> {}
type c++::Box<i32*> {}
"#,
        &empty_fs,
    );
    assert_eq!(warnings.len(), 3, "unexpected warnings: {warnings:#?}");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("member function") && w.contains("raw pointer"))
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("constructor") && w.contains("raw pointer"))
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("field accessor") && w.contains("raw pointer"))
    );

    // Templates that already wrote `T*` warn once at parse time rather than duplicating per
    // instantiation.
    let pre_existing_ptr_warnings = collect_warnings(
        r#"
#unstable(template_types)

type<T> c++::PtrBox<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;

    safe fn take_ptr<'a>(&'a const self, T*);
}

type c++::PtrBox<i32> {}
type c++::PtrBox<u32> {}
"#,
        &empty_fs,
    );
    assert_eq!(
        pre_existing_ptr_warnings.len(),
        1,
        "expected exactly 1 warning, got: {pre_existing_ptr_warnings:#?}"
    );

    // `#allow(safe_raw_pointer)` in either the template's file or the instantiation's file
    // silences post-substitution warnings, while `#allow` in an unrelated `merge`d file does not.
    let tpl_src = r#"
#unstable(template_types)

type<T> c++::Box<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;

    safe fn set<'a>(&'a unique var self, T);
}
"#;
    let fs_unrelated_allow = MockFilesystem::new(vec![
        ("./tpl.zng", tpl_src),
        (
            "./unrelated.zng",
            "#allow(safe_raw_pointer)\ntype c++::Other {}\n",
        ),
    ]);
    let warnings_unrelated = collect_warnings(
        "merge \"./tpl.zng\";\nmerge \"./unrelated.zng\";\ntype c++::Box<i32*> {}\n",
        &fs_unrelated_allow,
    );
    assert_eq!(warnings_unrelated.len(), 1);

    let warnings_inst_allow = collect_warnings(
        "#allow(safe_raw_pointer)\nmerge \"./tpl.zng\";\ntype c++::Box<i32*> {}\n",
        &fs_unrelated_allow,
    );
    assert!(warnings_inst_allow.is_empty());

    let fs_tpl_allow = MockFilesystem::new(vec![(
        "./tpl.zng",
        &format!("#allow(safe_raw_pointer)\n{tpl_src}"),
    )]);
    let warnings_tpl_allow = collect_warnings(
        "merge \"./tpl.zng\";\ntype c++::Box<i32*> {}\n",
        &fs_tpl_allow,
    );
    assert!(warnings_tpl_allow.is_empty());
}

fn unique_ptr_home() -> CppTemplateHome {
    CppTemplateHome {
        path: vec!["std".to_owned(), "unique_ptr".to_owned()],
        arity: 1,
        explicit: true,
    }
}

const BAR_UNIQUE_PTR_HOME: &str = r#"
#crate bar_bindings

type<T> c++::std::unique_ptr<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
    unsafe fn get(&const self) -> T*;
}
"#;

#[test]
fn cpp_template_declares_a_local_home_with_defaults() {
    let parsed = parse_str(
        r#"
type<T> c++::std::unique_ptr<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
    unsafe fn get(&const self) -> T*;
}

type c++::std::unique_ptr<i32> {
}
"#,
        NullCfg,
        |_| {},
    );
    assert_eq!(parsed.spec.cpp_template_homes, vec![unique_ptr_home()]);
    assert_eq!(parsed.spec.cpp_templates.len(), 1);
    let [instantiation] = parsed.spec.types.as_slice() else {
        panic!("expected one type");
    };
    assert_eq!(
        instantiation.trivially_relocatable,
        TriviallyRelocatable::Checked
    );
    assert_eq!(instantiation.cpp_methods.len(), 1);
    assert_eq!(
        instantiation.cpp_methods[0].output,
        RustType::cpp_ptr(
            Mutability::Mut,
            CppIndirection::Pointer,
            RustType::Primitive(PrimitiveRustType::Int(32))
        )
    );
}

#[test]
fn cpp_template_without_a_visible_home_is_homed_implicitly() {
    let parsed = parse_str(
        r#"
type c++::std::unique_ptr<i32> {
}
"#,
        NullCfg,
        |_| {},
    );
    assert_eq!(
        parsed.spec.cpp_template_homes,
        vec![CppTemplateHome {
            explicit: false,
            ..unique_ptr_home()
        }]
    );
}

#[test]
fn imported_cpp_template_home_applies_its_defaults() {
    let resolver = MockFilesystem::new(vec![("./bar.zng", BAR_UNIQUE_PTR_HOME)]);
    let parsed = parse_str_with_resolver(
        r#"
import "./bar.zng";

type c++::a::B {
}

type c++::std::unique_ptr<c++::a::B> {
}
"#,
        NullCfg,
        &resolver,
        |_| {},
    );
    let [module] = parsed.spec.imported_modules.as_slice() else {
        panic!("expected one imported module");
    };
    assert_eq!(module.cpp_template_homes, vec![unique_ptr_home()]);
    // Neither the template nor its home are this module's.
    assert!(parsed.spec.cpp_template_homes.is_empty());
    assert!(parsed.spec.cpp_templates.is_empty());
    let instantiation = &parsed.spec.types[1];
    assert_eq!(
        instantiation.trivially_relocatable,
        TriviallyRelocatable::Checked
    );
    assert_eq!(instantiation.cpp_methods.len(), 1);
    assert_eq!(
        instantiation.cpp_methods[0].output,
        RustType::cpp_ptr(
            Mutability::Mut,
            CppIndirection::Pointer,
            parsed.spec.types[0].ty.clone()
        )
    );
}

#[test]
fn cpp_template_instantiation_without_a_local_argument_is_rejected() {
    let resolver = MockFilesystem::new(vec![("./bar.zng", BAR_UNIQUE_PTR_HOME)]);
    check_import_fail(
        r#"
import "./bar.zng";

type i32 {
    #layout(size = 4, align = 4);
}

type c++::std::unique_ptr<i32> {
}
"#,
        expect![[r#"
            Error: this instantiation of `c++::std::unique_ptr`, whose home is imported module `./bar.zng`, can only be declared there or in a module that declares one of its template arguments (like Rust's orphan rule)
               ╭─[ test.zng:8:6 ]
               │
             8 │ type c++::std::unique_ptr<i32> {
               │      ────────────┬────────────  
               │                  ╰────────────── this instantiation of `c++::std::unique_ptr`, whose home is imported module `./bar.zng`, can only be declared there or in a module that declares one of its template arguments (like Rust's orphan rule)
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn cpp_template_with_an_imported_home_cant_be_declared_again() {
    let resolver = MockFilesystem::new(vec![("./bar.zng", BAR_UNIQUE_PTR_HOME)]);
    check_import_fail(
        r#"
import "./bar.zng";

type<T> c++::std::unique_ptr<T> {
}
"#,
        expect![[r#"
            Error: `c++::std::unique_ptr` already has a home: imported module `./bar.zng`
               ╭─[ test.zng:4:9 ]
               │
             4 │ type<T> c++::std::unique_ptr<T> {
               │         ───────────┬───────────  
               │                    ╰───────────── `c++::std::unique_ptr` already has a home: imported module `./bar.zng`
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn cpp_template_with_two_imported_homes_is_rejected() {
    let resolver = MockFilesystem::new(vec![
        ("./bar.zng", BAR_UNIQUE_PTR_HOME.to_owned()),
        (
            "./baz.zng",
            BAR_UNIQUE_PTR_HOME.replace("bar_bindings", "baz_bindings"),
        ),
    ]);
    check_import_fail(
        r#"
import "./bar.zng";
import "./baz.zng";
"#,
        expect![[r#"
            Error: imported modules `./bar.zng` and `./baz.zng` are both the home of `c++::std::unique_ptr`; a C++ class template can only have one home (declare `type<T> c++::std::unique_ptr<...>` in a module that both import)
               ╭─[ test.zng:3:1 ]
               │
             3 │ import "./baz.zng";
               │ ─────────┬─────────  
               │          ╰─────────── imported modules `./bar.zng` and `./baz.zng` are both the home of `c++::std::unique_ptr`; a C++ class template can only have one home (declare `type<T> c++::std::unique_ptr<...>` in a module that both import)
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn cpp_template_multi_param_and_const_local_arguments() {
    let resolver = MockFilesystem::new(vec![(
        "./std_homes.zng",
        r#"
#crate std_homes

type<K, V> c++::std::map<K, V> {
    #layout(size = 48, align = 8);
    #trivially_relocatable;
}

type<T> c++::std::span<T> {
    #layout(size = 16, align = 8);
    #trivially_relocatable;
}

type<T> c++::std::unique_ptr<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
}

type<T> c++::std::vector<T> {
    #layout(size = 24, align = 8);
    #trivially_relocatable;
}
"#,
    )]);

    // Multi-parameter template with local type in non-first position and `const`-qualified local
    // type argument both satisfy Rust's orphan rule.
    let parsed = parse_str_with_resolver(
        r#"
import "./std_homes.zng";

type c++::b::Val {}
type c++::std::map<i32, c++::b::Val> {}
type c++::std::span<const c++::b::Val> {}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.types.len(), 3);
    assert_eq!(
        parsed.spec.types[1].trivially_relocatable,
        TriviallyRelocatable::Checked
    );
    assert_eq!(
        parsed.spec.types[2].trivially_relocatable,
        TriviallyRelocatable::Checked
    );

    // Builtins (including `const i32`), pointers/references to local types, and nested template
    // instantiations (`<... as Trait>::Type` projections) are rejected outside the home.
    for bad_instantiation in [
        "type c++::std::span<const i32> {}",
        "type c++::std::span<c++::b::Val*> {}",
        "type c++::std::span<const c++::b::Val&> {}",
        "type c++::std::unique_ptr<c++::b::Val> {}\ntype c++::std::vector<c++::std::unique_ptr<c++::b::Val>> {}",
    ] {
        let src =
            format!("import \"./std_homes.zng\";\ntype c++::b::Val {{}}\n{bad_instantiation}\n");
        let r = catch_unwind(|| {
            let _ = parse_str_with_resolver(&src, NullCfg, &resolver, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains("can only be declared there or in a module that declares one of its template arguments"),
            "unexpected error for `{bad_instantiation}`:\n{err}"
        );
    }
}

#[test]
fn cpp_template_transitive_dag_and_cycle_detection() {
    // Diamond + depth-3 DAG: `test.zng` -> `m2.zng` -> `{m0.zng, m1.zng}` with `m1.zng` -> `m0.zng`.
    // `m0` is the home of `unique_ptr`; `m1` instantiates `unique_ptr<m1::A>` with a constructor;
    // `m2` passes `unique_ptr<m1::A>` by value; `test.zng` imports `m2` without directly importing
    // `m0` or `m1`.
    let resolver = MockFilesystem::new(vec![
        (
            "./m0.zng",
            r#"
#crate m0_bindings

type<T> c++::std::unique_ptr<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
    unsafe fn get<'a>(&'a const self) -> T*;
}
"#,
        ),
        (
            "./m1.zng",
            r#"
#crate m1_bindings
import "./m0.zng";

type c++::m1::A {}

type c++::std::unique_ptr<c++::m1::A> {
    safe constructor() as empty;
}
"#,
        ),
        (
            "./m2.zng",
            r#"
#crate m2_bindings
import "./m0.zng";
import "./m1.zng";

extern "C++" {
    safe fn consume(c++::std::unique_ptr<c++::m1::A>);
}
"#,
        ),
    ]);

    let parsed = parse_str_with_resolver(
        r#"
import "./m2.zng";
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    let [m2] = parsed.spec.imported_modules.as_slice() else {
        panic!("expected one imported module");
    };
    assert_eq!(m2.crate_name.as_deref(), Some("m2_bindings"));

    // Direct 3-module chain `top -> mid -> home` where `top` takes `mid`'s instantiation by value
    // without directly importing `home`.
    let parsed_top = parse_str_with_resolver(
        r#"
import "./m1.zng";

extern "C++" {
    safe fn take_from_mid(c++::std::unique_ptr<c++::m1::A>);
}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    let [m1] = parsed_top.spec.imported_modules.as_slice() else {
        panic!("expected one imported module");
    };
    assert_eq!(
        m1.cpp_templates,
        vec![vec!["std".to_owned(), "unique_ptr".to_owned()]]
    );
    assert_eq!(m1.cpp_by_value_types.len(), 1);
    assert_eq!(m1.cpp_by_value_types[0].1, "::std::unique_ptr< ::m1::A >");

    // Cyclic `import`s are reported cleanly instead of recursing forever.
    let cycle_resolver = MockFilesystem::new(vec![
        ("./a.zng", "#crate a\nimport \"./b.zng\";\n"),
        ("./b.zng", "#crate b\nimport \"./a.zng\";\n"),
    ]);
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver("import \"./a.zng\";\n", NullCfg, &cycle_resolver, |_| {});
    });
    let err = r
        .unwrap_err()
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("cyclic module `import` involving"),
        "expected cyclic module import error, got:\n{err}"
    );
}

#[test]
fn cpp_template_duplicate_module_imports_are_deduplicated() {
    let resolver = MockFilesystem::new(vec![
        ("./bar.zng", BAR_UNIQUE_PTR_HOME),
        (
            "./sub/part.zng",
            r#"
import "../bar.zng";

type c++::a::C {}
type c++::std::unique_ptr<c++::a::C> {}
"#,
        ),
    ]);
    let parsed = parse_str_with_resolver(
        r#"
import "./bar.zng";
merge "./sub/part.zng";

type c++::a::B {}
type c++::std::unique_ptr<c++::a::B> {}
"#,
        NullCfg,
        &resolver,
        |_| {},
    );
    assert_eq!(parsed.spec.imported_modules.len(), 1);
    assert_eq!(parsed.spec.types.len(), 4);
    assert_eq!(parsed.spec.types[1].cpp_methods.len(), 1);
    assert_eq!(parsed.spec.types[3].cpp_methods.len(), 1);
}

#[test]
fn cpp_template_declaration_validation() {
    // `type<T>` inside `mod c++::...` or via `use c++::...` is recognized as a C++ template
    // without `#unstable(template_types)`.
    let parsed = parse_str(
        r#"
mod c++::std {
    type<T> unique_ptr<T> {
        #layout(size = 8, align = 8);
        #trivially_relocatable;
    }
}

type c++::std::unique_ptr<i32> {}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.cpp_template_homes, vec![unique_ptr_home()]);

    for (src, expected) in [
        (
            "mod crate { type<T> Foo<T> { #layout(size = 8, align = 8); } }\n",
            "Template types are unstable. Enable them by using `#unstable(template_types)` at the top of the file.",
        ),
        (
            "mod c++::std { type<'a, T> unique_ptr<T> {} }\n",
            "`c++::` types don't have lifetime parameters",
        ),
        (
            "type<T> c++::Foo {}\n",
            "`c++::Foo` has no template arguments; C++ template declarations need template arguments",
        ),
        (
            "type<T, U> c++::std::unique_ptr<T> { unsafe fn f(&const self) -> U*; }\n",
            "type parameter `U` is not used in the template type",
        ),
        (
            "type<T> c++::std::map<i32, i32> {}\n",
            "type parameter `T` is not used in the template type",
        ),
        (
            "type<T, T> c++::std::map<T, T> {}\n",
            "type parameter `T` is declared twice",
        ),
        (
            "type<T> c++::std::vector<T> {}\ntype<T, A> c++::std::vector<T, A> {}\n",
            "every `type<...>` declaration of `c++::std::vector` must have the same number of template arguments",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn imported_cpp_template_conflict_and_layout_override() {
    let resolver = MockFilesystem::new(vec![("./bar.zng", BAR_UNIQUE_PTR_HOME)]);

    // Overriding `#layout` on an instantiation of an imported `type<T>` succeeds cleanly.
    let parsed = parse_str_with_resolver(
        r#"
import "./bar.zng";

type c++::a::CustomDeleterPtr {}
type c++::std::unique_ptr<c++::a::CustomDeleterPtr> {
    #layout(size = 16, align = 8);
}
"#,
        NullCfg,
        &resolver,
        |_| {},
    );
    assert_eq!(
        parsed.spec.types[1].layout,
        Some(LayoutPolicy::StackAllocated { size: 16, align: 8 })
    );

    // An incompatible C++ method signature on an instantiation reports a conflict against the
    // imported `type<T>`.
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            r#"
import "./bar.zng";

type c++::a::B {}
type c++::std::unique_ptr<c++::a::B> {
    safe fn get(&const self) -> i32;
}
"#,
            NullCfg,
            &resolver,
            |_| {},
        );
    });
    let err = r
        .unwrap_err()
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("Failed to apply template") && err.contains("C++ method mismatch"),
        "expected template method conflict error, got:\n{err}"
    );
}

#[test]
fn cpp_template_three_hop_indirect_homes_and_conflicts() {
    let resolver = MockFilesystem::new(vec![
        ("./home.zng", BAR_UNIQUE_PTR_HOME.to_owned()),
        (
            "./home_alt.zng",
            BAR_UNIQUE_PTR_HOME.replace("bar_bindings", "alt_bindings"),
        ),
        (
            "./mid.zng",
            r#"
#crate mid_bindings
import "./home.zng";

type c++::mid::A {}
type c++::std::unique_ptr<c++::mid::A> {}
"#
            .to_owned(),
        ),
        (
            "./mid_alt.zng",
            r#"
#crate mid_alt_bindings
import "./home_alt.zng";

type c++::mid_alt::B {}
type c++::std::unique_ptr<c++::mid_alt::B> {}
"#
            .to_owned(),
        ),
    ]);

    // 1. `leaf -> mid -> home`: `leaf` can refer to `mid`'s instantiation without importing `home`,
    //    and warnings from imported modules (`home.zng`'s elided `&const self`) are suppressed.
    let parsed_leaf = parse_str_with_resolver(
        r#"
import "./mid.zng";

extern "C++" {
    safe fn inspect<'a>(&'a const c++::std::unique_ptr<c++::mid::A>);
}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    assert!(parsed_leaf.spec.cpp_template_homes.is_empty());

    // 2. Diamond `leaf -> {mid, home}` with indirect `mid` imported BEFORE direct `home` allows
    //    `leaf` to declare its own instantiation using `home`.
    let parsed_diamond = parse_str_with_resolver(
        r#"
import "./mid.zng";
import "./home.zng";

type c++::leaf::Item {}
type c++::std::unique_ptr<c++::leaf::Item> {}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed_diamond.spec.types.len(), 2);
    assert_eq!(parsed_diamond.spec.types[1].cpp_methods.len(), 1);

    // 3. `leaf` cannot instantiate or re-home `unique_ptr` without directly importing `home.zng`,
    //    and conflicting transitive homes (`mid` + `mid_alt`, or `mid` + `home_alt`) are rejected.
    for (src, expected) in [
        (
            r#"
import "./mid.zng";
type c++::leaf::Item {}
type c++::std::unique_ptr<c++::leaf::Item> {}
"#,
            "`c++::std::unique_ptr` already has a home (`#crate bar_bindings`, imported via `./mid.zng`), which is not directly imported here; `import` the home `.zng` file of `c++::std::unique_ptr` directly",
        ),
        (
            r#"
import "./mid.zng";
type<T> c++::std::unique_ptr<T> {}
"#,
            "`c++::std::unique_ptr` already has a home (`#crate bar_bindings`, imported via `./mid.zng`)",
        ),
        (
            r#"
import "./mid.zng";
import "./mid_alt.zng";
"#,
            "use different homes for `c++::std::unique_ptr`",
        ),
        (
            r#"
import "./mid.zng";
import "./home_alt.zng";
"#,
            "use different homes for `c++::std::unique_ptr`",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str_with_resolver(src, NullCfg, &resolver, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn imported_plain_cpp_and_template_collisions_are_order_independent() {
    let resolver = MockFilesystem::new(vec![
        (
            "./plain.zng",
            "#crate plain_bindings\ntype c++::ns::Foo {}\n",
        ),
        (
            "./template.zng",
            "#crate template_bindings\ntype<T> c++::ns::Foo<T> {}\n",
        ),
    ]);

    for src in [
        "import \"./plain.zng\";\nimport \"./template.zng\";\n",
        "import \"./template.zng\";\nimport \"./plain.zng\";\n",
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str_with_resolver(src, NullCfg, &resolver, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(
                "`c++::ns::Foo` is imported both as a plain C++ type (`plain_bindings`) and as a C++ class template (`template_bindings`)"
            ),
            "unexpected error for `{src}`:\n{err}"
        );
    }

    // Local plain `c++::` type colliding with an imported C++ class template is also rejected.
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            "import \"./template.zng\";\ntype c++::ns::Foo {}\n",
            NullCfg,
            &resolver,
            |_| {},
        );
    });
    let err = r
        .unwrap_err()
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("`c++::ns::Foo` is already imported from `./template.zng`"),
        "unexpected error:\n{err}"
    );
}

#[test]
fn imported_cpp_template_rewrites_crate_paths_to_home_crate() {
    let resolver = MockFilesystem::new(vec![(
        "./home.zng",
        r#"
#crate home_bindings

type crate::Tag {
    #layout(size = 4, align = 4);
}

trait crate::HelperTrait {}

type Box<dyn crate::HelperTrait> {
    #layout(size = 16, align = 8);
}

type<T> c++::ns::Box<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
    safe constructor(crate::Tag) as from_tag;
    safe fn into_tag<'a>(&'a const self, crate::Tag, Box<dyn crate::HelperTrait>) -> crate::Tag;
}
"#,
    )]);

    let parsed = parse_str_with_resolver(
        r#"
#crate downstream
import "./home.zng";

type c++::ds::Item {}
type c++::ns::Box<c++::ds::Item> {}
"#,
        NullCfg,
        &resolver,
        |w| panic!("unexpected warning: {w}"),
    );

    let box_ty = &parsed.spec.types[1];
    let expected_tag = RustType::Adt(RustPathAndGenerics {
        path: vec!["home_bindings".to_owned(), "Tag".to_owned()],
        ..Default::default()
    });
    let expected_helper = RustType::Boxed(Box::new(RustType::Dyn(
        zngur_def::RustTrait::Normal(RustPathAndGenerics {
            path: vec!["home_bindings".to_owned(), "HelperTrait".to_owned()],
            ..Default::default()
        }),
        vec![],
    )));
    assert_eq!(
        box_ty.cpp_constructors[0].inputs,
        vec![expected_tag.clone()]
    );
    assert_eq!(
        box_ty.cpp_methods[0].inputs,
        vec![expected_tag.clone(), expected_helper]
    );
    assert_eq!(box_ty.cpp_methods[0].output, expected_tag);
    assert_eq!(
        box_ty.cpp_methods[0].output.to_string(),
        "::home_bindings::Tag"
    );
}

#[test]
fn transitive_cpp_template_home_propagates_across_unused_middle_module() {
    // `mid.zng` imports `home.zng` without instantiating or mentioning `c++::std::unique_ptr`.
    // `leaf.zng` importing `mid.zng` must still know that `home.zng` owns `unique_ptr`.
    let resolver = MockFilesystem::new(vec![
        ("./home.zng", BAR_UNIQUE_PTR_HOME.to_owned()),
        (
            "./other_home.zng",
            BAR_UNIQUE_PTR_HOME.replace("bar_bindings", "other_bindings"),
        ),
        (
            "./mid.zng",
            "#crate mid_bindings\nimport \"./home.zng\";\ntype c++::mid::Thing {}\n".to_owned(),
        ),
        (
            "./plain_unique_ptr.zng",
            "#crate plain_bindings\ntype c++::std::unique_ptr {}\n".to_owned(),
        ),
    ]);

    for (src, expected) in [
        (
            "import \"./mid.zng\";\ntype c++::leaf::Item {}\ntype c++::std::unique_ptr<c++::leaf::Item> {}\n",
            "`c++::std::unique_ptr` already has a home (`#crate bar_bindings`, imported via `./mid.zng`), which is not directly imported here",
        ),
        (
            "import \"./mid.zng\";\ntype c++::std::unique_ptr<i32> {}\n",
            "`c++::std::unique_ptr` already has a home (`#crate bar_bindings`, imported via `./mid.zng`), which is not directly imported here",
        ),
        (
            "import \"./mid.zng\";\ntype<T> c++::std::unique_ptr<T> {}\n",
            "`c++::std::unique_ptr` already has a home (`#crate bar_bindings`, imported via `./mid.zng`); `import` its home `.zng` file directly",
        ),
        (
            "import \"./mid.zng\";\nimport \"./other_home.zng\";\n",
            "use different homes for `c++::std::unique_ptr`",
        ),
        (
            "import \"./mid.zng\";\ntype c++::std::unique_ptr {}\n",
            "`c++::std::unique_ptr` is already a C++ class template (`#crate bar_bindings`, imported via `./mid.zng`), and cannot also be declared as a plain C++ type in this module",
        ),
        (
            "import \"./mid.zng\";\nimport \"./plain_unique_ptr.zng\";\n",
            "`c++::std::unique_ptr` is imported both as a plain C++ type (`plain_bindings`) and as a C++ class template (`bar_bindings via `./mid.zng``)",
        ),
        (
            "import \"./plain_unique_ptr.zng\";\nimport \"./mid.zng\";\n",
            "`c++::std::unique_ptr` is imported both as a plain C++ type (`plain_bindings`) and as a C++ class template (`bar_bindings via `./mid.zng``)",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str_with_resolver(src, NullCfg, &resolver, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn unused_cpp_template_home_validates_directives_and_members() {
    // Layout-less `type<T> c++::Foo<T>` with `#trivially_relocatable`, C++ constructors, and
    // methods returning `c++::Foo<T>` is valid (instantiations supply their own `#layout`).
    let parsed = parse_str(
        r#"
#crate home_bindings

type<T> c++::ns::Vec<T> {
    #trivially_relocatable;
    safe constructor() as new;
    safe fn clone_self<'a>(&'a const self) -> c++::ns::Vec<T>;
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.cpp_templates.len(), 1);
    assert_eq!(parsed.spec.cpp_template_homes.len(), 1);

    for (src, expected) in [
        (
            "type<T> c++::Foo<T> { #heap_allocated; }\n",
            "`#heap_allocated` and `#only_by_ref` are for Rust types",
        ),
        (
            "type<T> c++::Foo<T> { #only_by_ref; }\n",
            "`#heap_allocated` and `#only_by_ref` are for Rust types",
        ),
        (
            "type<T> c++::Foo<T> { wellknown_traits(?Sized); }\n",
            "`?Sized` is not supported on C++ types",
        ),
        (
            "type<T> c++::Foo<T> { #cpp_heap_allocated \"Foo\"; }\n",
            "`#cpp_heap_allocated` is no longer supported",
        ),
        (
            "type<T> c++::Foo<T> { #cpp_ref \"Foo\"; }\n",
            "`#cpp_ref` is no longer supported",
        ),
        (
            "type<T> c++::Foo<T> { #cpp_stack_owned \"Foo\"; }\n",
            "`#cpp_stack_owned` is no longer supported",
        ),
        (
            "type<T> c++::Foo<T> { constructor(i32); }\n",
            "C++ constructors start with `safe` or `unsafe`",
        ),
        (
            "type<T> c++::Foo<T> { field x (offset = 0, type = i32); }\n",
            "C++ field accessors omit `offset`",
        ),
        (
            "type<T> c++::Foo<T> { variant A {} }\n",
            "enum variants cannot be declared on `c++::` types",
        ),
        (
            "type<T> c++::Foo<T> { safe fn f<'a>(&'a const self, c++::Undeclared); }\n",
            "`c++::Undeclared` is not declared or imported",
        ),
        (
            "type c++::Opaque {}\ntype<T> c++::Foo<T> { safe fn f<'a>(&'a const self, c++::Opaque); }\n",
            "`c++::Opaque` cannot be passed or returned by value without `#layout(...)` and `#trivially_relocatable`",
        ),
        (
            "type c++::Plain {}\ntype<T> c++::Foo<T> { safe fn f<'a>(&'a const self, &'a const c++::Plain<i32>); }\n",
            "`c++::Plain` is declared as a plain C++ type, so it cannot take template arguments",
        ),
        (
            "type<T> c++::Foo<T> { safe fn f<'a>(&'a const self, &'a const c++::Foo<i32, i32>); }\n",
            "`c++::Foo` takes 1 template argument, not 2",
        ),
        (
            "type<T> c++::Foo<T> { safe fn f<'a>(&'a const self, &'a const c++::Foo<i64>); }\n",
            "instantiation `c++::Foo<i64>` of C++ class template `c++::Foo` is not declared",
        ),
        (
            "type<T> c++::Foo<T> { safe fn f<'a>(&'a const self, &'a const c++::Unknown<T>); }\n",
            "C++ class template `c++::Unknown` is not declared or imported",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn undeclared_and_transitive_cpp_template_instantiations_require_direct_import() {
    let resolver = MockFilesystem::new(vec![
        (
            "./m0.zng",
            r#"
#crate m0_bindings

type<T> c++::std::unique_ptr<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
}

type c++::std::unique_ptr<i32> {}
"#,
        ),
        (
            "./m1.zng",
            "#crate m1_bindings\nimport \"./m0.zng\";\ntype c++::m1::Helper {}\n",
        ),
    ]);

    for (src, expected) in [
        // Direct import of `m0.zng`, but `unique_ptr<i64>` is not declared anywhere: only the
        // undeclared-instantiation error should fire (no bogus by-value `#layout` error).
        (
            "import \"./m0.zng\";\nmod c++::ns { safe fn f() -> c++::std::unique_ptr<i64>; }\n",
            "instantiation `c++::std::unique_ptr<i64>` of C++ class template `c++::std::unique_ptr` is not declared; add `type c++::std::unique_ptr<i64> { ... }`",
        ),
        // `top -> m1 -> m0`: `m0`'s `unique_ptr<i32>` is NOT re-exported by `m1`, so both by-ref
        // and by-value uses in `top` are rejected with a clear direct-import diagnostic and no
        // bogus by-value `#layout` error.
        (
            "import \"./m1.zng\";\nmod c++::ns { safe fn by_ref<'a>(&'a const c++::std::unique_ptr<i32>); }\n",
            "instantiation `c++::std::unique_ptr<i32>` of C++ class template `c++::std::unique_ptr` is declared in `m0.zng` (imported transitively), which is not directly imported here; `import` that `.zng` file directly",
        ),
        (
            "import \"./m1.zng\";\nmod c++::ns { safe fn by_val() -> c++::std::unique_ptr<i32>; }\n",
            "instantiation `c++::std::unique_ptr<i32>` of C++ class template `c++::std::unique_ptr` is declared in `m0.zng` (imported transitively), which is not directly imported here; `import` that `.zng` file directly",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str_with_resolver(src, NullCfg, &resolver, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
        assert!(
            !err.contains("cannot be passed or returned by value"),
            "did not expect secondary by-value error for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn conflicting_cpp_template_blocks_in_home_module_are_rejected() {
    // Non-overlapping partial patterns (`Pair<T, i32>` vs `Pair<T, i64>`) can legitimately declare
    // different layouts or member signatures in an instantiation-less home module.
    let parsed = parse_str(
        r#"
#crate home_bindings

type<T> c++::ns::Pair<T, i32> {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn second<'a>(&'a const self) -> i32;
}

type<T> c++::ns::Pair<T, i64> {
    #layout(size = 16, align = 8);
    #trivially_relocatable;
    safe fn second<'a>(&'a const self) -> i64;
}
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.cpp_templates.len(), 2);

    // Overlapping `type<...>` blocks with conflicting `#layout` or `safe fn` signatures fail in the
    // home module itself (both in one file and across `merge`d files), even without instantiations.
    let resolver = MockFilesystem::new(vec![(
        "./extra.zng",
        r#"
type<U> c++::ns::Box<U> {
    safe fn get<'a>(&'a const self) -> i64;
}
"#,
    )]);
    let r = catch_unwind(|| {
        let _ = parse_str_with_resolver(
            r#"
#crate home_bindings
merge "./extra.zng";

type<T> c++::ns::Box<T> {
    safe fn get<'a>(&'a const self) -> i32;
}
"#,
            NullCfg,
            &resolver,
            |_| {},
        );
    });
    let err = r
        .unwrap_err()
        .downcast::<ErrorText>()
        .expect("expected ErrorText")
        .0;
    assert!(
        err.contains("Failed to apply template") && err.contains("C++ method mismatch"),
        "expected template method conflict error across merged files, got:\n{err}"
    );

    for (src, expected) in [
        (
            r#"
#crate home_bindings
type<T> c++::ns::Box<T> {
    #layout(size = 8, align = 8);
}
type<T> c++::ns::Box<T> {
    #layout(size = 16, align = 8);
}
"#,
            "Conflicting layout policy found",
        ),
        (
            r#"
#crate home_bindings
type<T> c++::ns::Pair<T, i32> {
    #layout(size = 8, align = 8);
}
type<U> c++::ns::Pair<i64, U> {
    #layout(size = 16, align = 8);
}
"#,
            "Conflicting layout policy found",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn cpp_function_template_args_parse() {
    let parsed = parse_str(
        r#"
#allow(elided_lifetimes)

type c++::geo::Point {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn get<i32>(&const self) -> i32 as get_i32;
    safe fn cast<'a, const c++::geo::Point,>(&'a const self,) -> &'a const c++::geo::Point;
}

type<T> c++::Box<T> {
    safe fn rebind<'a, const T, c++::geo::Point,>(&'a const self) -> &'a const T;
}

type c++::Box<i32> {}

type c++::std::unique_ptr<c++::geo::Point> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
}

mod c++::std {
    safe fn make_unique<c++::geo::Point,>(i32, i32) -> c++::std::unique_ptr<c++::geo::Point> as make_point;
    safe fn forward<'a, const i32>(&'a const i32) -> &'a const i32;
    safe fn id<'a,>(&'a const i32) -> &'a const i32;
    safe fn detail::helper<i32>();
}

safe fn c++::ns::foo<i32>();
"#,
        NullCfg,
        |w| panic!("unexpected warning: {w}"),
    );
    let point = RustType::Cpp(vec!["geo".to_owned(), "Point".to_owned()].into());
    let i32_ty = RustType::Primitive(PrimitiveRustType::Int(32));
    let [func, forward, id, helper, top_foo] = parsed.spec.cpp_funcs.as_slice() else {
        panic!("expected five C++ functions");
    };
    assert_eq!(func.cpp_name, "make_unique");
    assert_eq!(func.template_args, vec![point.clone()]);
    assert_eq!(func.cpp_path(), "::std::make_unique< ::geo::Point >");
    assert_eq!(
        forward.template_args,
        vec![RustType::CppConst(Box::new(i32_ty.clone()))]
    );
    assert_eq!(forward.cpp_path(), "::std::forward< const ::int32_t >");
    assert!(id.template_args.is_empty());
    assert_eq!(
        helper.namespace,
        vec!["std".to_owned(), "detail".to_owned()]
    );
    assert_eq!(helper.cpp_name, "helper");
    assert_eq!(helper.template_args, vec![i32_ty.clone()]);
    assert_eq!(helper.cpp_path(), "::std::detail::helper< ::int32_t >");
    assert_eq!(top_foo.namespace, vec!["ns".to_owned()]);
    assert_eq!(top_foo.cpp_name, "foo");
    assert_eq!(top_foo.template_args, vec![i32_ty.clone()]);
    assert_eq!(top_foo.cpp_path(), "::ns::foo< ::int32_t >");

    let point_ty = &parsed.spec.types[0];
    assert_eq!(point_ty.cpp_methods[0].template_args, vec![i32_ty.clone()]);
    assert_eq!(
        point_ty.cpp_methods[1].template_args,
        vec![RustType::CppConst(Box::new(point.clone()))]
    );
    assert_eq!(
        point_ty.cpp_methods[1].lifetimes,
        vec![Lifetime("a".to_owned())]
    );

    // `type<T> c++::Box<T>` substitutes `T = i32` into `ZngurCppMethod::template_args`.
    let box_i32 = &parsed.spec.types[1];
    assert_eq!(
        box_i32.cpp_methods[0].template_args,
        vec![RustType::CppConst(Box::new(i32_ty)), point]
    );
}

#[test]
fn cpp_function_template_args_ordering_and_validation_errors() {
    for (src, expected) in [
        (
            "mod c++::ns { safe fn f<i32, 'a>(&'a const i32); }\n",
            "lifetime parameters must come before template arguments",
        ),
        (
            "safe fn c++::ns::f<i32, 'a>(&'a const i32);\n",
            "lifetime parameters must come before template arguments",
        ),
        (
            "type c++::A { safe fn f<i32, 'a>(&'a const self); }\n",
            "lifetime parameters must come before template arguments",
        ),
        (
            "fn crate::f<i32, 'a>(&'a i32);\n",
            "lifetime parameters must come before type arguments",
        ),
        (
            "type crate::Foo { #layout(size = 8, align = 8); fn f<i32, 'a>(&'a i32); }\n",
            "lifetime parameters must come before type arguments",
        ),
        (
            "trait crate::Tr { fn f<i32, 'a>(&'a i32); }\n",
            "lifetime parameters must come before type arguments",
        ),
        (
            "extern \"C++\" { unsafe fn f<i32, 'a>(&'a i32); }\n",
            "lifetime parameters must come before type arguments",
        ),
        (
            "mod c++::ns { safe fn f<::std::string::String>(); }\n",
            "can't be a C++ template argument",
        ),
        (
            "mod c++::ns { safe fn f<'a, &'a const i32>(&'a const i32); }\n",
            "can't be a C++ template argument",
        ),
        (
            "mod c++::ns { safe fn f<str>(); }\n",
            "`str` has no C++ equivalent",
        ),
        (
            "mod c++::ns { safe fn f<char>(); }\n",
            "`char` has no C++ equivalent",
        ),
        (
            "type c++::A { safe fn f<(i32, i32)>(&'static const self); }\n",
            "can't be a C++ template argument",
        ),
        (
            // Even a `type<T>` with no instantiations validates non-variable parts of `template_args`.
            "type<T> c++::Box<T> { safe fn f<::std::vec::Vec<T>>(&'static const self); }\n",
            "can't be a C++ template argument",
        ),
        (
            // Post-substitution validation catches `const T` when `T` is instantiated with a C++ reference.
            "type<T> c++::Box<T> { safe fn f<const T>(); }\ntype c++::Box<i32&> {}\n",
            "only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`, not `i32&`",
        ),
        (
            // Post-substitution validation catches `T&` when `T` is instantiated with a C++ reference.
            "type<T> c++::Box<T> { safe fn f<T&>(); }\ntype c++::Box<i32&> {}\n",
            "references to C++ references (`T&&`) aren't valid",
        ),
        (
            // Rust functions still reject C++ `const T` in generic arguments.
            "fn crate::f<const i32>();\n",
            "`const T` can only be the type of a C++ pointer or reference",
        ),
        (
            // Rust methods still reject C++ `const T` in generic arguments.
            "type crate::Foo { #layout(size = 8, align = 8); fn f<const i32>(); }\n",
            "`const T` can only be the type of a C++ pointer or reference",
        ),
        (
            // Plain C++ type passed template arguments inside function/method `template_args`.
            "type c++::Plain {}\nsafe fn c++::ns::f<c++::Plain<i32>>();\n",
            "`c++::Plain` is declared as a plain C++ type, so it cannot take template arguments",
        ),
        (
            "type c++::Plain {}\ntype c++::A { safe fn f<c++::Plain<i32>>(); }\n",
            "`c++::Plain` is declared as a plain C++ type, so it cannot take template arguments",
        ),
        (
            // Declared C++ class template missing or having wrong-arity template arguments in `template_args`.
            "type<T> c++::Box<T> {}\nsafe fn c++::ns::f<c++::Box>();\n",
            "`c++::Box` is a C++ class template and needs 1 template argument",
        ),
        (
            "type<T> c++::Box<T> {}\nsafe fn c++::ns::f<c++::Box<i32, i32>>();\n",
            "`c++::Box` takes 1 template argument, not 2",
        ),
        (
            "type<T> c++::Box<T> {}\ntype<U> c++::Holder<U> { safe fn f<c++::Box<U, i32>>(); }\n",
            "`c++::Box` takes 1 template argument, not 2",
        ),
        (
            // Undeclared C++ class template or type inside `template_args`.
            "safe fn c++::ns::f<c++::UnknownTpl<i32>>();\n",
            "instantiation `c++::UnknownTpl<i32>` of C++ class template `c++::UnknownTpl` is not declared",
        ),
        (
            "safe fn c++::ns::f<c++::Undeclared>();\n",
            "`c++::Undeclared` is not declared or imported",
        ),
        (
            // Forgotten `c++::` prefix inside `mod c++::...` resolves relative to the namespace and is caught.
            "type c++::geo::Point {}\nmod c++::std { safe fn make_unique<geo::Point>(); }\n",
            "`c++::std::geo::Point` is not declared or imported",
        ),
        (
            // Rust-style `<T>` misuse on C++ functions/methods (both top-level and inside `mod c++::...`).
            "type c++::A { safe fn gen<T>(i32); }\n",
            "`::T` can't be a C++ template argument",
        ),
        (
            "safe fn c++::a::gen_fn<T>(i32);\n",
            "`::T` can't be a C++ template argument",
        ),
        (
            "mod c++::a { safe fn gen_fn<T>(i32); }\n",
            "`c++::a::T` is not declared or imported",
        ),
    ] {
        let r = catch_unwind(|| {
            let _ = parse_str(src, NullCfg, |_| {});
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn cpp_function_and_method_template_args_merge_and_collision() {
    // Identical instantiations deduplicate cleanly across `merge`d files (even with renamed
    // function/method lifetimes).
    let dedup_resolver = MockFilesystem::new(vec![(
        "./extra.zng",
        r#"
type c++::ns::Foo {
    safe fn cast<'b, const i32>(&'b const self) -> &'b const i32 as cast_i32;
}

mod c++::ns {
    safe fn forward<'b, const i32>(&'b const i32) -> &'b const i32 as fwd_i32;
}
"#,
    )]);
    let parsed = parse_str_with_resolver(
        r#"
merge "./extra.zng";

type c++::ns::Foo {
    safe fn cast<'a, const i32>(&'a const self) -> &'a const i32 as cast_i32;
}

mod c++::ns {
    safe fn forward<'a, const i32>(&'a const i32) -> &'a const i32 as fwd_i32;
}
"#,
        NullCfg,
        &dedup_resolver,
        |w| panic!("unexpected warning: {w}"),
    );
    assert_eq!(parsed.spec.types[0].cpp_methods.len(), 1);
    assert_eq!(parsed.spec.cpp_funcs.len(), 1);

    // Same-file collisions without `as` and cross-file `merge` conflicts on `template_args` fail.
    let conflict_fn_resolver = MockFilesystem::new(vec![(
        "./extra.zng",
        "mod c++::ns { safe fn foo<i64>(); }\n",
    )]);
    let conflict_method_resolver = MockFilesystem::new(vec![(
        "./extra.zng",
        "type c++::ns::Foo { safe fn get<i64>(&'static const self) -> i64 as get_val; }\n",
    )]);

    for (src, resolver_opt, expected) in [
        (
            "mod c++::ns {\n    safe fn foo<i32>();\n    safe fn foo<i64>();\n}\n",
            None,
            "is already the Rust name of another C++ function in `c++::ns`",
        ),
        (
            "type c++::ns::Foo {\n    safe fn get<i32>(&'static const self) -> i32;\n    safe fn get<i64>(&'static const self) -> i64;\n}\n",
            None,
            "is already the Rust name of another member of this type",
        ),
        (
            "merge \"./extra.zng\";\nmod c++::ns { safe fn foo<i32>(); }\n",
            Some(&conflict_fn_resolver),
            "C++ function mismatch",
        ),
        (
            "merge \"./extra.zng\";\ntype c++::ns::Foo { safe fn get<i32>(&'static const self) -> i32 as get_val; }\n",
            Some(&conflict_method_resolver),
            "C++ method mismatch",
        ),
    ] {
        let r = catch_unwind(|| {
            if let Some(resolver) = resolver_opt {
                let _ = parse_str_with_resolver(src, NullCfg, resolver, |_| {});
            } else {
                let _ = parse_str(src, NullCfg, |_| {});
            }
        });
        let err = r
            .unwrap_err()
            .downcast::<ErrorText>()
            .expect("expected ErrorText")
            .0;
        assert!(
            err.contains(expected),
            "expected `{expected}` for `{src}`, got:\n{err}"
        );
    }
}

#[test]
fn layout_without_relocatable_warns() {
    check_warnings(
        r#"
type c++::A {
    #layout(size = 8, align = 4);
}
type c++::Conservative {
    #layout_conservative(size = 16, align = 8);
}
// Relocatable (`#trivially_relocatable` or `#unsafe(trivially_relocatable)`), or without a layout:
// no warning.
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
type c++::UnsafeB {
    #layout_conservative(size = 16, align = 8);
    #unsafe(trivially_relocatable);
}
type c++::C {}
"#,
        expect![[r#"
            Warning: this layout is only checked in C++: Rust holds `c++::A` by value only if it's also `#trivially_relocatable`, so it only refers to it (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
               ╭─[ test.zng:3:5 ]
               │
             3 │     #layout(size = 8, align = 4);
               │     ──────────────┬─────────────  
               │                   ╰─────────────── this layout is only checked in C++: Rust holds `c++::A` by value only if it's also `#trivially_relocatable`, so it only refers to it (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
            ───╯
            Warning: this layout is only checked in C++: Rust holds `c++::Conservative` by value only if it's also `#trivially_relocatable`, so it only refers to it (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
               ╭─[ test.zng:6:5 ]
               │
             6 │     #layout_conservative(size = 16, align = 8);
               │     ─────────────────────┬────────────────────  
               │                          ╰────────────────────── this layout is only checked in C++: Rust holds `c++::Conservative` by value only if it's also `#trivially_relocatable`, so it only refers to it (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
            ───╯
        "#]],
    );
}

#[test]
fn layout_without_relocatable_can_be_allowed() {
    check_warnings(
        r#"
#allow(layout_without_relocatable)
type c++::A {
    #layout(size = 8, align = 4);
}
type c++::B {
    #layout_conservative(size = 16, align = 8);
}
"#,
        expect![[r#""#]],
    );
}

#[test]
fn layout_without_relocatable_is_checked_after_merging_and_scoped_per_file() {
    // `#trivially_relocatable` in another file suppresses the warning after merging.
    let relocatable_resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type c++::A {
    #trivially_relocatable;
}
"#,
    )]);
    let mut warnings = String::new();
    let _ = parse_str_with_resolver(
        r#"
merge "./a.zng";
type c++::A {
    #layout(size = 8, align = 4);
}
"#,
        NullCfg,
        &relocatable_resolver,
        |w| warnings.push_str(w),
    );
    assert_eq!(warnings, "");

    // `#allow(layout_without_relocatable)` in `test.zng` does not silence a `#layout` in `./b.zng`.
    let child_layout_resolver = MockFilesystem::new(vec![(
        "./b.zng",
        r#"
type c++::A {
    #layout(size = 8, align = 4);
}
"#,
    )]);
    let mut warnings = String::new();
    let _ = parse_str_with_resolver(
        r#"
#allow(layout_without_relocatable)
merge "./b.zng";
type c++::A {}
"#,
        NullCfg,
        &child_layout_resolver,
        |w| warnings.push_str(w),
    );
    assert!(
        warnings.contains("./b.zng:3:5") && !warnings.contains("test.zng"),
        "expected warning only in ./b.zng, got:\n{warnings}"
    );

    // `#allow(layout_without_relocatable)` in `./b.zng` keeps its `#layout` silent even when
    // `test.zng` (without `#allow`) reopens the type after `merge "./b.zng";`.
    let child_allowed_resolver = MockFilesystem::new(vec![(
        "./b.zng",
        r#"
#allow(layout_without_relocatable)
type c++::A {
    #layout(size = 8, align = 4);
}
"#,
    )]);
    let mut warnings = String::new();
    let _ = parse_str_with_resolver(
        r#"
merge "./b.zng";
type c++::A {
    safe fn ping(&'static const self);
}
"#,
        NullCfg,
        &child_allowed_resolver,
        |w| warnings.push_str(w),
    );
    assert_eq!(warnings, "");
}

#[test]
fn layout_without_relocatable_for_templates_and_imports() {
    // Template with multiple non-relocatable instantiations warns once at the template's `#layout`,
    // naming the first non-relocatable instantiation.
    check_warnings(
        r#"
type<T> c++::Box<T> {
    #layout(size = 8, align = 8);
}
type c++::Box<i32> {}
type c++::Box<i64> {}
"#,
        expect![[r#"
            Warning: for instantiation `c++::Box<i32>`, this layout is only checked in C++: Rust holds `c++::Box<T>` by value only when `#trivially_relocatable`, so `c++::Box<i32>` is only referred to (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
               ╭─[ test.zng:3:5 ]
               │
             3 │     #layout(size = 8, align = 8);
               │     ──────────────┬─────────────  
               │                   ╰─────────────── for instantiation `c++::Box<i32>`, this layout is only checked in C++: Rust holds `c++::Box<T>` by value only when `#trivially_relocatable`, so `c++::Box<i32>` is only referred to (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
            ───╯
        "#]],
    );

    // Template whose instantiation supplies `#trivially_relocatable` does not warn.
    check_warnings(
        r#"
type<T> c++::Box<T> {
    #layout_conservative(size = 16, align = 8);
}
type c++::Box<i32> {
    #trivially_relocatable;
}
"#,
        expect![[r#""#]],
    );

    // Template with both a relocatable and a non-relocatable instantiation warns naming the
    // non-relocatable instantiation.
    check_warnings(
        r#"
type c++::Pinned {}
type<T> c++::Box<T> {
    #layout_conservative(size = 16, align = 8);
}
type c++::Box<i32> {
    #trivially_relocatable;
}
type c++::Box<c++::Pinned> {}
"#,
        expect![[r#"
            Warning: for instantiation `c++::Box<c++::Pinned>`, this layout is only checked in C++: Rust holds `c++::Box<T>` by value only when `#trivially_relocatable`, so `c++::Box<c++::Pinned>` is only referred to (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
               ╭─[ test.zng:4:5 ]
               │
             4 │     #layout_conservative(size = 16, align = 8);
               │     ─────────────────────┬────────────────────  
               │                          ╰────────────────────── for instantiation `c++::Box<c++::Pinned>`, this layout is only checked in C++: Rust holds `c++::Box<T>` by value only when `#trivially_relocatable`, so `c++::Box<c++::Pinned>` is only referred to (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
            ───╯
        "#]],
    );

    // A local `type<T> c++::...` template with `#layout` and zero local instantiations warns in
    // its own module unless `#trivially_relocatable` is also present.
    check_warnings(
        r#"
type<T> c++::std::unique_ptr<T> {
    #layout(size = 8, align = 8);
}
type<T> c++::std::shared_ptr<T> {
    #layout(size = 16, align = 8);
    #trivially_relocatable;
}
"#,
        expect![[r#"
            Warning: this layout is only checked in C++: Rust holds `c++::std::unique_ptr<T>` by value only if it's also `#trivially_relocatable`, so it only refers to it (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
               ╭─[ test.zng:3:5 ]
               │
             3 │     #layout(size = 8, align = 8);
               │     ──────────────┬─────────────  
               │                   ╰─────────────── this layout is only checked in C++: Rust holds `c++::std::unique_ptr<T>` by value only if it's also `#trivially_relocatable`, so it only refers to it (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
            ───╯
        "#]],
    );

    // Cross-module `import`:
    // 1. Warnings inside `./bar.zng` are not re-emitted in downstream crates, nor do spans from
    //    `./bar.zng` leak into `test.zng`.
    // 2. Instantiating an imported template with `#layout` and without `#trivially_relocatable`
    //    warns at the local instantiation site in `test.zng` and respects `#allow` in `test.zng`.
    let import_resolver = MockFilesystem::new(vec![
        (
            "./bar.zng",
            r#"
#crate bar_bindings
merge "./bar_relocatable.zng";

type c++::bar::WarnInBarOnly {
    #layout(size = 8, align = 4);
}

type c++::bar::SplitRelocatable {
    #layout(size = 8, align = 4);
}

type<T> c++::Opt<T> {
    #layout_conservative(size = 32, align = 8);
}

type c++::bar::Item {}
type c++::Opt<c++::bar::Item> {
    #trivially_relocatable;
}
"#,
        ),
        (
            "./bar_relocatable.zng",
            r#"
type c++::bar::SplitRelocatable {
    #trivially_relocatable;
}
"#,
        ),
    ]);

    let mut warnings = String::new();
    let _ = parse_str_with_resolver(
        r#"
import "./bar.zng";

type c++::Main {}
type c++::Opt<c++::Main> {}
"#,
        NullCfg,
        &import_resolver,
        |w| warnings.push_str(w),
    );
    assert!(
        warnings.contains("test.zng:5:6")
            && warnings.contains("the layout of `c++::Opt<c++::Main>` (from `c++::Opt<T>`)")
            && !warnings.contains("bar.zng"),
        "expected warning only at test.zng instantiation, got:\n{warnings}"
    );

    let mut silenced_warnings = String::new();
    let _ = parse_str_with_resolver(
        r#"
#allow(layout_without_relocatable)
import "./bar.zng";

type c++::Main {}
type c++::Opt<c++::Main> {}
"#,
        NullCfg,
        &import_resolver,
        |w| silenced_warnings.push_str(w),
    );
    assert_eq!(silenced_warnings, "");

    // Reopening an imported template instantiation across multiple `type` blocks emits only one
    // warning for the inherited layout.
    let mut reopened_warnings = Vec::new();
    let _ = parse_str_with_resolver(
        r#"
import "./bar.zng";

type c++::Main {}
type c++::Opt<c++::Main> {}
type c++::Opt<c++::Main> {
    safe fn ping(&'static const self);
}
"#,
        NullCfg,
        &import_resolver,
        |w| reopened_warnings.push(w.to_owned()),
    );
    assert_eq!(
        reopened_warnings.len(),
        1,
        "expected a single warning on reopened imported-template instantiation, got: {reopened_warnings:?}"
    );

    // Overriding an imported template's `#layout` on a non-relocatable instantiation warns only
    // once (on the instantiation's own `#layout`), without a duplicate `(from c++::Opt<T>)` warning.
    let mut override_imported_warnings = Vec::new();
    let _ = parse_str_with_resolver(
        r#"
import "./bar.zng";

type c++::Main {}
type c++::Opt<c++::Main> {
    #layout(size = 16, align = 8);
}
"#,
        NullCfg,
        &import_resolver,
        |w| override_imported_warnings.push(w.to_owned()),
    );
    assert_eq!(
        override_imported_warnings.len(),
        1,
        "expected a single warning on overridden imported-template layout, got: {override_imported_warnings:?}"
    );
    assert!(
        override_imported_warnings[0].contains("this layout is only checked in C++")
            && !override_imported_warnings[0].contains("(from `c++::Opt<T>`)"),
        "unexpected warning text: {}",
        override_imported_warnings[0]
    );

    // Overriding a local template's `#layout` in an `#allow`-ed file does not falsely blame the
    // template's `#layout` when another instantiation inherits it with `#trivially_relocatable`.
    let override_local_resolver = MockFilesystem::new(vec![(
        "./override.zng",
        r#"
#allow(layout_without_relocatable)
type c++::Big {}
type c++::Box<c++::Big> {
    #layout(size = 32, align = 8);
}
"#,
    )]);
    let mut override_local_warnings = String::new();
    let _ = parse_str_with_resolver(
        r#"
merge "./override.zng";

type<T> c++::Box<T> {
    #layout(size = 8, align = 8);
}
type c++::Box<i32> {
    #trivially_relocatable;
}
"#,
        NullCfg,
        &override_local_resolver,
        |w| override_local_warnings.push_str(w),
    );
    assert_eq!(override_local_warnings, "");

    // When all local instantiations override a template's `#layout`, the template's own
    // non-relocatable `#layout` is never inherited locally and still warns.
    check_warnings(
        r#"
type<T> c++::Box<T> {
    #layout(size = 8, align = 8);
}
type c++::Box<i32> {
    #layout(size = 16, align = 8);
    #trivially_relocatable;
}
"#,
        expect![[r#"
            Warning: this layout is only checked in C++: Rust holds `c++::Box<T>` by value only if it's also `#trivially_relocatable`, so it only refers to it (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
               ╭─[ test.zng:3:5 ]
               │
             3 │     #layout(size = 8, align = 8);
               │     ──────────────┬─────────────  
               │                   ╰─────────────── this layout is only checked in C++: Rust holds `c++::Box<T>` by value only if it's also `#trivially_relocatable`, so it only refers to it (a zero-sized type); silence this with `#allow(layout_without_relocatable)`
            ───╯
        "#]],
    );
}
