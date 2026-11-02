use std::collections::HashSet;

use crate::{
    AdditionalIncludes, ConvertPanicToException, Lifetime, Mutability, RustTrait, RustType,
    ZngurConstructor, ZngurExternCppFn, ZngurExternCppImpl, ZngurField, ZngurFn, ZngurMethod,
    ZngurMethodDetails, ZngurMethodReceiver, ZngurSpec, ZngurTrait, ZngurType, ZngurVariant,
};

/// Trait for types with a partial union operation.
///
/// If a type T is Merge, it provides a partial union operation `merge`: T x T -> T.
///
/// Partial unions do not need to be homogeneous. If a type U is Merge<T>,
/// it provides a partial union operation `merge`: T X U -> U.
/// For example, T: usize, U: Set<usize>; the partial union is the result of
/// adding the lhs usize to the rhs Set.
///
/// "Partial" means the result is not necessarily defined for all inputs; the union may fail.
/// This is often because the instances are contradictory (as defined by the type).
///
/// There are no guarantees about the state of the mutable argument, `into`, in the case
/// of a failed merge. `merge` is not required to leave `into` in a valid state, or restore
/// it to its original state.
pub trait Merge<T = Self> {
    /// Writes the partial union of `self` and `into` to the latter.
    ///
    /// # Errors
    ///
    /// If the instances are contradictory, a `MergeFailure` is returned.
    fn merge(self, into: &mut T) -> MergeResult;
}

/// The result of a merge operation.
pub type MergeResult = Result<(), MergeFailure>;

