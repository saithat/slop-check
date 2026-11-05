//! Function names and parameter counts.

use super::{Code, Context};
use crate::{
    language::Language,
    syntax::{Field, NodeExt, Syntax},
};
use tree_sitter::Node;

/// A function's own name, else the name it is assigned to, else its position.
pub(super) fn name(function: Node<'_>, context: &Context<'_>, code: Code<'_>) -> String {
    if let Some(name) = function.field(Field::Name) {
        return code.text(name).to_owned();
    }
    // Closures usually have no name of their own; the assignment, property, or keyword
    // argument that immediately holds them names them instead.
    // An operand of `a || function () {}` or `cb or lambda: 0` is not held by name.
    let owner = context
        .parent
        .filter(|parent| parent.syntax != Syntax::Binary)
        .and_then(|parent| {
            [Field::Name, Field::Left, Field::Key]
                .into_iter()
                .filter_map(|field| parent.node.field(field))
                .find(|name| name.id() != function.id())
        });
    match owner {
        Some(owner) => code.text(owner).to_owned(),
        None => format!(
            "<anonymous@{}:{}>",
            function.start_position().row + 1,
            function.start_position().column + 1
        ),
    }
}

/// Counts declared parameters, including receivers such as Rust `self`. Python's bare
/// `/` and `*` separators and Rust parameter attributes are not parameters, and each name in a Go group such as
/// `x, y int` is one.
pub(super) fn parameter_count(function: Node<'_>, code: Code<'_>) -> u32 {
    let Some(parameters) = function.field(Field::Parameters) else {
        // An arrow function with one unparenthesized parameter.
        return u32::from(function.field(Field::Parameter).is_some());
    };
    let mut cursor = parameters.walk();
    parameters
        .named_children(&mut cursor)
        .filter(|parameter| {
            !matches!(
                code.syntax(*parameter),
                Syntax::Comment | Syntax::ParameterSeparator | Syntax::Attribute
            )
        })
        .map(|parameter| match code.language {
            Language::Go => {
                let mut cursor = parameter.walk();
                let names = parameter
                    .children_by_field_name(Field::Name.name(), &mut cursor)
                    .count();
                u32::try_from(names.max(1)).unwrap_or(u32::MAX)
            }
            Language::Python
            | Language::Javascript
            | Language::Typescript
            | Language::Tsx
            | Language::Rust => 1,
        })
        .sum()
}
