use arbitrary::Unstructured;

use apollo_compiler::ast::{Definition, Document};
use apollo_compiler::{Node, Schema};

use crate::next::ast::document::DocumentExt;
use crate::next::mutations::Mutation;

pub(crate) struct AddDirectiveDefinition;
impl Mutation for AddDirectiveDefinition {
    fn apply(
        &self,
        u: &mut Unstructured,
        doc: &mut Document,
        schema: &Schema,
    ) -> arbitrary::Result<()> {
        doc.definitions
            .push(Definition::DirectiveDefinition(Node::new(
                doc.arbitrary_directive_definition(u, schema)?,
            )));

        Ok(())
    }
    fn is_valid(&self) -> bool {
        true
    }
}
