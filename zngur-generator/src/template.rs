use crate::cpp::{
    CppExportedFnDefinition, CppExportedImplDefinition, CppFnDefinition, CppTraitDefinition,
    CppTypeDefinition,
};
use askama::Template;
use indexmap::IndexMap;
use zngur_def::*;
use zngur_def::{ZngurMethodReceiver, ZngurWellknownTraitData};

use crate::rust::IntoCpp;

/// Macro for template string interpolation over iterables with enumeration
///
/// Usage:
/// - splat!(items, |idx, ty|, "{ty} param{idx}")
/// - splat!(items.iter().skip(1), |n, t|, "{t} i{n}")
/// - splat!(items, "{el} i{n}")  // Default names: n (index), el (element)
macro_rules! splat {
    // Closure-style with custom variable names
    ($inputs:expr, |$n:ident, $el:ident|, $pattern:literal $(, $format_vars:expr)* $(,)?) => {{
        use itertools::Itertools;
        #[allow(unused_variables)]
        $inputs
            .into_iter()
            .enumerate()
            .map(|($n, $el)| format!($pattern $(, $format_vars)*))
            .join(", ")
    }};

    ($inputs:expr, |$n:ident, _|, $pattern:literal, $($format_vars:expr,)* $(,)?) => {{
      use itertools::Itertools;
      #[allow(unused_variables)]
      $inputs
          .into_iter()
          .enumerate()
          .map(|($n, $el)| format!($pattern $(, $format_vars)*))
          .join(", ")
  }};

    // Default names fallback
    ($inputs:expr, $pattern:literal) => {
        splat!($inputs, |n, el|, $pattern)
    };
}

#[derive(Template)]
#[template(path = "cpp_header.sptl", escape = "none")]
pub(crate) struct CppHeaderTemplate<'a> {
    pub(crate) panic_to_exception: bool,
    pub(crate) additional_includes: &'a String,
    pub(crate) fn_deps: &'a Vec<CppFnDefinition>,
    pub(crate) type_defs: &'a Vec<CppTypeDefinition>,
    pub(crate) trait_defs: &'a IndexMap<RustTrait, CppTraitDefinition>,
    pub(crate) exported_impls: &'a Vec<CppExportedImplDefinition>,
    pub(crate) exported_fn_defs: &'a Vec<CppExportedFnDefinition>,
    pub(crate) rust_cfg_defines: &'a Vec<String>,
    pub(crate) zng_header_in_place: bool,
    pub(crate) coro_support: &'a Option<crate::rust::CoroSupport>,
    pub(crate) namespace: &'a str,
    pub(crate) crate_name: &'a str,
}

impl<'a> CppHeaderTemplate<'a> {
    pub fn cpp_handle_field_name(&self, name: &str) -> String {
        crate::cpp::cpp_handle_field_name(name)
    }

    /// `self.type_defs` that specialize `CppTypeOf`, ordered so any inner `CppTypeOf` pointee is
    /// specialized before an outer template instantiation whose arguments mention `CppPtr<...>` to
    /// it.
    fn cpp_type_of_defs(&self) -> Vec<&'a CppTypeDefinition> {
        let defs: Vec<&'a CppTypeDefinition> = self
            .type_defs
            .iter()
            .filter(|td| td.cpp_object_type().is_some())
            .collect();
        fn visit<'a>(
            idx: usize,
            defs: &[&'a CppTypeDefinition],
            visited: &mut [bool],
            out: &mut Vec<&'a CppTypeDefinition>,
        ) {
            if visited[idx] {
                return;
            }
            visited[idx] = true;
            fn visit_nested<'a>(
                ty: &crate::cpp::CppType,
                defs: &[&'a CppTypeDefinition],
                visited: &mut [bool],
                out: &mut Vec<&'a CppTypeDefinition>,
            ) {
                for arg in &ty.generic_args {
                    visit_nested(arg, defs, visited, out);
                    if let Some(dep) = defs.iter().position(|d| &d.ty == arg) {
                        visit(dep, defs, visited, out);
                    }
                }
            }
            visit_nested(&defs[idx].ty, defs, visited, out);
            out.push(defs[idx]);
        }
        let mut visited = vec![false; defs.len()];
        let mut out = Vec::with_capacity(defs.len());
        for idx in 0..defs.len() {
            visit(idx, &defs, &mut visited, &mut out);
        }
        out
    }
}

