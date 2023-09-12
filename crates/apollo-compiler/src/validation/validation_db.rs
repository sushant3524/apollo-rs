use crate::{
    ast,
    database::db::Upcast,
    diagnostics::{ApolloDiagnostic, DiagnosticData, Label},
    hir::*,
    schema,
    validation::{
        self, argument, directive, enum_, extension, fragment, input_object, interface, object,
        operation, scalar, selection, union_, value, variable,
    },
    Arc, FileId, HirDatabase, InputDatabase, Node, ReprDatabase,
};
use apollo_parser::cst;
use apollo_parser::cst::CstNode;
use std::collections::{hash_map::Entry, HashMap};

use super::field;
use super::operation::OperationValidationConfig;

const BUILT_IN_SCALARS: [&str; 5] = ["Int", "Float", "Boolean", "String", "ID"];

#[salsa::query_group(ValidationStorage)]
pub trait ValidationDatabase:
    Upcast<dyn HirDatabase> + InputDatabase + ReprDatabase + HirDatabase
{
    fn ast_types(&self) -> Arc<ast::TypeSystem>;

    /// Validate all documents.
    fn validate(&self) -> Vec<ApolloDiagnostic>;

    /// Validate the type system, combined of all type system documents known to
    /// the compiler.
    #[salsa::invoke(validate_type_system)]
    fn validate_type_system(&self) -> Vec<ApolloDiagnostic>;

    /// Validate an executable document.
    #[salsa::invoke(validate_executable)]
    fn validate_executable(&self, file_id: FileId) -> Vec<ApolloDiagnostic>;

    /// Validate a standalone executable document, without knowledge of the type system it executes
    /// against.
    ///
    /// This runs a subset of the validations from `validate_executable`.
    #[salsa::invoke(validate_standalone_executable)]
    fn validate_standalone_executable(&self, file_id: FileId) -> Vec<ApolloDiagnostic>;

    /// Validate the names of all type definitions known to the compiler are unique.
    #[salsa::invoke(validate_type_system_names)]
    fn validate_type_system_names(&self) -> Vec<ApolloDiagnostic>;

    /// Validate names of operations and fragments in an executable document are unique.
    #[salsa::invoke(validate_executable_names)]
    fn validate_executable_names(&self, file_id: FileId) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(validation::schema::validate_schema_definition)]
    fn validate_schema_definition(
        &self,
        def: ast::TypeWithExtensions<ast::SchemaDefinition>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(scalar::validate_scalar_definitions)]
    fn validate_scalar_definitions(&self) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(scalar::validate_scalar_definition)]
    fn validate_scalar_definition(
        &self,
        scalar_def: Node<schema::ScalarType>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(enum_::validate_enum_definitions)]
    fn validate_enum_definitions(&self) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(enum_::validate_enum_definition)]
    fn validate_enum_definition(&self, def: Arc<EnumTypeDefinition>) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(enum_::validate_enum_value)]
    fn validate_enum_value(&self, def: EnumValueDefinition) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(union_::validate_union_definitions)]
    fn validate_union_definitions(&self) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(union_::validate_union_definition)]
    fn validate_union_definition(
        &self,
        union_: ast::TypeWithExtensions<ast::UnionTypeDefinition>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(interface::validate_interface_definitions)]
    fn validate_interface_definitions(&self) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(interface::validate_interface_definition)]
    fn validate_interface_definition(
        &self,
        interface: ast::TypeWithExtensions<ast::InterfaceTypeDefinition>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(directive::validate_directive_definition)]
    fn validate_directive_definition(
        &self,
        directive_definition: Node<ast::DirectiveDefinition>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(directive::validate_directive_definitions)]
    fn validate_directive_definitions(&self) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(directive::validate_directives)]
    fn validate_directives(
        &self,
        dirs: Vec<Directive>,
        loc: DirectiveLocation,
        var_defs: Arc<Vec<VariableDefinition>>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(input_object::validate_input_object_definitions)]
    fn validate_input_object_definitions(&self) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(input_object::validate_input_object_definition)]
    fn validate_input_object_definition(
        &self,
        input_object: ast::TypeWithExtensions<ast::InputObjectTypeDefinition>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(input_object::validate_input_values)]
    fn validate_input_values(
        &self,
        vals: Arc<Vec<InputValueDefinition>>,
        dir_loc: DirectiveLocation,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(object::validate_object_type_definitions)]
    fn validate_object_type_definitions(&self) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(extension::validate_extensions)]
    fn validate_extensions(&self) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(operation::validate_subscription_operations)]
    fn validate_subscription_operations(
        &self,
        defs: Arc<Vec<Arc<OperationDefinition>>>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(operation::validate_query_operations)]
    fn validate_query_operations(
        &self,
        defs: Arc<Vec<Arc<OperationDefinition>>>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(operation::validate_mutation_operations)]
    fn validate_mutation_operations(
        &self,
        mutations: Arc<Vec<Arc<OperationDefinition>>>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(object::validate_object_type_definition)]
    fn validate_object_type_definition(
        &self,
        def: ast::TypeWithExtensions<ast::ObjectTypeDefinition>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(field::validate_field_definitions)]
    fn validate_field_definitions(
        &self,
        fields: Vec<Node<ast::FieldDefinition>>,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(field::validate_field)]
    fn validate_field(
        &self,
        field: Arc<Field>,
        context: OperationValidationConfig,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(field::validate_leaf_field_selection)]
    fn validate_leaf_field_selection(
        &self,
        field: Arc<Field>,
        field_ty: Type,
    ) -> Result<(), ApolloDiagnostic>;

    #[salsa::invoke(argument::validate_arguments_definition)]
    fn validate_arguments_definition(
        &self,
        def: ArgumentsDefinition,
        loc: DirectiveLocation,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(argument::validate_arguments)]
    fn validate_arguments(&self, arg: Vec<Argument>) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(operation::validate_operation_definitions)]
    fn validate_operation_definitions(&self, file_id: FileId) -> Vec<ApolloDiagnostic>;

    /// Given a type definition, find all the types that can be used for fragment spreading.
    ///
    /// Spec: https://spec.graphql.org/October2021/#GetPossibleTypes()
    #[salsa::invoke(fragment::get_possible_types)]
    fn get_possible_types(&self, ty: TypeDefinition) -> Vec<TypeDefinition>;

    #[salsa::invoke(fragment::validate_fragment_selection)]
    fn validate_fragment_selection(&self, spread: FragmentSelection) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(fragment::validate_fragment_spread)]
    fn validate_fragment_spread(
        &self,
        spread: Arc<FragmentSpread>,
        context: OperationValidationConfig,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(fragment::validate_inline_fragment)]
    fn validate_inline_fragment(
        &self,
        inline: Arc<InlineFragment>,
        context: OperationValidationConfig,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(fragment::validate_fragment_definition)]
    #[salsa::transparent]
    fn validate_fragment_definition(
        &self,
        def: Arc<FragmentDefinition>,
        context: OperationValidationConfig,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(fragment::validate_fragment_cycles)]
    fn validate_fragment_cycles(&self, def: Arc<FragmentDefinition>) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(fragment::validate_fragment_type_condition)]
    fn validate_fragment_type_condition(
        &self,
        type_cond: String,
        loc: HirNodeLocation,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(fragment::validate_fragment_used)]
    fn validate_fragment_used(
        &self,
        def: Arc<FragmentDefinition>,
        file_id: FileId,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(selection::validate_selection_set)]
    fn validate_selection_set(
        &self,
        sel_set: SelectionSet,
        context: OperationValidationConfig,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(selection::validate_selection)]
    fn validate_selection(
        &self,
        sel: Arc<Vec<Selection>>,
        context: OperationValidationConfig,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::invoke(variable::validate_variable_definitions)]
    fn validate_variable_definitions(
        &self,
        defs: Arc<Vec<VariableDefinition>>,
        has_schema: bool,
    ) -> Vec<ApolloDiagnostic>;

    #[salsa::transparent]
    #[salsa::invoke(variable::validate_variable_usage)]
    fn validate_variable_usage(
        &self,
        var_usage: InputValueDefinition,
        var_defs: Arc<Vec<VariableDefinition>>,
        arg: Argument,
    ) -> Result<(), ApolloDiagnostic>;

    #[salsa::transparent]
    #[salsa::invoke(value::validate_values)]
    fn validate_values(
        &self,
        ty: &Type,
        arg: &Argument,
        var_defs: Arc<Vec<VariableDefinition>>,
    ) -> Result<(), Vec<ApolloDiagnostic>>;

    /// Check if two fields will output the same type.
    ///
    /// If the two fields output different types, returns an `Err` containing diagnostic information.
    /// To simply check if outputs are the same, you can use `.is_ok()`:
    /// ```rust,ignore
    /// let is_same = db.same_response_shape(field_a, field_b).is_ok();
    /// // `is_same` is `bool`
    /// ```
    ///
    /// Spec: https://spec.graphql.org/October2021/#SameResponseShape()
    #[salsa::invoke(selection::same_response_shape)]
    fn same_response_shape(
        &self,
        field_a: Arc<Field>,
        field_b: Arc<Field>,
    ) -> Result<(), ApolloDiagnostic>;

    /// Check if the fields in a given selection set can be merged.
    ///
    /// If the fields cannot be merged, returns an `Err` containing diagnostic information.
    ///
    /// Spec: https://spec.graphql.org/October2021/#FieldsInSetCanMerge()
    #[salsa::invoke(selection::fields_in_set_can_merge)]
    fn fields_in_set_can_merge(
        &self,
        selection_set: SelectionSet,
    ) -> Result<(), Vec<ApolloDiagnostic>>;
}

