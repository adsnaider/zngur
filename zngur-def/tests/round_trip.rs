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
    assert_eq!(parsed.spec.traits, reparsed.spec.traits);
    assert_eq!(parsed.spec.funcs, reparsed.spec.funcs);
    assert_eq!(parsed.spec.extern_cpp_funcs, reparsed.spec.extern_cpp_funcs);
    assert_eq!(parsed.spec.extern_cpp_impls, reparsed.spec.extern_cpp_impls);
    assert_eq!(parsed.spec.cpp_templates, reparsed.spec.cpp_templates);
    assert_eq!(
        parsed.spec.cpp_template_homes,
        reparsed.spec.cpp_template_homes
    );

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
fn reparse_cpp_methods() {
    let _ = check_round_trip(
        r#"
#crate my_crate
type c++::a::B {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
    unsafe fn value() const -> i32;
    unsafe fn set(i32, const c++::a::B&);
    safe static fn make(i32) -> c++::a::B;
    unsafe fn get(i32) const -> const i32& as get_const;
    safe constructor();
    safe constructor(i32, i32);
    unsafe constructor(const c++::a::B&);
    field const x (type = i32);
    field x (type = i32) as x_mut;
}
        "#,
        expect![[r##"
            #crate my_crate

            type c++::a::B {
                #layout(size = 8, align = 4);
                #trivially_relocatable;

                field const x (type = i32);
                field x (type = i32) as x_mut;

                safe constructor();
                safe constructor(i32, i32);
                unsafe constructor(const c++::a::B&);

                unsafe fn value() const -> i32;
                unsafe fn set(i32, const c++::a::B&);
                safe static fn make(i32) -> c++::a::B;
                unsafe fn get(i32) const -> const i32& as get_const;
            }
        "##]],
    );
}

#[test]
fn reparse_cpp_templates() {
    let _ = check_round_trip(
        r#"
#crate my_crate
type c++::std::unique_ptr<c++::a::B> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
    unsafe fn get() const -> c++::a::B*;
}
type c++::std::map<i32, const c++::a::B*> {
}
        "#,
        expect![[r##"
            #crate my_crate

            type c++::std::unique_ptr<c++::a::B> {
                #layout(size = 8, align = 8);
                #trivially_relocatable;

                unsafe fn get() const -> c++::a::B*;
            }

            type c++::std::map<i32, const c++::a::B*> {
            }
        "##]],
    );
}

#[test]
fn reparse_cpp_template_home() {
    let _ = check_round_trip(
        r#"
type<T> c++::std::unique_ptr<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
    unsafe fn get() const -> T*;
}
type c++::std::unique_ptr<i32> {
}
        "#,
        expect![[r#"
            type<T> c++::std::unique_ptr<T> {
                #layout(size = 8, align = 8);
                #trivially_relocatable;

                unsafe fn get() const -> T*;
            }

            type c++::std::unique_ptr<i32> {
                #layout(size = 8, align = 8);
                #trivially_relocatable;

                unsafe fn get() const -> i32*;
            }
        "#]],
    );
}

#[test]
fn reparse_cpp_functions() {
    let _ = check_round_trip(
        r#"
#crate my_crate
mod c++::a {
    type B {
        #layout(size = 8, align = 4);
        #trivially_relocatable;
    }
    unsafe fn f(const B&) -> i32;
    safe fn g(i32) -> B as make_b;
    mod b {
        safe fn h();
    }
}
        "#,
        expect![[r##"
            #crate my_crate

            type c++::a::B {
                #layout(size = 8, align = 4);
                #trivially_relocatable;
            }

            mod c++::a {
                unsafe fn f(const c++::a::B&) -> i32;
                safe fn g(i32) -> c++::a::B as make_b;
            }
            mod c++::a::b {
                safe fn h();
            }
        "##]],
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
        // `ZngurSpec` and some of its fields don't implement `PartialEq`, so
        // compare their `Debug` representations instead.
        macro_rules! debug_fields {
            ($($field:ident),*) => {
                [$((
                    stringify!($field),
                    format!("{:#?}", a.$field),
                    format!("{:#?}", b.$field),
                )),*]
            };
        }
        let fields = debug_fields!(
            types,
            traits,
            funcs,
            extern_cpp_funcs,
            extern_cpp_impls,
            imported_modules,
            additional_includes,
            convert_panic_to_exception
        );
        for (name, before, after) in fields {
            if before != after {
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
                    "{}: `{name}` changed after round trip\n--- before ---\n{}\n--- after ---\n{}",
                    path.display(),
                    context(&before),
                    context(&after),
                ));
            }
        }
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
fn reparse_lifetimes() {
    check_round_trip(
        r#"
type<'a> crate::Parser<'a> {
    #layout(size = 24, align = 8);
    field input (offset = 0, type = &'a str);
    fn rest(&self) -> &'a str;
    fn peek<'b>(&'b self) -> ::std::option::Option<&'b str>;
}

type ::std::slice::Iter<'static, i32> {
    #layout(size = 16, align = 8);
}

type Box<dyn for<'a> Fn(&'a i32, &'a i32) -> &'a i32> {
    #layout(size = 16, align = 8);
}

trait crate::Picker {
    fn pick<'a>(&self, &'a i32, &'a i32) -> &'a i32;
}

mod crate {
    fn longest<'a>(&'a str, &'a str) -> &'a str;
}

extern "C++" {
    safe fn pick<'a>(&'a crate::Foo, &'a mut crate::Foo) -> &'a i32;
    impl<'a> crate::Parser<'a> {
        safe fn first(&'a self) -> &'a str;
    }
}
"#,
        expect![[r#"
            type<'a> crate::Parser<'a> {
                #layout(size = 24, align = 8);

                field input (offset = 0, type = &'a str);

                fn rest(&self) -> &'a str;
                fn peek<'b>(&'b self) -> ::std::option::Option<&'b str>;
            }

            type ::std::slice::Iter<'static, i32> {
                #layout(size = 16, align = 8);
            }

            type Box<dyn for<'a> Fn(&'a i32, &'a i32) -> &'a i32> {
                #layout(size = 16, align = 8);
            }

            mod crate {
                fn longest<'a>(&'a str, &'a str) -> &'a str;
            }

            trait crate::Picker {
                fn pick<'a>(&self, &'a i32, &'a i32) -> &'a i32;
            }

            extern "C++" {

                safe fn pick<'a>(&'a crate::Foo, &'a mut crate::Foo) -> &'a i32;

                impl<'a> crate::Parser<'a> {
                    safe fn first(&'a self) -> &'a str;
                }

            }
        "#]],
    );
}
