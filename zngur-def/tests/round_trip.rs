use expect_test::{Expect, expect};
use zngur_def::{ZngurSpec, printing::IDLPrinter};
use zngur_parser::{DefaultImportResolver, ImportResolver, ParsedZngFile, cfg::NullCfg};

#[derive(Debug, Default)]
struct ErrorSink {
    error_buffer: Vec<u8>,
}

pub struct ErrorText(pub String);

impl zngur_parser::ReportSink for ErrorSink {
    fn sink_report(
        &mut self,
        report: &zngur_parser::ReportEntry,
        source_cache: zngur_parser::SourceCache,
    ) {
        if report.fatal {
            report
                .report
                .write(source_cache, &mut self.error_buffer)
                .unwrap();
        }
    }
}

impl ErrorSink {
    fn assert_no_errors(&mut self, src: Option<&str>) {
        {
            if !self.error_buffer.is_empty() {
                let errors = String::from_utf8_lossy(&self.error_buffer);
                match src {
                    Some(src) => {
                        panic!(
                            "assertion `no errors` failed parsing:\n\n{src}\n\nErrors:\n\n{errors}",
                        );
                    }
                    None => {
                        panic!("assertion `no errors` failed:\n\n{errors}");
                    }
                }
            }
        }
    }
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

fn check_round_trip(zng: &str, expected_output: Expect) -> (ZngurSpec, ZngurSpec) {
    check_round_trip_with_resolver(zng, None, expected_output)
}

/// parses the input source to a sepc, the reprints it and parses the printed source
/// returns (parsed: Spec, parsed_reprinted: Spec)
fn check_round_trip_with_resolver(
    zng: &str,
    resolver: Option<Box<dyn ImportResolver>>,
    expected_output: Expect,
) -> (ZngurSpec, ZngurSpec) {
    let resolver = resolver.unwrap_or_else(|| Box::new(DefaultImportResolver));
    let parsed = {
        let mut error_sink = ErrorSink::default();
        let ret = ParsedZngFile::parse_str_with_resolver(
            zng,
            "test.zng",
            NullCfg,
            &resolver,
            &mut error_sink,
        );
        error_sink.assert_no_errors(Some(zng));
        ret
    };
    let mut buf = std::io::BufWriter::new(Vec::new());
    let mut printer = IDLPrinter::new(&mut buf);
    printer.write_idl(&parsed.spec).expect("printing failure");
    let bytes = buf.into_inner().expect("buffer flush error");
    let idl = str::from_utf8(&bytes).expect("utf8 error");
    expected_output.assert_eq(idl);
    let reparsed = {
        let mut error_sink = ErrorSink::default();
        let ret = ParsedZngFile::parse_str_with_resolver(
            idl,
            "test.zng",
            NullCfg,
            &resolver,
            &mut error_sink,
        );
        error_sink.assert_no_errors(Some(idl));
        ret
    };

    assert_eq!(parsed.spec.types, reparsed.spec.types);
    assert_eq!(parsed.spec.cpp_templates, reparsed.spec.cpp_templates);
    assert_eq!(
        parsed.spec.cpp_template_homes,
        reparsed.spec.cpp_template_homes
    );
    assert_eq!(parsed.spec.traits, reparsed.spec.traits);
    assert_eq!(parsed.spec.funcs, reparsed.spec.funcs);
    assert_eq!(parsed.spec.cpp_funcs, reparsed.spec.cpp_funcs);
    assert_eq!(parsed.spec.extern_cpp_funcs, reparsed.spec.extern_cpp_funcs);
    assert_eq!(parsed.spec.extern_cpp_impls, reparsed.spec.extern_cpp_impls);
    assert_eq!(parsed.spec.imported_modules, reparsed.spec.imported_modules);
    assert_eq!(
        parsed.spec.additional_includes,
        reparsed.spec.additional_includes
    );
    assert_eq!(
        parsed.spec.convert_panic_to_exception,
        reparsed.spec.convert_panic_to_exception
    );
    assert_eq!(parsed.spec.crate_name, reparsed.spec.crate_name);

    (parsed.spec, reparsed.spec)
}

#[test]
fn reparse_tuple() {
    let _ = check_round_trip(
        r#"
type (i8, u8) {
    #layout(size = 0, align = 1);
}
    "#,
        expect![[r#"
            type (i8, u8) {
                #layout(size = 0, align = 1);
            }
        "#]],
    );
}

#[test]
fn reparse_slice() {
    let _ = check_round_trip(
        r#"
type [u8] {
    #layout(size = 0, align = 1);
}
    "#,
        expect![[r#"
            type [u8] {
                #layout(size = 0, align = 1);
            }
        "#]],
    );
}

#[test]
fn reparse_cpp_heap_allocated() {
    let _ = check_round_trip(
        r#"
type crate::Way {
    #layout(size = 16, align = 8);
    #cpp_heap_allocated "::osmium::Way";
}
    "#,
        expect![[r#"
            type crate::Way {
                #layout(size = 16, align = 8);
                #cpp_heap_allocated "::osmium::Way";
            }
        "#]],
    );
}
#[test]
fn reparse_cpp_ref() {
    let _ = check_round_trip(
        r#"
type crate::Way {
    #cpp_ref "::osmium::Way";
}
    "#,
        expect![[r#"
            type crate::Way {
                #cpp_ref "::osmium::Way";
            }
        "#]],
    );
}
#[test]
fn reparse_cpp_stack_owned() {
    let _ = check_round_trip(
        r#"
type crate::Way {
    #layout(size = 16, align = 8);
    #cpp_stack_owned "::osmium::Way";
}
    "#,
        expect![[r#"
            type crate::Way {
                #layout(size = 16, align = 8);
                #cpp_stack_owned "::osmium::Way";
            }
        "#]],
    );
}

#[test]
fn reparse_cpp_stack_owned_conservative() {
    let _ = check_round_trip(
        r#"
type crate::Way {
    #layout_conservative(size = 16, align = 8);
    #cpp_stack_owned "::osmium::Way";
}
    "#,
        expect![[r#"
            type crate::Way {
                #layout_conservative(size = 16, align = 8);
                #cpp_stack_owned "::osmium::Way";
            }
        "#]],
    );
}

#[test]
fn reparse_cpp_paths_and_modules() {
    let _ = check_round_trip(
        r#"
use c++::a::b::Item as MyItem;

type c++::Way {
    #cpp_ref "::osmium::Way";
}

mod c++::a::b {
    type Item {
        #layout(size = 16, align = 8);
        #cpp_heap_allocated "::x::Item";
    }
}

extern "C++" {
    safe fn make_item(&const c++::Way) -> MyItem;
    impl MyItem {
        safe fn touch(&mut self, &const c++::Way);
    }
}
    "#,
        expect![[r#"
            type c++::Way {
                #cpp_ref "::osmium::Way";
            }

            type c++::a::b::Item {
                #layout(size = 16, align = 8);
                #cpp_heap_allocated "::x::Item";
            }

            extern "C++" {

                safe fn make_item(&const c++::Way) -> c++::a::b::Item;

                impl c++::a::b::Item {
                    safe fn touch(&mut self, &const c++::Way);
                }

            }
        "#]],
    );
}

#[test]
fn reparse_cpp_reference_only() {
    let _ = check_round_trip(
        r#"
type c++::osmium::Way {
}
type c++::osmium::Node {
    #layout(size = 16, align = 8);
}
    "#,
        expect![[r#"
            type c++::osmium::Way {
            }

            type c++::osmium::Node {
                #layout(size = 16, align = 8);
            }
        "#]],
    );
}

#[test]
fn reparse_cpp_trivially_relocatable() {
    let _ = check_round_trip(
        r#"
type c++::osmium::Way {
    #layout(size = 16, align = 8);
    #trivially_relocatable;
}
type c++::osmium::Node {
    #layout_conservative(size = 16, align = 8);
    #trivially_relocatable;
}
type c++::osmium::Relation {
    #layout(size = 8, align = 8);
    #unsafe(trivially_relocatable);
}
    "#,
        expect![[r#"
            type c++::osmium::Way {
                #layout(size = 16, align = 8);
                #trivially_relocatable;
            }

            type c++::osmium::Node {
                #layout_conservative(size = 16, align = 8);
                #trivially_relocatable;
            }

            type c++::osmium::Relation {
                #layout(size = 8, align = 8);
                #unsafe(trivially_relocatable);
            }
        "#]],
    );
}

#[test]
fn reparse_alias() {
    let _ = check_round_trip(
        r#"
use ::std::string::String as MyString;
type MyString {
    #layout(size = 24, align = 8);
}
    "#,
        expect![[r#"
            type ::std::string::String {
                #layout(size = 24, align = 8);
            }
        "#]],
    );
}

#[test]
fn reparse_methods() {
    let _ = check_round_trip(
        r#"
type MyType {
    #layout(size = 24, align = 8);

    fn foo(self, &str);
    fn foo2(&self, &str) ->();
    fn foo3(&mut self, &str);
    fn foo4(&self, &str);
    fn borrow(&self) deref &str;
    fn from_trait(&self, i32) -> bool use MyTrait;

    fn renamed(&self) -> &[u8] as renamed_fn;

    fn other_self(Arc<MyType>, &str);

    fn static(&str) -> MyType;

}
    "#,
        expect![[r#"
            type ::MyType {
                #layout(size = 24, align = 8);

                fn foo(self, &str);
                fn foo2(&self, &str);
                fn foo3(&mut self, &str);
                fn foo4(&self, &str);
                fn borrow(&self) deref &str;
                fn from_trait(&self, i32) -> bool use ::MyTrait;
                fn renamed(&self) -> &[u8] as renamed_fn;
                fn other_self(::Arc<::MyType>, &str);
                fn static(&str) -> ::MyType;
            }
        "#]],
    );
}

#[test]
fn reparse_fields() {
    let _ = check_round_trip(
        r#"
type MyType {
    #layout(size = 24, align = 8);

    field foo (offset = 0, type = &str);
    field bar (offset = 8, type = i32);
    field baz (offset = auto, type = f64);
}
    "#,
        expect![[r#"
            type ::MyType {
                #layout(size = 24, align = 8);

                field foo (offset = 0, type = &str);
                field bar (offset = 8, type = i32);
                field baz (offset = auto, type = f64);
            }
        "#]],
    );
}

#[test]
fn reparse_variants() {
    let _ = check_round_trip(
        r#"
type Option<i32> {
    #layout(size = 8, align = 4);
    variant None { }
    variant Some {
        field 0 (offset = auto, type = i32);
    }
}
        "#,
        expect![[r#"
            type ::Option<i32> {
                #layout(size = 8, align = 4);
                variant None {}
                variant Some {
                    field 0 (offset = auto, type = i32);
                }
            }
        "#]],
    );
}

#[test]
fn reparse_imports() {
    let resolver = MockFilesystem::new(vec![(
        "./relative/path.zng",
        "type Imported { #layout(size = 1, align = 1); }",
    )]);

    let _ = check_round_trip_with_resolver(
        r#"
merge "./relative/path.zng";
type Example {
    #layout(size = 1, align = 1);
}
    "#,
        Some(Box::new(resolver)),
        expect![[r#"
            type ::Example {
                #layout(size = 1, align = 1);
            }

            type ::Imported {
                #layout(size = 1, align = 1);
            }
        "#]],
    );
}

#[test]
fn reparse_convert_panic_to_exception() {
    let _ = check_round_trip(
        r#"
#convert_panic_to_exception
type A {
    #layout(size = 1, align = 1);
}
        "#,
        expect![[r#"
            #convert_panic_to_exception

            type ::A {
                #layout(size = 1, align = 1);
            }
        "#]],
    );
}

#[test]
fn reparse_extern_cpp() {
    let _ = check_round_trip(
        r#"
extern "C++" {
    safe fn foo();
    unsafe fn bar();
    impl crate::Foo {
        safe fn s_foo();
        unsafe fn u_foo();
    }
    impl crate::MyTraitByCpp for crate::Foo {
        safe fn s_baz();
        unsafe fn u_baz();
    }
}
        "#,
        expect![[r#"
            extern "C++" {

                safe fn foo();
                unsafe fn bar();

                impl crate::Foo {
                    safe fn s_foo();
                    unsafe fn u_foo();
                }
                impl crate::MyTraitByCpp for crate::Foo {
                    safe fn s_baz();
                    unsafe fn u_baz();
                }

            }
        "#]],
    );
}

#[test]
fn reparse_top_level_and_mod_funcs() {
    let _ = check_round_trip(
        r#"
fn top_before();
mod crate::m {
    fn in_mod_a(i32) -> bool;
    safe fn in_mod_b<u32, u64>();
}
unsafe fn top_after(u8);
        "#,
        expect![[r#"
            fn top_before();
            mod crate::m {
                fn in_mod_a(i32) -> bool;
                safe fn in_mod_b<u32, u64>();
            }
            unsafe fn top_after(u8);
        "#]],
    );
}

#[test]
fn reparse_cpp_additional_includes_hash_escalation() {
    let _ = check_round_trip(
        r###"
#cpp_additional_includes r##"#include "foo.h" // contains "# inside"##
        "###,
        expect![[r####"
            #cpp_additional_includes r##"#include "foo.h" // contains "# inside"##
        "####]],
    );

    let _ = check_round_trip(
        r##"
#cpp_additional_includes r#"#include "trailing.h""#
        "##,
        expect![[r###"
            #cpp_additional_includes r#"#include "trailing.h""#
        "###]],
    );
}

#[test]
fn reparse_misc_printer_fixes() {
    let _ = check_round_trip(
        r#"
import "./dep.zng";

type str {
    wellknown_traits(?Sized);
}

type crate::HeapThing {
    #heap_allocated;
    wellknown_traits(Debug);
    fn generic_method<u32, i64>(&self, Box<dyn crate::MyTrait + Send>);
}

trait crate::MyTrait {
    fn step(&mut self, i32) -> bool;
}
        "#,
        expect![[r#"
            import "./dep.zng";

            type str {
                wellknown_traits(?Sized);
            }

            type crate::HeapThing {
                #heap_allocated;
                wellknown_traits(Debug);

                fn generic_method<u32, i64>(&self, Box<dyn crate::MyTrait + Send>);
            }

            trait crate::MyTrait {
                fn step(&mut self, i32) -> bool;
            }
        "#]],
    );
}

/// Recursively collects every `.zng` file under `dir`.
fn collect_zng_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_zng_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "zng") {
            out.push(path);
        }
    }
}

fn print_spec(spec: &ZngurSpec) -> String {
    let mut buf = std::io::BufWriter::new(Vec::new());
    IDLPrinter::new(&mut buf)
        .write_idl(spec)
        .expect("printing failure");
    String::from_utf8(buf.into_inner().expect("buffer flush error")).expect("utf8 error")
}

/// Every `.zng` file in `examples/` must survive parse -> print -> parse unchanged.
///
/// `merge`d files are flattened into the printed output, so the reparse needs
/// no import resolution.
#[test]
fn reparse_all_examples() {
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let mut files = vec![];
    collect_zng_files(&examples, &mut files);
    assert!(!files.is_empty(), "no example .zng files found");

    let mut failures = vec![];
    for path in &files {
        let mut sink = ErrorSink::default();
        let parsed = ParsedZngFile::parse(path, NullCfg, &mut sink);
        if parsed.errors > 0 {
            failures.push(format!(
                "{}: original failed to parse:\n{}",
                path.display(),
                String::from_utf8_lossy(&sink.error_buffer)
            ));
            continue;
        }
        let printed = print_spec(&parsed.spec);
        let mut sink = ErrorSink::default();
        let reparsed = ParsedZngFile::parse_str_with_resolver(
            &printed,
            &path.to_string_lossy(),
            NullCfg,
            &DefaultImportResolver,
            &mut sink,
        );
        if reparsed.errors > 0 {
            failures.push(format!(
                "{}: printed output failed to parse:\n{}\n--- printed ---\n{printed}",
                path.display(),
                strip_ansi_escapes::strip_str(String::from_utf8_lossy(&sink.error_buffer)),
            ));
            continue;
        }
        let (a, b) = (&parsed.spec, &reparsed.spec);
        macro_rules! check_fields {
            ($($field:ident),*) => {
                $(
                    if a.$field != b.$field {
                        let before = format!("{:#?}", a.$field);
                        let after = format!("{:#?}", b.$field);
                        let first_diff = before
                            .lines()
                            .zip(after.lines())
                            .position(|(x, y)| x != y)
                            .unwrap_or(0);
                        let context = |s: &str| {
                            s.lines()
                                .skip(first_diff.saturating_sub(3))
                                .take(8)
                                .collect::<Vec<_>>()
                                .join("\n")
                        };
                        failures.push(format!(
                            "{}: `{}` changed after round trip\n--- before ---\n{}\n--- after ---\n{}",
                            path.display(),
                            stringify!($field),
                            context(&before),
                            context(&after),
                        ));
                    }
                )*
            };
        }
        check_fields!(
            types,
            cpp_templates,
            cpp_template_homes,
            traits,
            funcs,
            cpp_funcs,
            extern_cpp_funcs,
            extern_cpp_impls,
            imported_modules,
            additional_includes,
            convert_panic_to_exception,
            crate_name
        );
    }
    assert!(
        failures.is_empty(),
        "{} of {} example round trips failed:\n\n{}",
        failures.len(),
        files.len(),
        failures.join("\n\n========\n\n")
    );
}

#[test]
fn reparse_crate_directive() {
    let _ = check_round_trip(
        r#"
#crate bar_bindings

type c++::ns::Bar {
}
    "#,
        expect![[r#"
            #crate bar_bindings

            type c++::ns::Bar {
            }
        "#]],
    );
}

#[test]
fn reparse_resolved_module_imports_including_nested_merge_rebasing() {
    let resolver = MockFilesystem::new(vec![
        (
            "./sub/part.zng",
            "import \"./bar.zng\";\nimport \"../other/baz.zng\";\n",
        ),
        (
            "./sub/bar.zng",
            "#crate bar_bindings\ntype c++::ns::Bar {}\n",
        ),
        (
            "./other/baz.zng",
            "#crate baz_bindings\ntype c++::ns::Baz {\n    #layout(size = 8, align = 4);\n    #trivially_relocatable;\n}\n",
        ),
    ]);
    let (parsed, reparsed) = check_round_trip_with_resolver(
        r#"
#crate app_crate
merge "./sub/part.zng";

mod crate {
    fn use_imported(&const c++::ns::Bar, c++::ns::Baz);
}
    "#,
        Some(Box::new(resolver)),
        expect![[r#"
            import "./sub/bar.zng";
            import "./other/baz.zng";

            #crate app_crate

            mod crate {
                fn use_imported(&const c++::ns::Bar, c++::ns::Baz);
            }
        "#]],
    );
    assert_eq!(parsed.imported_modules.len(), 2);
    assert_eq!(
        parsed.imported_modules[0].crate_name.as_deref(),
        Some("bar_bindings")
    );
    assert_eq!(
        parsed.imported_modules[1].crate_name.as_deref(),
        Some("baz_bindings")
    );
    assert_eq!(parsed.imported_modules, reparsed.imported_modules);
}

#[test]
fn reparse_lifetimes() {
    let _ = check_round_trip(
        r#"
#unstable(template_types)

type<'a, T> ::std::slice::Iter<'a, T> {
    #layout(size = 16, align = 8);
    fn next(&mut self) -> ::std::option::Option<&'a T>;
}

type<'a> crate::Parser<'a> {
    #layout(size = 24, align = 8);
    field input (offset = 0, type = &'a str);
    fn rest(&self) -> &'a str;
    fn peek<'b>(&'b self) -> ::std::option::Option<&'b str>;
    fn convert<'b, i32>(&'b self, i32) -> &'b str;
}

type<'b> ::std::slice::Iter<'b, u8> {
}

type ::std::slice::Iter<'static, i32> {
}

type Box<dyn for<'a> Fn(&'a i32, &'a i32) -> &'a i32> {
    #layout(size = 16, align = 8);
}

trait crate::Picker {
    fn pick<'a>(&self, &'a i32, &'a i32) -> &'a i32;
}

mod crate {
    fn longest<'a>(&'a str, &'a str) -> &'a str;
    fn identity<'a, &'a str>(&'a str) -> &'a str;
}

extern "C++" {
    safe fn pick<'a>(&'a crate::Foo, &'a mut crate::Foo) -> &'a i32;
    impl<'a> crate::Parser<'a> {
        safe fn first(&'a self) -> &'a str;
    }
    impl<'a> crate::MyTrait<'a> for crate::Parser<'a> {
        safe fn inspect<'b>(&'b self) -> &'a str;
    }
}
"#,
        expect![[r#"
            type<'a> crate::Parser<'a> {
                #layout(size = 24, align = 8);

                field input (offset = 0, type = &'a str);

                fn rest(&self) -> &'a str;
                fn peek<'b>(&'b self) -> ::std::option::Option<&'b str>;
                fn convert<'b, i32>(&'b self, i32) -> &'b str;
            }

            type<'b> ::std::slice::Iter<'b, u8> {
                #layout(size = 16, align = 8);

                fn next(&mut self) -> ::std::option::Option<&'b u8>;
            }

            type ::std::slice::Iter<'static, i32> {
                #layout(size = 16, align = 8);

                fn next(&mut self) -> ::std::option::Option<&'static i32>;
            }

            type Box<dyn for<'a> Fn(&'a i32, &'a i32) -> &'a i32> {
                #layout(size = 16, align = 8);
            }

            mod crate {
                fn longest<'a>(&'a str, &'a str) -> &'a str;
                fn identity<'a, &'a str>(&'a str) -> &'a str;
            }

            trait crate::Picker {
                fn pick<'a>(&self, &'a i32, &'a i32) -> &'a i32;
            }

            extern "C++" {

                safe fn pick<'a>(&'a crate::Foo, &'a mut crate::Foo) -> &'a i32;

                impl<'a> crate::Parser<'a> {
                    safe fn first(&'a self) -> &'a str;
                }
                impl<'a> crate::MyTrait<'a> for crate::Parser<'a> {
                    safe fn inspect<'b>(&'b self) -> &'a str;
                }

            }
        "#]],
    );
}

#[test]
fn reparse_lifetime_merge_and_template_capture_avoidance() {
    let resolver = MockFilesystem::new(vec![(
        "./part.zng",
        r#"
type<'a> crate::Parser<'a> {
    #layout(size = 40, align = 8);
    field callback (offset = 8, type = Box<dyn for<'b> Fn(&'b str) -> &'b str>);
    fn pair<'b, 'b_0>(&'b self, &'b_0 str) -> &'a str;
}
"#,
    )]);
    let _ = check_round_trip_with_resolver(
        r#"
#unstable(template_types)
merge "./part.zng";

type<'b> crate::Parser<'b> {
    field input (offset = 0, type = &'b str);
}

type<T> crate::Holder<T> {
    #layout(size = 16, align = 8);
    fn invoke<'b>(&'b self, T);
}

type crate::Holder<Box<dyn for<'b> Fn(&'b str)>> {
}
"#,
        Some(Box::new(resolver)),
        expect![[r#"
            type<'b> crate::Parser<'b> {
                #layout(size = 40, align = 8);

                field input (offset = 0, type = &'b str);
                field callback (offset = 8, type = Box<dyn for<'b_2> Fn(&'b_2 str) -> &'b_2 str>);

                fn pair<'b_1, 'b_0>(&'b_1 self, &'b_0 str) -> &'b str;
            }

            type crate::Holder<Box<dyn for<'b> Fn(&'b str)>> {
                #layout(size = 16, align = 8);

                fn invoke<'b_0>(&'b_0 self, Box<dyn for<'b> Fn(&'b str)>);
            }
        "#]],
    );
}

#[test]
fn reparse_cpp_templates() {
    let _ = check_round_trip(
        r#"
#crate my_crate
type c++::a::B {
}
type c++::std::unique_ptr<c++::a::B> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
    unsafe fn get(&const self) -> c++::a::B*;
}
type c++::std::map<i32, const c++::a::B*> {
}
type c++::std::span<const i32> {
}
type c++::View<i32* const> {
}
        "#,
        expect![[r#"
            #crate my_crate

            type c++::a::B {
            }

            type c++::std::unique_ptr<c++::a::B> {
                #layout(size = 8, align = 8);
                #trivially_relocatable;

                unsafe fn get(&const self) -> c++::a::B*;
            }

            type c++::std::map<i32, const c++::a::B*> {
            }

            type c++::std::span<const i32> {
            }

            type c++::View<i32* const> {
            }
        "#]],
    );
}

#[test]
fn reparse_cpp_template_home() {
    let _ = check_round_trip(
        r#"
type<T> c++::std::unique_ptr<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
    unsafe fn get(&const self) -> T*;
}
type c++::std::unique_ptr<i32> {
}
        "#,
        expect![[r#"
            type<T> c++::std::unique_ptr<T> {
                #layout(size = 8, align = 8);
                #trivially_relocatable;

                unsafe fn get(&const self) -> T*;
            }

            type c++::std::unique_ptr<i32> {
                #layout(size = 8, align = 8);
                #trivially_relocatable;

                unsafe fn get(&const self) -> i32*;
            }
        "#]],
    );
}

#[test]
fn reparse_cpp_methods() {
    let _ = check_round_trip(
        r#"
#crate my_crate
type c++::a::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    unsafe fn value(&const self) -> i32;
    unsafe fn set(&unique var self, i32, const c++::a::B&);
    safe fn make(i32) -> c++::a::B;
    unsafe fn get<'a>(&'a const self, i32) -> &'a const i32 as get_const;
    safe fn type<'a>(&'a const self) -> i32 as kind;
    safe constructor() as new;
    safe constructor(i32, i32) as from_xy;
    unsafe constructor(const c++::a::B&) as copy_from;
    field &const x (type = i32);
    field &unique var x (type = i32) as x_mut;
}

type crate::Foo {
    #layout(size = 8, align = 4);
    safe fn get(&self) -> i32;
    unsafe fn make_raw(i32) -> crate::Foo;
    fn plain(&mut self);
}
        "#,
        expect![[r#"
            #crate my_crate

            type c++::a::B {
                #layout(size = 8, align = 4);
                #trivially_relocatable;

                field &const x (type = i32);
                field &unique var x (type = i32) as x_mut;

                safe constructor() as new;
                safe constructor(i32, i32) as from_xy;
                unsafe constructor(const c++::a::B&) as copy_from;

                unsafe fn value(&const self) -> i32;
                unsafe fn set(&unique var self, i32, const c++::a::B&);
                safe fn make(i32) -> c++::a::B;
                unsafe fn get<'a>(&'a const self, i32) -> &'a const i32 as get_const;
                safe fn type<'a>(&'a const self) -> i32 as kind;
            }

            type crate::Foo {
                #layout(size = 8, align = 4);

                safe fn get(&self) -> i32;
                unsafe fn make_raw(i32) -> crate::Foo;
                fn plain(&mut self);
            }
        "#]],
    );
}

#[test]
fn reparse_cpp_functions() {
    let parsed = check_round_trip(
        r#"
#crate my_crate
mod c++::a {
    type B {
        #layout(size = 8, align = 4);
        #trivially_relocatable;
    }
    unsafe fn f(const B&) -> i32;
    safe fn g(i32) -> B as make_b;
    safe fn type(i32) -> i32 as get_type;
    safe fn b::j();
    mod b {
        safe fn h();
    }
}
safe fn c++::a::b::k(i32) -> i32;
        "#,
        expect![[r#"
            #crate my_crate

            type c++::a::B {
                #layout(size = 8, align = 4);
                #trivially_relocatable;
            }

            mod c++::a {
                unsafe fn f(const c++::a::B&) -> i32;
                safe fn g(i32) -> c++::a::B as make_b;
                safe fn type(i32) -> i32 as get_type;
            }
            mod c++::a::b {
                safe fn j();
                safe fn h();
                safe fn k(i32) -> i32;
            }
        "#]],
    );

    let mut standalone = Vec::new();
    zngur_def::printing::IDLPrinter::new(&mut standalone)
        .write_idl(&parsed.0.cpp_funcs[1])
        .unwrap();
    assert_eq!(
        String::from_utf8(standalone).unwrap(),
        "mod c++::a {\n    safe fn g(i32) -> c++::a::B as make_b;\n}\n"
    );
}

#[test]
fn reparse_cpp_receivers() {
    let _ = check_round_trip(
        r#"
type c++::a::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    safe fn get<'a>(&'a const self) -> &'a const i32;
    safe fn set<'a>(&'a unique var self, &'a const c++::a::B);
    unsafe fn raw_get(const self*) -> i32;
    unsafe fn raw_set(self&, i32);
    safe constructor<'a>(&'a const c++::a::B) as copy;
}

mod c++::a {
    safe fn pick<'a>(&'a const c++::a::B, &'a const c++::a::B) -> &'a const c++::a::B;
}
"#,
        expect![[r#"
            type c++::a::B {
                #layout(size = 8, align = 4);
                #trivially_relocatable;

                safe constructor<'a>(&'a const c++::a::B) as copy;

                safe fn get<'a>(&'a const self) -> &'a const i32;
                safe fn set<'a>(&'a unique var self, &'a const c++::a::B);
                unsafe fn raw_get(const self*) -> i32;
                unsafe fn raw_set(self&, i32);
            }

            mod c++::a {
                safe fn pick<'a>(&'a const c++::a::B, &'a const c++::a::B) -> &'a const c++::a::B;
            }
        "#]],
    );
}

#[test]
fn reparse_cpp_objects() {
    let _ = check_round_trip(
        r#"
type c++::a::B {
    #layout(size = 16, align = 8);
    #trivially_relocatable;
    safe fn refs<'a>(&'a const self, &'a const c++::a::B, &'a var c++::a::B, &'a unique var c++::a::B, &'a unique const c++::a::B);
    safe fn builtins<'a>(&'a var self, &'a const i32, &'a var i32, &'a unique var i32, &'a unique const i32, &'a i32, &'a mut i32) -> &'a var bool;
    safe fn peek<'a>(&'a unique const self) -> &'a const c++::a::B*;
    unsafe fn raw(c++::a::B*, const i32*);
    field &const x (type = i32);
    field &var x (type = i32) as x_var;
    field &unique var x (type = i32) as x_mut;
    field &unique const x (type = i32) as x_unique_const;
    field const x (type = i32) as x_const_copy;
    field &const type (type = i32) as type_;
    field var self_copy (type = c++::a::B);
    unsafe field &var next (type = c++::a::B*) as next_var;
    field var next (type = c++::a::B*) as next_copy;
}

type crate::Foo {
    #layout(size = 0, align = 1);
    fn f(&self, &const c++::a::B, &unique var c++::a::B, &i32);
}

mod c++::a {
    safe fn pick<'a>(&'a const c++::a::B, &'a var c++::a::B) -> &'a unique var c++::a::B;
}

extern "C++" {
    safe fn refs<'a>(&'a const c++::a::B, &'a var c++::a::B, &'a unique var c++::a::B, &'a unique const c++::a::B);
    safe fn nullable<'a>(::std::option::Option<&'a const c++::a::B>, ::core::option::Option<&'a var c++::a::B>, ::std::option::Option<&'a unique var c++::a::B>, ::core::option::Option<&'a unique const i32>) -> ::std::option::Option<&'a const c++::a::B>;
    safe fn builtins<'a>(&'a const i32, &'a var i32, &'a unique var i32, &'a unique const i32, &'a i32, &'a mut i32) -> &'a var bool;
    unsafe fn raw(c++::a::B*, const c++::a::B&, i32&, const i32*, c++::a::B**, const i32**, i32* const*);
}
"#,
        expect![[r#"
            type c++::a::B {
                #layout(size = 16, align = 8);
                #trivially_relocatable;

                field &const x (type = i32);
                field &var x (type = i32) as x_var;
                field &unique var x (type = i32) as x_mut;
                field &unique const x (type = i32) as x_unique_const;
                field const x (type = i32) as x_const_copy;
                field &const type (type = i32) as type_;
                field var self_copy (type = c++::a::B);
                unsafe field &var next (type = c++::a::B*) as next_var;
                field var next (type = c++::a::B*) as next_copy;

                safe fn refs<'a>(&'a const self, &'a const c++::a::B, &'a var c++::a::B, &'a unique var c++::a::B, &'a unique const c++::a::B);
                safe fn builtins<'a>(&'a var self, &'a const i32, &'a var i32, &'a unique var i32, &'a unique const i32, &'a i32, &'a mut i32) -> &'a var bool;
                safe fn peek<'a>(&'a unique const self) -> &'a const c++::a::B*;
                unsafe fn raw(c++::a::B*, const i32*);
            }

            type crate::Foo {
                #layout(size = 0, align = 1);

                fn f(&self, &const c++::a::B, &unique var c++::a::B, &i32);
            }

            mod c++::a {
                safe fn pick<'a>(&'a const c++::a::B, &'a var c++::a::B) -> &'a unique var c++::a::B;
            }

            extern "C++" {

                safe fn refs<'a>(&'a const c++::a::B, &'a var c++::a::B, &'a unique var c++::a::B, &'a unique const c++::a::B);
                safe fn nullable<'a>(::std::option::Option<&'a const c++::a::B>, ::core::option::Option<&'a var c++::a::B>, ::std::option::Option<&'a unique var c++::a::B>, ::core::option::Option<&'a unique const i32>) -> ::std::option::Option<&'a const c++::a::B>;
                safe fn builtins<'a>(&'a const i32, &'a var i32, &'a unique var i32, &'a unique const i32, &'a i32, &'a mut i32) -> &'a var bool;
                unsafe fn raw(c++::a::B*, const c++::a::B&, i32&, const i32*, c++::a::B**, const i32**, i32* const*);

            }
        "#]],
    );
}

#[test]
fn reparse_rust_ref_to_const_cpp_ptr() {
    let _ = check_round_trip(
        r#"
#unstable(template_types)

type c++::a::B {
}

type<T> crate::Holder<T> {
    #layout(size = 8, align = 8);
    fn borrow<'a>(&'a self) -> &'a T;
}

type crate::Holder<const i32*> {
}

type crate::Holder<const c++::a::B**> {
}

extern "C++" {
    unsafe fn ptr_refs<'a>(&'a i32 const*, &'a mut const i32*, &'a const i32 const*);
}
"#,
        expect![[r#"
            type c++::a::B {
            }

            type crate::Holder<const i32*> {
                #layout(size = 8, align = 8);

                fn borrow<'a>(&'a self) -> &'a i32 const*;
            }

            type crate::Holder<const c++::a::B**> {
                #layout(size = 8, align = 8);

                fn borrow<'a>(&'a self) -> &'a c++::a::B const**;
            }

            extern "C++" {

                unsafe fn ptr_refs<'a>(&'a i32 const*, &'a mut const i32*, &'a const i32 const*);

            }
        "#]],
    );
}
