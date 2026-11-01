use std::collections::{HashMap, HashSet};

use zngur_def::{
    CppConstness, Lifetime, Merge, RustPathAndGenerics, RustTrait, RustType, TypeVar,
    ZngurConstructor, ZngurCppConstructor, ZngurCppField, ZngurCppMethod, ZngurField, ZngurMethod,
    ZngurMethodDetails, ZngurMethodReceiver, ZngurType, ZngurVariant,
};

fn match_lifetime<'a, 'b>(
    l1: Option<&'a Lifetime>,
    l2: Option<&'b Lifetime>,
    template_lts: &'b [Lifetime],
    bound_fn_lts: &[(&'a Lifetime, &'b Lifetime)],
    lt_mapping: &mut HashMap<&'b Lifetime, Option<&'a Lifetime>>,
) -> bool {
    if let Some(l2_lt) = l2
        && let Some((expected_l1, _)) = bound_fn_lts.iter().rev().find(|(_, b2)| *b2 == l2_lt)
    {
        return l1 == Some(*expected_l1);
    }
    if l1.is_some_and(|l1_lt| bound_fn_lts.iter().any(|(b1, _)| *b1 == l1_lt)) {
        return false;
    }
    if let Some(l2_lt) = l2
        && template_lts.contains(l2_lt)
    {
        return lt_mapping
            .insert(l2_lt, l1)
            .map(|prev| prev == l1)
            .unwrap_or(true);
    }
    l1 == l2
}

fn matches_template<'a, 'b>(
    ty: &'a RustType,
    generic: &'b RustType,
    template_lts: &'b [Lifetime],
    bound_fn_lts: &[(&'a Lifetime, &'b Lifetime)],
    mapping: &mut HashMap<&'b TypeVar, &'a RustType>,
    lt_mapping: &mut HashMap<&'b Lifetime, Option<&'a Lifetime>>,
) -> bool {
    fn match_lists<'a, 'b>(
        v1: &'a [RustType],
        v2: &'b [RustType],
        template_lts: &'b [Lifetime],
        bound_fn_lts: &[(&'a Lifetime, &'b Lifetime)],
        mapping: &mut HashMap<&'b TypeVar, &'a RustType>,
        lt_mapping: &mut HashMap<&'b Lifetime, Option<&'a Lifetime>>,
    ) -> bool {
        v1.len() == v2.len() && match_iters(v1, v2, template_lts, bound_fn_lts, mapping, lt_mapping)
    }

    fn match_iters<'a, 'b>(
        i1: impl IntoIterator<Item = &'a RustType>,
        i2: impl IntoIterator<Item = &'b RustType>,
        template_lts: &'b [Lifetime],
        bound_fn_lts: &[(&'a Lifetime, &'b Lifetime)],
        mapping: &mut HashMap<&'b TypeVar, &'a RustType>,
        lt_mapping: &mut HashMap<&'b Lifetime, Option<&'a Lifetime>>,
    ) -> bool {
        i1.into_iter().zip(i2).all(|(ty1, ty2)| {
            matches_template(ty1, ty2, template_lts, bound_fn_lts, mapping, lt_mapping)
        })
    }

    fn match_generics<'a, 'b>(
        t1: &'a RustPathAndGenerics,
        t2: &'b RustPathAndGenerics,
        template_lts: &'b [Lifetime],
        bound_fn_lts: &[(&'a Lifetime, &'b Lifetime)],
        mapping: &mut HashMap<&'b TypeVar, &'a RustType>,
        lt_mapping: &mut HashMap<&'b Lifetime, Option<&'a Lifetime>>,
    ) -> bool {
        // For now named generics must be in the same order
        t1.path == t2.path
            && t1.lifetimes.len() == t2.lifetimes.len()
            && t1
                .lifetimes
                .iter()
                .zip(t2.lifetimes.iter())
                .all(|(l1, l2)| {
                    match_lifetime(Some(l1), Some(l2), template_lts, bound_fn_lts, lt_mapping)
                })
            && match_lists(
                &t1.generics,
                &t2.generics,
                template_lts,
                bound_fn_lts,
                mapping,
                lt_mapping,
            )
            && t1.named_generics.len() == t2.named_generics.len()
            && t1
                .named_generics
                .iter()
                .zip(t2.named_generics.iter())
                .all(|((n1, t1), (n2, t2))| {
                    n1 == n2
                        && matches_template(t1, t2, template_lts, bound_fn_lts, mapping, lt_mapping)
                })
    }

    fn match_trait<'a, 'b>(
        t1: &'a RustTrait,
        t2: &'b RustTrait,
        template_lts: &'b [Lifetime],
        bound_fn_lts: &[(&'a Lifetime, &'b Lifetime)],
        mapping: &mut HashMap<&'b TypeVar, &'a RustType>,
        lt_mapping: &mut HashMap<&'b Lifetime, Option<&'a Lifetime>>,
    ) -> bool {
        match (t1, t2) {
            (RustTrait::Normal(t1), RustTrait::Normal(t2)) => {
                match_generics(t1, t2, template_lts, bound_fn_lts, mapping, lt_mapping)
            }
            (
                RustTrait::Fn {
                    name: n1,
                    lifetimes: l1,
                    inputs: i1,
                    output: o1,
                },
                RustTrait::Fn {
                    name: n2,
                    lifetimes: l2,
                    inputs: i2,
                    output: o2,
                },
            ) => {
                if n1 != n2 || l1.len() != l2.len() {
                    return false;
                }
                let mut inner_bound: Vec<(&'a Lifetime, &'b Lifetime)> = bound_fn_lts.to_vec();
                inner_bound.extend(l1.iter().zip(l2.iter()));
                match_lists(i1, i2, template_lts, &inner_bound, mapping, lt_mapping)
                    && matches_template(o1, o2, template_lts, &inner_bound, mapping, lt_mapping)
            }
            (_, _) => false,
        }
    }

    match (ty, generic) {
        (ty, RustType::TypeVar(v)) => {
            if ty
                .free_lifetimes()
                .iter()
                .any(|fl| bound_fn_lts.iter().any(|(b1, _)| *b1 == fl))
            {
                return false;
            }
            mapping
                .insert(v, ty)
                .map(|prev_binding| prev_binding == ty)
                .unwrap_or(true)
        }
        (RustType::Primitive(p1), RustType::Primitive(p2)) => p1 == p2,
        (RustType::Ref(m1, l1, t1), RustType::Ref(m2, l2, t2)) => {
            m1 == m2
                && match_lifetime(
                    l1.as_ref(),
                    l2.as_ref(),
                    template_lts,
                    bound_fn_lts,
                    lt_mapping,
                )
                && matches_template(t1, t2, template_lts, bound_fn_lts, mapping, lt_mapping)
        }
        (RustType::Raw(m1, t1), RustType::Raw(m2, t2)) => {
            m1 == m2 && matches_template(t1, t2, template_lts, bound_fn_lts, mapping, lt_mapping)
        }
        (RustType::Boxed(t1), RustType::Boxed(t2)) | (RustType::Slice(t1), RustType::Slice(t2)) => {
            matches_template(t1, t2, template_lts, bound_fn_lts, mapping, lt_mapping)
        }
        (RustType::Dyn(t1, b1), RustType::Dyn(t2, b2))
        | (RustType::Impl(t1, b1), RustType::Impl(t2, b2)) => {
            (b1 == b2) && match_trait(t1, t2, template_lts, bound_fn_lts, mapping, lt_mapping)
        }
        (RustType::Tuple(tys1), RustType::Tuple(tys2)) => {
            match_lists(tys1, tys2, template_lts, bound_fn_lts, mapping, lt_mapping)
        }
        (RustType::Adt(adt1), RustType::Adt(adt2)) => {
            match_generics(adt1, adt2, template_lts, bound_fn_lts, mapping, lt_mapping)
        }
        (RustType::Cpp(a), RustType::Cpp(b)) => a == b,
        (RustType::CppPtr(k1, t1), RustType::CppPtr(k2, t2)) => {
            k1 == k2 && matches_template(t1, t2, template_lts, bound_fn_lts, mapping, lt_mapping)
        }
        (RustType::CppConst(t1), RustType::CppConst(t2))
        | (RustType::CppVar(t1), RustType::CppVar(t2)) => {
            matches_template(t1, t2, template_lts, bound_fn_lts, mapping, lt_mapping)
        }
        (_, _) => false,
    }
}

