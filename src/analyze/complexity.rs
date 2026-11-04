//! Cyclomatic and cognitive complexity contributions of individual syntax nodes.

use super::{Code, Context};
use crate::{
    language::Language,
    syntax::{Field, NodeExt, Syntax},
};
use tree_sitter::Node;

/// What one node adds to its function's complexity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Decision {
    pub cyclomatic: u32,
    /// The base cognitive increment, before any nesting cost.
    pub cognitive: u32,
    /// Whether the node opens a nesting level for cognitive complexity.
    pub nests: bool,
}

impl Decision {
    const NONE: Self = Self::new(0, 0, false);
    /// `if`, loops, `catch`, conditional expressions, and comprehension clauses.
    const BRANCH: Self = Self::new(1, 1, true);
    /// `else if`, `elif`, and cases or arms that test a value.
    const FLAT_BRANCH: Self = Self::new(1, 1, false);
    /// A plain `else`.
    const ELSE: Self = Self::new(0, 1, false);
    /// A switch or match container: its cases are the decisions.
    const CONTAINER: Self = Self::new(0, 0, true);

    const fn new(cyclomatic: u32, cognitive: u32, nests: bool) -> Self {
        Self {
            cyclomatic,
            cognitive,
            nests,
        }
    }
}

pub(super) fn decision(
    node: Node<'_>,
    syntax: Syntax,
    context: &Context<'_>,
    code: Code<'_>,
) -> Decision {
    match syntax {
        Syntax::Binary => logical_operator(node, context, code),
        Syntax::If if is_else_if(node, context) => Decision::FLAT_BRANCH,
        // A Python match guard only makes its arm refutable, as a Rust guard does.
        Syntax::Filter if is_match_guard(context) => Decision::NONE,
        Syntax::If | Syntax::Loop | Syntax::Filter | Syntax::Catch | Syntax::Conditional => {
            Decision::BRANCH
        }
        Syntax::Elif | Syntax::Case => Decision::FLAT_BRANCH,
        Syntax::ElseClause if continues_with_if(node, code) => Decision::NONE,
        Syntax::ElseClause => Decision::ELSE,
        Syntax::ElseKeyword => else_keyword(context, code),
        Syntax::MatchArm if is_fallback_arm(node, code) => Decision::NONE,
        Syntax::MatchArm => Decision::FLAT_BRANCH,
        Syntax::Switch => Decision::CONTAINER,
        _ => Decision::NONE,
    }
}

/// Whether an `if` is the `else if` of an enclosing `if`, rather than the head of a chain.
pub(super) fn is_else_if(node: Node<'_>, context: &Context<'_>) -> bool {
    context.parent.is_some_and(|parent| {
        parent.syntax == Syntax::ElseClause
            || parent
                .node
                .field(Field::Alternative)
                .is_some_and(|alternative| alternative.id() == node.id())
    })
}

/// Every boolean operator is a cyclomatic decision, but a run of the same operator
/// (`a && b && c`) costs one cognitive point rather than one per operator.
fn logical_operator(node: Node<'_>, context: &Context<'_>, code: Code<'_>) -> Decision {
    let Some(operator) = operator_of(node, code).filter(|operator| is_logical(operator)) else {
        return Decision::NONE;
    };
    let continues_run = context.parent.is_some_and(|parent| {
        parent.syntax == Syntax::Binary && operator_of(parent.node, code) == Some(operator)
    });
    Decision::new(1, u32::from(!continues_run), false)
}

fn operator_of<'source>(node: Node<'_>, code: Code<'source>) -> Option<&'source str> {
    node.field(Field::Operator)
        .map(|operator| code.text(operator))
}

fn is_logical(operator: &str) -> bool {
    matches!(operator, "and" | "or" | "&&" | "||" | "??")
}

/// An `else` whose body is another `if` belongs to an `else if` chain, which counts at
/// the `if`.
fn continues_with_if(else_clause: Node<'_>, code: Code<'_>) -> bool {
    else_clause
        .named_child(0)
        .is_some_and(|child| code.syntax(child) == Syntax::If)
}

fn else_keyword(context: &Context<'_>, code: Code<'_>) -> Decision {
    let Some(parent) = context.parent else {
        return Decision::NONE;
    };
    match parent.syntax {
        // The clause or conditional expression that owns the keyword is what counts.
        Syntax::ElseClause | Syntax::Conditional => Decision::NONE,
        // Go has no else clause: the keyword belongs to the `if` itself.
        Syntax::If => {
            let chained = parent
                .node
                .field(Field::Alternative)
                .is_some_and(|alternative| code.syntax(alternative) == Syntax::If);
            if chained {
                Decision::NONE
            } else {
                Decision::ELSE
            }
        }
        // Rust `let ... else`.
        _ => Decision::ELSE,
    }
}

fn is_match_guard(context: &Context<'_>) -> bool {
    context.field == Some(Field::Guard.name())
        && context
            .parent
            .is_some_and(|parent| parent.syntax == Syntax::MatchArm)
}

/// A catch-all arm is not a decision: `_`, or an unguarded pattern that binds any value.
/// A bare Rust identifier binds only when it is not a constant or unit variant, which
/// syntax alone cannot tell; by convention those are capitalized, so only a lowercase
/// identifier is treated as a binding.
fn is_fallback_arm(arm: Node<'_>, code: Code<'_>) -> bool {
    let Some(pattern) = arm.field(Field::Pattern).or_else(|| arm.named_child(0)) else {
        return false;
    };
    // A guard can reject even an otherwise irrefutable pattern.
    let guarded = match code.language {
        Language::Python => arm.field(Field::Guard).is_some(),
        Language::Rust => pattern.field(Field::Condition).is_some(),
        Language::Javascript | Language::Typescript | Language::Tsx | Language::Go => false,
    };
    if guarded {
        return false;
    }
    if code.text(pattern).trim() == "_" {
        return true;
    }
    match code.language {
        Language::Python => pattern.named_child(0).is_some_and(|capture| {
            code.syntax(capture) == Syntax::DottedName && capture.named_child_count() == 1
        }),
        Language::Rust => pattern.named_child(0).is_some_and(|capture| {
            code.syntax(capture) == Syntax::Identifier
                && code
                    .text(capture)
                    .starts_with(|c: char| c == '_' || c.is_lowercase())
        }),
        Language::Javascript | Language::Typescript | Language::Tsx | Language::Go => false,
    }
}