/// An unsuccessful merge operation.
pub enum MergeFailure {
    /// The merge was not successful because of a conflict.
    /// the second member is a tuple of conflict sources:
    ///     (source in self, source in the item merging into)
    Conflict(String, (ConflictSource, ConflictSource)),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConflictSource {
    Type(RustType),
    Layout,
    Constructor(Option<Vec<(String, RustType)>>),
    Method(ZngurMethod),
    CppMethod(Box<crate::ZngurCppMethod>),
    /// A C++ constructor, by its Rust name.
    CppConstructor(String),
    /// A C++ field accessor, by its Rust name.
    CppField(String),
    /// A free C++ function, by its namespace and Rust name.
    CppFn(Vec<String>, String),
    Field(String),
}

/// Push an item onto a vector if it is not already present, in linear time.
fn push_unique<T: Eq>(item: T, smallvec: &mut std::vec::Vec<T>) {
    if !smallvec.contains(&item) {
        smallvec.push(item);
    }
}

/// Writes the union of `other` and `smallvec` to the latter in O(N * M) time.
fn inplace_union<T: Eq>(other: Vec<T>, smallvec: &mut std::vec::Vec<T>) {
    for item in other {
        push_unique(item, smallvec);
    }
}

/// Writes the union of `other` and `smallvec` to the latter in O(N * M) time.
/// If an element in `other` has the same identity as an element in `smallvec`,
/// the elements are merged. Otherwise, the element is added to `smallvec`.
fn merge_by_identity<T: Merge>(
    other: Vec<T>,
    smallvec: &mut std::vec::Vec<T>,
    identity: impl Fn(&T, &T) -> bool,
) -> MergeResult {
    for item in other {
        if let Some(existing) = smallvec.iter_mut().find(|e| identity(e, &item)) {
            item.merge(existing)?;
        } else {
            smallvec.push(item);
        }
    }
    Ok(())
}

impl<T: Merge> Merge for Option<T> {
    /// Writes the partial union of `self` and `into` to the latter.
    ///
    /// If both `self` and `into` are Some, the underlying values are merged.
    /// Otherwise, the result is whichever value is Some, or None if neither is.
    fn merge(self, into: &mut Self) -> MergeResult {
        match self {
            Some(src) => match into.as_mut() {
                Some(dst) => src.merge(dst),
                None => {
                    *into = Some(src);
                    Ok(())
                }
            },
            None => Ok(()),
        }
    }
}

impl<K, V, I: IntoIterator<Item = (K, V)>> Merge<indexmap::IndexMap<K, V>> for I
where
    K: Eq + std::hash::Hash,
    V: Merge,
{
    /// Merges a sequence of key-value pairs into a map.
    ///
    /// If a key is present in both `self` and `into`, the corresponding values are merged.
    /// Otherwise, the entry from `self` is inserted into `into`.
    ///
    /// This implementation implies `indexmap::IndexMap<K,V>` is `Merge` for all `V: Merge`,
    /// because IndexMap is `IntoIterator`. We use `IntoIterator` to allow literal sequences of
    /// key-value pairs to be merged into a map.
    fn merge(self, into: &mut indexmap::IndexMap<K, V>) -> MergeResult {
        for (key, value) in self {
            match into.entry(key) {
                indexmap::map::Entry::Vacant(e) => {
                    e.insert(value);
                }
                indexmap::map::Entry::Occupied(mut e) => match value.merge(e.get_mut()) {
                    Ok(()) => {}
                    Err(message) => {
                        return Err(message);
                    }
                },
            }
        }
        Ok(())
    }
}

impl Merge for ZngurType {
    /// Writes the partial union of `self` and `into` to the latter.
    ///
    /// PRECONDITION: `self.ty == into.ty`.
    fn merge(self, into: &mut Self) -> MergeResult {
        if self.ty != into.ty {
            panic!(
                "Attempt to merge different types: {} and {}",
                self.ty, into.ty
            );
        }

        if let (Some(layout1), Some(layout2)) = &(self.layout, into.layout)
            && layout1 != layout2
        {
            return Err(MergeFailure::Conflict(
                "Conflicting layout policy found".to_string(),
                (ConflictSource::Layout, ConflictSource::Layout),
            ));
        } else {
            into.layout = into.layout.or(self.layout);
        }

        // An unchecked promise (`#unsafe(trivially_relocatable)`) wins over a checked one.
        into.trivially_relocatable = into
            .trivially_relocatable
            .combine(self.trivially_relocatable);

        inplace_union(self.wellknown_traits, &mut into.wellknown_traits);
        merge_by_identity(self.methods, &mut into.methods, |a, b| {
            a.data.name == b.data.name
        })?;
        merge_by_identity(self.cpp_methods, &mut into.cpp_methods, |a, b| {
            a.rust_name() == b.rust_name()
        })?;
        merge_by_identity(self.cpp_constructors, &mut into.cpp_constructors, |a, b| {
            a.rust_name == b.rust_name
        })?;
        merge_by_identity(self.cpp_fields, &mut into.cpp_fields, |a, b| {
            a.rust_name() == b.rust_name()
        })?;
        self.constructor.merge(&mut into.constructor)?;
        into.exhaustive = self.exhaustive && into.exhaustive;
        merge_by_identity(self.variants, &mut into.variants, |a, b| a.name == b.name)?;
        merge_by_identity(self.fields, &mut into.fields, |a, b| a.name == b.name)?;

        Ok(())
    }
}

impl Merge for ZngurTrait {
    /// Writes the partial union of `self` and `into` to the latter.
    ///
    /// PRECONDITION: `self.tr == into.tr`.
    fn merge(self, into: &mut Self) -> MergeResult {
        if self.tr != into.tr {
            panic!(
                "Attempt to merge different traits: {} and {}",
                self.tr, into.tr
            );
        }

        merge_by_identity(self.methods, &mut into.methods, |a, b| a.name == b.name)?;

        Ok(())
    }
}

impl Merge for crate::ZngurCppMethod {
    fn merge(self, into: &mut Self) -> MergeResult {
        if canonicalize_cpp_method(&self) != canonicalize_cpp_method(into) {
            return Err(MergeFailure::Conflict(
                "C++ method mismatch".to_string(),
                (
                    ConflictSource::CppMethod(Box::new(self)),
                    ConflictSource::CppMethod(Box::new(into.clone())),
                ),
            ));
        }
        Ok(())
    }
}

impl Merge for crate::ZngurCppField {
    fn merge(self, into: &mut Self) -> MergeResult {
        let unwrap_cpp_const = |ty: &RustType| match ty {
            RustType::CppConst(inner) => (**inner).clone(),
            other => other.clone(),
        };
        let canonical = |f: &Self| crate::ZngurCppField {
            name: f.name.clone(),
            rust_name: f
                .rust_name
                .as_deref()
                .filter(|&n| n != f.name)
                .map(str::to_owned),
            ty: unwrap_cpp_const(&f.ty),
            access: f.access,
            is_safe: f.is_safe,
        };
        if canonical(&self) != canonical(into) {
            return Err(MergeFailure::Conflict(
                "C++ field accessor mismatch".to_string(),
                (
                    ConflictSource::CppField(self.rust_name().to_owned()),
                    ConflictSource::CppField(into.rust_name().to_owned()),
                ),
            ));
        }
        if matches!(self.ty, RustType::CppConst(_)) && !matches!(into.ty, RustType::CppConst(_)) {
            into.ty = self.ty;
        }
        Ok(())
    }
}

impl Merge for crate::ZngurCppConstructor {
    fn merge(self, into: &mut Self) -> MergeResult {
        if canonicalize_cpp_constructor(&self) != canonicalize_cpp_constructor(into) {
            let source = |c: &Self| ConflictSource::CppConstructor(c.rust_name.clone());
            return Err(MergeFailure::Conflict(
                "C++ constructor mismatch".to_string(),
                (source(&self), source(into)),
            ));
        }
        Ok(())
    }
}

fn collect_method_lifetimes(
    m: &ZngurMethod,
    deref: Option<&(RustType, Mutability)>,
    out: &mut HashSet<Lifetime>,
) {
    out.extend(m.lifetimes.iter().cloned());
    if let Some(rl) = &m.receiver_lifetime
        && !rl.is_builtin()
    {
        out.insert(rl.clone());
    }
    for t in m
        .inputs
        .iter()
        .chain([&m.output])
        .chain(&m.generics)
        .chain(deref.map(|(t, _)| t))
    {
        out.extend(t.all_lifetimes());
    }
}

fn align_method_lifetimes(
    m: &mut ZngurMethod,
    deref: Option<&mut (RustType, Mutability)>,
    src_outer: &[Lifetime],
    dst_outer: &[Lifetime],
    avoid: &mut HashSet<Lifetime>,
) {
    collect_method_lifetimes(m, deref.as_deref(), avoid);
    let mapping: Vec<(Lifetime, Lifetime)> = src_outer
        .iter()
        .cloned()
        .zip(dst_outer.iter().cloned())
        .collect();
    let mut method_renames: Vec<(Lifetime, Lifetime)> = Vec::new();
    let mut counter = 0usize;
    for ml in &mut m.lifetimes {
        if dst_outer.contains(ml) {
            let fresh = loop {
                let candidate = Lifetime(format!("{}_{counter}", ml.0));
                counter += 1;
                if !avoid.contains(&candidate) {
                    break candidate;
                }
            };
            avoid.insert(fresh.clone());
            method_renames.push((ml.clone(), fresh.clone()));
            *ml = fresh;
        }
    }
    let method_bound = m.lifetimes.clone();
    let remap_method_lt = |l: &Lifetime| -> Lifetime {
        if let Some((_, fresh)) = method_renames.iter().find(|(old, _)| old == l) {
            return fresh.clone();
        }
        if method_bound.contains(l) {
            return l.clone();
        }
        mapping
            .iter()
            .find(|(from, _)| from == l)
            .map(|(_, to)| to.clone())
            .unwrap_or_else(|| l.clone())
    };
    if let Some(rl) = &mut m.receiver_lifetime {
        *rl = remap_method_lt(rl);
    }
    let in_scope: HashSet<Lifetime> = dst_outer.iter().chain(&m.lifetimes).cloned().collect();
    let mut remap_method_ty = |ty: &mut RustType| {
        let remapped = ty.map_free_lifetimes(&mut |l| remap_method_lt(l));
        *ty = if in_scope.is_empty() {
            remapped
        } else {
            remapped.rename_colliding_for_binders(&in_scope, avoid)
        };
    };
    m.inputs.iter_mut().for_each(&mut remap_method_ty);
    remap_method_ty(&mut m.output);
    m.generics.iter_mut().for_each(&mut remap_method_ty);
    if let Some((deref_ty, _)) = deref {
        remap_method_ty(deref_ty);
    }
}

/// Renames `src.lifetimes` (and `src.ty`) to match `dst_lifetimes` and `dst_ty`, avoiding capture
/// by method-level lifetime parameters or `for<...>` binders.
fn align_type_lifetimes(src: &mut ZngurType, dst_lifetimes: &[Lifetime], dst_ty: &RustType) {
    if src.lifetimes != dst_lifetimes {
        let mut avoid: HashSet<Lifetime> = dst_lifetimes
            .iter()
            .chain(&src.lifetimes)
            .cloned()
            .collect();
        avoid.extend(dst_ty.all_lifetimes());
        avoid.extend(src.ty.all_lifetimes());
        for m in &src.methods {
            collect_method_lifetimes(&m.data, m.deref.as_ref(), &mut avoid);
        }
        for m in &src.cpp_methods {
            avoid.extend(m.lifetimes.iter().cloned());
            if let Some(rl) = &m.receiver_lifetime
                && !rl.is_builtin()
            {
                avoid.insert(rl.clone());
            }
            for t in m.inputs.iter().chain([&m.output]).chain(&m.template_args) {
                avoid.extend(t.all_lifetimes());
            }
        }
        for c in &src.cpp_constructors {
            avoid.extend(c.lifetimes.iter().cloned());
            for t in &c.inputs {
                avoid.extend(t.all_lifetimes());
            }
        }
        for f in &src.fields {
            avoid.extend(f.ty.all_lifetimes());
        }
        for v in &src.variants {
            for f in &v.fields {
                avoid.extend(f.ty.all_lifetimes());
            }
        }
        if let Some(c) = &src.constructor {
            for (_, t) in &c.inputs {
                avoid.extend(t.all_lifetimes());
            }
        }

        for m in &mut src.methods {
            align_method_lifetimes(
                &mut m.data,
                m.deref.as_mut(),
                &src.lifetimes,
                dst_lifetimes,
                &mut avoid,
            );
        }
        for m in &mut src.cpp_methods {
            let mut tmp = ZngurMethod {
                name: m.rust_name().to_owned(),
                lifetimes: m.lifetimes.clone(),
                generics: std::mem::take(&mut m.template_args),
                receiver: m.receiver,
                receiver_lifetime: m.receiver_lifetime.clone(),
                inputs: m.inputs.clone(),
                output: m.output.clone(),
                is_safe: Some(m.is_safe),
            };
            align_method_lifetimes(&mut tmp, None, &src.lifetimes, dst_lifetimes, &mut avoid);
            m.lifetimes = tmp.lifetimes;
            m.template_args = tmp.generics;
            m.receiver_lifetime = tmp.receiver_lifetime;
            m.inputs = tmp.inputs;
            m.output = tmp.output;
        }
        for c in &mut src.cpp_constructors {
            let mut tmp = ZngurMethod {
                name: c.rust_name.clone(),
                lifetimes: c.lifetimes.clone(),
                generics: vec![],
                receiver: ZngurMethodReceiver::Static,
                receiver_lifetime: None,
                inputs: c.inputs.clone(),
                output: RustType::UNIT,
                is_safe: Some(c.is_safe),
            };
            align_method_lifetimes(&mut tmp, None, &src.lifetimes, dst_lifetimes, &mut avoid);
            c.lifetimes = tmp.lifetimes;
            c.inputs = tmp.inputs;
        }

        let mapping: Vec<(Lifetime, Lifetime)> = src
            .lifetimes
            .iter()
            .cloned()
            .zip(dst_lifetimes.iter().cloned())
            .collect();
        let remap_lt = |l: &Lifetime| -> Lifetime {
            mapping
                .iter()
                .find(|(from, _)| from == l)
                .map(|(_, to)| to.clone())
                .unwrap_or_else(|| l.clone())
        };
        let outer_in_scope: HashSet<Lifetime> = dst_lifetimes.iter().cloned().collect();
        let mut remap_ty = |ty: &mut RustType| {
            let remapped = ty.map_free_lifetimes(&mut |l| remap_lt(l));
            *ty = if outer_in_scope.is_empty() {
                remapped
            } else {
                remapped.rename_colliding_for_binders(&outer_in_scope, &mut avoid)
            };
        };
        for field in &mut src.fields {
            remap_ty(&mut field.ty);
        }
        for variant in &mut src.variants {
            for field in &mut variant.fields {
                remap_ty(&mut field.ty);
            }
        }
        if let Some(constructor) = &mut src.constructor {
            for (_, ty) in &mut constructor.inputs {
                remap_ty(ty);
            }
        }
    }
    src.lifetimes = dst_lifetimes.to_vec();
    src.ty = dst_ty.clone();
}

fn canonicalize_method(m: &ZngurMethod) -> ZngurMethod {
    let outer = &m.lifetimes;
    let canon_ty = |t: &RustType| t.canonicalize_lifetimes(outer);
    let canon_lt = |l: &Lifetime| {
        outer
            .iter()
            .position(|ol| ol == l)
            .map(|i| Lifetime(format!("__outer_{i}")))
            .unwrap_or_else(|| l.clone())
    };
    ZngurMethod {
        name: m.name.clone(),
        lifetimes: (0..outer.len())
            .map(|i| Lifetime(format!("__outer_{i}")))
            .collect(),
        generics: m.generics.iter().map(canon_ty).collect(),
        receiver: m.receiver,
        receiver_lifetime: m.receiver_lifetime.as_ref().map(canon_lt),
        inputs: m.inputs.iter().map(canon_ty).collect(),
        output: canon_ty(&m.output),
        is_safe: m.is_safe,
    }
}

fn canonicalize_method_details(m: &ZngurMethodDetails) -> ZngurMethodDetails {
    let outer = &m.data.lifetimes;
    ZngurMethodDetails {
        data: canonicalize_method(&m.data),
        use_path: m.use_path.clone(),
        deref: m
            .deref
            .as_ref()
            .map(|(t, mu)| (t.canonicalize_lifetimes(outer), *mu)),
        cpp_name: m.cpp_name.clone(),
    }
}

fn canonicalize_cpp_method(m: &crate::ZngurCppMethod) -> crate::ZngurCppMethod {
    let outer = &m.lifetimes;
    let canon_ty = |t: &RustType| t.canonicalize_lifetimes(outer);
    let canon_lt = |l: &Lifetime| {
        outer
            .iter()
            .position(|ol| ol == l)
            .map(|i| Lifetime(format!("__outer_{i}")))
            .unwrap_or_else(|| l.clone())
    };
    crate::ZngurCppMethod {
        cpp_name: m.cpp_name.clone(),
        lifetimes: (0..outer.len())
            .map(|i| Lifetime(format!("__outer_{i}")))
            .collect(),
        template_args: m.template_args.iter().map(canon_ty).collect(),
        rust_name: m.rust_name.clone(),
        receiver: m.receiver,
        receiver_constness: m.receiver_constness,
        receiver_lifetime: m.receiver_lifetime.as_ref().map(canon_lt),
        raw_receiver: m.raw_receiver,
        inputs: m.inputs.iter().map(canon_ty).collect(),
        output: canon_ty(&m.output),
        is_safe: m.is_safe,
    }
}

fn canonicalize_cpp_constructor(c: &crate::ZngurCppConstructor) -> crate::ZngurCppConstructor {
    let outer = &c.lifetimes;
    let canon_ty = |t: &RustType| t.canonicalize_lifetimes(outer);
    crate::ZngurCppConstructor {
        rust_name: c.rust_name.clone(),
        lifetimes: (0..outer.len())
            .map(|i| Lifetime(format!("__outer_{i}")))
            .collect(),
        inputs: c.inputs.iter().map(canon_ty).collect(),
        is_safe: c.is_safe,
    }
}

impl Merge<ZngurSpec> for ZngurType {
    /// Merges a type into a specification's type list.
    fn merge(mut self, into: &mut ZngurSpec) -> MergeResult {
        let erased = self.ty.with_erased_lifetimes();
        if let Some(existing) = into
            .types
            .iter_mut()
            .find(|t| t.ty.with_erased_lifetimes() == erased)
        {
            let existing_canon = existing.ty.canonicalize_lifetimes(&existing.lifetimes);
            let self_canon = self.ty.canonicalize_lifetimes(&self.lifetimes);
            if existing.lifetimes.len() != self.lifetimes.len() || existing_canon != self_canon {
                return Err(MergeFailure::Conflict(
                    format!(
                        "Conflicting lifetime declarations `{}` and `{}` map to the same C++ type",
                        self.ty, existing.ty
                    ),
                    (
                        ConflictSource::Type(self_canon),
                        ConflictSource::Type(existing_canon),
                    ),
                ));
            }
            align_type_lifetimes(&mut self, &existing.lifetimes, &existing.ty);
            self.merge(existing)?;
        } else {
            into.types.push(self);
        }
        Ok(())
    }
}

impl Merge for ZngurMethod {
    fn merge(self, into: &mut Self) -> MergeResult {
        if canonicalize_method(&self) != canonicalize_method(into) {
            return Err(MergeFailure::Conflict(
                "Method mismatch".to_string(),
                (
                    ConflictSource::Method(self),
                    ConflictSource::Method(into.clone()),
                ),
            ));
        }
        Ok(())
    }
}

impl Merge for ZngurMethodDetails {
    fn merge(self, into: &mut Self) -> MergeResult {
        if canonicalize_method_details(&self) != canonicalize_method_details(into) {
            return Err(MergeFailure::Conflict(
                "Method mismatch".to_string(),
                (
                    ConflictSource::Method(self.data.clone()),
                    ConflictSource::Method(into.data.clone()),
                ),
            ));
        }
        Ok(())
    }
}

impl Merge for ZngurConstructor {
    fn merge(self, into: &mut Self) -> MergeResult {
        let canon = |c: &ZngurConstructor| -> Vec<(String, RustType)> {
            c.inputs
                .iter()
                .map(|(n, t)| (n.clone(), t.canonicalize_lifetimes(&[])))
                .collect()
        };
        if canon(&self) != canon(into) {
            return Err(MergeFailure::Conflict(
                "Constructor mismatch".to_string(),
                (
                    ConflictSource::Constructor(Some(self.inputs.clone())),
                    ConflictSource::Constructor(Some(into.inputs.clone())),
                ),
            ));
        }
        Ok(())
    }
}

impl Merge for ZngurVariant {
    fn merge(self, into: &mut Self) -> MergeResult {
        if self.name != into.name {
            panic!(
                "Attempt to merge different variants: {} and {}",
                self.name, into.name
            );
        }
        merge_by_identity(self.fields, &mut into.fields, |a, b| a.name == b.name)?;
        into.exhaustive = self.exhaustive && into.exhaustive;
        Ok(())
    }
}

impl Merge for ZngurField {
    fn merge(self, into: &mut Self) -> MergeResult {
        if self.name != into.name
            || self.offset != into.offset
            || self.ty.canonicalize_lifetimes(&[]) != into.ty.canonicalize_lifetimes(&[])
        {
            return Err(MergeFailure::Conflict(
                "Field mismatch".to_string(),
                (
                    ConflictSource::Field(self.name.clone()),
                    ConflictSource::Field(into.name.clone()),
                ),
            ));
        }
        Ok(())
    }
}

impl Merge<ZngurSpec> for ZngurTrait {
    /// Merges a trait into a specification's trait list.
    fn merge(mut self, into: &mut ZngurSpec) -> MergeResult {
        let erased = self.tr.with_erased_lifetimes();
        if let Some((_, existing)) = into
            .traits
            .iter_mut()
            .find(|(k, _)| k.with_erased_lifetimes() == erased)
        {
            let existing_canon = existing.tr.canonicalize_lifetimes(&[]);
            let self_canon = self.tr.canonicalize_lifetimes(&[]);
            if existing_canon != self_canon {
                return Err(MergeFailure::Conflict(
                    format!(
                        "Conflicting lifetime declarations `{}` and `{}` map to the same C++ trait",
                        self.tr, existing.tr
                    ),
                    (
                        ConflictSource::Type(RustType::Dyn(self_canon, vec![])),
                        ConflictSource::Type(RustType::Dyn(existing_canon, vec![])),
                    ),
                ));
            }
            self.tr = existing.tr.clone();
            self.merge(existing)
        } else {
            into.traits.insert(self.tr.clone(), self);
            Ok(())
        }
    }
}

fn canonicalize_fn(f: &ZngurFn) -> ZngurFn {
    let outer = &f.lifetimes;
    ZngurFn {
        path: f.path.canonicalize_lifetimes(outer),
        lifetimes: (0..outer.len())
            .map(|i| Lifetime(format!("__outer_{i}")))
            .collect(),
        inputs: f
            .inputs
            .iter()
            .map(|t| t.canonicalize_lifetimes(outer))
            .collect(),
        output: f.output.canonicalize_lifetimes(outer),
        is_safe: f.is_safe,
    }
}

impl Merge<ZngurSpec> for ZngurFn {
    /// Merges a function into a specification's function list.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        let erased_sig = (
            self.path.with_erased_lifetimes(),
            self.inputs
                .iter()
                .map(RustType::with_erased_lifetimes)
                .collect::<Vec<_>>(),
        );
        if let Some(existing) = into.funcs.iter().find(|f| {
            f.path.with_erased_lifetimes() == erased_sig.0
                && f.inputs
                    .iter()
                    .map(RustType::with_erased_lifetimes)
                    .eq(erased_sig.1.iter().cloned())
        }) {
            if canonicalize_fn(&self) != canonicalize_fn(existing) {
                return Err(MergeFailure::Conflict(
                    format!("Function `{}` mismatch", self.path),
                    (
                        ConflictSource::Type(RustType::Adt(self.path)),
                        ConflictSource::Type(RustType::Adt(existing.path.clone())),
                    ),
                ));
            }
        } else {
            into.funcs.push(self);
        }
        Ok(())
    }
}

