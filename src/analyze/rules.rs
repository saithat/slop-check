//! Structural rules: the versioned `structural/3` rule pack.
//!
//! Each rule is a purely syntactic check. A finding marks code for review; it is not a
//! proof of a defect.

use super::{Code, Context, complexity::is_else_if};
use crate::{
    language::Language,
    lines::LineSpan,
    model::{Finding, POLICY, Rule},
    syntax::{Field, NodeExt, Syntax},
};
use std::iter;
use tree_sitter::Node;

pub(super) fn inspect(
    node: Node<'_>,
    syntax: Syntax,
    context: &Context<'_>,
    code: Code<'_>,
    out: &mut Vec<Finding>,
) {
    let finding = match syntax {
        Syntax::Block | Syntax::StatementList => {
            unreachable_statements(node, code, out);
            None
        }
        Syntax::Assignment => self_assignment(node, context, code),
        Syntax::Catch => empty_catch(node, code),
        Syntax::Function | Syntax::Closure | Syntax::FunctionSignature => {
            wide_tuple_return(node, context, code)
        }
        Syntax::FieldAccess => chained_tuple_index(node, code),
        Syntax::MatchArm => unreachable_wildcard_arm(node, code),
        Syntax::If => equality_dispatch_chain(node, context, code),
        _ => None,
    };
    out.extend(finding.filter(|finding| finding.rule.applies_to(code.language)));
}

/// A finding at `node`. The traversal fills in the enclosing function.
fn finding(rule: Rule, node: Node<'_>) -> Finding {
    Finding {
        rule,
        lines: LineSpan::of(node),
        function: None,
    }
}

fn named_children<'tree>(node: Node<'tree>, code: Code<'_>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| code.syntax(*child) != Syntax::Comment)
        .collect()
}

/// `x = x`: a plain assignment of an identifier to itself. In a Python class body this
/// copies an enclosing-scope name into a class attribute, so it is not reported there.
fn self_assignment(node: Node<'_>, context: &Context<'_>, code: Code<'_>) -> Option<Finding> {
    let (left, right) = (node.field(Field::Left)?, node.field(Field::Right)?);
    let plain = node
        .field(Field::Operator)
        .is_none_or(|operator| code.text(operator) == "=");
    let same_name = code.syntax(left) == Syntax::Identifier
        && code.syntax(right) == Syntax::Identifier
        && code.text(left) == code.text(right);
    let class_attribute = code.language == Language::Python
        && context
            .grandparent
            .and_then(|block| block.node.parent())
            .is_some_and(|owner| code.syntax(owner) == Syntax::Class);
    (plain && same_name && !class_attribute).then(|| finding(Rule::SelfAssignment, node))
}

/// A handler whose body holds nothing but comments or `pass`.
fn empty_catch(node: Node<'_>, code: Code<'_>) -> Option<Finding> {
    let body = node.field(Field::Body).or_else(|| {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|child| code.syntax(*child) == Syntax::Block)
    })?;
    let mut cursor = body.walk();
    let empty = body
        .named_children(&mut cursor)
        .all(|child| matches!(code.syntax(child), Syntax::Comment | Syntax::Pass));
    empty.then(|| finding(Rule::EmptyCatch, node))
}

/// Sibling statements after an unconditional `return`, `throw`, `raise`, `break`, or
/// `continue`. Function declarations are exempt because they may be hoisted, except in
/// Python, where a `def` is an ordinary statement.
fn unreachable_statements(block: Node<'_>, code: Code<'_>, out: &mut Vec<Finding>) {
    if !Rule::UnreachableStatement.applies_to(code.language) {
        return;
    }
    let mut terminated = false;
    let mut conditional = false;
    let mut cursor = block.walk();
    for statement in block.named_children(&mut cursor) {
        let syntax = code.syntax(statement);
        match syntax {
            Syntax::Comment => continue,
            // A jump under `cfg` or `cfg_attr` may be compiled out, so it cannot make the
            // next statement unconditionally unreachable.
            Syntax::Attribute if is_cfg_attribute(statement, code) => {
                conditional = true;
                continue;
            }
            // A label is a jump target, so code after it can be reached.
            Syntax::Labeled => terminated = false,
            _ => {}
        }
        let hoisted = syntax.is_function_like() && code.language != Language::Python;
        if terminated && !hoisted {
            out.push(finding(Rule::UnreachableStatement, statement));
        }
        terminated |= is_jump(statement, code) && !conditional;
        conditional = false;
    }
}

