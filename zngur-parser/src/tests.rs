use std::panic::catch_unwind;

use expect_test::{Expect, expect};
use zngur_def::{
    CppHeapAllocated, CppRef, CppStackOwned, LayoutPolicy, RustPathAndGenerics, RustType, ZngurSpec,
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

#[test]
fn cpp_ref_should_not_need_layout_info() {
    check_fail(
        r#"
type crate::Way {
    #layout(size = 1, align = 2);

    #cpp_ref "::osmium::Way";
}
    "#,
        expect![[r#"
            Error: Duplicate layout policy found
               ╭─[ test.zng:3:5 ]
               │
             3 │     #layout(size = 1, align = 2);
               │     ──────────────┬─────────────  
               │                   ╰─────────────── Duplicate layout policy found
            ───╯
        "#]],
    );
    check_success(
        r#"
type crate::Way {
    #cpp_ref "::osmium::Way";
}
    "#,
    );
}

#[test]
fn cpp_heap_allocated_directive_parses() {
    let result = parse_str(
        r#"
type crate::Way {
    #layout(size = 16, align = 8);
    #cpp_heap_allocated "::osmium::Way";
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = result.spec.types.first().expect("no type parsed");
    assert_eq!(
        ty.cpp_heap_allocated,
        Some(CppHeapAllocated("::osmium::Way".to_owned())),
    );
}

#[test]
fn cpp_stack_owned_valid_exact_layout() {
    let result = parse_str(
        r#"
type crate::Way {
    #layout(size = 16, align = 8);
    #cpp_stack_owned "::osmium::Way";
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = result.spec.types.first().expect("no type parsed");
    assert_eq!(
        ty.cpp_stack_owned,
        Some(CppStackOwned("::osmium::Way".to_owned()))
    );
    assert_eq!(
        ty.layout,
        Some(LayoutPolicy::StackAllocated { size: 16, align: 8 })
    );
}

#[test]
fn cpp_stack_owned_valid_conservative_layout() {
    let result = parse_str(
        r#"
type crate::Way {
    #layout_conservative(size = 32, align = 16);
    #cpp_stack_owned "::osmium::Way";
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = result.spec.types.first().expect("no type parsed");
    assert_eq!(
        ty.cpp_stack_owned,
        Some(CppStackOwned("::osmium::Way".to_owned()))
    );
    assert_eq!(
        ty.layout,
        Some(LayoutPolicy::Conservative {
            size: 32,
            align: 16
        })
    );
}

#[test]
fn cpp_stack_owned_error_with_heap_allocated() {
    check_fail(
        r#"
type crate::Way {
    #heap_allocated;
    #cpp_stack_owned "::osmium::Way";
}
    "#,
        expect![[r#"
            Error: `#cpp_stack_owned` cannot be used with `#heap_allocated` for type crate::Way.
               ╭─[ test.zng:4:5 ]
               │
             4 │     #cpp_stack_owned "::osmium::Way";
               │     ────────────────┬────────────────  
               │                     ╰────────────────── `#cpp_stack_owned` declared here
               │
               ├─[ test.zng:4:5 ]
               │
             3 │     #heap_allocated;
               │     ───────┬───────  
               │            ╰───────── `#heap_allocated` declared here
               │ 
               │ Note: `#cpp_stack_owned` types must use `#layout` or `#layout_conservative`.
            ───╯
        "#]],
    );
}

#[test]
fn cpp_stack_owned_error_with_only_by_ref() {
    check_fail(
        r#"
type crate::Way {
    #only_by_ref;
    #cpp_stack_owned "::osmium::Way";
}
    "#,
        expect![[r#"
            Error: `#cpp_stack_owned` cannot be used with `#only_by_ref` for type crate::Way.
               ╭─[ test.zng:4:5 ]
               │
             4 │     #cpp_stack_owned "::osmium::Way";
               │     ────────────────┬────────────────  
               │                     ╰────────────────── `#cpp_stack_owned` declared here
               │
               ├─[ test.zng:4:5 ]
               │
             3 │     #only_by_ref;
               │     ──────┬─────  
               │           ╰─────── `#only_by_ref` declared here
               │ 
               │ Note: `#cpp_stack_owned` types must use `#layout` or `#layout_conservative`.
            ───╯
        "#]],
    );
}

#[test]
fn cpp_stack_owned_error_with_no_layout() {
    check_fail(
        r#"
type crate::Way {
    #cpp_stack_owned "::osmium::Way";
}
    "#,
        expect![[r#"
            Error: No layout policy found for type crate::Way.
               ╭─[ test.zng:1:1 ]
               │
             2 │ type crate::Way {
               │      ─────┬────  
               │           ╰────── Type first declared here.
               │ 
               │ Note 1: Use one of `#layout(size = X, align = Y)`, `#heap_allocated` or `#only_by_ref`.
               │ 
               │ Note 2: Type is declared with `#cpp_stack_owned` which requires a layout directive like `#layout(size = X, align = Y)` or `#layout_conservative(size = X, align = Y)`.
            ───╯
        "#]],
    );
}

#[test]
fn cpp_stack_owned_old_syntax_error_with_help() {
    check_fail(
        r#"
type crate::Way {
    #cpp_stack_owned "::osmium::Way" (size = 16, align = 8);
}
    "#,
        expect![[r#"
            Error: Old syntax `#cpp_stack_owned` with layout properties is no longer supported.
               ╭─[ test.zng:3:5 ]
               │
             3 │     #cpp_stack_owned "::osmium::Way" (size = 16, align = 8);
               │     ────────────────────────────┬───────────────────────────  
               │                                 ╰───────────────────────────── layout parameters no longer accepted here
               │ 
               │ Help: layout moved to its own directive: `#cpp_stack_owned "::osmium::Way"; #layout(size = 16, align = 8);`
            ───╯
            Error: No layout policy found for type crate::Way.
               ╭─[ test.zng:1:1 ]
               │
             2 │ type crate::Way {
               │      ─────┬────  
               │           ╰────── Type first declared here.
               │ 
               │ Note: Use one of `#layout(size = X, align = Y)`, `#heap_allocated` or `#only_by_ref`.
            ───╯
        "#]],
    );
}

#[test]
fn cpp_value_emits_deprecation_warning_and_forwards_to_cpp_heap_allocated() {
    let mut warnings = Vec::new();
    let result = parse_str(
        r#"
type crate::Way {
    #layout(size = 16, align = 8);
    #cpp_value "0" "::osmium::Way";
}
    "#,
        NullCfg,
        &mut |w: &str| warnings.push(w.to_owned()),
    );
    assert_eq!(warnings.len(), 1);
    expect![[r#"
        Warning: #cpp_value is deprecated; use #cpp_heap_allocated instead
           ╭─[ test.zng:4:5 ]
           │
         4 │     #cpp_value "0" "::osmium::Way";
           │     ───────────────┬───────────────  
           │                    ╰───────────────── #cpp_value is deprecated; use #cpp_heap_allocated instead
        ───╯
    "#]].assert_eq(&warnings[0]);
    let ty = result.spec.types.first().expect("no type parsed");
    assert_eq!(
        ty.cpp_heap_allocated,
        Some(CppHeapAllocated("::osmium::Way".to_owned())),
    );
}

#[test]
fn cpp_value_and_cpp_heap_allocated_merge_as_equivalent_across_files() {
    let resolver = MockFilesystem::new(vec![(
        "./a.zng",
        r#"
type crate::Way {
    #layout(size = 16, align = 8);
    #cpp_value "0" "::osmium::Way";
}
    "#,
    )]);

    let mut warnings = Vec::new();
    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
type crate::Way {
    #cpp_heap_allocated "::osmium::Way";
}
    "#,
        NullCfg,
        &resolver,
        &mut |w: &str| warnings.push(w.to_owned()),
    );
    // The #cpp_value directive that triggered this warning lives in the imported
    // a.zng, not the root file, so the rendered warning must show a.zng's own
    // source snippet -- this exercises the cross-file source lookup used when
    // rendering a warning that was merged in from an imported file's context.
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("a.zng"));
    assert!(warnings[0].contains(r#"#cpp_value "0" "::osmium::Way";"#));
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_eq!(
        ty.cpp_heap_allocated,
        Some(CppHeapAllocated("::osmium::Way".to_owned())),
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
    #cpp_stack_owned;
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
    fn use_bar(&c++::ns::Bar);
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
    // The imported module's types aren't part of this module's spec.
    assert!(parsed.spec.types.is_empty());
}

#[test]
fn imported_module_with_cpp_types_needs_a_crate() {
    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        r#"
type c++::ns::Bar {
    #cpp_ref;
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
fn cpp_path_with_cpp_heap_allocated_parses() {
    let parsed = parse_str(
        r#"
type c++::a::b::Name {
    #layout(size = 16, align = 8);
    #cpp_heap_allocated "::foo::Bar";
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_cpp(&["a", "b", "Name"], &ty.ty);
    // The c++:: segments are a Rust-side placement hint only; the C++-side path
    // in #cpp_heap_allocated's string argument is independent of them.
    assert_eq!(
        ty.cpp_heap_allocated,
        Some(CppHeapAllocated("::foo::Bar".to_owned())),
    );
}

#[test]
fn cpp_path_with_cpp_ref_parses_and_forces_zero_sized_layout() {
    let parsed = parse_str(
        r#"
type c++::a::b::Name {
    #cpp_ref "::foo::Bar";
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_cpp(&["a", "b", "Name"], &ty.ty);
    assert_eq!(ty.cpp_ref, Some(CppRef("::foo::Bar".to_owned())));
    assert_eq!(ty.layout, Some(LayoutPolicy::ZERO_SIZED_TYPE));
}

#[test]
fn cpp_path_with_cpp_stack_owned_parses() {
    let parsed = parse_str(
        r#"
type c++::a::b::Name {
    #cpp_stack_owned "::foo::Bar";
    #layout(size = 8, align = 4);
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    assert_cpp(&["a", "b", "Name"], &ty.ty);
    assert_eq!(
        ty.cpp_stack_owned,
        Some(CppStackOwned("::foo::Bar".to_owned())),
    );
    assert_eq!(
        ty.layout,
        Some(LayoutPolicy::StackAllocated { size: 8, align: 4 }),
    );
}

#[test]
fn cpp_type_name_defaults_to_the_cpp_path() {
    let parsed = parse_str(
        r#"
type c++::a::b::Stack {
    #cpp_stack_owned;
    #layout(size = 8, align = 4);
}

mod c++::a {
    type Ref {
        #cpp_ref;
    }

    type Heap {
        #layout(size = 16, align = 8);
        #cpp_heap_allocated;
    }
}

type c++::a::Explicit {
    #cpp_ref "::other::Name";
}
    "#,
        NullCfg,
        |_| {},
    );
    let ty = |path: &[&str]| {
        parsed
            .spec
            .types
            .iter()
            .find(|t| matches!(&t.ty, RustType::Cpp(segs) if segs.path == path))
            .unwrap_or_else(|| panic!("no type {path:?}"))
    };
    assert_eq!(
        ty(&["a", "b", "Stack"]).cpp_stack_owned,
        Some(CppStackOwned("::a::b::Stack".to_owned())),
    );
    assert_eq!(
        ty(&["a", "Ref"]).cpp_ref,
        Some(CppRef("::a::Ref".to_owned()))
    );
    assert_eq!(
        ty(&["a", "Heap"]).cpp_heap_allocated,
        Some(CppHeapAllocated("::a::Heap".to_owned())),
    );
    // An explicit name still wins.
    assert_eq!(
        ty(&["a", "Explicit"]).cpp_ref,
        Some(CppRef("::other::Name".to_owned())),
    );
}

#[test]
fn cpp_type_name_required_for_rust_paths() {
    check_fail(
        r#"
type crate::Foo {
    #cpp_ref;
}
    "#,
        expect![[r#"
            Error: the C++ type name can only be omitted for `c++::` types, whose path is the name
               ╭─[ test.zng:3:5 ]
               │
             3 │     #cpp_ref;
               │     ────┬────  
               │         ╰────── the C++ type name can only be omitted for `c++::` types, whose path is the name
            ───╯
            Error: No layout policy found for type crate::Foo.
               ╭─[ test.zng:1:1 ]
               │
             2 │ type crate::Foo {
               │      ─────┬────  
               │           ╰────── Type first declared here.
               │ 
               │ Note: Use one of `#layout(size = X, align = Y)`, `#heap_allocated` or `#only_by_ref`.
            ───╯
        "#]],
    );
}

#[test]
fn cpp_pointers_and_references_parse() {
    use zngur_def::{CppIndirection, Mutability, PrimitiveRustType};

    let parsed = parse_str(
        r#"
type c++::a::B {
    #cpp_ref;
}

extern "C++" {
    unsafe fn f(const c++::a::B&, c++::a::B&, const i32*, i32*) -> const c++::a::B*;
}
"#,
        NullCfg,
        |_| {},
    );
    let f = &parsed.spec.extern_cpp_funcs[0];
    let b = || Box::new(RustType::Cpp(vec!["a".to_owned(), "B".to_owned()].into()));
    let i32 = || Box::new(RustType::Primitive(PrimitiveRustType::Int(32)));
    assert_eq!(
        f.inputs,
        vec![
            RustType::CppPtr(Mutability::Not, CppIndirection::Reference, b()),
            RustType::CppPtr(Mutability::Mut, CppIndirection::Reference, b()),
            RustType::CppPtr(Mutability::Not, CppIndirection::Pointer, i32()),
            RustType::CppPtr(Mutability::Mut, CppIndirection::Pointer, i32()),
        ]
    );
    assert_eq!(
        f.output,
        RustType::CppPtr(Mutability::Not, CppIndirection::Pointer, b())
    );
}

#[test]
fn mod_cpp_shorthand_matches_explicit_cpp_path() {
    let direct = parse_str(
        r#"
type c++::a::b::Name {
    #layout(size = 16, align = 8);
    #cpp_heap_allocated "::x";
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
        #cpp_heap_allocated "::x";
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
    #cpp_heap_allocated "::x";
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
    #cpp_heap_allocated "::x";
}
    "#,
        NullCfg,
        |_| {},
    );
    let spaced = parse_str(
        r#"
type c++ :: Name {
    #layout(size = 16, align = 8);
    #cpp_heap_allocated "::x";
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
            #cpp_heap_allocated "::x";
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
        #cpp_heap_allocated "::x";
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
        #cpp_stack_owned;

        unsafe fn value() const -> i32;
        unsafe fn set(i32, const B&);
        safe static fn make(i32) -> B;
        unsafe fn get(i32) const -> const i32& as get_const;
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
            RustType::CppPtr(
                Mutability::Not,
                CppIndirection::Reference,
                Box::new(b.clone())
            ),
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
        RustType::CppPtr(Mutability::Not, CppIndirection::Reference, Box::new(i32))
    );
}

#[test]
fn safe_cpp_method_with_this_is_rejected() {
    check_fail(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #cpp_stack_owned;
    safe fn value() const -> i32;
}
"#,
        expect![[r#"
            Error: this C++ member function must be `unsafe`, because it takes a C++ pointer or reference (including `this`, for non-static member functions), which may be invalid
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe fn value() const -> i32;
               │     ──────────────┬──────────────  
               │                   ╰──────────────── this C++ member function must be `unsafe`, because it takes a C++ pointer or reference (including `this`, for non-static member functions), which may be invalid
            ───╯
        "#]],
    );
}

#[test]
fn safe_static_cpp_method_with_cpp_ptr_is_rejected() {
    check_fail(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #cpp_stack_owned;
    safe static fn sum(const c++::B&) -> i32;
}
"#,
        expect![[r#"
            Error: this C++ member function must be `unsafe`, because it takes a C++ pointer or reference (including `this`, for non-static member functions), which may be invalid
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe static fn sum(const c++::B&) -> i32;
               │     ────────────────────┬────────────────────  
               │                         ╰────────────────────── this C++ member function must be `unsafe`, because it takes a C++ pointer or reference (including `this`, for non-static member functions), which may be invalid
            ───╯
        "#]],
    );
}

#[test]
fn static_const_cpp_method_is_rejected() {
    check_fail(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #cpp_stack_owned;
    safe static fn make() const -> c++::B;
}
"#,
        expect![[r#"
            Error: a static member function can't be `const`
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe static fn make() const -> c++::B;
               │     ───────────────────┬──────────────────  
               │                        ╰──────────────────── a static member function can't be `const`
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
            Error: C++ member functions (`safe fn` / `unsafe fn`) can only be declared on `#cpp_stack_owned` or `#cpp_ref` types
               ╭─[ test.zng:2:6 ]
               │
             2 │ type crate::Foo {
               │      ─────┬────  
               │           ╰────── C++ member functions (`safe fn` / `unsafe fn`) can only be declared on `#cpp_stack_owned` or `#cpp_ref` types
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
    #cpp_stack_owned;
    unsafe fn value() const -> i32;
}
"#,
    )]);

    check_import_fail(
        r#"
merge "./a.zng";
type c++::B {
    unsafe fn value() const -> i64;
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
             5 │     unsafe fn value() const -> i32;
               │     ───────────────┬───────────────  
               │                    ╰───────────────── declaration here ...
               │
               ├─[ test.zng:4:5 ]
               │
             4 │     unsafe fn value() const -> i64;
               │     ───────────────┬───────────────  
               │                    ╰───────────────── conflicts with the declaration here.
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
    #cpp_stack_owned;
    unsafe fn value() const -> i32;
}
"#,
    )]);
    let parsed = parse_str_with_resolver(
        r#"
merge "./a.zng";
type c++::B {
    unsafe fn set(i32);
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
    #cpp_stack_owned;

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
        vec![RustType::CppPtr(
            Mutability::Not,
            CppIndirection::Reference,
            Box::new(b)
        )]
    );
    assert!(!constructors[2].is_safe);
}

#[test]
fn safe_cpp_constructor_with_cpp_ptr_is_rejected() {
    check_fail(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #cpp_stack_owned;
    safe constructor(const c++::B&);
}
"#,
        expect![[r#"
            Error: this C++ constructor must be `unsafe`, because it takes a C++ pointer or reference, which may be invalid
               ╭─[ test.zng:5:5 ]
               │
             5 │     safe constructor(const c++::B&);
               │     ────────────────┬───────────────  
               │                     ╰───────────────── this C++ constructor must be `unsafe`, because it takes a C++ pointer or reference, which may be invalid
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
    #cpp_stack_owned;
    unsafe constructor(const c++::B&);
    unsafe constructor(const c++::B*);
}
"#,
        expect![[r#"
            Error: another C++ constructor has the same Rust argument types (C++ pointers and references to the same type are the same `CppPtr` in Rust)
               ╭─[ test.zng:6:5 ]
               │
             6 │     unsafe constructor(const c++::B*);
               │     ─────────────────┬────────────────  
               │                      ╰────────────────── another C++ constructor has the same Rust argument types (C++ pointers and references to the same type are the same `CppPtr` in Rust)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_constructor_on_cpp_ref_type_is_rejected() {
    check_fail(
        r#"
type c++::B {
    #cpp_ref;
    safe constructor(i32);
}
"#,
        expect![[r#"
            Error: C++ constructors can only be declared on `#cpp_stack_owned` types
               ╭─[ test.zng:2:6 ]
               │
             2 │ type c++::B {
               │      ───┬──  
               │         ╰──── C++ constructors can only be declared on `#cpp_stack_owned` types
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
        #cpp_stack_owned;
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
        vec![RustType::CppPtr(
            Mutability::Not,
            CppIndirection::Reference,
            Box::new(b.clone())
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
fn safe_cpp_function_with_cpp_ptr_is_rejected() {
    check_fail(
        r#"
mod c++::a {
    safe fn f(i32*);
}
"#,
        expect![[r#"
            Error: this C++ function must be `unsafe`, because it takes a C++ pointer or reference, which may be invalid
               ╭─[ test.zng:3:5 ]
               │
             3 │     safe fn f(i32*);
               │     ───────┬───────  
               │            ╰───────── this C++ function must be `unsafe`, because it takes a C++ pointer or reference, which may be invalid
            ───╯
        "#]],
    );
}

#[test]
fn cpp_fields_parse() {
    use zngur_def::{Mutability, PrimitiveRustType};

    let parsed = parse_str(
        r#"
type c++::B {
    #layout(size = 8, align = 4);
    #cpp_stack_owned;

    field const x (type = i32);
    field x (type = i32) as x_mut;
}
"#,
        NullCfg,
        |_| {},
    );
    let ty = parsed.spec.types.first().expect("no type parsed");
    let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
    let fields = &ty.cpp_fields;
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "x");
    assert_eq!(fields[0].rust_name(), "x");
    assert_eq!(fields[0].ty, i32);
    assert_eq!(fields[0].mutability, Mutability::Not);
    assert_eq!(fields[1].name, "x");
    assert_eq!(fields[1].rust_name(), "x_mut");
    assert_eq!(fields[1].mutability, Mutability::Mut);
    // Fields of Rust types are unaffected.
    assert!(ty.fields.is_empty());
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
            Error: C++ field accessors (`field name (type = T)`) can only be declared on `#cpp_stack_owned` or `#cpp_ref` types
               ╭─[ test.zng:2:6 ]
               │
             2 │ type crate::Foo {
               │      ─────┬────  
               │           ╰────── C++ field accessors (`field name (type = T)`) can only be declared on `#cpp_stack_owned` or `#cpp_ref` types
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
    #cpp_ref;
}

type c++::std::unique_ptr<c++::a::B> {
    #layout(size = 8, align = 8);
    #cpp_stack_owned;
}

type c++::std::map<i32, const c++::a::B*> {
    #cpp_ref;
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
    assert_eq!(
        types[1].cpp_stack_owned.as_ref().unwrap().0,
        "::std::unique_ptr< ::a::B >"
    );
    assert_eq!(
        types[2].ty,
        RustType::Cpp(CppTypePath {
            path: vec!["std".to_owned(), "map".to_owned()],
            generics: vec![
                RustType::Primitive(PrimitiveRustType::Int(32)),
                RustType::CppPtr(Mutability::Not, CppIndirection::Pointer, Box::new(b)),
            ],
        })
    );
    assert_eq!(
        types[2].cpp_ref.as_ref().unwrap().0,
        "::std::map< ::int32_t, const ::a::B* >"
    );
}

#[test]
fn cpp_template_with_named_generics_is_rejected() {
    check_fail(
        r#"
type c++::std::vector<T = i32> {
    #cpp_ref;
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
    #cpp_ref;
}
"#,
        expect![[r#"
            Error: `::std::string::String` can't be a C++ template argument (only `c++::` types, builtins and C++ pointers and references can)
               ╭─[ test.zng:3:5 ]
               │
             3 │     #cpp_ref;
               │     ────┬────  
               │         ╰────── `::std::string::String` can't be a C++ template argument (only `c++::` types, builtins and C++ pointers and references can)
            ───╯
            Error: No layout policy found for type std::vector::<::std::string::String>.
               ╭─[ test.zng:1:1 ]
               │
             2 │ type c++::std::vector<::std::string::String> {
               │      ───────────────────┬───────────────────  
               │                         ╰───────────────────── Type first declared here.
               │ 
               │ Note: Use one of `#layout(size = X, align = Y)`, `#heap_allocated` or `#only_by_ref`.
            ───╯
        "#]],
    );
}

#[test]
fn cpp_template_with_different_arities_is_rejected() {
    check_fail(
        r#"
type c++::std::vector<i32> {
    #cpp_ref;
}

type c++::std::vector<i32, c++::MyAlloc> {
    #cpp_ref;
}
"#,
        expect![[r#"
            Error: every instantiation of `c++::std::vector` must have the same number of template arguments as its first instantiation, which has 1
               ╭─[ test.zng:6:6 ]
               │
             6 │ type c++::std::vector<i32, c++::MyAlloc> {
               │      ─────────────────┬─────────────────  
               │                       ╰─────────────────── every instantiation of `c++::std::vector` must have the same number of template arguments as its first instantiation, which has 1
            ───╯
        "#]],
    );
}

#[test]
fn cpp_template_instantiations_with_the_same_rust_type_are_rejected() {
    check_fail(
        r#"
type c++::std::pair<const i32&> {
    #cpp_ref;
}

type c++::std::pair<const i32*> {
    #cpp_ref;
}
"#,
        expect![[r#"
            Error: another instantiation of this C++ class template is the same Rust type (C++ pointers and references to the same type are the same `CppPtr` in Rust)
               ╭─[ test.zng:6:6 ]
               │
             6 │ type c++::std::pair<const i32*> {
               │      ─────────────┬────────────  
               │                   ╰────────────── another instantiation of this C++ class template is the same Rust type (C++ pointers and references to the same type are the same `CppPtr` in Rust)
            ───╯
        "#]],
    );
}

#[test]
fn cpp_template_heap_allocated_is_rejected() {
    check_fail(
        r#"
type c++::std::vector<i32> {
    #layout(size = 24, align = 8);
    #cpp_heap_allocated;
}
"#,
        expect![[r#"
            Error: instantiations of C++ class templates can't be `#cpp_heap_allocated`
               ╭─[ test.zng:2:6 ]
               │
             2 │ type c++::std::vector<i32> {
               │      ──────────┬──────────  
               │                ╰──────────── instantiations of C++ class templates can't be `#cpp_heap_allocated`
            ───╯
        "#]],
    );
}

#[test]
fn instantiating_an_imported_cpp_template_is_rejected() {
    let resolver = MockFilesystem::new(vec![(
        "./bar.zng",
        r#"
#crate bar_bindings

type c++::std::unique_ptr<i32> {
    #layout(size = 8, align = 8);
    #cpp_stack_owned;
}
"#,
    )]);
    check_import_fail(
        r#"
import "./bar.zng";

type c++::std::unique_ptr<i64> {
    #layout(size = 8, align = 8);
    #cpp_stack_owned;
}
"#,
        expect![[r#"
            Error: `c++::std::unique_ptr` is declared in an imported module, so its instantiations must be declared there too
               ╭─[ test.zng:4:6 ]
               │
             4 │ type c++::std::unique_ptr<i64> {
               │      ────────────┬────────────  
               │                  ╰────────────── `c++::std::unique_ptr` is declared in an imported module, so its instantiations must be declared there too
            ───╯
        "#]],
        &resolver,
    );
}