fn ast_types(db: &dyn ValidationDatabase) -> Arc<ast::TypeSystem> {
    let mut objects = HashMap::new();
    let mut scalars = HashMap::new();
    let mut interfaces = HashMap::new();
    let mut unions = HashMap::new();
    let mut enums = HashMap::new();
    let mut input_objects = HashMap::new();

    let mut schema_definition = None;
    let mut schema_extensions = vec![];

    for file_id in db.type_definition_files() {
        if file_id == FileId::BUILT_IN {
            continue;
        }

        let document = db.ast(file_id);
        println!("{document}");
        for definition in document.definitions.iter() {
            match definition {
                ast::Definition::SchemaDefinition(schema) => {
                    schema_definition = Some(schema.clone());
                }
                ast::Definition::ObjectTypeDefinition(object) => {
                    objects.insert(
                        object.name.clone(),
                        ast::TypeWithExtensions {
                            definition: object.clone(),
                            extensions: vec![],
                        },
                    );
                }
                ast::Definition::ScalarTypeDefinition(scalar) => {
                    scalars.insert(
                        scalar.name.clone(),
                        ast::TypeWithExtensions {
                            definition: scalar.clone(),
                            extensions: vec![],
                        },
                    );
                }
                ast::Definition::InterfaceTypeDefinition(interface) => {
                    interfaces.insert(
                        interface.name.clone(),
                        ast::TypeWithExtensions {
                            definition: interface.clone(),
                            extensions: vec![],
                        },
                    );
                }
                ast::Definition::UnionTypeDefinition(union_) => {
                    unions.insert(
                        union_.name.clone(),
                        ast::TypeWithExtensions {
                            definition: union_.clone(),
                            extensions: vec![],
                        },
                    );
                }
                ast::Definition::EnumTypeDefinition(enum_) => {
                    enums.insert(
                        enum_.name.clone(),
                        ast::TypeWithExtensions {
                            definition: enum_.clone(),
                            extensions: vec![],
                        },
                    );
                }
                ast::Definition::InputObjectTypeDefinition(input_object) => {
                    input_objects.insert(
                        input_object.name.clone(),
                        ast::TypeWithExtensions {
                            definition: input_object.clone(),
                            extensions: vec![],
                        },
                    );
                }
                _ => (),
            }
        }
        for definition in document.definitions.iter() {
            match definition {
                ast::Definition::SchemaExtension(schema) => {
                    schema_extensions.push(schema.clone());
                }
                ast::Definition::ObjectTypeExtension(extension) => {
                    if let Some(ty) = objects.get_mut(&extension.name) {
                        ty.extensions.push(extension.clone());
                    }
                }
                ast::Definition::ScalarTypeExtension(extension) => {
                    if let Some(ty) = scalars.get_mut(&extension.name) {
                        ty.extensions.push(extension.clone());
                    }
                }
                ast::Definition::InterfaceTypeExtension(extension) => {
                    if let Some(ty) = interfaces.get_mut(&extension.name) {
                        ty.extensions.push(extension.clone());
                    }
                }
                ast::Definition::UnionTypeExtension(extension) => {
                    if let Some(ty) = unions.get_mut(&extension.name) {
                        ty.extensions.push(extension.clone());
                    }
                }
                ast::Definition::EnumTypeExtension(extension) => {
                    if let Some(ty) = enums.get_mut(&extension.name) {
                        ty.extensions.push(extension.clone());
                    }
                }
                ast::Definition::InputObjectTypeExtension(extension) => {
                    if let Some(ty) = input_objects.get_mut(&extension.name) {
                        ty.extensions.push(extension.clone());
                    }
                }
                _ => (),
            }
        }
    }

    let schema = ast::TypeWithExtensions {
        definition: schema_definition.unwrap_or_else(|| {
            Node::new_synthetic(ast::SchemaDefinition {
                description: None,
                directives: vec![],
                root_operations: {
                    let mut operations = Vec::with_capacity(3);
                    let query_name = ast::Name::new_synthetic("Query");
                    if objects.contains_key(&query_name) {
                        operations.push((ast::OperationType::Query, query_name));
                    }
                    let mutation_name = ast::Name::new_synthetic("Mutation");
                    if objects.contains_key(&mutation_name) {
                        operations.push((ast::OperationType::Mutation, mutation_name));
                    }
                    let subscription_name = ast::Name::new_synthetic("Subscription");
                    if objects.contains_key(&subscription_name) {
                        operations.push((ast::OperationType::Subscription, subscription_name));
                    }
                    operations
                },
            })
        }),
        extensions: schema_extensions,
    };

    Arc::new(ast::TypeSystem {
        schema,
        objects,
        scalars,
        interfaces,
        unions,
        enums,
        input_objects,
    })
}