fn is_cfg_attribute(attribute: Node<'_>, code: Code<'_>) -> bool {
    let text = code.text(attribute);
    let inner = text
        .strip_prefix("#[")
        .or_else(|| text.strip_prefix("#!["))
        .unwrap_or(text)
        .trim_start();
    ["cfg", "cfg_attr"].into_iter().any(|name| {
        inner
            .strip_prefix(name)
            .is_some_and(|rest| rest.trim_start().starts_with('('))
    })
}

fn is_jump(mut statement: Node<'_>, code: Code<'_>) -> bool {
    loop {
        match code.syntax(statement) {
            Syntax::Jump => return true,
            // Rust `return x;` is an expression statement.
            Syntax::ExpressionStatement => {
                return statement
                    .named_child(0)
                    .is_some_and(|expression| code.syntax(expression) == Syntax::JumpExpression);
            }
            // A label is a jump target, but its wrapped statement may itself exit.
            Syntax::Labeled => {
                let mut cursor = statement.walk();
                let Some(body) = statement
                    .named_children(&mut cursor)
                    .filter(|child| code.syntax(*child) != Syntax::Comment)
                    .last()
                else {
                    return false;
                };
                statement = body;
            }
            _ => return false,
        }
    }
}

/// A declared return type that is a tuple of several unlabeled values, such as
/// `-> (u32, u32, bool)`. Callers then read results by position; a named type would
/// label each one. TypeScript tuples with labeled elements are exempt, and so are Rust
/// trait implementations, whose signatures the trait declaration already fixes.
fn wide_tuple_return(function: Node<'_>, context: &Context<'_>, code: Code<'_>) -> Option<Finding> {
    // `impl Trait for T { fn ... }`: the function's grandparent is the impl.
    let implements_trait = context
        .grandparent
        .is_some_and(|owner| owner.node.field(Field::Trait).is_some());
    if implements_trait {
        return None;
    }
    let mut return_type = function.field(Field::ReturnType)?;
    if code.syntax(return_type) == Syntax::TypeAnnotation {
        return_type = *named_children(return_type, code).first()?;
    }
    let elements = match code.syntax(return_type) {
        Syntax::TupleType => {
            let elements = named_children(return_type, code);
            let labeled = elements
                .iter()
                .any(|element| code.syntax(*element) == Syntax::NamedParameter);
            if labeled {
                return None;
            }
            elements.len()
        }
        Syntax::GenericType => python_tuple_arity(return_type, code)?,
        _ => return None,
    };
    (elements >= POLICY.min_tuple_return_elements as usize)
        .then(|| finding(Rule::WideTupleReturn, return_type))
}

/// The number of element types in a Python `tuple[...]` or `Tuple[...]` annotation.
fn python_tuple_arity(generic: Node<'_>, code: Code<'_>) -> Option<usize> {
    let [name, parameters] = named_children(generic, code)[..] else {
        return None;
    };
    matches!(code.text(name), "tuple" | "Tuple").then(|| named_children(parameters, code).len())
}

/// A Rust tuple position read through another tuple position, such as `pair.1[0].1` or
/// `nested.0.1`. Each hop discards a name the reader has to reconstruct.
fn chained_tuple_index(access: Node<'_>, code: Code<'_>) -> Option<Finding> {
    if !is_tuple_index(access, code) {
        return None;
    }
    let mut receiver = access.field(Field::Value)?;
    loop {
        receiver = match code.syntax(receiver) {
            Syntax::FieldAccess if is_tuple_index(receiver, code) => {
                return Some(finding(Rule::ChainedTupleIndex, access));
            }
            Syntax::FieldAccess => receiver.field(Field::Value)?,
            Syntax::IndexAccess => receiver.named_child(0)?,
            _ => return None,
        };
    }
}

