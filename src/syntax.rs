//! The grammar boundary: every tree-sitter node kind and field name the analyzer uses.
//!
//! Analysis code matches on [`Syntax`] and [`Field`] rather than on grammar strings. Each
//! language's table below is the only place its node-kind names appear, and the tests at
//! the end of this file check every name against the bundled grammars. A node renamed by a
//! grammar upgrade therefore fails a test instead of silently changing a metric.

use crate::language::Language;
use std::sync::OnceLock;
use tree_sitter::Node;

/// The analyzer's language-neutral role for a node kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    /// Every node kind the analyzer does not treat specially.
    Other,

    // Structure.
    Comment,
    /// A Python module, which can have a docstring.
    Module,
    /// A Python class, which can have a docstring.
    Class,
    Block,
    /// Go's wrapper around the statements of a block or a case.
    StatementList,
    Function,
    /// An anonymous function: a lambda, closure, arrow, or function expression.
    Closure,
    /// A function or method declaration without a body.
    FunctionSignature,
    /// Python's bare `/` and `*` parameter separators.
    ParameterSeparator,
    /// A TypeScript parameter; TypeScript also uses it for labeled tuple elements.
    NamedParameter,

    // Names and values.
    Identifier,
    /// JavaScript `{ a }` shorthand, which names a local and a property at once.
    ShorthandIdentifier,
    /// A name reached through another name: `a.b` or `a::b`.
    QualifiedName,
    /// Python `name=value` in a call.
    KeywordArgument,
    DottedName,
    Parenthesized,
    String,
    /// Adjacent Python string literals.
    ConcatenatedString,
    Integer,
    /// Any other literal: floats, characters, booleans, and null values.
    Literal,

    // Control flow.
    If,
    /// Python `elif`.
    Elif,
    ElseClause,
    /// The `else` keyword token itself.
    ElseKeyword,
    Loop,
    /// A comprehension `if` filter or a Python case guard.
    Filter,
    Catch,
    /// A conditional expression: `a if c else b` or `c ? a : b`.
    Conditional,
    /// A switch or match, which holds cases or arms.
    Switch,
    /// A switch case that tests a value.
    Case,
    /// A Rust match arm or a Python match case, either of which may catch everything.
    MatchArm,
    /// An operator with two operands. Its operator token tells logical and equality
    /// operators apart.
    Binary,
    /// A Python comparison, possibly chained: `a == b` or `a < b < c`.
    Comparison,

    // Statements.
    Assignment,
    ExpressionStatement,
    /// Python `pass`.
    Pass,
    /// A `return`, `throw`, `raise`, `break`, or `continue` statement.
    Jump,
    /// A Rust `return`, `break`, or `continue` expression.
    JumpExpression,
    Labeled,
    /// A Rust `#[...]` or `#![...]` attribute.
    Attribute,

    // Types and member access.
    /// A wrapper around a type: TypeScript `: T` or Python `-> T`.
    TypeAnnotation,
    TupleType,
    /// A Python subscripted type such as `tuple[int, str]`.
    GenericType,
    /// A Rust field access, including tuple positions such as `.0`.
    FieldAccess,
    IndexAccess,
    MacroInvocation,

    // Imports.
    Import,
    ImportFrom,
    Export,
    Call,
}

impl Syntax {
    pub fn is_function_like(self) -> bool {
        matches!(self, Self::Function | Self::Closure)
    }

    pub fn is_literal(self) -> bool {
        matches!(
            self,
            Self::String | Self::ConcatenatedString | Self::Integer | Self::Literal
        )
    }

    /// Keyword roles name anonymous tokens; every other role names a named node.
    fn is_keyword(self) -> bool {
        matches!(self, Self::ElseKeyword)
    }
}

/// A tree-sitter field name the analyzer reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Alternative,
    Arguments,
    Attribute,
    Body,
    Condition,
    /// `field`: the member of a Rust field access or a Go selector.
    Member,
    Function,
    Guard,
    Initializer,
    Key,
    Left,
    Macro,
    ModuleName,
    Name,
    Operator,
    Operators,
    Parameter,
    Parameters,
    Pattern,
    Property,
    ReturnType,
    Right,
    Source,
    Trait,
    Value,
}

