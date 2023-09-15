use std::collections::{hash_map::Entry, HashMap};

use crate::{
    ast,
    diagnostics::{ApolloDiagnostic, DiagnosticData, Label},
    schema,
    validation::ValidationDatabase,
    Node,
};

use super::operation::OperationValidationConfig;
/// TODO(@goto-bus-stop) test pathological query with many of the same field

/// A field and the type it selects from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FieldAgainstType<'a> {
    pub against_type: &'a ast::NamedType,
    pub field: &'a Node<ast::Field>,
}

// TODO(@goto-bus-stop) remove intermediate allocations
fn operation_fields<'a>(
    named_fragments: &'a HashMap<ast::Name, Node<ast::FragmentDefinition>>,
    against_type: &'a ast::NamedType,
    selections: &'a [ast::Selection],
) -> Vec<FieldAgainstType<'a>> {
    fn inner<'a>(
        named_fragments: &'a HashMap<ast::Name, Node<ast::FragmentDefinition>>,
        seen: &mut std::collections::HashSet<ast::Name>,
        against_type: &'a ast::NamedType,
        selections: &'a [ast::Selection],
    ) -> Vec<FieldAgainstType<'a>> {
        selections
            .iter()
            .flat_map(|selection| match selection {
                ast::Selection::Field(field) => vec![FieldAgainstType {
                    against_type,
                    field,
                }],
                ast::Selection::InlineFragment(inline) => inner(
                    named_fragments,
                    seen,
                    inline.type_condition.as_ref().unwrap_or(against_type),
                    &inline.selection_set,
                ),
                ast::Selection::FragmentSpread(spread) => {
                    if seen.contains(&spread.fragment_name) {
                        return vec![];
                    }
                    seen.insert(spread.fragment_name.clone());

                    named_fragments
                        .get(&spread.fragment_name)
                        .map(|fragment| {
                            inner(
                                named_fragments,
                                seen,
                                &fragment.type_condition,
                                &fragment.selection_set,
                            )
                        })
                        .unwrap_or_default()
                }
            })
            .collect()
    }
    inner(
        named_fragments,
        &mut Default::default(),
        against_type,
        selections,
    )
}