#[derive(Template)]
#[template(path = "zng_header.sptl", escape = "none")]
pub(crate) struct ZngHeaderTemplate {
    pub(crate) panic_to_exception: bool,
    pub(crate) cpp_namespace: String,
}

impl<'a> CppHeaderTemplate<'a> {
    fn panic_handler(&self) -> String {
        if self.panic_to_exception {
            format!(
                r#"
            if (__zngur_read_and_reset_rust_panic()) {{
                throw ::{}::Panic{{}};
            }}
            "#,
                self.namespace
            )
        } else {
            "".to_owned()
        }
    }

    /// The body of a C++ function that calls the Rust function `link_name` with its parameters
    /// `i0`, `i1`, ... (of the C++ types `inputs`) and returns the result (an `out`). C++
    /// references (`CppRef<W>`) cross as pointers, so they're passed through a local pointer, and
    /// a returned one comes back as a pointer.
    fn call_rust_body(
        &self,
        link_name: &str,
        inputs: &[crate::cpp::CppType],
        out: &crate::cpp::CppType,
    ) -> String {
        let namespace = self.namespace;
        let mut prologue = String::new();
        let mut args = String::new();
        for (n, ty) in inputs.iter().enumerate() {
            match ty.native_ref_as_ptr(namespace) {
                Some(ptr) => {
                    prologue.push_str(&format!("{ptr} __zngur_i{n} = &i{n}; "));
                    args.push_str(&format!(
                        "::{namespace}::__zngur_internal_data_ptr(__zngur_i{n}), "
                    ));
                }
                None => {
                    prologue.push_str(&format!(
                        "::{namespace}::__zngur_internal_assume_deinit(i{n}); "
                    ));
                    args.push_str(&format!("::{namespace}::__zngur_internal_data_ptr(i{n}), "));
                }
            }
        }
        let (storage, ret) = match out.native_ref_as_ptr(namespace) {
            Some(ptr) => (ptr.to_string(), "return *o;".to_owned()),
            None => (
                out.to_string(),
                format!("::{namespace}::__zngur_internal_assume_init(o);\n      return o;"),
            ),
        };
        format!(
            r#"{storage} o{{}};
      {prologue}
      {link_name} (
        {args}
        ::{namespace}::__zngur_internal_data_ptr(o)
      );
      {panic_handler}
      {ret}"#,
            panic_handler = self.panic_handler()
        )
    }

    /// The body of a C++ constructor wrapper that initializes `target` by calling `link_name`.
    fn call_rust_constructor_body(
        &self,
        link_name: &str,
        inputs: &[crate::cpp::CppType],
        target: &str,
    ) -> String {
        let namespace = self.namespace;
        let mut prologue = format!("::{namespace}::__zngur_internal_assume_init({target});");
        let mut args = String::new();
        let mut epilogue = String::new();
        for (n, ty) in inputs.iter().enumerate() {
            match ty.native_ref_as_ptr(namespace) {
                Some(ptr) => {
                    prologue.push_str(&format!("\n      {ptr} __zngur_i{n} = &i{n};"));
                    args.push_str(&format!(
                        "::{namespace}::__zngur_internal_data_ptr(__zngur_i{n}),\n        "
                    ));
                }
                None => {
                    args.push_str(&format!(
                        "::{namespace}::__zngur_internal_data_ptr(i{n}),\n        "
                    ));
                    epilogue.push_str(&format!(
                        "\n      ::{namespace}::__zngur_internal_assume_deinit(i{n});"
                    ));
                }
            }
        }
        format!(
            r#"{prologue}
      {link_name}(
        {args}::{namespace}::__zngur_internal_data_ptr({target})
      );{epilogue}"#
        )
    }

    /// How a wrapper forwards its parameter `i{n}` (of the C++ type `ty`): moved, unless it's a
    /// C++ reference.
    fn forward_arg(&self, ty: &crate::cpp::CppType, n: usize) -> String {
        if ty.native_ref_as_ptr(self.namespace).is_some() {
            format!(", i{n}")
        } else {
            format!(", ::std::move(i{n})")
        }
    }