pub fn validate(db: &dyn ValidationDatabase) -> Vec<ApolloDiagnostic> {
    let mut diagnostics = Vec::new();
    for file_id in db.source_files() {
        diagnostics.extend(db.syntax_errors(file_id).iter().cloned());
    }

    diagnostics.extend(db.validate_type_system());

    for file_id in db.executable_definition_files() {
        diagnostics.extend(db.validate_executable(file_id));
    }

    diagnostics
}

fn validate_type_system_names(db: &dyn ValidationDatabase) -> Vec<ApolloDiagnostic> {
    let mut diagnostics = Vec::new();

    // Different node types use different namespaces.
    let mut directive_scope = HashMap::<String, (FileId, cst::Name)>::new();
    let mut type_scope = HashMap::<String, (FileId, cst::Name)>::new();

    let all_types = db
        .type_definition_files()
        .into_iter()
        .flat_map(move |file_id| {
            db.cst(file_id)
                .document()
                .syntax()
                .children()
                .filter_map(cst::Definition::cast)
                // Extension names are allowed to be duplicates,
                // and schema definitions don't have names.
                .filter(|def| {
                    !def.is_extension_definition()
                        && !def.is_executable_definition()
                        && !matches!(def, cst::Definition::SchemaDefinition(_))
                })
                .map(move |def| (file_id, def))
        });

    for (file_id, cst_def) in all_types {
        let ty_ = match cst_def {
            cst::Definition::DirectiveDefinition(_) => "directive",
            cst::Definition::ScalarTypeDefinition(_)
            | cst::Definition::ObjectTypeDefinition(_)
            | cst::Definition::InterfaceTypeDefinition(_)
            | cst::Definition::UnionTypeDefinition(_)
            | cst::Definition::EnumTypeDefinition(_)
            | cst::Definition::InputObjectTypeDefinition(_) => "type",
            // Only validate type system definitions.
            cst::Definition::OperationDefinition(_) | cst::Definition::FragmentDefinition(_) => {
                unreachable!()
            }
            // Schemas do not have a name.
            cst::Definition::SchemaDefinition(_) => unreachable!(),
            // Extension names are always duplicate.
            cst::Definition::SchemaExtension(_)
            | cst::Definition::ScalarTypeExtension(_)
            | cst::Definition::ObjectTypeExtension(_)
            | cst::Definition::InterfaceTypeExtension(_)
            | cst::Definition::UnionTypeExtension(_)
            | cst::Definition::EnumTypeExtension(_)
            | cst::Definition::InputObjectTypeExtension(_) => unreachable!(),
        };
        let scope = match cst_def {
            cst::Definition::DirectiveDefinition(_) => &mut directive_scope,
            _ => &mut type_scope,
        };

        if let Some(name_node) = cst_def.name() {
            let name = &*name_node.text();
            match scope.entry(name.to_string()) {
                Entry::Occupied(entry) => {
                    let (original_file_id, original) = entry.get();
                    let original_definition = (*original_file_id, original.syntax().text_range());
                    let redefined_definition = (file_id, name_node.syntax().text_range());
                    let is_built_in = db.input(file_id).source_type().is_built_in();
                    let is_scalar = matches!(cst_def, cst::Definition::ScalarTypeDefinition(_));

                    if is_scalar && BUILT_IN_SCALARS.contains(&name) && !is_built_in {
                        diagnostics.push(
                            ApolloDiagnostic::new(
                                db,
                                redefined_definition.into(),
                                DiagnosticData::BuiltInScalarDefinition,
                            )
                            .label(Label::new(
                                redefined_definition,
                                "remove this scalar definition",
                            )),
                        );
                    } else {
                        diagnostics.push(
                            ApolloDiagnostic::new(
                                db,
                                redefined_definition.into(),
                                DiagnosticData::UniqueDefinition {
                                    ty: ty_,
                                    name: name.to_string(),
                                    original_definition: original_definition.into(),
                                    redefined_definition: redefined_definition.into(),
                                },
                            )
                            .labels([
                                Label::new(
                                    original_definition,
                                    format!("previous definition of `{name}` here"),
                                ),
                                Label::new(
                                    redefined_definition,
                                    format!("`{name}` redefined here"),
                                ),
                            ])
                            .help(format!(
                                "`{name}` must only be defined once in this document."
                            )),
                        );
                    }
                }
                Entry::Vacant(entry) => {
                    entry.insert((file_id, name_node));
                }
            }
        }
    }

    diagnostics
}