/// Check if two fields will output the same type.
///
/// If the two fields output different types, returns an `Err` containing diagnostic information.
/// To simply check if outputs are the same, you can use `.is_ok()`:
/// ```rust,ignore
/// let is_same = same_response_shape(db, field_a, field_b).is_ok();
/// // `is_same` is `bool`
/// ```
///
/// Spec: https://spec.graphql.org/October2021/#SameResponseShape()
pub(crate) fn same_response_shape(
    db: &dyn ValidationDatabase,
    field_a: FieldAgainstType<'_>,
    field_b: FieldAgainstType<'_>,
) -> Result<(), ApolloDiagnostic> {
    let schema = db.schema();
    // 1. Let typeA be the return type of fieldA.
    let Some(full_type_a) = schema.type_field(field_a.against_type, &field_a.field.name) else {
        return Ok(()); // Can't do much if we don't know the type
    };
    let mut type_a = &full_type_a.ty;
    // 2. Let typeB be the return type of fieldB.
    let Some(full_type_b) = schema.type_field(field_b.against_type, &field_b.field.name) else {
        return Ok(()); // Can't do much if we don't know the type
    };
    let mut type_b = &full_type_b.ty;

    let mismatching_type_diagnostic = || {
        ApolloDiagnostic::new(
            db,
            (*field_b.field.location().unwrap()).into(),
            DiagnosticData::ConflictingField {
                field: field_a.field.name.to_string(),
                original_selection: (*field_a.field.location().unwrap()).into(),
                redefined_selection: (*field_b.field.location().unwrap()).into(),
            },
        )
        .label(Label::new(
            *field_a.field.location().unwrap(),
            format!(
                "`{}` has type `{}` here",
                field_a.field.response_name(),
                full_type_a.ty,
            ),
        ))
        .label(Label::new(
            *field_b.field.location().unwrap(),
            format!("but the same field name has type `{}` here", full_type_b.ty),
        ))
    };

    // Steps 3 and 4 of the spec text unwrap both types simultaneously down to the named type.
    // The apollo-rs representation of NonNull and Lists makes it tricky to follow the steps
    // exactly.
    //
    // Instead we unwrap lists and non-null lists first, which leaves just a named type or a
    // non-null named type...
    while !type_a.is_named() || !type_b.is_named() {
        // 4. If typeA or typeB is List.
        // 4a. If typeA or typeB is not List, return false.
        // 4b. Let typeA be the item type of typeA
        // 4c. Let typeB be the item type of typeB
        (type_a, type_b) = match (type_a, type_b) {
            (ast::Type::List(type_a), ast::Type::List(type_b))
            | (ast::Type::NonNullList(type_a), ast::Type::NonNullList(type_b)) => {
                (type_a.as_ref(), type_b.as_ref())
            }
            (ast::Type::List(_), _)
            | (_, ast::Type::List(_))
            | (ast::Type::NonNullList(_), _)
            | (_, ast::Type::NonNullList(_)) => return Err(mismatching_type_diagnostic()),
            // Now it's a named type.
            (type_a, type_b) => (type_a, type_b),
        };
    }

    // Now we are down to two named types, we can check that they have the same nullability...
    let (type_a, type_b) = match (type_a, type_b) {
        (ast::Type::NonNullNamed(a), ast::Type::NonNullNamed(b)) => (a, b),
        (ast::Type::Named(a), ast::Type::Named(b)) => (a, b),
        _ => return Err(mismatching_type_diagnostic()),
    };

    let (Some(def_a), Some(def_b)) = (schema.types.get(type_a), schema.types.get(type_b)) else {
        return Ok(()); // Cannot do much if we don't know the type
    };

    fn is_composite(ty: &schema::ExtendedType) -> bool {
        type T = schema::ExtendedType;
        matches!(ty, T::Object(_) | T::Interface(_) | T::Union(_))
    }

    match (def_a, def_b) {
        // 5. If typeA or typeB is Scalar or Enum.
        (
            def_a @ (schema::ExtendedType::Scalar(_) | schema::ExtendedType::Enum(_)),
            def_b @ (schema::ExtendedType::Scalar(_) | schema::ExtendedType::Enum(_)),
        ) => {
            // 5a. If typeA and typeB are the same type return true, otherwise return false.
            if def_a == def_b {
                Ok(())
            } else {
                Err(mismatching_type_diagnostic())
            }
        }
        // 6. Assert: typeA and typeB are both composite types.
        (def_a, def_b) if is_composite(def_a) && is_composite(def_b) => {
            let named_fragments =
                db.ast_named_fragments(field_a.field.location().unwrap().file_id());
            let mut merged_set = operation_fields(
                &named_fragments,
                field_a.against_type,
                &field_a.field.selection_set,
            );
            merged_set.extend(operation_fields(
                &named_fragments,
                field_b.against_type,
                &field_b.field.selection_set,
            ));
            let grouped_by_name = group_fields_by_name(merged_set);

            for (_, fields_for_name) in grouped_by_name {
                // 9. Given each pair of members subfieldA and subfieldB in fieldsForName:
                let Some((subfield_a, rest)) = fields_for_name.split_first() else {
                    continue;
                };
                for subfield_b in rest {
                    // 9a. If SameResponseShape(subfieldA, subfieldB) is false, return false.
                    same_response_shape(db, *subfield_a, *subfield_b)?;
                }
            }

            Ok(())
        }
        (_, _) => Ok(()),
    }
}

/// Given a list of fields, group them by response name.
fn group_fields_by_name(
    fields: Vec<FieldAgainstType<'_>>,
) -> HashMap<ast::Name, Vec<FieldAgainstType<'_>>> {
    let mut map = HashMap::<ast::Name, Vec<FieldAgainstType<'_>>>::new();
    for field in fields {
        match map.entry(field.field.response_name().clone()) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(field);
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![field]);
            }
        }
    }
    map
}