impl Field {
    #[cfg(test)]
    const ALL: [Self; 25] = [
        Self::Alternative,
        Self::Arguments,
        Self::Attribute,
        Self::Body,
        Self::Condition,
        Self::Member,
        Self::Function,
        Self::Guard,
        Self::Initializer,
        Self::Key,
        Self::Left,
        Self::Macro,
        Self::ModuleName,
        Self::Name,
        Self::Operator,
        Self::Operators,
        Self::Parameter,
        Self::Parameters,
        Self::Pattern,
        Self::Property,
        Self::ReturnType,
        Self::Right,
        Self::Source,
        Self::Trait,
        Self::Value,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Alternative => "alternative",
            Self::Arguments => "arguments",
            Self::Attribute => "attribute",
            Self::Body => "body",
            Self::Condition => "condition",
            Self::Member => "field",
            Self::Function => "function",
            Self::Guard => "guard",
            Self::Initializer => "initializer",
            Self::Key => "key",
            Self::Left => "left",
            Self::Macro => "macro",
            Self::ModuleName => "module_name",
            Self::Name => "name",
            Self::Operator => "operator",
            Self::Operators => "operators",
            Self::Parameter => "parameter",
            Self::Parameters => "parameters",
            Self::Pattern => "pattern",
            Self::Property => "property",
            Self::ReturnType => "return_type",
            Self::Right => "right",
            Self::Source => "source",
            Self::Trait => "trait",
            Self::Value => "value",
        }
    }
}

/// Field access by [`Field`] instead of by string.
pub trait NodeExt<'tree> {
    fn field(&self, field: Field) -> Option<Node<'tree>>;
}

impl<'tree> NodeExt<'tree> for Node<'tree> {
    fn field(&self, field: Field) -> Option<Node<'tree>> {
        self.child_by_field_name(field.name())
    }
}

/// A language's node kinds classified by role, indexed by tree-sitter kind id.
pub struct Grammar {
    roles: Box<[Syntax]>,
}

impl Grammar {
    pub fn of(language: Language) -> &'static Self {
        static GRAMMARS: [OnceLock<Grammar>; Language::ALL.len()] =
            [const { OnceLock::new() }; Language::ALL.len()];
        GRAMMARS[language as usize].get_or_init(|| Self::build(language))
    }

    pub fn syntax(&self, node: Node<'_>) -> Syntax {
        self.roles
            .get(usize::from(node.kind_id()))
            .copied()
            .unwrap_or(Syntax::Other)
    }

    fn build(language: Language) -> Self {
        // Classify every symbol rather than looking names up one at a time: grammars
        // alias rules, so one visible kind name can have several ids.
        let grammar = language.tree_sitter();
        let roles = (0..u16::MAX)
            .take(grammar.node_kind_count())
            .map(|id| {
                grammar.node_kind_for_id(id).map_or(Syntax::Other, |kind| {
                    role(language, kind, grammar.node_kind_is_named(id))
                })
            })
            .collect();
        Self { roles }
    }
}

fn role(language: Language, kind: &str, named: bool) -> Syntax {
    entries(language)
        .find(|(name, syntax)| *name == kind && syntax.is_keyword() != named)
        .map_or(Syntax::Other, |&(_, syntax)| syntax)
}

fn entries(language: Language) -> impl Iterator<Item = &'static (&'static str, Syntax)> {
    let tables: &[KindTable] = match language {
        Language::Python => &[PYTHON],
        Language::Javascript => &[JAVASCRIPT],
        Language::Typescript | Language::Tsx => &[JAVASCRIPT, TYPESCRIPT],
        Language::Rust => &[RUST],
        Language::Go => &[GO],
    };
    tables.iter().flat_map(|table| table.iter())
}

type KindTable = &'static [(&'static str, Syntax)];