#[derive(Debug)]
enum SubstitutionError<'a> {
    UnboundVar(&'a TypeVar),
}

fn fresh_lifetime(base: &str, avoid: &HashSet<Lifetime>) -> Lifetime {
    let mut counter = 0usize;
    loop {
        let candidate = Lifetime(format!("{base}_{counter}"));
        counter += 1;
        if !avoid.contains(&candidate) {
            return candidate;
        }
    }
}

fn substitute_vars<'a>(
    ty: &'a RustType,
    mapping: &HashMap<&TypeVar, &RustType>,
    lt_mapping: &HashMap<&Lifetime, Option<&Lifetime>>,
    target_lts: &HashSet<Lifetime>,
    fresh_avoid: &HashSet<Lifetime>,
    in_scope_lts: &HashSet<Lifetime>,
) -> Result<RustType, SubstitutionError<'a>> {
    fn substitute_vec<'a>(
        vec: &'a [RustType],
        mapping: &HashMap<&TypeVar, &RustType>,
        lt_mapping: &HashMap<&Lifetime, Option<&Lifetime>>,
        target_lts: &HashSet<Lifetime>,
        fresh_avoid: &HashSet<Lifetime>,
        in_scope_lts: &HashSet<Lifetime>,
    ) -> Result<Vec<RustType>, &'a TypeVar> {
        vec.iter()
            .map(|ty| {
                substitute_type(
                    ty,
                    mapping,
                    lt_mapping,
                    target_lts,
                    fresh_avoid,
                    in_scope_lts,
                )
            })
            .collect()
    }

    fn substitute_generics<'a>(
        path_and_generics: &'a RustPathAndGenerics,
        mapping: &HashMap<&TypeVar, &RustType>,
        lt_mapping: &HashMap<&Lifetime, Option<&Lifetime>>,
        target_lts: &HashSet<Lifetime>,
        fresh_avoid: &HashSet<Lifetime>,
        in_scope_lts: &HashSet<Lifetime>,
    ) -> Result<RustPathAndGenerics, &'a TypeVar> {
        let RustPathAndGenerics {
            path,
            lifetimes,
            generics,
            named_generics,
        } = path_and_generics;
        let result = RustPathAndGenerics {
            path: path.clone(),
            lifetimes: lifetimes
                .iter()
                .map(|l| match lt_mapping.get(l) {
                    Some(Some(bound)) => (*bound).clone(),
                    Some(None) => Lifetime("_".to_owned()),
                    None => l.clone(),
                })
                .collect(),
            generics: substitute_vec(
                generics,
                mapping,
                lt_mapping,
                target_lts,
                fresh_avoid,
                in_scope_lts,
            )?,
            named_generics: named_generics
                .iter()
                .map(|(name, ty)| {
                    substitute_type(
                        ty,
                        mapping,
                        lt_mapping,
                        target_lts,
                        fresh_avoid,
                        in_scope_lts,
                    )
                    .map(|ty| (name.clone(), ty))
                })
                .collect::<Result<_, _>>()?,
        };
        Ok(result)
    }

    fn substitute_trait<'a>(
        rust_trait: &'a RustTrait,
        mapping: &HashMap<&TypeVar, &RustType>,
        lt_mapping: &HashMap<&Lifetime, Option<&Lifetime>>,
        target_lts: &HashSet<Lifetime>,
        fresh_avoid: &HashSet<Lifetime>,
        in_scope_lts: &HashSet<Lifetime>,
    ) -> Result<RustTrait, &'a TypeVar> {
        let result = match rust_trait {
            RustTrait::Normal(path_and_generics) => RustTrait::Normal(substitute_generics(
                path_and_generics,
                mapping,
                lt_mapping,
                target_lts,
                fresh_avoid,
                in_scope_lts,
            )?),
            RustTrait::Fn {
                name,
                lifetimes,
                inputs,
                output,
            } => {
                // Rename any `for<...>` binder lifetime that would capture a lifetime in the
                // substituted target arguments or collide with an enclosing in-scope lifetime.
                let mut local_avoid = fresh_avoid.clone();
                local_avoid.extend(in_scope_lts.iter().cloned());
                local_avoid.extend(lifetimes.iter().cloned());
                let mut binder_renames: Vec<(Lifetime, Lifetime)> = Vec::new();
                let new_lifetimes: Vec<Lifetime> = lifetimes
                    .iter()
                    .map(|fl| {
                        if target_lts.contains(fl) || in_scope_lts.contains(fl) {
                            let fresh = fresh_lifetime(&fl.0, &local_avoid);
                            local_avoid.insert(fresh.clone());
                            binder_renames.push((fl.clone(), fresh.clone()));
                            fresh
                        } else {
                            fl.clone()
                        }
                    })
                    .collect();
                let mut next_in_scope = in_scope_lts.clone();
                next_in_scope.extend(new_lifetimes.iter().cloned());
                if binder_renames.is_empty() {
                    RustTrait::Fn {
                        name: name.clone(),
                        lifetimes: new_lifetimes,
                        inputs: substitute_vec(
                            inputs,
                            mapping,
                            lt_mapping,
                            target_lts,
                            &local_avoid,
                            &next_in_scope,
                        )?,
                        output: Box::new(substitute_type(
                            output,
                            mapping,
                            lt_mapping,
                            target_lts,
                            &local_avoid,
                            &next_in_scope,
                        )?),
                    }
                } else {
                    let rename_fn = |l: &Lifetime| -> Lifetime {
                        binder_renames
                            .iter()
                            .find(|(old, _)| old == l)
                            .map(|(_, new)| new.clone())
                            .unwrap_or_else(|| l.clone())
                    };
                    let renamed_inputs: Vec<RustType> = inputs
                        .iter()
                        .map(|t| t.map_free_lifetimes(&mut { rename_fn }))
                        .collect();
                    let renamed_output = output.map_free_lifetimes(&mut { rename_fn });
                    let new_inputs = renamed_inputs
                        .iter()
                        .map(|t| {
                            substitute_type(
                                t,
                                mapping,
                                lt_mapping,
                                target_lts,
                                &local_avoid,
                                &next_in_scope,
                            )
                            // Safety: `map_free_lifetimes` preserves the exact `TypeVar`s of `inputs`,
                            // so remap any unbound `&TypeVar` back to the `'a`-lived reference in `inputs`.
                            .map_err(|unbound| find_type_var_ref(inputs, unbound).unwrap())
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let new_output = substitute_type(
                        &renamed_output,
                        mapping,
                        lt_mapping,
                        target_lts,
                        &local_avoid,
                        &next_in_scope,
                    )
                    .map_err(|unbound| {
                        find_type_var_ref(std::slice::from_ref(output), unbound).unwrap()
                    })?;
                    RustTrait::Fn {
                        name: name.clone(),
                        lifetimes: new_lifetimes,
                        inputs: new_inputs,
                        output: Box::new(new_output),
                    }
                }
            }
        };
        Ok(result)
    }

    fn find_type_var_ref<'a>(tys: &'a [RustType], target: &TypeVar) -> Option<&'a TypeVar> {
        for ty in tys {
            match ty {
                RustType::TypeVar(v) if v == target => return Some(v),
                RustType::Ref(_, _, t)
                | RustType::Raw(_, t)
                | RustType::Boxed(t)
                | RustType::Slice(t)
                | RustType::CppPtr(_, t)
                | RustType::CppConst(t)
                | RustType::CppVar(t) => {
                    if let Some(v) = find_type_var_ref(std::slice::from_ref(t), target) {
                        return Some(v);
                    }
                }
                RustType::Tuple(ts) => {
                    if let Some(v) = find_type_var_ref(ts, target) {
                        return Some(v);
                    }
                }
                RustType::Adt(pg)
                | RustType::Dyn(RustTrait::Normal(pg), _)
                | RustType::Impl(RustTrait::Normal(pg), _) => {
                    if let Some(v) = find_type_var_ref(&pg.generics, target) {
                        return Some(v);
                    }
                    for (_, t) in &pg.named_generics {
                        if let Some(v) = find_type_var_ref(std::slice::from_ref(t), target) {
                            return Some(v);
                        }
                    }
                }
                RustType::Dyn(RustTrait::Fn { inputs, output, .. }, _)
                | RustType::Impl(RustTrait::Fn { inputs, output, .. }, _) => {
                    if let Some(v) = find_type_var_ref(inputs, target) {
                        return Some(v);
                    }
                    if let Some(v) = find_type_var_ref(std::slice::from_ref(output), target) {
                        return Some(v);
                    }
                }
                RustType::TypeVar(_) | RustType::Primitive(_) | RustType::Cpp(_) => {}
            }
        }
        None
    }

    fn substitute_type<'a>(
        ty: &'a RustType,
        mapping: &HashMap<&TypeVar, &RustType>,
        lt_mapping: &HashMap<&Lifetime, Option<&Lifetime>>,
        target_lts: &HashSet<Lifetime>,
        fresh_avoid: &HashSet<Lifetime>,
        in_scope_lts: &HashSet<Lifetime>,
    ) -> Result<RustType, &'a TypeVar> {
        let ty = match ty {
            RustType::TypeVar(v) => match mapping.get(v) {
                Some(ty) => {
                    if in_scope_lts.is_empty() {
                        (*ty).clone()
                    } else {
                        let mut var_avoid = fresh_avoid.clone();
                        ty.rename_colliding_for_binders(in_scope_lts, &mut var_avoid)
                    }
                }
                None => return Err(v),
            },
            p @ RustType::Primitive(_) => p.clone(),
            RustType::Ref(m, l, t) => {
                let new_l = match l {
                    Some(lt) => match lt_mapping.get(lt) {
                        Some(bound) => (*bound).cloned(),
                        None => Some(lt.clone()),
                    },
                    None => None,
                };
                RustType::Ref(
                    *m,
                    new_l,
                    Box::new(substitute_type(
                        t,
                        mapping,
                        lt_mapping,
                        target_lts,
                        fresh_avoid,
                        in_scope_lts,
                    )?),
                )
            }
            RustType::Raw(m, t) => RustType::Raw(
                *m,
                Box::new(substitute_type(
                    t,
                    mapping,
                    lt_mapping,
                    target_lts,
                    fresh_avoid,
                    in_scope_lts,
                )?),
            ),
            RustType::Boxed(t) => RustType::Boxed(Box::new(substitute_type(
                t,
                mapping,
                lt_mapping,
                target_lts,
                fresh_avoid,
                in_scope_lts,
            )?)),
            RustType::Slice(t) => RustType::Slice(Box::new(substitute_type(
                t,
                mapping,
                lt_mapping,
                target_lts,
                fresh_avoid,
                in_scope_lts,
            )?)),
            RustType::Dyn(rust_trait, bounds) => RustType::Dyn(
                substitute_trait(
                    rust_trait,
                    mapping,
                    lt_mapping,
                    target_lts,
                    fresh_avoid,
                    in_scope_lts,
                )?,
                bounds.clone(),
            ),
            RustType::Impl(rust_trait, bounds) => RustType::Impl(
                substitute_trait(
                    rust_trait,
                    mapping,
                    lt_mapping,
                    target_lts,
                    fresh_avoid,
                    in_scope_lts,
                )?,
                bounds.clone(),
            ),
            RustType::Tuple(tys) => RustType::Tuple(substitute_vec(
                tys,
                mapping,
                lt_mapping,
                target_lts,
                fresh_avoid,
                in_scope_lts,
            )?),
            RustType::Adt(path_and_generics) => RustType::Adt(substitute_generics(
                path_and_generics,
                mapping,
                lt_mapping,
                target_lts,
                fresh_avoid,
                in_scope_lts,
            )?),
            c @ RustType::Cpp(_) => c.clone(),
            RustType::CppPtr(kind, t) => RustType::CppPtr(
                *kind,
                Box::new(substitute_type(
                    t,
                    mapping,
                    lt_mapping,
                    target_lts,
                    fresh_avoid,
                    in_scope_lts,
                )?),
            ),
            // A `const T` or `var T` where `T` is `const i32` is a `const i32`.
            RustType::CppConst(t) => RustType::cpp_object(
                CppConstness::Const,
                substitute_type(
                    t,
                    mapping,
                    lt_mapping,
                    target_lts,
                    fresh_avoid,
                    in_scope_lts,
                )?,
            ),
            RustType::CppVar(t) => RustType::cpp_object(
                CppConstness::Var,
                substitute_type(
                    t,
                    mapping,
                    lt_mapping,
                    target_lts,
                    fresh_avoid,
                    in_scope_lts,
                )?,
            ),
        };
        Ok(ty)
    }

    substitute_type(
        ty,
        mapping,
        lt_mapping,
        target_lts,
        fresh_avoid,
        in_scope_lts,
    )
    .map_err(SubstitutionError::UnboundVar)
}