/// Check if the arguments provided to two fields are the same, so the fields can be merged.
fn identical_arguments(
    db: &dyn ValidationDatabase,
    field_a: &Node<ast::Field>,
    field_b: &Node<ast::Field>,
) -> Result<(), ApolloDiagnostic> {
    let args_a = &field_a.arguments;
    let args_b = &field_b.arguments;

    let loc_a = *field_a.location().unwrap();
    let loc_b = *field_b.location().unwrap();

    // Check if fieldB provides the same argument names and values as fieldA (order-independent).
    for arg in args_a {
        let Some(other_arg) = args_b.iter().find(|other_arg| other_arg.name == arg.name) else {
            return Err(
                ApolloDiagnostic::new(
                    db,
                    loc_b.into(),
                    DiagnosticData::ConflictingField {
                        field: field_a.name.to_string(),
                        original_selection: loc_a.into(),
                        redefined_selection: loc_b.into(),
                    },
                )
                .label(Label::new(*arg.location().unwrap(), format!("field `{}` is selected with argument `{}` here", field_a.name, arg.name)))
                .label(Label::new(loc_b, format!("but argument `{}` is not provided here", arg.name)))
                .help("Fields with the same response name must provide the same set of arguments. Consider adding an alias if you need to select fields with different arguments.")
            );
        };

        if other_arg.value != arg.value {
            return Err(
                ApolloDiagnostic::new(
                    db,
                    loc_b.into(),
                    DiagnosticData::ConflictingField {
                        field: field_a.name.to_string(),
                        original_selection: loc_a.into(),
                        redefined_selection: loc_b.into(),
                    },
                )
                .label(Label::new(*arg.location().unwrap(), format!("field `{}` provides one argument value here", field_a.name)))
                .label(Label::new(*other_arg.location().unwrap(), "but a different value here"))
                .help("Fields with the same response name must provide the same set of arguments. Consider adding an alias if you need to select fields with different arguments.")
            );
        }
    }
    // Check if fieldB provides any arguments that fieldA does not provide.
    for arg in args_b {
        if !args_a.iter().any(|other_arg| other_arg.name == arg.name) {
            return Err(
                ApolloDiagnostic::new(
                    db,
                    loc_b.into(),
                    DiagnosticData::ConflictingField {
                        field: field_a.name.to_string(),
                        original_selection: loc_a.into(),
                        redefined_selection: loc_b.into(),
                    },
                )
                .label(Label::new(*arg.location().unwrap(), format!("field `{}` is selected with argument `{}` here", field_b.name, arg.name)))
                .label(Label::new(loc_a, format!("but argument `{}` is not provided here", arg.name)))
                .help("Fields with the same response name must provide the same set of arguments. Consider adding an alias if you need to select fields with different arguments.")
            );
        };
    }

    Ok(())
}