    pub fn render_type_methods(&self, td: &crate::cpp::CppTypeDefinition) -> String {
        use itertools::Itertools;
        use zngur_def::{Mutability, ZngurMethodReceiver};
        let mut s = String::new();

        let is_unsized = td.has_unsized();

        for method in &td.methods {
            let ty_str = td.ty.to_string();
            let fn_name = format!(
                "{}::{}",
                ty_str.strip_prefix("::").unwrap_or(&ty_str),
                method.name
            );
            let inputs = &method.sig.inputs;
            let out = &method.sig.output;
            let splat_inputs = inputs
                .iter()
                .enumerate()
                .map(|(n, ty)| format!("{ty} i{n}"))
                .join(", ");
            let splat_skip_inputs = inputs
                .iter()
                .skip(1)
                .enumerate()
                .map(|(n, ty)| format!("{ty} i{n}"))
                .join(", ");

            let body = self.call_rust_body(&method.sig.rust_link_name, inputs, out);
            s.push_str(&format!(
                r#"
    inline {out} {fn_name} ({splat_inputs}) noexcept {{
      {body}
    }}
"#,
            ));

            if let ZngurMethodReceiver::Ref(m) = method.kind {
                let ref_kinds: &[&str] = match m {
                    Mutability::Mut => &["RefMut"],
                    Mutability::Not => &["Ref", "RefMut"],
                };
                let field_kinds: &[&str] = match m {
                    Mutability::Mut => &["FieldOwned", "FieldRefMut"],
                    Mutability::Not => &["FieldOwned", "FieldRefMut", "FieldRef"],
                };

                let move_args = inputs
                    .iter()
                    .skip(1)
                    .enumerate()
                    .map(|(n, ty)| self.forward_arg(ty, n))
                    .join("");

                for field_kind in field_kinds {
                    s.push_str(&format!(
                        r#"
        template<typename Offset, typename... Offsets>
        inline {out} {namespace}::{field_kind}< {ty}, Offset, Offsets... >::{method_name}(
            {splat_skip_inputs}
        ) const noexcept {{
          return {fn_name}(
            *this
            {move_args}
          );
        }}
"#,
                        out = out,
                        namespace = self.namespace,
                        field_kind = field_kind,
                        ty = td.ty,
                        method_name = method.name,
                        splat_skip_inputs = splat_skip_inputs,
                        fn_name = fn_name,
                        move_args = move_args
                    ));
                }

                for ref_kind in ref_kinds {
                    s.push_str(&format!(
                        r#"
        inline {out} {namespace}::{ref_kind}< {ty} >::{method_name}(
            {splat_skip_inputs}
        ) const noexcept {{
          return {fn_name}(
            *this
            {move_args}
          );
        }}
"#,
                        out = out,
                        namespace = self.namespace,
                        ref_kind = ref_kind,
                        ty = td.ty,
                        method_name = method.name,
                        splat_skip_inputs = splat_skip_inputs,
                        fn_name = fn_name,
                        move_args = move_args
                    ));
                }
            }

            if !is_unsized
                && !td.layout.is_only_by_ref()
                && method.kind != ZngurMethodReceiver::Static
            {
                let this_arg = match method.kind {
                    ZngurMethodReceiver::Ref(_) => "*this",
                    ZngurMethodReceiver::Move => "::std::move(*this)",
                    ZngurMethodReceiver::Static => unreachable!(),
                };
                let const_str = if method.kind == ZngurMethodReceiver::Ref(Mutability::Not) {
                    "const"
                } else {
                    ""
                };
                let move_args = inputs
                    .iter()
                    .skip(1)
                    .enumerate()
                    .map(|(n, ty)| self.forward_arg(ty, n))
                    .join("");

                s.push_str(&format!(
                    r#"
      inline {out} {fn_name}(
            {splat_skip_inputs}
      ) {const_str} noexcept {{
        return {fn_name}(
          {this_arg}
          {move_args}
        );
      }}
"#,
                    out = out,
                    fn_name = fn_name,
                    splat_skip_inputs = splat_skip_inputs,
                    const_str = const_str,
                    this_arg = this_arg,
                    move_args = move_args
                ));
            }
        }
        s
    }