fn is_tuple_index(access: Node<'_>, code: Code<'_>) -> bool {
    access
        .field(Field::Member)
        .is_some_and(|field| code.syntax(field) == Syntax::Integer)
}

/// A Rust `_ =>` arm that only panics. The matched value's type admits values the match
/// does not handle; a narrower type would let the compiler check exhaustiveness.
fn unreachable_wildcard_arm(arm: Node<'_>, code: Code<'_>) -> Option<Finding> {
    let pattern = arm.field(Field::Pattern)?;
    if pattern.field(Field::Condition).is_some() || code.text(pattern).trim() != "_" {
        return None;
    }
    let mut body = arm.field(Field::Value)?;
    // `_ => { unreachable!() }` or `_ => { panic!(); }`
    while matches!(
        code.syntax(body),
        Syntax::Block | Syntax::ExpressionStatement
    ) {
        let [inner] = named_children(body, code)[..] else {
            return None;
        };
        body = inner;
    }
    if code.syntax(body) != Syntax::MacroInvocation {
        return None;
    }
    let path = body.field(Field::Macro)?;
    let name = path.field(Field::Name).unwrap_or(path);
    matches!(code.text(name), "unreachable" | "panic")
        .then(|| finding(Rule::UnreachableWildcardArm, arm))
}

/// An `if`/`else if` chain whose every condition compares the same expression with
/// `==` or `===` against a literal or qualified constant. A `match` or `switch` states
/// those cases directly. Only the head of a chain is reported.
fn equality_dispatch_chain(
    head: Node<'_>,
    context: &Context<'_>,
    code: Code<'_>,
) -> Option<Finding> {
    if is_else_if(head, context) {
        return None;
    }
    let conditions = chain_conditions(head, code)?;
    if conditions.len() < POLICY.min_dispatch_branches as usize {
        return None;
    }
    let mut subjects: Option<Vec<&str>> = None;
    for condition in conditions {
        let [left, right] = equality_operands(condition, code)?;
        // A candidate subject is any non-literal compared against a constant. Qualified
        // names can be either: `request.method == Method.GET`.
        let found: Vec<&str> = [(left, right), (right, left)]
            .into_iter()
            .filter(|(subject, constant)| {
                is_constant(*constant, code) && !code.syntax(*subject).is_literal()
            })
            .map(|(subject, _)| code.text(subject))
            .collect();
        let shared = match subjects {
            None => found,
            Some(previous) => previous
                .into_iter()
                .filter(|subject| found.contains(subject))
                .collect(),
        };
        if shared.is_empty() {
            return None;
        }
        subjects = Some(shared);
    }
    Some(finding(Rule::EqualityDispatchChain, head))
}

/// The conditions of an `if` and each of its `else if`s.
fn chain_conditions<'tree>(head: Node<'tree>, code: Code<'_>) -> Option<Vec<Node<'tree>>> {
    match code.language {
        Language::Python => elif_conditions(head, code),
        Language::Javascript
        | Language::Typescript
        | Language::Tsx
        | Language::Rust
        | Language::Go => else_if_conditions(head, code),
    }
}

/// Python keeps every `elif` as an alternative of the `if` itself.
fn elif_conditions<'tree>(head: Node<'tree>, code: Code<'_>) -> Option<Vec<Node<'tree>>> {
    let mut cursor = head.walk();
    let elifs = head
        .children_by_field_name(Field::Alternative.name(), &mut cursor)
        .filter(|alternative| code.syntax(*alternative) == Syntax::Elif);
    iter::once(head)
        .chain(elifs)
        .map(|link| link.field(Field::Condition))
        .collect()
}

/// Other languages nest each `else if` inside the previous link. A link that declares a
/// variable first (Go `if x := f(); x == 1`) scopes it to the chain, so such a chain is
/// not reported.
fn else_if_conditions<'tree>(head: Node<'tree>, code: Code<'_>) -> Option<Vec<Node<'tree>>> {
    iter::successors(Some(head), |link| else_if(*link, code))
        .map(|link| match link.field(Field::Initializer) {
            Some(_) => None,
            None => link.field(Field::Condition),
        })
        .collect()
}