const PYTHON: KindTable = &[
    ("comment", Syntax::Comment),
    ("module", Syntax::Module),
    ("class_definition", Syntax::Class),
    ("block", Syntax::Block),
    ("function_definition", Syntax::Function),
    ("lambda", Syntax::Closure),
    ("positional_separator", Syntax::ParameterSeparator),
    ("keyword_separator", Syntax::ParameterSeparator),
    ("identifier", Syntax::Identifier),
    ("attribute", Syntax::QualifiedName),
    ("keyword_argument", Syntax::KeywordArgument),
    ("dotted_name", Syntax::DottedName),
    ("parenthesized_expression", Syntax::Parenthesized),
    ("string", Syntax::String),
    ("concatenated_string", Syntax::ConcatenatedString),
    ("integer", Syntax::Integer),
    ("float", Syntax::Literal),
    ("true", Syntax::Literal),
    ("false", Syntax::Literal),
    ("none", Syntax::Literal),
    ("if_statement", Syntax::If),
    ("elif_clause", Syntax::Elif),
    ("else_clause", Syntax::ElseClause),
    ("else", Syntax::ElseKeyword),
    ("for_statement", Syntax::Loop),
    ("while_statement", Syntax::Loop),
    ("for_in_clause", Syntax::Loop),
    ("if_clause", Syntax::Filter),
    ("except_clause", Syntax::Catch),
    ("conditional_expression", Syntax::Conditional),
    ("match_statement", Syntax::Switch),
    ("case_clause", Syntax::MatchArm),
    ("boolean_operator", Syntax::Binary),
    ("comparison_operator", Syntax::Comparison),
    ("assignment", Syntax::Assignment),
    ("expression_statement", Syntax::ExpressionStatement),
    ("pass_statement", Syntax::Pass),
    ("return_statement", Syntax::Jump),
    ("raise_statement", Syntax::Jump),
    ("break_statement", Syntax::Jump),
    ("continue_statement", Syntax::Jump),
    ("type", Syntax::TypeAnnotation),
    ("generic_type", Syntax::GenericType),
    ("import_statement", Syntax::Import),
    ("import_from_statement", Syntax::ImportFrom),
];

const JAVASCRIPT: KindTable = &[
    ("comment", Syntax::Comment),
    ("hash_bang_line", Syntax::Comment),
    ("statement_block", Syntax::Block),
    ("function_declaration", Syntax::Function),
    ("generator_function_declaration", Syntax::Function),
    ("method_definition", Syntax::Function),
    ("function_expression", Syntax::Closure),
    ("generator_function", Syntax::Closure),
    ("arrow_function", Syntax::Closure),
    ("identifier", Syntax::Identifier),
    ("shorthand_property_identifier", Syntax::ShorthandIdentifier),
    (
        "shorthand_property_identifier_pattern",
        Syntax::ShorthandIdentifier,
    ),
    ("member_expression", Syntax::QualifiedName),
    ("parenthesized_expression", Syntax::Parenthesized),
    ("string", Syntax::String),
    ("number", Syntax::Literal),
    ("true", Syntax::Literal),
    ("false", Syntax::Literal),
    ("null", Syntax::Literal),
    ("undefined", Syntax::Literal),
    ("if_statement", Syntax::If),
    ("else_clause", Syntax::ElseClause),
    ("else", Syntax::ElseKeyword),
    ("for_statement", Syntax::Loop),
    ("for_in_statement", Syntax::Loop),
    ("while_statement", Syntax::Loop),
    ("do_statement", Syntax::Loop),
    ("catch_clause", Syntax::Catch),
    ("ternary_expression", Syntax::Conditional),
    ("switch_statement", Syntax::Switch),
    ("switch_case", Syntax::Case),
    ("binary_expression", Syntax::Binary),
    ("assignment_expression", Syntax::Assignment),
    ("return_statement", Syntax::Jump),
    ("throw_statement", Syntax::Jump),
    ("break_statement", Syntax::Jump),
    ("continue_statement", Syntax::Jump),
    ("labeled_statement", Syntax::Labeled),
    ("import_statement", Syntax::Import),
    ("export_statement", Syntax::Export),
    ("call_expression", Syntax::Call),
];

/// TypeScript and TSX kinds on top of [`JAVASCRIPT`].
const TYPESCRIPT: KindTable = &[
    ("function_signature", Syntax::FunctionSignature),
    ("method_signature", Syntax::FunctionSignature),
    ("abstract_method_signature", Syntax::FunctionSignature),
    ("required_parameter", Syntax::NamedParameter),
    ("optional_parameter", Syntax::NamedParameter),
    ("type_annotation", Syntax::TypeAnnotation),
    ("tuple_type", Syntax::TupleType),
];

