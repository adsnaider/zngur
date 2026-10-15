//! The C++ side of C++ member functions that Rust calls (`ZngurCppMethod`): `extern "C"`
//! functions that convert the arguments and call the member function directly, so that nobody
//! has to write a forwarding body by hand.

use std::collections::HashMap;

use zngur_def::{
    CppIndirection, Mutability, PrimitiveRustType, RustType, ZngurCppMethod, ZngurMethodReceiver,
};

use crate::rust::IntoCpp;

/// The C++ type of each type that Rust holds by value (`#cpp_stack_owned` types).
pub type StackOwnedTypes = HashMap<RustType, String>;

/// Renders bridge functions. They take their arguments and output the way Rust's generated code
/// passes them: as pointers to the Rust values (`i0`, `i1`, ..., and `o`).
pub struct BridgeRenderer<'a> {
    pub stack_owned: &'a StackOwnedTypes,
    pub namespace: &'a str,
    pub crate_name: &'a str,
}

impl BridgeRenderer<'_> {
    fn cpp(&self, ty: &RustType) -> String {
        ty.into_cpp(self.namespace, self.crate_name).to_string()
    }

    /// The Rust value `i{n}`, as a `ty` in C++.
    fn move_from_rust(&self, ty: &RustType, n: usize) -> String {
        format!(
            "::{}::__zngur_internal_move_from_rust< {} >(i{n})",
            self.namespace,
            self.cpp(ty)
        )
    }

    /// Adds the parameters for `inputs` to `params`, and returns the C++ arguments for them.
    fn args(&self, inputs: &[RustType], params: &mut Vec<String>) -> String {
        let mut args = vec![];
        for ty in inputs {
            let n = params.len();
            params.push(format!("uint8_t* i{n}"));
            args.push(match ty {
                RustType::CppPtr(_, CppIndirection::Pointer, _) => self.move_from_rust(ty, n),
                RustType::CppPtr(_, CppIndirection::Reference, _) => {
                    format!("*{}", self.move_from_rust(ty, n))
                }
                // Moved out of a temporary wrapper (`cpp() &&`), which then destroys what's left.
                ty if self.stack_owned.contains_key(ty) => {
                    format!("{}.cpp()", self.move_from_rust(ty, n))
                }
                ty => self.move_from_rust(ty, n),
            });
        }
        args.join(", ")
    }

    /// The bridge function `mangled_name` for `method` of `owner` (whose C++ type is
    /// `owner_cpp_type`).
    pub fn method(
        &self,
        mangled_name: &str,
        owner: &RustType,
        owner_cpp_type: &str,
        method: &ZngurCppMethod,
    ) -> String {
        let namespace = self.namespace;
        let mut params = vec![];
        let this = match method.receiver {
            ZngurMethodReceiver::Ref(m) => {
                let ptr = RustType::CppPtr(m, CppIndirection::Pointer, Box::new(owner.clone()));
                params.push("uint8_t* i0".to_owned());
                Some(self.move_from_rust(&ptr, 0))
            }
            ZngurMethodReceiver::Static | ZngurMethodReceiver::Move => None,
        };
        let args = self.args(&method.inputs, &mut params);
        params.push("uint8_t* o".to_owned());

        let name = &method.cpp_name;
        let call = match this {
            Some(this) => format!("({this})->{name}({args})"),
            None => format!("{owner_cpp_type}::{name}({args})"),
        };
        let move_to_rust = |init: String| {
            format!(
                "{} __zngur_o = {init};\n  ::{namespace}::__zngur_internal_move_to_rust(o, __zngur_o);",
                self.cpp(&method.output)
            )
        };
        // The C++ type that the member function must return, so that nothing is converted
        // implicitly (`None` if zngur can't name it, e.g. for Rust types).
        let expected_output = match &method.output {
            output if *output == RustType::UNIT => Some("void".to_owned()),
            RustType::CppPtr(_, CppIndirection::Pointer, _) => Some(self.cpp(&method.output)),
            RustType::CppPtr(m, CppIndirection::Reference, inner) => {
                let constness = if *m == Mutability::Not { "const " } else { "" };
                Some(format!(
                    "{constness}::{namespace}::CppTypeOf< {} >::type&",
                    self.cpp(inner)
                ))
            }
            output if self.stack_owned.contains_key(output) => {
                Some(self.stack_owned[output].clone())
            }
            RustType::Primitive(p)
                if !matches!(p, PrimitiveRustType::Str | PrimitiveRustType::Char) =>
            {
                Some(format!(
                    "::{namespace}::CppTypeOf< {} >::type",
                    self.cpp(&method.output)
                ))
            }
            _ => None,
        };
        let check = match expected_output {
            Some(expected) => {
                format!(
                    "static_assert(::{namespace}::__zngur_same_repr< decltype({call}), {expected} \
                     >::value, \"`{owner_cpp_type}::{name}` doesn't return the type declared in \
                     the .zng file\");\n  "
                )
            }
            None => String::new(),
        };
        let body = match &method.output {
            output if *output == RustType::UNIT => format!("(void)o;\n  {call};"),
            RustType::CppPtr(_, CppIndirection::Pointer, _) => move_to_rust(call),
            RustType::CppPtr(_, CppIndirection::Reference, _) => move_to_rust(format!("&({call})")),
            // Constructed in place, in the Rust value.
            output if self.stack_owned.contains_key(output) => {
                format!("new (o) {}({call});", self.stack_owned[output])
            }
            _ => move_to_rust(call),
        };
        format!(
            "void {mangled_name}({}) {{\n  {check}{body}\n}}\n",
            params.join(", ")
        )
    }
}