/// The `if` in an `else if`: Go makes it the alternative itself, while JavaScript and
/// Rust wrap it in an else clause.
fn else_if<'tree>(node: Node<'tree>, code: Code<'_>) -> Option<Node<'tree>> {
    let alternative = node.field(Field::Alternative)?;
    let candidate = match code.syntax(alternative) {
        Syntax::ElseClause => *named_children(alternative, code).first()?,
        _ => alternative,
    };
    (code.syntax(candidate) == Syntax::If).then_some(candidate)
}

/// The two operands of `a == b` or `a === b`, looking through parentheses.
fn equality_operands<'tree>(condition: Node<'tree>, code: Code<'_>) -> Option<[Node<'tree>; 2]> {
    let mut condition = condition;
    while code.syntax(condition) == Syntax::Parenthesized {
        condition = *named_children(condition, code).first()?;
    }
    match code.syntax(condition) {
        Syntax::Binary => {
            let operator = code.text(condition.field(Field::Operator)?);
            if !matches!(operator, "==" | "===") {
                return None;
            }
            Some([
                condition.field(Field::Left)?,
                condition.field(Field::Right)?,
            ])
        }
        Syntax::Comparison => {
            let mut cursor = condition.walk();
            let operators: Vec<_> = condition
                .children_by_field_name(Field::Operators.name(), &mut cursor)
                .map(|operator| code.text(operator))
                .collect();
            let [left, right] = named_children(condition, code)[..] else {
                return None;
            };
            (operators == ["=="]).then_some([left, right])
        }
        _ => None,
    }
}

/// A literal, or a qualified name with a capitalized segment, like a type, enum member,
/// or constant (`Kind.a`, `Color::RED`, `pkg.Max`). Other member accesses such as `b.x`
/// read object state.
fn is_constant(operand: Node<'_>, code: Code<'_>) -> bool {
    let syntax = code.syntax(operand);
    if syntax.is_literal() {
        return true;
    }
    syntax == Syntax::QualifiedName
        && code
            .text(operand)
            .split(['.', ':'])
            .any(|segment| segment.trim().starts_with(|c: char| c.is_uppercase()))
}

#[cfg(test)]
mod tests {
    use crate::{
        language::Language::{self, *},
        model::Rule,
        testing::analysis,
    };

    /// The rule and start line of each finding.
    fn findings(language: Language, source: &str) -> Vec<(Rule, u32)> {
        analysis(language, source)
            .findings
            .iter()
            .map(|finding| (finding.rule, finding.lines.start()))
            .collect()
    }

    fn rules(language: Language, source: &str) -> Vec<Rule> {
        findings(language, source)
            .into_iter()
            .map(|(rule, _)| rule)
            .collect()
    }

    #[test]
    fn python_unreachable_self_assignment_and_empty_catch() {
        let source = "def f(x):\n    x = x\n    try:\n        x()\n    except Exception:\n        pass\n    return x\n    print(x)\n    print(2)\n";
        assert_eq!(
            findings(Python, source),
            [
                (Rule::SelfAssignment, 2),
                (Rule::EmptyCatch, 5),
                (Rule::UnreachableStatement, 8),
                (Rule::UnreachableStatement, 9),
            ]
        );
    }

    #[test]
    fn python_class_attribute_copies_are_not_self_assignments() {
        let source =
            "x = 1\nclass A:\n    x = x\n    def f(self, y):\n        y = y\n        return y\n";
        assert_eq!(findings(Python, source), [(Rule::SelfAssignment, 5)]);
    }

    #[test]
    fn python_definitions_after_a_return_are_unreachable() {
        let source = "def f():\n    return 1\n    def g():\n        pass\n    @dec\n    def h():\n        pass\n";
        assert_eq!(
            findings(Python, source),
            [
                (Rule::UnreachableStatement, 3),
                (Rule::UnreachableStatement, 5)
            ]
        );
        let source = "function f() { return g(); function g() { return 1; } }\n";
        assert_eq!(rules(Javascript, source), []);
    }

    #[test]
    fn rust_self_assignment_is_reported() {
        let source = "fn f(mut x: i32) -> i32 { x = x; x }\n";
        assert_eq!(rules(Rust, source), [Rule::SelfAssignment]);
    }