fn canonicalize_extern_cpp_fn(f: &ZngurExternCppFn) -> ZngurExternCppFn {
    let outer = &f.lifetimes;
    ZngurExternCppFn {
        name: f.name.clone(),
        lifetimes: (0..outer.len())
            .map(|i| Lifetime(format!("__outer_{i}")))
            .collect(),
        inputs: f
            .inputs
            .iter()
            .map(|t| t.canonicalize_lifetimes(outer))
            .collect(),
        output: f.output.canonicalize_lifetimes(outer),
        is_safe: f.is_safe,
    }
}

fn canonicalize_cpp_fn(f: &crate::ZngurCppFn) -> crate::ZngurCppFn {
    let outer = &f.lifetimes;
    crate::ZngurCppFn {
        namespace: f.namespace.clone(),
        cpp_name: f.cpp_name.clone(),
        template_args: f
            .template_args
            .iter()
            .map(|t| t.canonicalize_lifetimes(outer))
            .collect(),
        rust_name: f.rust_name.clone(),
        lifetimes: (0..outer.len())
            .map(|i| Lifetime(format!("__outer_{i}")))
            .collect(),
        inputs: f
            .inputs
            .iter()
            .map(|t| t.canonicalize_lifetimes(outer))
            .collect(),
        output: f.output.canonicalize_lifetimes(outer),
        is_safe: f.is_safe,
    }
}

