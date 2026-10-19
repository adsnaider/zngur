use std::panic::catch_unwind;

use expect_test::{Expect, expect};
use zngur_def::{
    CppIndirection, CppTemplateHome, CppTypePath, LayoutPolicy, Mutability, PrimitiveRustType,
    RustPathAndGenerics, RustType, TriviallyRelocatable, ZngurSpec,
};

use crate::{
    DefaultImportResolver, EntityPath, ImportResolver, ParsedZngFile, ReportSink, Scope,
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

/// Parses `zng`, which must succeed, and checks the warnings it emits.
fn check_warnings(zng: &str, warnings: Expect) {
    let mut emitted = String::new();
    let _ = parse_str(zng, NullCfg, |msg| emitted.push_str(msg));
    warnings.assert_eq(&emitted);
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

fn check_fail_with_cfg(
    zng: &str,
    cfg: impl RustCfgProvider + std::panic::UnwindSafe + 'static,
    error: Expect,
) {
    let r = catch_unwind(|| {
        let _ = parse_str(zng, cfg, |_| {});
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
fn parse_unit() {
    check_fail(
        r#"
type () {
    #layout(size = 0, align = 1);
    wellknown_traits(Copy);
}
    "#,
        expect![[r#"
            Error: Unit type is declared implicitly. Remove this entirely.
               ╭─[ test.zng:2:6 ]
               │
             2 │ type () {
               │      ─┬  
               │       ╰── Unit type is declared implicitly. Remove this entirely.
            ───╯
        "#]],
    );
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
fn typo_in_wellknown_trait() {
    check_fail(
        r#"
type () {
    #layout(size = 0, align = 1);
    welcome_traits(Copy);
}
    "#,
        expect![[r#"
            Error: found 'welcome_traits' expected '#', 'variant', 'wellknown_traits', 'non_exhaustive', 'constructor', 'field', 'safe', 'unsafe', 'async', 'fn', or '}'
               ╭─[ test.zng:4:5 ]
               │
             4 │     welcome_traits(Copy);
               │     ───────┬──────  
               │            ╰──────── found 'welcome_traits' expected '#', 'variant', 'wellknown_traits', 'non_exhaustive', 'constructor', 'field', 'safe', 'unsafe', 'async', 'fn', or '}'
            ───╯
        "#]],
    );
}

#[test]
fn multiple_layout_policies() {
    check_fail(
        r#"
type ::std::string::String {
    #layout(size = 24, align = 8);
    #heap_allocated;
}
    "#,
        expect![[r#"
            Error: Duplicate layout policy found
               ╭─[ test.zng:4:5 ]
               │
             4 │     #heap_allocated;
               │     ───────┬───────  
               │            ╰───────── Duplicate layout policy found
            ───╯
        "#]],
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
        self.files
            .get(&path)
            .cloned()
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
fn module_import_prohibited() {
    let resolver = MockFilesystem::new(vec![] as Vec<(&str, &str)>);

    check_import_fail(
        r#"
    merge "foo/bar.zng";
    "#,
        expect![[r#"
            Error: Module import is not supported. Use a relative path instead.
               ╭─[ test.zng:2:5 ]
               │
             2 │     merge "foo/bar.zng";
               │     ──────────┬─────────  
               │               ╰─────────── Module import is not supported. Use a relative path instead.
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn import_has_conflict() {
    // Test that an import which introduces a conflict produces a reasonable error message.
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
      type A {
        #layout(size = 1, align = 1);
      }
    "#,
    )]);

    check_import_fail(
        r#"
    merge "./a.zng";
    type A {
      #layout(size = 2, align = 2);
    }
"#,
        expect![[r#"
            Error: Conflicting layout policy found
               ╭─[ ./a.zng:2:12 ]
               │
             2 │       type A {
               │            ┬  
               │            ╰── Conflicting layout policy found
             3 │         #layout(size = 1, align = 1);
               │         ──────────────┬─────────────  
               │                       ╰─────────────── declaration here ...
               │
               ├─[ test.zng:4:7 ]
               │
             4 │       #layout(size = 2, align = 2);
               │       ──────────────┬─────────────  
               │                     ╰─────────────── conflicts with the declaration here.
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn missing_layout_across_files() {
    // Test that a type defined in multiple places without a layout produces a reasonable error message.
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
      type A {
      }
    "#,
    )]);

    check_import_fail(
        r#"
    merge "./a.zng";
    type A {}
"#,
        expect![[r#"
            Error: No layout policy found for type ::A.
               ╭─[ test.zng:1:1 ]
               │
             3 │     type A {}
               │          ┬  
               │          ╰── Type first declared here.
               │
               ├─[ ./a.zng:2:12 ]
               │
             2 │       type A {
               │            ┬  
               │            ╰── Type also declared here
               │ 
               │ Note: Use one of `#layout(size = X, align = Y)`, `#heap_allocated` or `#only_by_ref`.
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn missing_layout_across_files_with_template() {
    // Test that a type defined in multiple places without a layout produces a reasonable error message.
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
      type A<B> {
      }
    "#,
    )]);

    check_import_fail(
        r#"
    #unstable(template_types)
    merge "./a.zng";
    type<T> A<T> {}
    type A<B> {}
"#,
        expect![[r#"
            Error: No layout policy found for type ::A::<::B>.
               ╭─[ test.zng:1:1 ]
               │
             5 │     type A<B> {}
               │          ──┬─  
               │            ╰─── Type first declared here.
               │
               ├─[ ./a.zng:2:12 ]
               │
             2 │       type A<B> {
               │            ──┬─  
               │              ╰─── Type also declared here
               │
               ├─[ test.zng:1:1 ]
               │
             4 │     type<T> A<T> {}
               │             ──┬─  
               │               ╰─── Matching template defined here
               │ 
               │ Note: Use one of `#layout(size = X, align = Y)`, `#heap_allocated` or `#only_by_ref`.
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn import_not_found() {
    let resolver = MockFilesystem::new(vec![] as Vec<(&str, &str)>);
    check_import_fail(
        r#"
    merge "./a.zng";
    "#,
        expect![[r#"
            Error: Failed to process merge file `./a.zng`: File not found: ./a.zng
        "#]],
        &resolver,
    );
}

#[test]
fn import_has_mismatched_method_signature() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        "type A { #layout(size = 1, align = 1); fn foo(i32) -> i32; }",
    )]);

    check_import_fail(
        r#"
  merge "./a.zng";
  type A {
    #layout(size = 1, align = 1);
    fn foo(i64) -> i64;
  }
  "#,
        expect![[r#"
            Error: Method mismatch
               ╭─[ ./a.zng:1:6 ]
               │
             1 │ type A { #layout(size = 1, align = 1); fn foo(i32) -> i32; }
               │      ┬                                 ─────────┬─────────  
               │      ╰────────────────────────────────────────────────────── Method mismatch
               │                                                 │           
               │                                                 ╰─────────── declaration here ...
               │
               ├─[ test.zng:5:5 ]
               │
             5 │     fn foo(i64) -> i64;
               │     ─────────┬─────────  
               │              ╰─────────── conflicts with the declaration here.
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn import_has_mismatched_field() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        "type A {
        #layout(size = 1, align = 1);
        field x (offset = 0, type = i32);
    }",
    )]);

    check_import_fail(
        r#"
  merge "./a.zng";
  type A {
    #layout(size = 1, align = 1);
    field x (offset = 0, type = i64);
  }
  "#,
        expect![[r#"
            Error: Field mismatch
               ╭─[ ./a.zng:1:6 ]
               │
             1 │ type A {
               │      ┬  
               │      ╰── Field mismatch
               │ 
             3 │         field x (offset = 0, type = i32);
               │         ────────────────┬────────────────  
               │                         ╰────────────────── declaration here ...
               │
               ├─[ test.zng:5:5 ]
               │
             5 │     field x (offset = 0, type = i64);
               │     ────────────────┬────────────────  
               │                     ╰────────────────── conflicts with the declaration here.
            ───╯
        "#]],
        &resolver,
    );
}

#[test]
fn convert_panic_to_exception_in_imported_file_fails() {
    let resolver = MockFilesystem::new(vec![(
        "./imported.zng",
        r#"
        #convert_panic_to_exception
        type A {
            #layout(size = 1, align = 1);
        }
        "#,
    )]);

    check_import_fail(
        r#"
merge "./imported.zng";
type B {
    #layout(size = 1, align = 1);
}
        "#,
        expect![[r#"
            Error: Using `#convert_panic_to_exception` in imported zngur files is not supported. This directive can only be used in the main zngur file.
               ╭─[ ./imported.zng:2:10 ]
               │
             2 │         #convert_panic_to_exception
               │          ─────────────┬────────────  
               │                       ╰────────────── Using `#convert_panic_to_exception` in imported zngur files is not supported. This directive can only be used in the main zngur file.
            ───╯
        "#]],
        &resolver,
    );
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
        #layout(size = 27, align = 9);
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

    assert_layout(27, 9, &ty.layout);
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
            #layout(size = 27, align = 9);
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

    assert_layout(27, 9, &ty.layout);
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
fn match_pattern_multi_cfg_bad_pattern() {
    let source = r#"
#unstable(cfg_match)

#match (cfg!(feature.foo), cfg!(target_pointer_width)) {
    (Some, "32") => type crate::Foo32 {
        // would succeed if cfg match attempted
        #layout(size = 1, align = 1);
    }
    "64" => type crate::NoFoo64 {
        // will fail: cardinality of pattern and tuple don't match
        #layout(size = 1, align = 1);
    }
    _ => {
        type crate::SpecialFoo {
            #layout(size = 1, align = 1);
        }
    }
}
    "#;
    check_fail_with_cfg(
        source,
        InMemoryRustCfgProvider::default().with_values([("target_pointer_width", &["64"])]),
        expect![[r#"
            Error: Can not match single pattern against multiple cfg values.
               ╭─[ test.zng:9:5 ]
               │
             9 │     "64" => type crate::NoFoo64 {
               │     ──┬─  
               │       ╰─── Can not match single pattern against multiple cfg values.
            ───╯
        "#]],
    );
}

#[test]
fn match_pattern_multi_cfg_bad_pattern2() {
    let source = r#"
#unstable(cfg_match)

#match (cfg!(feature.foo), cfg!(target_pointer_width), cfg!(target_feature) ) {
    (Some, "32", "avx" & "avx2") => type crate::Foo32 {
        // would succeed if cfg match attempted
        #layout(size = 1, align = 1);
    }
    (None, "64") => type crate::NoFoo64 {
        // will fail: cardinality of pattern and tuple don't match
        #layout(size = 1, align = 1);
    }
    _ => {
        type crate::SpecialFoo {
            #layout(size = 1, align = 1);
        }
    }
}
    "#;
    let cfg: [(&str, &[&str]); 2] = [
        ("target_pointer_width", &["64"]),
        ("target_feature", &["avx", "avx2"]),
    ];
    check_fail_with_cfg(
        source,
        InMemoryRustCfgProvider::default().with_values(cfg),
        expect![[r#"
            Error: Number of patterns and number of scrutinees do not match.
               ╭─[ test.zng:9:5 ]
               │
             9 │     (None, "64") => type crate::NoFoo64 {
               │     ──────┬─────  
               │           ╰─────── Number of patterns and number of scrutinees do not match.
            ───╯
        "#]],
    );
}

#[test]
fn cfg_match_unstable() {
    let source = r#"
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
    check_fail_with_cfg(
        source,
        InMemoryRustCfgProvider::default().with_values([("feature", &["foo"])]),
        expect![[r#"
            Error: `#match` statements are unstable. Enable them by using `#unstable(cfg_match)` at the top of the file.
                ╭─[ test.zng:2:1 ]
                │
              2 │ ╭─▶ #match cfg!(feature) {
                ┆ ┆   
             11 │ ├─▶ }
                │ │       
                │ ╰─────── `#match` statements are unstable. Enable them by using `#unstable(cfg_match)` at the top of the file.
            ────╯
        "#]],
    );
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
fn crate_directive_sets_the_crate_name() {
    let parsed = parse_str(
        r#"
#crate my_bindings
"#,
        NullCfg,
        |_| {},
    );
    assert_eq!(parsed.spec.crate_name.as_deref(), Some("my_bindings"));
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
fn imported_module_cpp_types_and_crate_are_resolved() {
    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        r#"
#crate bar_bindings

type c++::ns::Bar {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}

type crate::RustOnly {
    #layout(size = 1, align = 1);
}
"#,
    )]);
    let parsed = parse_str_with_resolver(
        r#"
import "./bar.zng";

mod crate {
    fn use_bar(&const c++::ns::Bar);
}
"#,
        NullCfg,
        &resolver,
        |_| {},
    );
    let [module] = parsed.spec.imported_modules.as_slice() else {
        panic!("expected one imported module");
    };
    assert_eq!(module.crate_name.as_deref(), Some("bar_bindings"));
    // Only the `c++::` types, whose Rust wrappers live in `bar_bindings`.
    assert_eq!(
        module.cpp_types,
        vec![vec!["ns".to_owned(), "Bar".to_owned()]]
    );
    // `Bar` is held by value, so C++ functions of this module move it by value.
    assert_eq!(
        module.cpp_by_value_types,
        vec![(
            RustType::Cpp(CppTypePath {
                path: vec!["ns".to_owned(), "Bar".to_owned()],
                generics: vec![],
            }),
            "::ns::Bar".to_owned()
        )]
    );
    // The imported module's types aren't part of this module's spec.
    assert!(parsed.spec.types.is_empty());
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
fn extern_cpp_requires_safety() {
    let source = r#"
extern "C++" {
    fn foo();
}
    "#;
    check_fail(
        source,
        expect![[r#"
            Error: found 'fn' expected 'safe', 'unsafe', 'impl', or '}'
               ╭─[ test.zng:3:5 ]
               │
             3 │     fn foo();
               │     ─┬  
               │      ╰── found 'fn' expected 'safe', 'unsafe', 'impl', or '}'
            ───╯
        "#]],
    );
    let source = r#"
extern "C++" {
    impl crate::Foo {
        fn foo();
    }
}
    "#;
    check_fail(
        source,
        expect![[r#"
            Error: found 'fn' expected 'safe', 'unsafe', or '}'
               ╭─[ test.zng:4:9 ]
               │
             4 │         fn foo();
               │         ─┬  
               │          ╰── found 'fn' expected 'safe', 'unsafe', or '}'
            ───╯
        "#]],
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
    assert!(segs.generics.is_empty());
    assert_eq!(segs.path.as_slice(), expected);
}

#[test]
fn cpp_pointers_and_references_parse() {
    use zngur_def::{CppIndirection, Mutability, PrimitiveRustType};

    let parsed = parse_str(
        r#"
type c++::a::B {
}

extern "C++" {
    unsafe fn f(const c++::a::B&, c++::a::B&, const i32*, i32*) -> const c++::a::B*;
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
}

#[test]
fn const_cpp_template_arguments_parse() {
    use zngur_def::PrimitiveRustType;

    let parsed = parse_str(
        r#"
type c++::std::span<const i32> {
    #layout(size = 16, align = 8);
}
"#,
        NullCfg,
        |_| {},
    );
    assert_eq!(
        parsed.spec.types[0].ty,
        RustType::Cpp(CppTypePath {
            path: vec!["std".to_owned(), "span".to_owned()],
            generics: vec![RustType::CppConst(Box::new(RustType::Primitive(
                PrimitiveRustType::Int(32)
            )))],
        })
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
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used in a method's `use` path
               ╭─[ test.zng:4:42 ]
               │
             4 │     fn bar(self) -> usize use c++::a::Bar;
               │                                          ┬  
               │                                          ╰── `c++::` paths cannot be used in a method's `use` path
            ───╯
        "#]],
    );
}

#[test]
fn cpp_path_rejected_as_trait() {
    check_fail(
        r#"
extern "C++" {
    impl c++::Foo for crate::X {
    }
}
    "#,
        expect![[r#"
            Error: `c++::` paths cannot be used as a trait
               ╭─[ test.zng:3:19 ]
               │
             3 │     impl c++::Foo for crate::X {
               │                   ─┬─  
               │                    ╰─── `c++::` paths cannot be used as a trait
            ───╯
        "#]],
    );
}

#[test]
#[should_panic(expected = "a c++::-only path was used somewhere that can't support it")]
fn cpp_path_via_alias_indirection_as_trait_is_a_known_ice() {
    // The syntactic check above only catches an *explicit* `c++::` prefix in
    // trait position. An alias that itself targets a `c++::` path slips
    // past it (nothing about `MyTrait` looks like a `c++::` path until it's
    // resolved), and hits a deliberate `todo!()` instead of a clean
    // diagnostic -- see the `RustPathAndGenerics::to_zngur` `Cpp` arm.
    let _ = parse_str(
        r#"
use c++::Foo as MyTrait;

extern "C++" {
    impl MyTrait for crate::X {
    }
}
    "#,
        NullCfg,
        |_| {},
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
            Error: found '+' expected '::', '<', '*', '&', or '{'
               ╭─[ test.zng:2:8 ]
               │
             2 │ type c ++ :: Name {
               │        ┬  
               │        ╰── found '+' expected '::', '<', '*', '&', or '{'
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
    // module tree, so a top-level `fn` declaration inside `mod c++::a {
    // ... }` must be rejected rather than silently misinterpreting the
    // c++::-only path as if it were a real, absolute Rust path.
    check_fail(
        r#"
mod c++::a {
    fn foo(i32) -> bool;
}
    "#,
        expect![[r#"
            Error: a free function cannot be declared inside a c++:: scope (declare C++ functions as `safe fn` or `unsafe fn`)
               ╭─[ test.zng:3:5 ]
               │
             3 │     fn foo(i32) -> bool;
               │     ─────────┬─────────  
               │              ╰─────────── a free function cannot be declared inside a c++:: scope (declare C++ functions as `safe fn` or `unsafe fn`)
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
        #layout(size = 16, align = 8);
        field s (offset = auto, type = MyString);
    }
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    // The type's own declaration still composes onto the c++:: prefix:
    assert_cpp(&["a", "Name"], &ty.ty);
    // But the aliased field type resolves as the real Rust path, not
    // `Cpp(["a", "s"])`:
    let field = ty.fields.first().expect("no field parsed");
    assert_ty_path!(["std", "string", "String"], &field.ty);
}

#[test]
fn scope_reference_to_cpp_target_from_top_level_cpp_base_is_bare() {
    // At the root of the c++::-only tree (an empty `Cpp` base), there's
    // nothing to climb out of, so the reference is just the target's own
    // segments.
    let scope = scope_with_base(EntityPath::Cpp(Vec::new()));
    assert_eq!(
        scope.reference_to(&EntityPath::cpp(["foo", "Bar"])),
        Some("foo::Bar".to_owned()),
    );
}

fn scope_with_base(base: EntityPath) -> Scope<'static> {
    Scope {
        aliases: Vec::new(),
        base,
        type_vars: Default::default(),
    }
}

#[test]
fn scope_reference_to_rust_target_is_always_reachable_regardless_of_base() {
    // A `Rust` target is crate- or globally-qualified, so it's reachable the
    // same way no matter what the referencing scope's own base is -- even
    // from a `Cpp` base.
    let scope = scope_with_base(EntityPath::cpp(["a", "b"]));
    assert_eq!(
        scope.reference_to(&EntityPath::crate_relative(["foo", "Bar"])),
        Some("crate::foo::Bar".to_owned()),
    );
    assert_eq!(
        scope.reference_to(&EntityPath::rust(["foo", "Bar"])),
        Some("::foo::Bar".to_owned()),
    );
}

#[test]
fn scope_reference_to_cpp_target_from_rust_base_is_impossible() {
    // We don't know which module the c++::-only tree will itself be
    // generated into relative to an arbitrary Rust path, so there's no way
    // to reference it from a `Rust` base.
    let scope = scope_with_base(EntityPath::Rust(Vec::new()));
    assert_eq!(scope.reference_to(&EntityPath::cpp(["a", "Foo"])), None);
}

#[test]
fn scope_reference_to_cpp_target_from_cpp_base_uses_super_as_needed() {
    let scope = scope_with_base(EntityPath::cpp(["a", "b"]));
    // Different branch entirely: climb out twice, then descend.
    assert_eq!(
        scope.reference_to(&EntityPath::cpp(["x", "y"])),
        Some("super::super::x::y".to_owned()),
    );
    // Sibling module under the shared parent `a`: climb out once.
    assert_eq!(
        scope.reference_to(&EntityPath::cpp(["a", "c"])),
        Some("super::c".to_owned()),
    );
    // Child of the current base: no climbing needed at all.
    assert_eq!(
        scope.reference_to(&EntityPath::cpp(["a", "b", "Name"])),
        Some("Name".to_owned()),
    );
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
    // `this` and the other arguments are references, which refer to valid objects.
    let parsed = parse_str(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn value(&const self) -> i32;
    safe fn set(&unique var self, const c++::B&);
}
"#,
        NullCfg,
        |_| {},
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
    unsafe fn value() const -> i32;
}
"#,
        expect![[r#"
            Error: C++ member functions take an explicit receiver instead of a trailing `const`: `&'a const self` for `const` member functions, and `&'a unique var self` (or `&'a var self`) for the others
               ╭─[ test.zng:4:5 ]
               │
             4 │     unsafe fn value() const -> i32;
               │     ───────────────┬───────────────  
               │                    ╰───────────────── C++ member functions take an explicit receiver instead of a trailing `const`: `&'a const self` for `const` member functions, and `&'a unique var self` (or `&'a var self`) for the others
            ───╯
        "#]],
    );
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
fn cpp_constructors_parse() {
    use zngur_def::{CppIndirection, Mutability, PrimitiveRustType};

    let parsed = parse_str(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;

    safe constructor();
    safe constructor(i32, i32);
    unsafe constructor(const c++::B&);
}
"#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let b = RustType::Cpp(vec!["B".to_owned()].into());
    let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
    let constructors = &ty.cpp_constructors;
    assert_eq!(constructors.len(), 3);
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
}

#[test]
fn safe_cpp_constructor_with_cpp_ptr_warns() {
    check_warnings(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor(const c++::B*);
}
"#,
        expect![[r#"
            Warning: this C++ constructor is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe constructor(const c++::B*);
               │     ────────────────┬───────────────  
               │                     ╰───────────────── this C++ constructor is `safe` but takes a raw pointer, which may be null or invalid; it should probably be `unsafe` (silence this with `#allow(safe_raw_pointer)`)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_constructors_with_the_same_rust_signature_are_rejected() {
    check_fail(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor(i32);
    unsafe constructor(i32);
}
"#,
        expect![[r#"
            Error: another C++ constructor has the same argument types
               ╭─[ test.zng:6:5 ]
               │
             6 │     unsafe constructor(i32);
               │     ────────────┬───────────  
               │                 ╰───────────── another C++ constructor has the same argument types
            ───╯
        "#]],
    );
}

#[test]
fn cpp_constructors_taking_a_pointer_or_a_reference_are_different() {
    let parsed = parse_str(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe constructor(const c++::B&);
    unsafe constructor(const c++::B*);
}
"#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_eq!(ty.cpp_constructors.len(), 2);
}

#[test]
fn cpp_constructor_on_cpp_ref_type_is_rejected() {
    check_fail(
        r#"
type c++::B {
    safe constructor(i32);
}
"#,
        expect![[r#"
            Error: C++ constructors can only be declared on `c++::` types that Rust holds by value (with a layout and `#trivially_relocatable`)
               ╭─[ test.zng:2:6 ]
               │
             2 │ type c++::B {
               │      ───┬──  
               │         ╰──── C++ constructors can only be declared on `c++::` types that Rust holds by value (with a layout and `#trivially_relocatable`)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_functions_parse() {
    use zngur_def::{CppIndirection, Mutability, PrimitiveRustType};

    let parsed = parse_str(
        r#"
mod c++::a {
    type B {
        #layout(size = 8, align = 4);
        #trivially_relocatable;
    }

    mod b {
        unsafe fn f(const c++::a::B&) -> i32;
        safe fn g(i32) -> c++::a::B as make_b;
        safe fn h();
    }
}
"#,
        NullCfg,
        |_| {},
    );
    let funcs = &parsed.spec.cpp_funcs;
    assert_eq!(funcs.len(), 3);
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
}

#[test]
fn cpp_function_outside_cpp_scope_is_rejected() {
    check_fail(
        r#"
mod crate::a {
    safe fn f(i32);
}
"#,
        expect![[r#"
            Error: C++ functions (`safe fn` / `unsafe fn`) can only be declared inside a `mod c++::...` scope, which is their C++ namespace
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn f(i32);
               │     ───────┬──────  
               │            ╰──────── C++ functions (`safe fn` / `unsafe fn`) can only be declared inside a `mod c++::...` scope, which is their C++ namespace
            ───╯
        "#]],
    );
}

#[test]
fn safe_cpp_function_with_cpp_ptr_warns() {
    check_warnings(
        r#"
mod c++::a {
    safe fn f(i32*);
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
        "#]],
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
    field &unique var x (type = i32) as x_mut;
    field &var x (type = i32) as x_var;
    field &unique const x (type = i32) as x_unique_const;
    field const x (type = i32) as x_const_copy;
    field var x (type = i32) as x_copy;
}
"#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
    let fields = &ty.cpp_fields;
    assert_eq!(fields.len(), 6);
    assert_eq!(fields[0].name, "x");
    assert_eq!(fields[0].rust_name(), "x");
    assert_eq!(fields[0].ty, i32);
    let reference = |unique, constness| CppFieldAccess::Ref { unique, constness };
    assert_eq!(fields[0].access, reference(false, CppConstness::Const));
    assert_eq!(fields[1].name, "x");
    assert_eq!(fields[1].rust_name(), "x_mut");
    assert_eq!(fields[1].access, reference(true, CppConstness::Var));
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
               ╭─[ test.zng:4:6 ]
               │
             4 │ type c++::B {
               │      ───┬──  
               │         ╰──── the accessor `a` returns a copy of the field `a`, but Rust doesn't hold `c++::A` by value (it needs a layout and `#trivially_relocatable`); return a reference instead (`field &const a ...`)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_field_of_a_rust_type_is_rejected() {
    check_fail(
        r#"
type c++::B {
    field &const s (type = ::std::string::String);
}
"#,
        expect![[r#"
            Error: the field `s` has the type `::std::string::String`, but C++ fields are C++ objects: `c++::` types, builtins (other than `char` and `str`) or C++ pointers
               ╭─[ test.zng:3:5 ]
               │
             3 │     field &const s (type = ::std::string::String);
               │     ───────────────────────┬──────────────────────  
               │                            ╰──────────────────────── the field `s` has the type `::std::string::String`, but C++ fields are C++ objects: `c++::` types, builtins (other than `char` and `str`) or C++ pointers
            ───╯
        "#]],
    );
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
               ╭─[ test.zng:2:6 ]
               │
             2 │ type crate::Foo {
               │      ─────┬────  
               │           ╰────── C++ field accessors (`field name (type = T)`) can only be declared on `c++::` types
            ───╯
        "#]],
    );
}

#[test]
fn cpp_template_instantiations_parse() {
    use zngur_def::{CppIndirection, CppTypePath, Mutability, PrimitiveRustType};

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
fn cpp_type_capabilities_follow_from_layout_and_trivially_relocatable() {
    let parsed = parse_str(
        r#"
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
        |_| {},
    );
    let ty = |name: &str| {
        parsed
            .spec
            .types
            .iter()
            .find(|t| matches!(&t.ty, RustType::Cpp(p) if p.path.last().unwrap() == name))
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
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert!(ty.cpp_by_value_layout().is_some());
}

#[test]
fn trivially_relocatable_without_layout_is_rejected() {
    check_fail(
        r#"
type c++::B {
    #trivially_relocatable;
}
"#,
        expect![[r#"
            Error: `#trivially_relocatable` needs a `#layout(...)` or `#layout_conservative(...)`, which Rust needs to hold the C++ type by value
               ╭─[ test.zng:2:6 ]
               │
             2 │ type c++::B {
               │      ───┬──  
               │         ╰──── `#trivially_relocatable` needs a `#layout(...)` or `#layout_conservative(...)`, which Rust needs to hold the C++ type by value
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
"#,
        expect![[r#"
            Error: `#trivially_relocatable` is only for `c++::` types (Rust types are always trivially relocatable)
               ╭─[ test.zng:4:5 ]
               │
             4 │     #trivially_relocatable;
               │     ───────────┬───────────  
               │                ╰───────────── `#trivially_relocatable` is only for `c++::` types (Rust types are always trivially relocatable)
            ───╯
        "#]],
    );
}

#[test]
fn heap_allocated_and_only_by_ref_on_cpp_type_are_rejected() {
    check_fail(
        r#"
type c++::A {
    #heap_allocated;
}

type c++::B {
    #only_by_ref;
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
        "#]],
    );
}

#[test]
fn removed_cpp_directives_explain_the_new_syntax() {
    check_fail(
        r#"
type crate::A {
    #layout(size = 8, align = 4);
    #cpp_stack_owned "::A";
}

type c++::B {
    #cpp_ref;
}

type crate::C {
    #layout(size = 8, align = 8);
    #cpp_heap_allocated "::C";
}

type crate::D {
    #layout(size = 8, align = 8);
    #cpp_value "0" "::D";
}
"#,
        expect![[r#"
            Error: `#cpp_stack_owned` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
               ╭─[ test.zng:4:5 ]
               │
             4 │     #cpp_stack_owned "::A";
               │     ───────────┬───────────  
               │                ╰───────────── no longer supported
               │ 
               │ Help: declare the type as `type c++::ns::T { #layout(size = X, align = Y); #trivially_relocatable; }`; Rust holds C++ types by value when they have a layout and are trivially relocatable
            ───╯
            Error: `#cpp_ref` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
               ╭─[ test.zng:8:5 ]
               │
             8 │     #cpp_ref;
               │     ────┬────  
               │         ╰────── no longer supported
               │ 
               │ Help: declare the type as `type c++::ns::T { ... }` with no layout; Rust can then only refer to it
            ───╯
            Error: `#cpp_heap_allocated` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
                ╭─[ test.zng:13:5 ]
                │
             13 │     #cpp_heap_allocated "::C";
                │     ─────────────┬────────────  
                │                  ╰────────────── no longer supported
                │ 
                │ Help: declare the type as `type c++::ns::T { ... }` with no layout, and own it through a C++ smart pointer, e.g. `type c++::std::unique_ptr<c++::ns::T> { #layout(size = 8, align = 8); #trivially_relocatable; }`
            ────╯
            Error: `#cpp_value` is no longer supported: C++ types are `c++::` types, whose C++ name is their path
                ╭─[ test.zng:18:5 ]
                │
             18 │     #cpp_value "0" "::D";
                │     ──────────┬──────────  
                │               ╰──────────── no longer supported
                │ 
                │ Help: declare the type as `type c++::ns::T { ... }` with no layout, and own it through a C++ smart pointer, e.g. `type c++::std::unique_ptr<c++::ns::T> { #layout(size = 8, align = 8); #trivially_relocatable; }`
            ────╯
        "#]],
    );
}

#[test]
fn files_with_only_comments_parse() {
    for text in [
        "",
        "// Only a comment.",
        "// Only a comment, with a final newline.\n",
        "\n// Two comments,\n// and no items, with an apostrophe: isn't it?\n\n",
    ] {
        let parsed = parse_str(text, NullCfg, |_| {});
        assert!(parsed.spec.types.is_empty(), "{text:?}");
    }
}

#[test]
fn comments_after_the_last_item_parse() {
    let parsed = parse_str(
        r#"
type c++::a::B {
}
// A trailing comment.
"#,
        NullCfg,
        |_| {},
    );
    assert_eq!(parsed.spec.types.len(), 1);
}

#[test]
fn cpp_names_can_be_zngur_keywords() {
    let parsed = parse_str(
        r#"
type c++::a::B {
    field &const type (type = i32) as type_;
    unsafe fn match(&const self, i32) -> i32 as match_;
}

mod c++::a {
    safe fn use(i32) as use_;
}
"#,
        NullCfg,
        |_| {},
    );
    let ty = &parsed.spec.types[0];
    assert_eq!(ty.cpp_fields[0].name, "type");
    assert_eq!(ty.cpp_fields[0].rust_name(), "type_");
    assert_eq!(ty.cpp_methods[0].cpp_name, "match");
    assert_eq!(ty.cpp_methods[0].rust_name(), "match_");
    assert_eq!(parsed.spec.cpp_funcs[0].cpp_name, "use");
    assert_eq!(parsed.spec.cpp_funcs[0].rust_name(), "use_");
}

#[test]
fn cpp_function_template_args_parse() {
    let parsed = parse_str(
        r#"
type c++::geo::Point {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn get<i32>(&const self) -> i32 as get_i32;
}

type c++::std::unique_ptr<c++::geo::Point> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
}

mod c++::std {
    safe fn make_unique<c++::geo::Point>(i32, i32) -> c++::std::unique_ptr<c++::geo::Point> as make_point;
}
"#,
        NullCfg,
        |_| {},
    );
    let point = RustType::Cpp(vec!["geo".to_owned(), "Point".to_owned()].into());
    let [func] = parsed.spec.cpp_funcs.as_slice() else {
        panic!("expected one C++ function");
    };
    assert_eq!(func.cpp_name, "make_unique");
    assert_eq!(func.template_args, vec![point]);
    assert_eq!(func.cpp_path(), "::std::make_unique< ::geo::Point >");
    let method = &parsed.spec.types[0].cpp_methods[0];
    assert_eq!(
        method.template_args,
        vec![RustType::Primitive(PrimitiveRustType::Int(32))]
    );
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
        |_| {},
    );
    let a = || Lifetime("a".to_owned());
    let i32 = || Box::new(RustType::Primitive(PrimitiveRustType::Int(32)));
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
fn undeclared_type_lifetime_is_rejected() {
    check_fail(
        r#"
type crate::Parser<'a> {
    #layout(size = 24, align = 8);
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
        "#]],
    );
}

#[test]
fn duplicate_lifetime_is_rejected() {
    check_fail(
        r#"
type<'a> crate::Parser<'a> {
    #layout(size = 24, align = 8);
    fn rest<'a>(&'a self) -> &'a str;
}
"#,
        expect![[r#"
            Error: lifetime `'a` is declared twice
               ╭─[ test.zng:4:5 ]
               │
             4 │     fn rest<'a>(&'a self) -> &'a str;
               │     ────────────────┬────────────────  
               │                     ╰────────────────── lifetime `'a` is declared twice
            ───╯
        "#]],
    );
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
fn lifetime_after_type_argument_is_rejected() {
    check_fail(
        r#"
type ::std::slice::Iter<i32, 'static> {
    #layout(size = 16, align = 8);
}
"#,
        expect![[r#"
            Error: lifetime arguments must come before type arguments
               ╭─[ test.zng:2:6 ]
               │
             2 │ type ::std::slice::Iter<i32, 'static> {
               │      ────────────────┬───────────────  
               │                      ╰───────────────── lifetime arguments must come before type arguments
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
    use zngur_def::{CppIndirection, Lifetime, ZngurMethodReceiver};

    let parsed = parse_str(
        r#"
type c++::B {
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
    use zngur_def::CppConstness;
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
fn references_to_cpp_objects_parse() {
    use zngur_def::{CppConstness, PrimitiveRustType};

    let parsed = parse_str(
        r#"
type c++::B {
    safe fn f<'a>(&'a const self, &'a const c++::B, &'a var c++::B, &'a unique var c++::B, &'a unique const c++::B);
    safe fn g<'a>(&'a const self, &'a const i32, &'a var i32, &'a unique var i32, &'a i32, &'a mut i32) -> &'a var bool;
    unsafe fn h<'a>(&'a const self, &'a const c++::B*, c++::B*, const i32*);
}
"#,
        NullCfg,
        |_| {},
    );
    let methods = &parsed.spec.types[0].cpp_methods;
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
        methods[0].inputs,
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
        methods[1].inputs,
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
        methods[2].inputs,
        [
            // `&'a const c++::B*` is a reference to a `const` pointer (`B* const&`).
            reference(Mutability::Not, CppConstness::Const, &b_ptr),
            b_ptr.clone(),
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, i32.clone()),
        ]
    );
}

#[test]
fn references_to_cpp_objects_through_aliases_and_templates() {
    check_success(
        r#"
use c++::a::B as B;

type c++::a::B {
    safe fn f<'a>(&'a const self, &'a const B) -> &'a var B;
}

type<T> c++::Box<T> {
    safe fn get<'a>(&'a const self) -> &'a const T;
}

type c++::Box<c++::a::B> {}
type c++::Box<i32> {}
type c++::Box<const i32> {}
"#,
    );
}

#[test]
fn reference_to_cpp_class_without_constness_is_rejected() {
    check_fail(
        r#"
type c++::B {
    safe fn f<'a>(&'a const self, &'a c++::B);
}
"#,
        expect![[r#"
            Error: a reference to a C++ class must say whether the object is `const`: `&'a const c++::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&'a var c++::B` (a shared `T&`, `&T` in Rust)
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn f<'a>(&'a const self, &'a c++::B);
               │     ─────────────────────┬────────────────────  
               │                          ╰────────────────────── a reference to a C++ class must say whether the object is `const`: `&'a const c++::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&'a var c++::B` (a shared `T&`, `&T` in Rust)
            ───╯
        "#]],
    );
}

#[test]
fn mutable_reference_to_cpp_class_is_rejected() {
    check_fail(
        r#"
mod c++::ns {
    safe fn f<'a>(&'a mut c++::B);
}
"#,
        expect![[r#"
            Error: a reference to a C++ class must say whether the object is `const`: `&'a unique var c++::B` (a `T&` that Rust uses uniquely, `&mut T` in Rust)
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn f<'a>(&'a mut c++::B);
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

type c++::a::B {
    safe fn f<'a>(&'a const self, &'a B);
}
"#,
        expect![[r#"
            Error: a reference to a C++ class must say whether the object is `const`: `&'a const c++::a::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&'a var c++::a::B` (a shared `T&`, `&T` in Rust)
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe fn f<'a>(&'a const self, &'a B);
               │     ──────────────────┬──────────────────  
               │                       ╰──────────────────── a reference to a C++ class must say whether the object is `const`: `&'a const c++::a::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&'a var c++::a::B` (a shared `T&`, `&T` in Rust)
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
type c++::B {
    safe fn f<'a>(&'a const self, &'a const crate::Foo);
}
"#,
        expect![[r#"
            Error: only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`, and `crate::Foo` is a Rust type: use `&'a crate::Foo`
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn f<'a>(&'a const self, &'a const crate::Foo);
               │     ──────────────────────────┬─────────────────────────  
               │                               ╰─────────────────────────── only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`, and `crate::Foo` is a Rust type: use `&'a crate::Foo`
            ───╯
        "#]],
    );
}

#[test]
fn var_reference_to_str_is_rejected() {
    check_fail(
        r#"
type c++::B {
    safe fn f<'a>(&'a const self, &'a unique var str);
}
"#,
        expect![[r#"
            Error: only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`, and `str` is a Rust type: use `&'a mut str`
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn f<'a>(&'a const self, &'a unique var str);
               │     ─────────────────────────┬────────────────────────  
               │                              ╰────────────────────────── only C++ objects (`c++::` types, builtins and C++ pointers) can be `const` or `var`, and `str` is a Rust type: use `&'a mut str`
            ───╯
        "#]],
    );
}

#[test]
fn const_reference_to_char_is_rejected() {
    check_fail(
        r#"
type c++::B {
    safe fn f<'a>(&'a const self) -> &'a const char;
}
"#,
        expect![[r#"
            Error: `char` can't be a C++ object (C++ can store invalid code points in it), so references to it can't be `const` or `var`: use `&'a char` (a Rust value)
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn f<'a>(&'a const self) -> &'a const char;
               │     ────────────────────────┬───────────────────────  
               │                             ╰───────────────────────── `char` can't be a C++ object (C++ can store invalid code points in it), so references to it can't be `const` or `var`: use `&'a char` (a Rust value)
            ───╯
        "#]],
    );
}

#[test]
fn unique_reference_without_constness_is_rejected() {
    check_fail(
        r#"
type c++::B {
    safe fn f<'a>(&'a const self, &'a unique c++::B);
}
"#,
        expect![[r#"
            Error: a `unique` reference is a reference to a C++ object, which says whether it's `const`: `&'a unique var T` (a C++ `T&` that Rust uses uniquely, `&mut T` in Rust) or `&'a unique const T`; a reference to a Rust value is `&'a mut T`
               ╭─[ test.zng:3:35 ]
               │
             3 │     safe fn f<'a>(&'a const self, &'a unique c++::B);
               │                                   ────────┬────────  
               │                                           ╰────────── a `unique` reference is a reference to a C++ object, which says whether it's `const`: `&'a unique var T` (a C++ `T&` that Rust uses uniquely, `&mut T` in Rust) or `&'a unique const T`; a reference to a Rust value is `&'a mut T`
            ───╯
        "#]],
    );
}

#[test]
fn mut_const_reference_is_rejected() {
    check_fail(
        r#"
type c++::B {
    safe fn f<'a>(&'a const self, &'a mut const c++::B);
}
"#,
        expect![[r#"
            Error: references to C++ objects say `unique` rather than `mut`: write `&'a unique const T` (or `&'a const T` for a shared reference)
               ╭─[ test.zng:3:35 ]
               │
             3 │     safe fn f<'a>(&'a const self, &'a mut const c++::B);
               │                                   ──────────┬─────────  
               │                                             ╰─────────── references to C++ objects say `unique` rather than `mut`: write `&'a unique const T` (or `&'a const T` for a shared reference)
            ───╯
        "#]],
    );
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
fn const_receiver_on_rust_method_is_rejected() {
    check_fail(
        r#"
type crate::Foo {
    #layout(size = 0, align = 1);
    fn f(&const self);
}
"#,
        expect![[r#"
            Error: `const self` / `var self` receivers are for C++ member functions (which start with `safe` or `unsafe`); methods of Rust types and `extern "C++"` impls take `&self` or `&mut self`
               ╭─[ test.zng:4:5 ]
               │
             4 │     fn f(&const self);
               │     ────────┬────────  
               │             ╰────────── `const self` / `var self` receivers are for C++ member functions (which start with `safe` or `unsafe`); methods of Rust types and `extern "C++"` impls take `&self` or `&mut self`
            ───╯
        "#]],
    );
}

#[test]
fn template_instantiation_with_a_cpp_class_reference_without_constness_is_rejected() {
    check_fail(
        r#"
type c++::a::B {}

type<T> c++::Box<T> {
    safe fn get<'a>(&'a const self) -> &'a T;
}

type c++::Box<i32> {}
type c++::Box<c++::a::B> {}
"#,
        expect![[r#"
            Error: in the instantiation `c++::Box<c++::a::B>` of a template, in `get`: a reference to a C++ class must say whether the object is `const`: `&'a const c++::a::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&'a var c++::a::B` (a shared `T&`, `&T` in Rust)
               ╭─[ test.zng:9:6 ]
               │
             9 │ type c++::Box<c++::a::B> {}
               │      ─────────┬─────────  
               │               ╰─────────── in the instantiation `c++::Box<c++::a::B>` of a template, in `get`: a reference to a C++ class must say whether the object is `const`: `&'a const c++::a::B` (a C++ `const T&`, `&T<Const>` in Rust) or `&'a var c++::a::B` (a shared `T&`, `&T` in Rust)
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
fn unknown_allow_is_rejected() {
    check_fail(
        r#"
#allow(everything)
"#,
        expect![[r#"
            Error: unknown warning `everything` (known ones: elided_lifetimes, safe_raw_pointer)
               ╭─[ test.zng:2:1 ]
               │
             2 │ #allow(everything)
               │ ─────────┬────────  
               │          ╰────────── unknown warning `everything` (known ones: elided_lifetimes, safe_raw_pointer)
            ───╯
        "#]],
    );
}