#[derive(Debug)]
enum OwnedSubstitutionError {
    UnboundVar(TypeVar),
}

fn substitute_method_vars(
    m: &ZngurMethodDetails,
    mapping: &HashMap<&TypeVar, &RustType>,
    lt_mapping: &HashMap<&Lifetime, Option<&Lifetime>>,
    target_lts: &HashSet<Lifetime>,
    fresh_avoid: &HashSet<Lifetime>,
    outer_in_scope: &HashSet<Lifetime>,
) -> Result<ZngurMethodDetails, OwnedSubstitutionError> {
    let mut local_avoid = fresh_avoid.clone();
    local_avoid.extend(m.data.lifetimes.iter().cloned());
    let mut method_renames: Vec<(Lifetime, Lifetime)> = Vec::new();
    let new_method_lts: Vec<Lifetime> = m
        .data
        .lifetimes
        .iter()
        .map(|ml| {
            if target_lts.contains(ml) || outer_in_scope.contains(ml) {
                let fresh = fresh_lifetime(&ml.0, &local_avoid);
                local_avoid.insert(fresh.clone());
                method_renames.push((ml.clone(), fresh.clone()));
                fresh
            } else {
                ml.clone()
            }
        })
        .collect();

    let mut method_in_scope = outer_in_scope.clone();
    method_in_scope.extend(new_method_lts.iter().cloned());

    let rename_method_lt = |l: &Lifetime| -> Lifetime {
        method_renames
            .iter()
            .find(|(old, _)| old == l)
            .map(|(_, new)| new.clone())
            .unwrap_or_else(|| l.clone())
    };
    let sub_ty = |ty: &RustType| -> Result<RustType, OwnedSubstitutionError> {
        let pre = if method_renames.is_empty() {
            ty.clone()
        } else {
            ty.map_free_lifetimes(&mut { rename_method_lt })
        };
        substitute_vars(
            &pre,
            mapping,
            lt_mapping,
            target_lts,
            &local_avoid,
            &method_in_scope,
        )
        .map_err(|SubstitutionError::UnboundVar(v)| OwnedSubstitutionError::UnboundVar(v.clone()))
    };

    let receiver_lifetime = match &m.data.receiver_lifetime {
        Some(rl) => {
            let renamed = rename_method_lt(rl);
            if new_method_lts.contains(&renamed) {
                Some(renamed)
            } else {
                match lt_mapping.get(&renamed) {
                    Some(bound) => (*bound).cloned(),
                    None => Some(renamed),
                }
            }
        }
        None => None,
    };

    Ok(ZngurMethodDetails {
        data: ZngurMethod {
            name: m.data.name.clone(),
            lifetimes: new_method_lts,
            generics: m
                .data
                .generics
                .iter()
                .map(&sub_ty)
                .collect::<Result<_, _>>()?,
            receiver: m.data.receiver,
            receiver_lifetime,
            inputs: m
                .data
                .inputs
                .iter()
                .map(&sub_ty)
                .collect::<Result<_, _>>()?,
            output: sub_ty(&m.data.output)?,
            is_safe: m.data.is_safe,
        },
        use_path: m.use_path.clone(),
        deref: match &m.deref {
            Some((ty, mutability)) => Some((sub_ty(ty)?, *mutability)),
            None => None,
        },
        cpp_name: m.cpp_name.clone(),
    })
}