/// Check if the fields in a given selection set can be merged.
///
/// If the fields cannot be merged, returns an `Err` containing diagnostic information.
///
/// Spec: https://spec.graphql.org/October2021/#FieldsInSetCanMerge()
pub(crate) fn fields_in_set_can_merge(
    db: &dyn ValidationDatabase,
    named_fragments: &HashMap<ast::Name, Node<ast::FragmentDefinition>>,
    against_type: &ast::NamedType,
    selection_set: &[ast::Selection],
) -> Result<(), Vec<ApolloDiagnostic>> {
    let schema = db.schema();

    // 1. Let `fieldsForName` be the set of selections with a given response name in set including visiting fragments and inline fragments.
    let fields = operation_fields(named_fragments, against_type, selection_set);
    let grouped_by_name = group_fields_by_name(fields);

    let mut diagnostics = vec![];

    for (_, fields_for_name) in grouped_by_name {
        let Some((field_a, rest)) = fields_for_name.split_first() else {
            continue; // Nothing to merge
        };
        let Some(parent_a) = schema.type_field(field_a.against_type, &field_a.field.name) else {
            continue; // Can't do much if we don't know the type
        };

        // 2. Given each pair of members fieldA and fieldB in fieldsForName:
        for field_b in rest {
            // 2a. SameResponseShape(fieldA, fieldB) must be true.
            if let Err(diagnostic) = same_response_shape(db, *field_a, *field_b) {
                diagnostics.push(diagnostic);
                continue;
            }
            // 2b. If the parent types of fieldA and fieldB are equal or if either is not an Object Type:
            if field_a.against_type == field_b.against_type {
                // 2bi. fieldA and fieldB must have identical field names.
                if field_a.field.name != field_b.field.name {
                    diagnostics.push(
                        ApolloDiagnostic::new(
                            db,
                            (*field_b.field.location().unwrap()).into(),
                            DiagnosticData::ConflictingField {
                                field: field_b.field.name.to_string(),
                                original_selection: (*field_a.field.location().unwrap()).into(),
                                redefined_selection: (*field_b.field.location().unwrap()).into(),
                            },
                        )
                        .label(Label::new(
                            *field_a.field.location().unwrap(),
                            format!(
                                "field `{}` is selected from field `{}` here",
                                field_a.field.response_name(),
                                field_a.field.name
                            ),
                        ))
                        .label(Label::new(
                            *field_b.field.location().unwrap(),
                            format!(
                                "but the same field `{}` is also selected from field `{}` here",
                                field_b.field.response_name(),
                                field_b.field.name
                            ),
                        ))
                        .help("Alias is already used for a different field"),
                    );
                    continue;
                }
                // 2bii. fieldA and fieldB must have identical sets of arguments.
                if let Err(diagnostic) = identical_arguments(db, field_a.field, field_b.field) {
                    diagnostics.push(diagnostic);
                    continue;
                }
                // 2biii. Let mergedSet be the result of adding the selection set of fieldA and the selection set of fieldB.
                let mut merged_set = field_a.field.selection_set.clone();
                merged_set.extend_from_slice(&field_b.field.selection_set);
                // 2biv. FieldsInSetCanMerge(mergedSet) must be true.
                if let Err(sub_diagnostics) = fields_in_set_can_merge(
                    db,
                    named_fragments,
                    parent_a.ty.inner_named_type(),
                    &merged_set,
                ) {
                    diagnostics.extend(sub_diagnostics);
                    continue;
                }
            }
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

pub fn validate_selection_set2(
    db: &dyn ValidationDatabase,
    against_type: Option<&ast::NamedType>,
    selection_set: &[ast::Selection],
    context: OperationValidationConfig<'_>,
) -> Vec<ApolloDiagnostic> {
    let mut diagnostics = vec![];

    // TODO(@goto-bus-stop) named fragments for synthetic ASTs
    let named_fragments = selection_set
        .get(0)
        .and_then(|field| Some(db.ast_named_fragments(field.location()?.file_id())));
    // `named_fragments` will be None if we have 0 fields, where this validation is irrelevant
    // anyways.
    // If `against_type` is None, we don't know the actual type of anything, so we cannot run this
    // validation.
    if let (Some(named_fragments), Some(against_type)) = (named_fragments, against_type) {
        if let Err(diagnostic) =
            fields_in_set_can_merge(db, &named_fragments, against_type, selection_set)
        {
            diagnostics.extend(diagnostic);
        }
    }

    diagnostics.extend(validate_selections(
        db,
        against_type,
        selection_set,
        context,
    ));

    diagnostics
}

pub fn validate_selections(
    db: &dyn ValidationDatabase,
    against_type: Option<&ast::NamedType>,
    selection_set: &[ast::Selection],
    context: OperationValidationConfig<'_>,
) -> Vec<ApolloDiagnostic> {
    let mut diagnostics = vec![];

    for selection in selection_set {
        match selection {
            ast::Selection::Field(field) => diagnostics.extend(super::field::validate_field(
                db,
                against_type,
                field.clone(),
                context.clone(),
            )),
            ast::Selection::FragmentSpread(fragment) => {
                diagnostics.extend(super::fragment::validate_fragment_spread(
                    db,
                    against_type,
                    fragment.clone(),
                    context.clone(),
                ))
            }
            ast::Selection::InlineFragment(inline) => {
                diagnostics.extend(super::fragment::validate_inline_fragment(
                    db,
                    against_type,
                    inline.clone(),
                    context.clone(),
                ))
            }
        }
    }

    diagnostics
}