impl Merge for crate::ZngurCppFn {
    fn merge(self, into: &mut Self) -> MergeResult {
        if canonicalize_cpp_fn(&self) != canonicalize_cpp_fn(into) {
            let source =
                |f: &Self| ConflictSource::CppFn(f.namespace.clone(), f.rust_name().to_owned());
            return Err(MergeFailure::Conflict(
                "C++ function mismatch".to_string(),
                (source(&self), source(into)),
            ));
        }
        Ok(())
    }
}

impl Merge<ZngurSpec> for crate::ZngurCppFn {
    /// Merges a C++ function into a specification's C++ function list.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        merge_by_identity(vec![self], &mut into.cpp_funcs, |a, b| {
            a.namespace == b.namespace && a.rust_name() == b.rust_name()
        })
    }
}

impl Merge<ZngurSpec> for ZngurExternCppFn {
    /// Merges an extern C++ function into a specification's C++ function list.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        if let Some(existing) = into.extern_cpp_funcs.iter().find(|f| f.name == self.name) {
            if canonicalize_extern_cpp_fn(&self) != canonicalize_extern_cpp_fn(existing) {
                return Err(MergeFailure::Conflict(
                    format!("Extern C++ function `{}` mismatch", self.name),
                    (
                        ConflictSource::Method(ZngurMethod {
                            name: self.name,
                            lifetimes: self.lifetimes,
                            generics: vec![],
                            receiver: ZngurMethodReceiver::Static,
                            receiver_lifetime: None,
                            inputs: self.inputs,
                            output: self.output,
                            is_safe: Some(self.is_safe),
                        }),
                        ConflictSource::Method(ZngurMethod {
                            name: existing.name.clone(),
                            lifetimes: existing.lifetimes.clone(),
                            generics: vec![],
                            receiver: ZngurMethodReceiver::Static,
                            receiver_lifetime: None,
                            inputs: existing.inputs.clone(),
                            output: existing.output.clone(),
                            is_safe: Some(existing.is_safe),
                        }),
                    ),
                ));
            }
        } else {
            into.extern_cpp_funcs.push(self);
        }
        Ok(())
    }
}