fn validate_executable_names(
    db: &dyn ValidationDatabase,
    file_id: FileId,
) -> Vec<ApolloDiagnostic> {
    let mut diagnostics = Vec::new();

    // Different node types use different namespaces.
    let mut fragment_scope = HashMap::<String, cst::Name>::new();
    let mut operation_scope = HashMap::new();

    let executable_definitions = db
        .cst(file_id)
        .document()
        .syntax()
        .children()
        .filter_map(cst::Definition::cast)
        .filter(|def| def.is_executable_definition());

    for cst_def in executable_definitions {
        let ty_ = match cst_def {
            cst::Definition::OperationDefinition(_) => "operation",
            cst::Definition::FragmentDefinition(_) => "fragment",
            // Type system definitions are not checked here.
            _ => unreachable!(),
        };
        let scope = match cst_def {
            cst::Definition::OperationDefinition(_) => &mut operation_scope,
            cst::Definition::FragmentDefinition(_) => &mut fragment_scope,
            // Type system definitions are not checked here.
            _ => unreachable!(),
        };

        if let Some(name_node) = cst_def.name() {
            let name = &*name_node.text();
            match scope.entry(name.to_string()) {
                Entry::Occupied(entry) => {
                    let original = entry.get();
                    let original_definition = (file_id, original.syntax().text_range());
                    let redefined_definition = (file_id, name_node.syntax().text_range());

                    diagnostics.push(
                        ApolloDiagnostic::new(
                            db,
                            redefined_definition.into(),
                            DiagnosticData::UniqueDefinition {
                                ty: ty_,
                                name: name.to_string(),
                                original_definition: original_definition.into(),
                                redefined_definition: redefined_definition.into(),
                            },
                        )
                        .labels([
                            Label::new(
                                original_definition,
                                format!("previous definition of `{name}` here"),
                            ),
                            Label::new(redefined_definition, format!("`{name}` redefined here")),
                        ])
                        .help(format!(
                            "`{name}` must only be defined once in this document."
                        )),
                    );
                }
                Entry::Vacant(entry) => {
                    entry.insert(name_node);
                }
            }
        }
    }

    diagnostics
}