const RUST: KindTable = &[
    ("line_comment", Syntax::Comment),
    ("block_comment", Syntax::Comment),
    ("shebang", Syntax::Comment),
    ("block", Syntax::Block),
    ("function_item", Syntax::Function),
    ("closure_expression", Syntax::Closure),
    ("function_signature_item", Syntax::FunctionSignature),
    ("identifier", Syntax::Identifier),
    ("scoped_identifier", Syntax::QualifiedName),
    ("parenthesized_expression", Syntax::Parenthesized),
    ("string_literal", Syntax::String),
    ("raw_string_literal", Syntax::String),
    ("integer_literal", Syntax::Integer),
    ("float_literal", Syntax::Literal),
    ("boolean_literal", Syntax::Literal),
    ("char_literal", Syntax::Literal),
    ("if_expression", Syntax::If),
    ("else_clause", Syntax::ElseClause),
    ("else", Syntax::ElseKeyword),
    ("for_expression", Syntax::Loop),
    ("while_expression", Syntax::Loop),
    ("loop_expression", Syntax::Loop),
    ("match_expression", Syntax::Switch),
    ("match_arm", Syntax::MatchArm),
    ("binary_expression", Syntax::Binary),
    ("assignment_expression", Syntax::Assignment),
    ("expression_statement", Syntax::ExpressionStatement),
    ("return_expression", Syntax::JumpExpression),
    ("break_expression", Syntax::JumpExpression),
    ("continue_expression", Syntax::JumpExpression),
    ("attribute_item", Syntax::Attribute),
    ("inner_attribute_item", Syntax::Attribute),
    ("tuple_type", Syntax::TupleType),
    ("field_expression", Syntax::FieldAccess),
    ("index_expression", Syntax::IndexAccess),
    ("macro_invocation", Syntax::MacroInvocation),
];

const GO: KindTable = &[
    ("comment", Syntax::Comment),
    ("block", Syntax::Block),
    ("statement_list", Syntax::StatementList),
    ("function_declaration", Syntax::Function),
    ("method_declaration", Syntax::Function),
    ("func_literal", Syntax::Closure),
    ("identifier", Syntax::Identifier),
    ("selector_expression", Syntax::QualifiedName),
    ("parenthesized_expression", Syntax::Parenthesized),
    ("interpreted_string_literal", Syntax::String),
    ("raw_string_literal", Syntax::String),
    ("int_literal", Syntax::Integer),
    ("float_literal", Syntax::Literal),
    ("rune_literal", Syntax::Literal),
    ("true", Syntax::Literal),
    ("false", Syntax::Literal),
    ("nil", Syntax::Literal),
    ("if_statement", Syntax::If),
    ("else", Syntax::ElseKeyword),
    ("for_statement", Syntax::Loop),
    ("expression_switch_statement", Syntax::Switch),
    ("type_switch_statement", Syntax::Switch),
    ("expression_case", Syntax::Case),
    ("type_case", Syntax::Case),
    ("binary_expression", Syntax::Binary),
    ("return_statement", Syntax::Jump),
    ("break_statement", Syntax::Jump),
    ("continue_statement", Syntax::Jump),
    ("labeled_statement", Syntax::Labeled),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn symbols(language: Language) -> HashSet<(&'static str, bool)> {
        let grammar = language.tree_sitter();
        (0..u16::MAX)
            .take(grammar.node_kind_count())
            .filter_map(|id| {
                Some((
                    grammar.node_kind_for_id(id)?,
                    grammar.node_kind_is_named(id),
                ))
            })
            .collect()
    }

    #[test]
    fn every_table_entry_names_a_node_in_its_grammar() {
        for language in Language::ALL {
            let symbols = symbols(language);
            for &(kind, syntax) in entries(language) {
                let named = !syntax.is_keyword();
                assert!(
                    symbols.contains(&(kind, named)),
                    "{language} has no {} node `{kind}`",
                    if named { "named" } else { "keyword" }
                );
            }
        }
    }

    #[test]
    fn each_kind_has_one_role_per_language() {
        for language in Language::ALL {
            let mut seen = HashSet::new();
            for (kind, _) in entries(language) {
                assert!(seen.insert(kind), "{language} lists `{kind}` twice");
            }
        }
    }

    #[test]
    fn every_field_exists_in_a_bundled_grammar() {
        for field in Field::ALL {
            assert!(
                Language::ALL.iter().any(|language| language
                    .tree_sitter()
                    .field_id_for_name(field.name())
                    .is_some()),
                "no bundled grammar has a `{}` field",
                field.name()
            );
        }
    }
}