impl Merge<ZngurSpec> for ZngurExternCppImpl {
    /// Merges an extern C++ implementation into a specification's C++ implementation list.
    fn merge(mut self, into: &mut ZngurSpec) -> MergeResult {
        let erased_key = (
            self.tr.as_ref().map(RustTrait::with_erased_lifetimes),
            self.ty.with_erased_lifetimes(),
        );
        if let Some(existing) = into.extern_cpp_impls.iter_mut().find(|imp| {
            (
                imp.tr.as_ref().map(RustTrait::with_erased_lifetimes),
                imp.ty.with_erased_lifetimes(),
            ) == erased_key
        }) {
            let existing_canon = (
                existing
                    .tr
                    .as_ref()
                    .map(|tr| tr.canonicalize_lifetimes(&existing.lifetimes)),
                existing.ty.canonicalize_lifetimes(&existing.lifetimes),
            );
            let self_canon = (
                self.tr
                    .as_ref()
                    .map(|tr| tr.canonicalize_lifetimes(&self.lifetimes)),
                self.ty.canonicalize_lifetimes(&self.lifetimes),
            );
            if existing.lifetimes.len() != self.lifetimes.len() || existing_canon != self_canon {
                return Err(MergeFailure::Conflict(
                    format!(
                        "Conflicting lifetime declarations in `impl` for `{}`",
                        self.ty
                    ),
                    (
                        ConflictSource::Type(self_canon.1),
                        ConflictSource::Type(existing_canon.1),
                    ),
                ));
            }
            if self.lifetimes != existing.lifetimes {
                let mut avoid: HashSet<Lifetime> = existing
                    .lifetimes
                    .iter()
                    .chain(&self.lifetimes)
                    .cloned()
                    .collect();
                avoid.extend(existing.ty.all_lifetimes());
                avoid.extend(self.ty.all_lifetimes());
                if let Some(tr) = &existing.tr {
                    avoid.extend(tr.all_lifetimes());
                }
                if let Some(tr) = &self.tr {
                    avoid.extend(tr.all_lifetimes());
                }
                for m in &self.methods {
                    collect_method_lifetimes(m, None, &mut avoid);
                }
                for m in &mut self.methods {
                    align_method_lifetimes(
                        m,
                        None,
                        &self.lifetimes,
                        &existing.lifetimes,
                        &mut avoid,
                    );
                }
            }
            merge_by_identity(self.methods, &mut existing.methods, |a, b| a.name == b.name)?;
        } else {
            into.extern_cpp_impls.push(self);
        }
        Ok(())
    }
}

impl Merge<ZngurSpec> for AdditionalIncludes {
    /// Merges #include directives into a specification's additional includes string.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        into.additional_includes.0 += self.0.as_str();
        Ok(())
    }
}

impl Merge<ZngurSpec> for ConvertPanicToException {
    /// Merges a CPtE flag into a specification's CPtE flag.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        // TODO: There's an architectural decision here.
        // I'd like to encode the "unimportability" of CPTE here, rather than in the parser.
        // But checking this requires knowledge of the parse depth, which seems inappropriate
        // to pass through here.
        into.convert_panic_to_exception.0 |= self.0;
        Ok(())
    }
}
