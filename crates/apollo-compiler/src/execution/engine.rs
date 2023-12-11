use crate::ast::Name;
use crate::ast::Value;
use crate::executable::Field;
use crate::executable::Selection;
use crate::execution::input_coercion::coerce_argument_values;
use crate::execution::resolver::ObjectValue;
use crate::execution::resolver::ResolverError;
use crate::execution::result_coercion::complete_value;
use crate::execution::GraphQLError;
use crate::execution::JsonMap;
use crate::execution::JsonValue;
use crate::execution::ResponseDataPathElement;
use crate::node::NodeLocation;
use crate::schema::ExtendedType;
use crate::schema::FieldDefinition;
use crate::schema::ObjectType;
use crate::schema::Type;
use crate::validation::SuspectedValidationBug;
use crate::validation::Valid;
use crate::ExecutableDocument;
use crate::Schema;
use crate::SourceMap;
use indexmap::IndexMap;
use std::collections::HashSet;

/// <https://spec.graphql.org/October2021/#sec-Normal-and-Serial-Execution>
#[derive(Debug, Copy, Clone)]
pub(crate) enum ExecutionMode {
    /// Allowed to resolve fields in any order, including in parellel
    Normal,
    /// Top-level fields of a mutation operation must be executed in order
    #[allow(unused)]
    Sequential,
}

/// Return in `Err` when a field error occurred at some non-nullable place
///
/// <https://spec.graphql.org/October2021/#sec-Handling-Field-Errors>
pub(crate) struct PropagateNull;

/// Linked-list version of `Vec<PathElement>`, taking advantage of the call stack
pub(crate) type LinkedPath<'a> = Option<&'a LinkedPathElement<'a>>;

pub(crate) struct LinkedPathElement<'a> {
    pub(crate) element: ResponseDataPathElement,
    pub(crate) next: LinkedPath<'a>,
}

/// <https://spec.graphql.org/October2021/#ExecuteSelectionSet()>
#[allow(clippy::too_many_arguments)] // yes it’s not a nice API but it’s internal
pub(crate) fn execute_selection_set<'a>(
    schema: &Valid<Schema>,
    document: &'a Valid<ExecutableDocument>,
    variable_values: &Valid<JsonMap>,
    errors: &mut Vec<GraphQLError>,
    path: LinkedPath<'_>,
    mode: ExecutionMode,
    object_type_name: &str,
    object_type: &ObjectType,
    object_value: &ObjectValue<'_>,
    selections: impl IntoIterator<Item = &'a Selection>,
) -> Result<JsonMap, PropagateNull> {
    let mut grouped_field_set = IndexMap::new();
    collect_fields(
        schema,
        document,
        variable_values,
        object_type_name,
        object_type,
        selections,
        &mut HashSet::new(),
        &mut grouped_field_set,
    );

    match mode {
        ExecutionMode::Normal => {}
        ExecutionMode::Sequential => {
            // If we want parallelism, use `futures::future::join_all` (async)
            // or Rayon’s `par_iter` (sync) here.
        }
    }

    let mut response_map = JsonMap::with_capacity(grouped_field_set.len());
    for (&response_key, fields) in &grouped_field_set {
        // Indexing should not panic: `collect_fields` only creates a `Vec` to push to it
        let field_name = &fields[0].name;
        let Ok(field_def) = schema.type_field(object_type_name, field_name) else {
            // TODO: Return a `validation_bug`` field error here?
            // The spec specifically has a “If fieldType is defined” condition,
            // but it being undefined would make the request invalid, right?
            continue;
        };
        let value = if field_name == "__typename" {
            JsonValue::from(object_type_name)
        } else {
            let field_path = LinkedPathElement {
                element: ResponseDataPathElement::Field(response_key.clone()),
                next: path,
            };
            execute_field(
                schema,
                document,
                variable_values,
                errors,
                Some(&field_path),
                mode,
                object_value,
                field_def,
                fields,
            )?
        };
        response_map.insert(response_key.as_str(), value);
    }
    Ok(response_map)
}

/// <https://spec.graphql.org/October2021/#CollectFields()>
#[allow(clippy::too_many_arguments)] // yes it’s not a nice API but it’s internal
fn collect_fields<'a>(
    schema: &Schema,
    document: &'a ExecutableDocument,
    variable_values: &Valid<JsonMap>,
    object_type_name: &str,
    object_type: &ObjectType,
    selections: impl IntoIterator<Item = &'a Selection>,
    visited_fragments: &mut HashSet<&'a Name>,
    grouped_fields: &mut IndexMap<&'a Name, Vec<&'a Field>>,
) {
    for selection in selections {
        if eval_if_arg(selection, "skip", variable_values).unwrap_or(false)
            || !eval_if_arg(selection, "include", variable_values).unwrap_or(true)
        {
            continue;
        }
        match selection {
            Selection::Field(field) => grouped_fields
                .entry(field.response_key())
                .or_default()
                .push(field.as_ref()),
            Selection::FragmentSpread(spread) => {
                let new = visited_fragments.insert(&spread.fragment_name);
                if !new {
                    continue;
                }
                let Some(fragment) = document.fragments.get(&spread.fragment_name) else {
                    continue;
                };
                if !does_fragment_type_apply(
                    schema,
                    object_type_name,
                    object_type,
                    fragment.type_condition(),
                ) {
                    continue;
                }
                collect_fields(
                    schema,
                    document,
                    variable_values,
                    object_type_name,
                    object_type,
                    &fragment.selection_set.selections,
                    visited_fragments,
                    grouped_fields,
                )
            }
            Selection::InlineFragment(inline) => {
                if let Some(condition) = &inline.type_condition {
                    if !does_fragment_type_apply(schema, object_type_name, object_type, condition) {
                        continue;
                    }
                }
                collect_fields(
                    schema,
                    document,
                    variable_values,
                    object_type_name,
                    object_type,
                    &inline.selection_set.selections,
                    visited_fragments,
                    grouped_fields,
                )
            }
        }
    }
}