fn location_sort_key(diagnostic: &ApolloDiagnostic) -> (FileId, usize) {
    (diagnostic.location.file_id(), diagnostic.location.offset())
}

pub fn validate_type_system(db: &dyn ValidationDatabase) -> Vec<ApolloDiagnostic> {
    let mut diagnostics = Vec::new();

    diagnostics.extend(db.validate_type_system_names());

    let schema = db.ast_types().schema.clone();
    diagnostics.extend(db.validate_schema_definition(schema));

    diagnostics.extend(db.validate_scalar_definitions());
    diagnostics.extend(db.validate_enum_definitions());
    diagnostics.extend(db.validate_union_definitions());

    diagnostics.extend(db.validate_interface_definitions());
    diagnostics.extend(db.validate_directive_definitions());
    diagnostics.extend(db.validate_input_object_definitions());
    diagnostics.extend(db.validate_object_type_definitions());

    diagnostics.extend(db.validate_extensions());

    diagnostics.sort_by_key(location_sort_key);
    diagnostics
}

fn validate_executable_inner(
    db: &dyn ValidationDatabase,
    file_id: FileId,
    has_schema: bool,
) -> Vec<ApolloDiagnostic> {
    let mut diagnostics = Vec::new();

    if db.source_type(file_id).is_executable() {
        let document = db.ast(file_id);
        for def in &document.definitions {
            if def.is_executable_definition() {
                continue;
            }
            let Some(&location) = def.location() else {
                continue;
            };
            diagnostics.push(
                ApolloDiagnostic::new(
                    db,
                    location.into(),
                    DiagnosticData::ExecutableDefinition { kind: def.kind() },
                )
                .label(Label::new(
                    location,
                    "not supported in executable documents",
                )),
            );
        }
    }

    diagnostics.extend(db.validate_executable_names(file_id));

    diagnostics.extend(super::operation::validate_operation_definitions_inner(
        db, file_id, has_schema,
    ));
    for def in db.fragments(file_id).values() {
        diagnostics.extend(db.validate_fragment_used(Arc::clone(def), file_id));
    }

    diagnostics.sort_by_key(location_sort_key);
    diagnostics
}