fn substitute_cpp_method_vars(
    m: &ZngurCppMethod,
    mapping: &HashMap<&TypeVar, &RustType>,
    lt_mapping: &HashMap<&Lifetime, Option<&Lifetime>>,
    target_lts: &HashSet<Lifetime>,
    fresh_avoid: &HashSet<Lifetime>,
    outer_in_scope: &HashSet<Lifetime>,
) -> Result<ZngurCppMethod, OwnedSubstitutionError> {
    let mut local_avoid = fresh_avoid.clone();
    local_avoid.extend(m.lifetimes.iter().cloned());
    let mut method_renames: Vec<(Lifetime, Lifetime)> = Vec::new();
    let new_method_lts: Vec<Lifetime> = m
        .lifetimes
        .iter()
        .map(|ml| {
            if target_lts.contains(ml) || outer_in_scope.contains(ml) {
                let fresh = fresh_lifetime(&ml.0, &local_avoid);
                local_avoid.insert(fresh.clone());
                method_renames.push((ml.clone(), fresh.clone()));
                fresh
            } else {
                ml.clone()
            }
        })
        .collect();

    let mut method_in_scope = outer_in_scope.clone();
    method_in_scope.extend(new_method_lts.iter().cloned());

    let rename_method_lt = |l: &Lifetime| -> Lifetime {
        method_renames
            .iter()
            .find(|(old, _)| old == l)
            .map(|(_, new)| new.clone())
            .unwrap_or_else(|| l.clone())
    };
    let sub_ty = |ty: &RustType| -> Result<RustType, OwnedSubstitutionError> {
        let pre = if method_renames.is_empty() {
            ty.clone()
        } else {
            ty.map_free_lifetimes(&mut { rename_method_lt })
        };
        substitute_vars(
            &pre,
            mapping,
            lt_mapping,
            target_lts,
            &local_avoid,
            &method_in_scope,
        )
        .map_err(|SubstitutionError::UnboundVar(v)| OwnedSubstitutionError::UnboundVar(v.clone()))
    };

    let receiver_lifetime = match &m.receiver_lifetime {
        Some(rl) => {
            let renamed = rename_method_lt(rl);
            if new_method_lts.contains(&renamed) {
                Some(renamed)
            } else {
                match lt_mapping.get(&renamed) {
                    Some(bound) => (*bound).cloned(),
                    None => Some(renamed),
                }
            }
        }
        None => None,
    };

    Ok(ZngurCppMethod {
        cpp_name: m.cpp_name.clone(),
        lifetimes: new_method_lts,
        rust_name: m.rust_name.clone(),
        receiver: m.receiver,
        receiver_constness: m.receiver_constness,
        receiver_lifetime,
        raw_receiver: m.raw_receiver,
        inputs: m.inputs.iter().map(&sub_ty).collect::<Result<_, _>>()?,
        output: sub_ty(&m.output)?,
        is_safe: m.is_safe,
    })
}