    pub fn render_from_trait(&self, td: &crate::cpp::CppTypeDefinition) -> String {
        use crate::cpp::CppTraitDefinition;

        use itertools::Itertools;
        let tr = td.from_trait.as_ref().and_then(|k| {
            self.trait_defs.get(k).or_else(|| {
                let k_canon = k.canonicalize_lifetimes(&[]);
                self.trait_defs
                    .iter()
                    .find_map(|(tk, v)| (tk.canonicalize_lifetimes(&[]) == k_canon).then_some(v))
            })
        });
        let name = td
            .ty
            .to_string()
            .strip_prefix("::")
            .unwrap_or(&td.ty.to_string())
            .to_string();
        match tr {
            Some(CppTraitDefinition::Fn { sig }) => {
                let as_std_function = format!(
                    "::std::function< {}({})>",
                    sig.output,
                    sig.inputs.iter().join(", ")
                );
                let ii_names = sig
                    .inputs
                    .iter()
                    .enumerate()
                    .map(|(n, x)| {
                        format!(
                            "::{}::__zngur_internal_move_from_rust< {x} >(i{n})",
                            self.namespace
                        )
                    })
                    .join(", ");
                let uint8_t_ix = if sig.inputs.is_empty() {
                    "".to_owned()
                } else {
                    sig.inputs
                        .iter()
                        .enumerate()
                        .map(|(n, _ty)| format!("uint8_t* i{n}"))
                        .join(", ")
                        + ", "
                };
                let out_ty = &sig.output;
                let link_name = &sig.rust_link_name;
                format!(
                    r#"
    inline {name} {name}::make_box({as_std_function} f) {{
      auto __zngur_data = new {as_std_function}(f);
      {name} o;
      ::{namespace}::__zngur_internal_assume_init(o);
      {link_name} (
        reinterpret_cast<uint8_t*>(__zngur_data),
        [](uint8_t *d) {{ delete reinterpret_cast< {as_std_function}*>(d); }},
        [](uint8_t *d, {uint8_t_ix} uint8_t* o) {{
          auto dd = reinterpret_cast< {as_std_function} *>(d);
          {out_ty} oo = (*dd)({ii_names});
          ::{namespace}::__zngur_internal_move_to_rust< {out_ty} >(o, oo);
        }},
        ::{namespace}::__zngur_internal_data_ptr(o)
      );
      return o;
    }}
"#,
                    namespace = self.namespace
                )
            }
            Some(CppTraitDefinition::Normal {
                as_ty, link_name, ..
            }) => {
                format!(
                    r#"
    template <typename T, typename... Args>
    {name} {name}::make_box(Args&&... args) {{
      auto __zngur_data = new T(::std::forward<Args>(args)...);
      auto data_as_impl = dynamic_cast< {as_ty}*>(__zngur_data);
      {name} o;
      ::{namespace}::__zngur_internal_assume_init(o);
      {link_name} (
        reinterpret_cast<uint8_t*>(data_as_impl),
        [](uint8_t *d) {{ delete reinterpret_cast< {as_ty}*>(d); }},
        ::{namespace}::__zngur_internal_data_ptr(o)
      );
      return o;
    }}
"#,
                    namespace = self.namespace
                )
            }
            None => "".to_owned(),
        }
    }

    pub fn render_from_trait_ref(&self, td: &crate::cpp::CppTypeDefinition) -> String {
        use crate::cpp::CppTraitDefinition;
        let tr = td.from_trait_ref.as_ref().and_then(|k| {
            self.trait_defs.get(k).or_else(|| {
                let k_canon = k.canonicalize_lifetimes(&[]);
                self.trait_defs
                    .iter()
                    .find_map(|(tk, v)| (tk.canonicalize_lifetimes(&[]) == k_canon).then_some(v))
            })
        });
        let name = td
            .ty
            .to_string()
            .strip_prefix("::")
            .unwrap_or(&td.ty.to_string())
            .to_string();
        match tr {
            Some(CppTraitDefinition::Fn { .. }) => "".to_owned(),
            Some(CppTraitDefinition::Normal {
                as_ty,
                link_name_ref,
                ..
            }) => {
                let mut s = String::new();
                for ref_kind in ["Ref", "RefMut"] {
                    s.push_str(&format!(
                        r#"
      {namespace}::{ref_kind}< {name} >::{ref_kind}({as_ty}& args) {{
        auto data_as_impl = &args;
        ::{namespace}::__zngur_internal_assume_init(*this);
        {link_name_ref}(
          (uint8_t *)data_as_impl,
          ::{namespace}::__zngur_internal_data_ptr(*this)
        );
      }}
"#,
                        namespace = self.namespace
                    ));
                }
                s
            }
            None => "".to_owned(),
        }
    }