/// <https://spec.graphql.org/October2021/#DoesFragmentTypeApply()>
fn does_fragment_type_apply(
    schema: &Schema,
    object_type_name: &str,
    object_type: &ObjectType,
    fragment_type: &Name,
) -> bool {
    match schema.types.get(fragment_type) {
        Some(ExtendedType::Object(_)) => fragment_type == object_type_name,
        Some(ExtendedType::Interface(_)) => {
            object_type.implements_interfaces.contains(fragment_type)
        }
        Some(ExtendedType::Union(def)) => def.members.contains(object_type_name),
        // Undefined or not an output type: validation should have caught this
        _ => false,
    }
}

fn eval_if_arg(
    selection: &Selection,
    directive_name: &str,
    variable_values: &Valid<JsonMap>,
) -> Option<bool> {
    match selection
        .directives()
        .get(directive_name)?
        .argument_by_name("if")?
        .as_ref()
    {
        Value::Boolean(value) => Some(*value),
        Value::Variable(var) => variable_values.get(var.as_str())?.as_bool(),
        _ => None,
    }
}

/// <https://spec.graphql.org/October2021/#ExecuteField()>
#[allow(clippy::too_many_arguments)] // yes it’s not a nice API but it’s internal
fn execute_field(
    schema: &Valid<Schema>,
    document: &Valid<ExecutableDocument>,
    variable_values: &Valid<JsonMap>,
    errors: &mut Vec<GraphQLError>,
    path: LinkedPath<'_>,
    mode: ExecutionMode,
    object_value: &ObjectValue<'_>,
    field_def: &FieldDefinition,
    fields: &[&Field],
) -> Result<JsonValue, PropagateNull> {
    let field = fields[0];
    let argument_values = match coerce_argument_values(
        schema,
        document,
        variable_values,
        errors,
        path,
        field_def,
        field,
    ) {
        Ok(argument_values) => argument_values,
        Err(PropagateNull) => return try_nullify(&field_def.ty, Err(PropagateNull)),
    };
    let resolved_result = object_value.resolve_field(&field.name, &argument_values);
    let completed_result = match resolved_result {
        Ok(resolved) => complete_value(
            schema,
            document,
            variable_values,
            errors,
            path,
            mode,
            field.ty(),
            resolved,
            fields,
        ),
        Err(ResolverError { message }) => {
            errors.push(GraphQLError::field_error(
                format!("resolver error: {message}"),
                path,
                field.name.location(),
                &document.sources,
            ));
            Err(PropagateNull)
        }
    };
    try_nullify(&field_def.ty, completed_result)
}

/// Try to insert a propagated null if possible, or keep propagating it.
///
/// <https://spec.graphql.org/October2021/#sec-Handling-Field-Errors>
pub(crate) fn try_nullify(
    ty: &Type,
    result: Result<JsonValue, PropagateNull>,
) -> Result<JsonValue, PropagateNull> {
    match result {
        Ok(json) => Ok(json),
        Err(PropagateNull) => {
            if ty.is_non_null() {
                Err(PropagateNull)
            } else {
                Ok(JsonValue::Null)
            }
        }
    }
}

pub(crate) fn path_to_vec(mut link: LinkedPath<'_>) -> Vec<ResponseDataPathElement> {
    let mut path = Vec::new();
    while let Some(node) = link {
        path.push(node.element.clone());
        link = node.next;
    }
    path.reverse();
    path
}

impl GraphQLError {
    pub(crate) fn field_error(
        message: impl Into<String>,
        path: LinkedPath<'_>,
        location: Option<NodeLocation>,
        sources: &SourceMap,
    ) -> Self {
        let mut err = Self::new(message, location, sources);
        err.path = path_to_vec(path);
        err
    }
}

impl SuspectedValidationBug {
    pub(crate) fn into_field_error(
        self,
        sources: &SourceMap,
        path: LinkedPath<'_>,
    ) -> GraphQLError {
        let mut err = self.into_graphql_error(sources);
        err.path = path_to_vec(path);
        err
    }
}