    #[test]
    fn rust_attributed_returns_are_not_assumed_unconditional() {
        let source = "fn f() -> i32 {\n    #[cfg(not(feature = \"fast\"))]\n    return 0;\n    #[cfg(feature = \"fast\")]\n    1\n}\nfn g() { return; println!(\"unreachable\"); }\nfn h() { #[allow(unreachable_code)] return; println!(\"still unreachable\"); }\n";
        assert_eq!(
            findings(Rust, source),
            [
                (Rule::UnreachableStatement, 7),
                (Rule::UnreachableStatement, 8)
            ]
        );
    }

    #[test]
    fn rust_cfg_attributes_allow_space_before_the_argument_list() {
        let source = "fn f() {\n    #[cfg (feature = \"fast\")]\n    return;\n    println!(\"reachable without fast\");\n}\nfn g() {\n    #[cfg_attr (feature = \"fast\", cfg(other))]\n    return;\n    println!(\"also reachable\");\n}\n";
        assert_eq!(findings(Rust, source), []);
    }

    #[test]
    fn go_statements_after_a_jump_are_unreachable() {
        // Go wraps block and case bodies in statement lists.
        let source = "package p\nfunc f(x int) int {\n\treturn 1\n\tx = 2\n\tswitch x {\n\tcase 1:\n\t\treturn 2\n\t\tx = 3\n\t}\n\treturn x\n}\n";
        assert_eq!(
            findings(Go, source),
            [
                (Rule::UnreachableStatement, 4),
                (Rule::UnreachableStatement, 5),
                (Rule::UnreachableStatement, 8),
                (Rule::UnreachableStatement, 10),
            ]
        );
    }

    #[test]
    fn go_labels_end_the_unreachable_region() {
        let source = "package p\nfunc f() {\n\tgoto done\n\tbreak\ndone:\n\treturn\n}\n";
        assert_eq!(findings(Go, source), []);
    }

    #[test]
    fn go_labeled_returns_end_the_reachable_region() {
        let source = "package p\nfunc f() {\n    goto done\ndone: return\n    println(1)\n}\n";
        assert_eq!(findings(Go, source), [(Rule::UnreachableStatement, 5)]);
    }

    #[test]
    fn wide_tuple_returns_are_reported() {
        assert_eq!(
            findings(
                Rust,
                "fn f() -> (u32, u32, bool) { (1, 2, true) }\ntrait T {\n    fn g(&self) -> (i32, i32, i32);\n}\nimpl T for U {\n    fn g(&self) -> (i32, i32, i32) { (1, 2, 3) }\n}\n"
            ),
            // The trait declaration is reported; its implementation cannot differ.
            [(Rule::WideTupleReturn, 1), (Rule::WideTupleReturn, 3)]
        );
        assert_eq!(
            findings(
                Typescript,
                "function f(): [number, string, boolean] { return [1, '', true]; }\ninterface I { m(): [number, number, number]; }\n"
            ),
            [(Rule::WideTupleReturn, 1), (Rule::WideTupleReturn, 2)]
        );
        assert_eq!(
            findings(
                Python,
                "def f() -> tuple[int, int, bool]:\n    return 1, 2, True\n"
            ),
            [(Rule::WideTupleReturn, 1)]
        );
    }

    #[test]
    fn narrow_labeled_and_go_multiple_returns_are_not_wide_tuples() {
        assert_eq!(rules(Rust, "fn f() -> (u32, bool) { (1, true) }\n"), []);
        assert_eq!(
            rules(
                Typescript,
                "function f(): [x: number, y: number, z: number] { return [1, 2, 3]; }\n"
            ),
            []
        );
        assert_eq!(
            rules(Python, "def f() -> tuple[int, ...]:\n    return (1,)\n"),
            []
        );
        assert_eq!(
            rules(
                Go,
                "package p\nfunc f() (int, int, error) { return 1, 2, nil }\n"
            ),
            []
        );
    }

    #[test]
    fn chained_tuple_positions_are_reported() {
        let source = "fn f() {\n    let a = groups.1[0].1;\n    let b = nested.0.1;\n    let c = pair.0 + pair.1;\n    let d = self.0.len();\n}\n";
        assert_eq!(
            findings(Rust, source),
            [(Rule::ChainedTupleIndex, 2), (Rule::ChainedTupleIndex, 3)]
        );
    }