    pub fn render_fn_deps(&self) -> String {
        use itertools::Itertools;
        let mut s = String::new();
        for fd in self.fn_deps {
            let open_ns = fd.name.open_namespace();
            let close_ns = fd.name.close_namespace();
            let out = &fd.sig.output;
            let name = fd.name.name();
            let inputs = &fd.sig.inputs;

            let splat_inputs = inputs
                .iter()
                .enumerate()
                .map(|(n, ty)| format!("{ty} i{n}"))
                .join(", ");

            let body = self.call_rust_body(&fd.sig.rust_link_name, inputs, out);
            s.push_str(&format!(
                r#"
{open_ns}
    inline {out} {name}({splat_inputs}) noexcept {{
      {body}
    }}
{close_ns}
"#,
            ));
        }
        s
    }
    pub fn render_exported_impls(&self) -> String {
        use itertools::Itertools;
        let mut s = String::new();
        for imp in self.exported_impls {
            let x = match &imp.tr {
                Some(x) => format!("{x}"),
                None => format!("::{}::Inherent", self.namespace),
            };

            s.push_str(&format!(
                r#"
  template<>
  class Impl< {ty}, {x} > {{
    public:
"#,
                ty = imp.ty,
                x = x
            ));

            for (name, sig) in &imp.methods {
                let inputs = &sig.inputs;
                let splat_inputs = inputs.iter().map(|ty| format!("{ty}")).join(", ");

                s.push_str(&format!(
                    r#"
        static {out} {name}(
          {splat_inputs}
        );
"#,
                    out = sig.output,
                    name = name,
                    splat_inputs = splat_inputs
                ));
            }

            s.push_str("  };\n");
        }
        s
    }

    fn render_zng_header(&self) -> String {
        let generator = ZngHeaderTemplate {
            panic_to_exception: self.panic_to_exception,
            cpp_namespace: self.namespace.to_owned(),
        };
        generator.render().unwrap()
    }
}

impl ZngHeaderTemplate {
    pub fn is_ref_kind_ref(&self, ref_kind: &str) -> bool {
        ref_kind == "Ref"
    }

    pub fn is_size_t(&self, ty: &str) -> bool {
        ty == "::size_t"
    }

    pub fn is_printable(&self, ty: &str) -> bool {
        ty.starts_with("int")
            || ty.starts_with("uint")
            || ty.starts_with("::size_t")
            || ty.starts_with("::double")
            || ty.starts_with("::float")
    }
    // TODO: Docs - what do these represent? When will we change this list?
    fn builtin_types(&self) -> Vec<String> {
        let builtins = [8, 16, 32, 64]
            .into_iter()
            .flat_map(|x| [format!("int{x}_t"), format!("uint{x}_t")])
            .chain(["::double_t".to_owned(), "::float_t".to_owned()])
            .flat_map(|x| {
                [
                    x.clone(),
                    format!("::{}::Ref<{x}>", &self.cpp_namespace),
                    format!("::{}::RefMut<{x}>", &self.cpp_namespace),
                ]
            });
        builtins
            .chain([
                format!("::{}::ZngurCppOpaqueOwnedObject", &self.cpp_namespace),
                "::size_t".to_owned(),
            ])
            .collect()
    }
}

#[derive(Template)]
#[template(path = "cpp_source.sptl", escape = "none")]
pub(crate) struct CppSourceTemplate<'a> {
    pub(crate) header_file_name: &'a String,
    pub(crate) type_defs: &'a Vec<CppTypeDefinition>,
    pub(crate) trait_defs: &'a IndexMap<RustTrait, CppTraitDefinition>,
    pub(crate) exported_fn_defs: &'a Vec<CppExportedFnDefinition>,
    pub(crate) exported_impls: &'a Vec<CppExportedImplDefinition>,
    pub(crate) cpp_method_bridges: &'a Vec<String>,
    pub(crate) cpp_namespace: &'a str,
    pub(crate) coro_support: &'a Option<crate::rust::CoroSupport>,
}