fn substitute_cpp_constructor_vars(
    c: &ZngurCppConstructor,
    mapping: &HashMap<&TypeVar, &RustType>,
    lt_mapping: &HashMap<&Lifetime, Option<&Lifetime>>,
    target_lts: &HashSet<Lifetime>,
    fresh_avoid: &HashSet<Lifetime>,
    outer_in_scope: &HashSet<Lifetime>,
) -> Result<ZngurCppConstructor, OwnedSubstitutionError> {
    let mut local_avoid = fresh_avoid.clone();
    local_avoid.extend(c.lifetimes.iter().cloned());
    let mut ctor_renames: Vec<(Lifetime, Lifetime)> = Vec::new();
    let new_ctor_lts: Vec<Lifetime> = c
        .lifetimes
        .iter()
        .map(|cl| {
            if target_lts.contains(cl) || outer_in_scope.contains(cl) {
                let fresh = fresh_lifetime(&cl.0, &local_avoid);
                local_avoid.insert(fresh.clone());
                ctor_renames.push((cl.clone(), fresh.clone()));
                fresh
            } else {
                cl.clone()
            }
        })
        .collect();

    let mut ctor_in_scope = outer_in_scope.clone();
    ctor_in_scope.extend(new_ctor_lts.iter().cloned());

    let rename_ctor_lt = |l: &Lifetime| -> Lifetime {
        ctor_renames
            .iter()
            .find(|(old, _)| old == l)
            .map(|(_, new)| new.clone())
            .unwrap_or_else(|| l.clone())
    };
    let sub_ty = |ty: &RustType| -> Result<RustType, OwnedSubstitutionError> {
        let pre = if ctor_renames.is_empty() {
            ty.clone()
        } else {
            ty.map_free_lifetimes(&mut { rename_ctor_lt })
        };
        substitute_vars(
            &pre,
            mapping,
            lt_mapping,
            target_lts,
            &local_avoid,
            &ctor_in_scope,
        )
        .map_err(|SubstitutionError::UnboundVar(v)| OwnedSubstitutionError::UnboundVar(v.clone()))
    };

    Ok(ZngurCppConstructor {
        rust_name: c.rust_name.clone(),
        lifetimes: new_ctor_lts,
        inputs: c.inputs.iter().map(&sub_ty).collect::<Result<_, _>>()?,
        is_safe: c.is_safe,
    })
}

