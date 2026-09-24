//! Python docstrings, which are documentation rather than executable code.

use super::{Code, Context};
use crate::{language::Language, syntax::Syntax};
use tree_sitter::Node;

/// A docstring is a constant string statement that comes first, ignoring comments, in a
/// module, class, or function. Formatted, byte, and template strings are executable.
pub(super) fn is_docstring(node: Node<'_>, context: &Context<'_>, code: Code<'_>) -> bool {
    match code.language {
        Language::Python => {
            context.first_statement
                && is_documentable(context)
                && is_constant_string_statement(node, code)
        }
        Language::Javascript
        | Language::Typescript
        | Language::Tsx
        | Language::Rust
        | Language::Go => false,
    }
}

fn is_documentable(context: &Context<'_>) -> bool {
    match (context.parent, context.grandparent) {
        (Some(parent), _) if parent.syntax == Syntax::Module => true,
        (Some(parent), Some(owner)) => {
            parent.syntax == Syntax::Block
                && matches!(owner.syntax, Syntax::Function | Syntax::Class)
        }
        _ => false,
    }
}

fn is_constant_string_statement(node: Node<'_>, code: Code<'_>) -> bool {
    if code.syntax(node) != Syntax::ExpressionStatement {
        return false;
    }
    let Some(value) = node.named_child(0) else {
        return false;
    };
    match code.syntax(value) {
        Syntax::String => is_constant_string(value, code),
        Syntax::ConcatenatedString => {
            let mut cursor = value.walk();
            value
                .named_children(&mut cursor)
                .all(|part| is_constant_string(part, code))
        }
        _ => false,
    }
}

fn is_constant_string(string: Node<'_>, code: Code<'_>) -> bool {
    let prefix = code.text(string).split(['\'', '"']).next().unwrap_or("");
    !prefix
        .bytes()
        .any(|byte| matches!(byte.to_ascii_lowercase(), b'f' | b'b' | b't'))
}
