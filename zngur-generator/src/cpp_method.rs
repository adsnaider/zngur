//! The C++ side of C++ member functions and constructors that Rust calls (`ZngurCppMethod`,
//! `ZngurCppConstructor`): `extern "C"` functions that convert the arguments and call them
//! directly, so that nobody has to write a forwarding body by hand.

use std::collections::HashMap;

use zngur_def::{
    CppConstness, CppIndirection, PrimitiveRustType, RustType, ZngurCppConstructor, ZngurCppMethod,
    ZngurMethodReceiver,
};

use crate::rust::IntoCpp;

/// The C++ type of each type that Rust holds by value (`c++::` types with a layout and
/// `#trivially_relocatable`).
pub type StackOwnedTypes = HashMap<RustType, String>;

/// The C++ `this` of a member function of `owner` whose receiver has the constness `constness`:
/// `const T*` or `T*` (via `CppPtr` on `const T` / `var T`).
fn this_ptr(constness: CppConstness, owner: &RustType) -> RustType {
    RustType::CppPtr(
        CppIndirection::Pointer,
        Box::new(RustType::cpp_object(constness, owner.clone())),
    )
}

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
            // C++ object and builtin references are C++ references, which cross as pointers.
            let ty = &ty.cpp_bridge_type();
            args.push(match ty {
                RustType::CppPtr(CppIndirection::Pointer, _) => self.move_from_rust(ty, n),
                RustType::CppPtr(CppIndirection::Reference, pointee) => {
                    let ptr = RustType::CppPtr(CppIndirection::Pointer, pointee.clone());
                    format!("*{}", self.move_from_rust(&ptr, n))
                }
                RustType::Primitive(PrimitiveRustType::Bool) => {
                    format!("static_cast<bool>(*i{n})")
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
        let mut params = vec![];
        let this = match method.receiver {
            ZngurMethodReceiver::Ref(_) => {
                let ptr = this_ptr(method.receiver_constness, owner);
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
        self.call(
            mangled_name,
            &params,
            &call,
            &method.output,
            &format!("{owner_cpp_type}::{name}"),
        )
    }

    /// A bridge function that makes the C++ call `call`, of the function `callee`, and passes its
    /// result, which must be an `output`, to Rust.
    fn call(
        &self,
        mangled_name: &str,
        params: &[String],
        call: &str,
        output: &RustType,
        callee: &str,
    ) -> String {
        let namespace = self.namespace;
        // C++ object and builtin references are C++ references, which cross as pointers.
        let output = &output.cpp_bridge_type();
        let move_to_rust = |ty: &RustType, init: String| {
            format!(
                "{} __zngur_o = {init};\n  ::{namespace}::__zngur_internal_move_to_rust(o, __zngur_o);",
                self.cpp(ty)
            )
        };
        // The C++ type that the function must return, so that nothing is converted implicitly.
        let expected_output = match output {
            output if *output == RustType::UNIT => "void".to_owned(),
            RustType::CppPtr(CppIndirection::Pointer, _) => self.cpp(output),
            RustType::CppPtr(CppIndirection::Reference, pointee) => {
                format!("::{namespace}::CppTypeOf< {} >::type&", self.cpp(pointee))
            }
            output if self.stack_owned.contains_key(output) => self.stack_owned[output].clone(),
            RustType::Primitive(p)
                if !matches!(p, PrimitiveRustType::Str | PrimitiveRustType::Char) =>
            {
                format!("::{namespace}::CppTypeOf< {} >::type", self.cpp(output))
            }
            _ => self.cpp(output),
        };
        let check = format!(
            "static_assert(::{namespace}::__zngur_same_repr< decltype({call}), {expected_output} \
             >::value, \"`{callee}` doesn't return the type declared in the .zng file\");\n  "
        );
        let body = match output {
            output if *output == RustType::UNIT => format!("(void)o;\n  {call};"),
            RustType::CppPtr(CppIndirection::Pointer, _) => move_to_rust(output, call.to_owned()),
            RustType::CppPtr(CppIndirection::Reference, pointee) => {
                let ptr = RustType::CppPtr(CppIndirection::Pointer, pointee.clone());
                move_to_rust(&ptr, format!("&({call})"))
            }
            RustType::Primitive(PrimitiveRustType::Bool) => {
                format!("bool __zngur_o = {call};\n  *o = static_cast<uint8_t>(__zngur_o);")
            }
            // Constructed in place, in the Rust value.
            output if self.stack_owned.contains_key(output) => {
                format!(
                    "::new (static_cast<void*>(o)) {}({call});",
                    self.stack_owned[output]
                )
            }
            _ => move_to_rust(output, call.to_owned()),
        };
        format!(
            "void {mangled_name}({}) {{\n  {check}{body}\n}}\n",
            params.join(", ")
        )
    }

    /// The bridge function `mangled_name` for `constructor` of the C++ type `owner_cpp_type`,
    /// which constructs the object in place, in `o`.
    pub fn constructor(
        &self,
        mangled_name: &str,
        owner_cpp_type: &str,
        constructor: &ZngurCppConstructor,
    ) -> String {
        let mut params = vec![];
        let args = self.args(&constructor.inputs, &mut params);
        params.push("uint8_t* o".to_owned());
        format!(
            "void {mangled_name}({}) {{\n  ::new (static_cast<void*>(o)) {owner_cpp_type}({args});\n}}\n",
            params.join(", ")
        )
    }
}