pub fn try_match_template(target: &ZngurType, template: &ZngurType) -> Option<TemplateMatch> {
    let mut mapping = HashMap::new();
    let mut lt_mapping = HashMap::new();
    if !matches_template(
        &target.ty,
        &template.ty,
        &template.lifetimes,
        &[],
        &mut mapping,
        &mut lt_mapping,
    ) {
        return None;
    }
    if lt_mapping.len() < template.lifetimes.len() {
        return None;
    }

    let mut target_lts: HashSet<Lifetime> = target.lifetimes.iter().cloned().collect();
    target_lts.extend(target.ty.all_lifetimes());
    for bound_ty in mapping.values() {
        target_lts.extend(bound_ty.all_lifetimes());
    }
    for bound_lt in lt_mapping.values().flatten() {
        target_lts.insert((*bound_lt).clone());
    }

    let mut fresh_avoid = target_lts.clone();
    fresh_avoid.extend(template.lifetimes.iter().cloned());
    fresh_avoid.extend(template.ty.all_lifetimes());
    for m in &template.methods {
        fresh_avoid.extend(m.data.lifetimes.iter().cloned());
        if let Some(rl) = &m.data.receiver_lifetime
            && !rl.is_builtin()
        {
            fresh_avoid.insert(rl.clone());
        }
        for t in m
            .data
            .inputs
            .iter()
            .chain([&m.data.output])
            .chain(&m.data.generics)
            .chain(m.deref.as_ref().map(|(t, _)| t))
        {
            fresh_avoid.extend(t.all_lifetimes());
        }
    }
    for m in &template.cpp_methods {
        fresh_avoid.extend(m.lifetimes.iter().cloned());
        if let Some(rl) = &m.receiver_lifetime
            && !rl.is_builtin()
        {
            fresh_avoid.insert(rl.clone());
        }
        for t in m.inputs.iter().chain([&m.output]) {
            fresh_avoid.extend(t.all_lifetimes());
        }
    }
    for c in &template.cpp_constructors {
        fresh_avoid.extend(c.lifetimes.iter().cloned());
        for t in &c.inputs {
            fresh_avoid.extend(t.all_lifetimes());
        }
    }
    for f in &template.fields {
        fresh_avoid.extend(f.ty.all_lifetimes());
    }
    for v in &template.variants {
        for f in &v.fields {
            fresh_avoid.extend(f.ty.all_lifetimes());
        }
    }
    if let Some(c) = &template.constructor {
        for (_, t) in &c.inputs {
            fresh_avoid.extend(t.all_lifetimes());
        }
    }

    let mut outer_in_scope: HashSet<Lifetime> = target.lifetimes.iter().cloned().collect();
    outer_in_scope.extend(target.ty.free_lifetimes());

    let ZngurType {
        ty: template_ty,
        lifetimes: _,
        layout,
        trivially_relocatable,
        wellknown_traits,
        exhaustive,
        methods,
        cpp_methods,
        cpp_constructors,
        cpp_fields,
        constructor,
        variants,
        fields,
        cpp_ref,
        cpp_heap_allocated,
        cpp_stack_owned,
    } = template;
    debug_assert_eq!(
        substitute_vars(
            template_ty,
            &mapping,
            &lt_mapping,
            &target_lts,
            &fresh_avoid,
            &outer_in_scope
        )
        .unwrap()
        .canonicalize_lifetimes(&target.lifetimes),
        target.ty.canonicalize_lifetimes(&target.lifetimes)
    );
    let ty = &target.ty;
    let mut new_cpp_methods: Vec<ZngurCppMethod> = cpp_methods
        .iter()
        .map(|method| {
            match substitute_cpp_method_vars(
                method,
                &mapping,
                &lt_mapping,
                &target_lts,
                &fresh_avoid,
                &outer_in_scope,
            ) {
                Ok(m) => m,
                Err(OwnedSubstitutionError::UnboundVar(var)) => unreachable!(
                    "Unbound type variable {} in C++ method {} in template {} for type {}",
                    var.0,
                    method.rust_name(),
                    template.ty,
                    ty
                ),
            }
        })
        .collect();
    let mut new_methods: Vec<ZngurMethodDetails> = Vec::new();
    for method in methods {
        let m = match substitute_method_vars(
            method,
            &mapping,
            &lt_mapping,
            &target_lts,
            &fresh_avoid,
            &outer_in_scope,
        ) {
            Ok(m) => m,
            Err(OwnedSubstitutionError::UnboundVar(var)) => unreachable!(
                "Unbound type variable {} in method {} in template {} for type {}",
                var.0, method.data.name, template.ty, ty
            ),
        };
        if matches!(ty, RustType::Cpp(_))
            && let Some(is_safe) = m.data.is_safe
            && m.data.receiver == ZngurMethodReceiver::Static
            && m.data.generics.is_empty()
            && m.use_path.is_none()
            && m.deref.is_none()
        {
            let cpp_name = m.data.name;
            let rust_name = m.cpp_name.filter(|n| n != &cpp_name);
            new_cpp_methods.push(ZngurCppMethod {
                cpp_name,
                lifetimes: m.data.lifetimes,
                rust_name,
                receiver: ZngurMethodReceiver::Static,
                receiver_constness: CppConstness::Var,
                receiver_lifetime: None,
                raw_receiver: None,
                inputs: m.data.inputs,
                output: m.data.output,
                is_safe,
            });
        } else {
            new_methods.push(m);
        }
    }
    let new_ty = ZngurType {
        ty: ty.clone(),
        lifetimes: target.lifetimes.clone(),
        layout: *layout,
        trivially_relocatable: *trivially_relocatable,
        wellknown_traits: wellknown_traits.clone(),
        exhaustive: *exhaustive,
        cpp_methods: new_cpp_methods,
        cpp_constructors: cpp_constructors
            .iter()
            .map(|ctor| {
                match substitute_cpp_constructor_vars(
                    ctor,
                    &mapping,
                    &lt_mapping,
                    &target_lts,
                    &fresh_avoid,
                    &outer_in_scope,
                ) {
                    Ok(c) => c,
                    Err(OwnedSubstitutionError::UnboundVar(var)) => unreachable!(
                        "Unbound type variable {} in C++ constructor {} in template {} for type {}",
                        var.0, ctor.rust_name, template.ty, ty
                    ),
                }
            })
            .collect(),
        cpp_fields: cpp_fields
            .iter()
            .map(|f| {
                match substitute_vars(
                    &f.ty,
                    &mapping,
                    &lt_mapping,
                    &target_lts,
                    &fresh_avoid,
                    &outer_in_scope,
                ) {
                    Ok(fty) => ZngurCppField {
                        ty: fty,
                        ..f.clone()
                    },
                    Err(SubstitutionError::UnboundVar(var)) => unreachable!(
                        "Unbound type variable {} in C++ field {} in template {} for type {}",
                        var.0, f.name, template.ty, ty
                    ),
                }
            })
            .collect(),
        methods: new_methods,
        constructor: constructor.as_ref().map(|constructor| {
            match constructor
                .inputs
                .iter()
                .map(|(name, fty)| {
                    substitute_vars(
                        fty,
                        &mapping,
                        &lt_mapping,
                        &target_lts,
                        &fresh_avoid,
                        &outer_in_scope,
                    )
                    .map(|fty| (name.clone(), fty))
                })
                .collect()
            {
                Ok(inputs) => ZngurConstructor { inputs },
                Err(SubstitutionError::UnboundVar(var)) => unreachable!(
                    "Unbound type variable {} in constructor in template {} for type {}",
                    var.0, template.ty, ty
                ),
            }
        }),
        variants: variants
            .iter()
            .map(|variant| {
                let fields = variant
                    .fields
                    .iter()
                    .map(|field| {
                        match substitute_vars(
                            &field.ty,
                            &mapping,
                            &lt_mapping,
                            &target_lts,
                            &fresh_avoid,
                            &outer_in_scope,
                        ) {
                            Ok(fty) => ZngurField {
                                name: field.name.clone(),
                                ty: fty,
                                offset: field.offset,
                            },
                            Err(SubstitutionError::UnboundVar(var)) => unreachable!(
                                "Unbound type variable {} in field {} of variant {}, in template {} for type {}",
                                var.0, field.name, variant.name, template.ty, ty
                            ),
                        }
                    })
                    .collect();
                ZngurVariant {
                    name: variant.name.clone(),
                    exhaustive: variant.exhaustive,
                    fields,
                }
            })
            .collect(),
        fields: fields
            .iter()
            .map(|field| {
                match substitute_vars(
                    &field.ty,
                    &mapping,
                    &lt_mapping,
                    &target_lts,
                    &fresh_avoid,
                    &outer_in_scope,
                ) {
                    Ok(fty) => ZngurField {
                        name: field.name.clone(),
                        ty: fty,
                        offset: field.offset,
                    },
                    Err(SubstitutionError::UnboundVar(var)) => unreachable!(
                        "Unbound type variable {} in field {} in template {} for type {}",
                        var.0, field.name, template.ty, ty
                    ),
                }
            })
            .collect(),
        cpp_heap_allocated: cpp_heap_allocated.clone(),
        cpp_ref: cpp_ref.clone(),
        cpp_stack_owned: cpp_stack_owned.clone(),
    };
    Some(TemplateMatch(new_ty))
}

// Represents a ZngurType created from a template type
pub struct TemplateMatch(pub(crate) ZngurType);

impl Merge<ZngurType> for TemplateMatch {
    fn merge(self, into: &mut ZngurType) -> zngur_def::MergeResult {
        let TemplateMatch(mut ty) = self;
        // The concrete type's layout should override the template's layout without causing a conflict
        if into.layout.is_some() {
            ty.layout = None;
        }
        ty.merge(into)
    }
}