pub fn validate_standalone_executable(
    db: &dyn ValidationDatabase,
    file_id: FileId,
) -> Vec<ApolloDiagnostic> {
    validate_executable_inner(db, file_id, false)
}

pub fn validate_executable(db: &dyn ValidationDatabase, file_id: FileId) -> Vec<ApolloDiagnostic> {
    validate_executable_inner(db, file_id, true)
}

#[cfg(test)]
mod tests {
    use super::ValidationDatabase;
    use crate::ApolloCompiler;
    use crate::HirDatabase;

    #[test]
    fn executable_and_type_system_definitions() {
        let input_type_system = r#"
type Query {
    name: String
}
"#;
        let input_executable = r#"
fragment q on Query { name }
query {
    ...q
}
"#;

        let mut compiler = ApolloCompiler::new();
        compiler.add_type_system(input_type_system, "schema.graphql");
        compiler.add_executable(input_executable, "query.graphql");

        let diagnostics = compiler.validate();
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn executable_definition_does_not_contain_type_system_definitions() {
        let input_type_system = r#"
type Query {
    name: String
}
"#;
        let input_executable = r#"
type Object {
    notAllowed: Boolean!
}
fragment q on Query { name }
query {
    ...q
}
"#;

        let mut compiler = ApolloCompiler::new();
        compiler.add_type_system(input_type_system, "schema.graphql");
        compiler.add_executable(input_executable, "query.graphql");

        let diagnostics = compiler.validate();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].data.to_string(),
            "executable documents must not contain ObjectTypeDefinition"
        );
    }

    #[test]
    fn executable_definition_with_cycles_do_not_overflow_stack() {
        let input_type_system = r#"
type Query {
    name: String
}
"#;

        let input_executable = r#"
{
    ...q
}
fragment q on Query {
    ...q
}
"#;

        let mut compiler = ApolloCompiler::new();
        compiler.add_type_system(input_type_system, "schema.graphql");
        compiler.add_executable(input_executable, "query.graphql");

        let diagnostics = compiler.validate();

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].data.to_string(),
            "`q` fragment cannot reference itself"
        );
    }

    #[test]
    fn executable_definition_with_nested_cycles_do_not_overflow_stack() {
        let input_type_system = r#"
type Query {
    obj: TestObject
}

type TestObject {
    name: String
}
"#;

        let input_executable = r#"
{
    obj {
        ...q
    }
}

fragment q on TestObject {
    ...q
}
"#;

        let mut compiler = ApolloCompiler::new();
        compiler.add_type_system(input_type_system, "schema.graphql");
        compiler.add_executable(input_executable, "query.graphql");

        let diagnostics = compiler.validate();

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].data.to_string(),
            "`q` fragment cannot reference itself"
        );
    }

    #[test]
    fn validation_with_type_system_hir() {
        let input_type_system = r#"
type Query {
    obj: TestObject
}

type TestObject {
    name: String
}
"#;

        let input_executable = r#"
{
    obj {
        name
        nickname
    }
}
"#;

        let mut root_compiler = ApolloCompiler::new();
        root_compiler.add_type_system(input_type_system, "schema.graphql");
        assert!(root_compiler.validate().is_empty());

        let mut child_compiler = ApolloCompiler::new();
        child_compiler.set_type_system_hir(root_compiler.db.type_system());
        let executable_id = child_compiler.add_executable(input_executable, "query.graphql");
        let diagnostics = child_compiler.db.validate_executable(executable_id);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].data.to_string(),
            "cannot query field `nickname` on type `TestObject`"
        );
    }

    #[test]
    fn validation_without_type_system() {
        let mut compiler = ApolloCompiler::new();

        let valid_id = compiler.add_executable(r#"{ obj { name nickname } }"#, "valid.graphql");
        let diagnostics = compiler.db.validate_standalone_executable(valid_id);
        // We don't know what `obj` refers to, so assume it is valid.
        assert!(diagnostics.is_empty());

        let unused_frag_id = compiler.add_executable(
            r#"
            fragment A on Type { a }
            query { b }
        "#,
            "dupe_frag.graphql",
        );
        let diagnostics = compiler.db.validate_standalone_executable(unused_frag_id);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].data.to_string(),
            "fragment `A` must be used in an operation"
        );

        let dupe_frag_id = compiler.add_executable(
            r#"
            fragment A on Type { a }
            fragment A on Type { b }
            query { ...A }
        "#,
            "dupe_frag.graphql",
        );
        let diagnostics = compiler.db.validate_standalone_executable(dupe_frag_id);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].data.to_string(),
            "the fragment `A` is defined multiple times in the document"
        );

        let unknown_frag_id = compiler.add_executable(r#"{ ...A }"#, "unknown_frag.graphql");
        let diagnostics = compiler.db.validate_standalone_executable(unknown_frag_id);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].data.to_string(),
            "cannot find fragment `A` in this document"
        );
    }

    #[test]
    fn validate_variable_usage_without_type_system() {
        let mut compiler = ApolloCompiler::new();
        let id = compiler.add_executable(r#"
query nullableStringArg($nonNullableVar: String!, $nonNullableList: [String!]!, $nonNullableListList: [[Int!]!]) {
  arguments {
    nullableString(nullableString: $nonNullableVar)
    nullableList(nullableList: $nonNullableList)
    nullableListList(nullableListList: $nonNullableListList)
  }
}
"#, "query.graphql");

        let diagnostics = compiler.db.validate_standalone_executable(id);
        for diag in &diagnostics {
            println!("{diag}")
        }
        assert_eq!(diagnostics.len(), 0);
    }
}