    #[test]
    fn wildcard_arms_that_panic_are_reported() {
        let source = "fn f(k: u8) -> u8 {\n    match k {\n        0 => 1,\n        _ => unreachable!(\"k\"),\n    };\n    match k {\n        0 => 1,\n        _ => { std::panic!() }\n    }\n}\n";
        assert_eq!(
            findings(Rust, source),
            [
                (Rule::UnreachableWildcardArm, 4),
                (Rule::UnreachableWildcardArm, 8)
            ]
        );
    }

    #[test]
    fn guarded_binding_and_ordinary_wildcard_arms_are_not_reported() {
        let source = "fn f(k: u8, c: bool) -> u8 {\n    match k {\n        _ if c => panic!(),\n        other => unreachable!(\"{other}\"),\n    };\n    match k { 0 => 1, _ => 0 }\n}\n";
        assert_eq!(rules(Rust, source), []);
    }

    #[test]
    fn equality_dispatch_chains_are_reported_in_every_language() {
        for (language, source) in [
            (
                Python,
                "if x == 'a':\n    pass\nelif x == B.c:\n    pass\nelif 3 == x:\n    pass\n",
            ),
            (
                Python,
                "if request.method == 'GET':\n    pass\nelif request.method == 'POST':\n    pass\nelif request.method == Method.DELETE:\n    pass\n",
            ),
            (
                Javascript,
                "if (this.kind === Kind.A) {} else if (this.kind === Kind.B) {} else if (this.kind === 'c') {}\n",
            ),
            (
                Javascript,
                "if (x === 'a') {} else if (x === 'b') {} else if ((x == 1)) {} else {}\n",
            ),
            (
                Typescript,
                "if (x === Kind.A) {} else if (x === Kind.B) {} else if (x === null) {}\n",
            ),
            (
                Rust,
                "fn f() {\n    if x == Lang::A {} else if x == 2 {} else if x == 'c' {}\n}\n",
            ),
            (
                Go,
                "package p\nfunc f() {\n\tif x == 1 {} else if x == pkg.B {} else if x == \"c\" {}\n}\n",
            ),
        ] {
            let found = rules(language, source);
            assert_eq!(found, [Rule::EqualityDispatchChain], "{language}");
        }
    }

    #[test]
    fn short_mixed_and_variable_chains_are_not_dispatch_chains() {
        for (language, source) in [
            // Two comparisons.
            (Javascript, "if (x === 1) {} else if (x === 2) {}\n"),
            // Different subjects.
            (
                Javascript,
                "if (x === 1) {} else if (y === 2) {} else if (x === 3) {}\n",
            ),
            // Not every condition is an equality test.
            (
                Javascript,
                "if (x === 1) {} else if (x > 2) {} else if (x === 3) {}\n",
            ),
            // Another object's field is state, not a case label.
            (
                Javascript,
                "if (a.x === b.x) {} else if (a.x === c.y) {} else if (a.x === d.z) {}\n",
            ),
            (
                Python,
                "if a.x == b.x:\n    pass\nelif a.x == c.y:\n    pass\nelif a.x == d.z:\n    pass\n",
            ),
            // A variable is not a case label: in Rust it would bind a new name.
            (
                Rust,
                "fn f() {\n    if x == y {} else if x == 2 {} else if x == 3 {}\n}\n",
            ),
            (
                Rust,
                "fn f() {\n    if let Some(v) = x {} else if x == None {} else if x == Some(1) {}\n}\n",
            ),
            // A Go statement before the condition scopes a variable to the chain.
            (
                Go,
                "package p\nfunc f() {\n\tif v := g(); v == 1 {} else if v == 2 {} else if v == 3 {}\n}\n",
            ),
            (
                Python,
                "if x == 1:\n    pass\nelif x in (2, 3):\n    pass\nelif x == 4:\n    pass\n",
            ),
        ] {
            assert_eq!(rules(language, source), [], "{language}: {source}");
        }
    }
}
