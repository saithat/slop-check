//! Per-file measurements, checked through the typed analysis API.

use super::{AnalysisError, Worker};
use crate::{
    language::Language::{self, *},
    testing::{analysis, function},
};

/// (cyclomatic, cognitive) for the first function in `source`.
fn complexity(language: Language, source: &str) -> (u32, u32) {
    let function = function(language, source);
    (function.cyclomatic, function.cognitive)
}

#[test]
fn python_complexity_nesting_and_docstrings() {
    let source = "# header\n\"\"\"module docs\"\"\"\ndef f(x):\n    \"\"\"function docs\"\"\"\n    if x and x > 0:\n        for i in range(x):\n            if i % 2:\n                return 1\n    return 0\n";
    let analysis = analysis(Python, source);
    let function = &analysis.functions[0];
    assert_eq!(function.cyclomatic, 5);
    assert_eq!(function.cognitive, 7);
    assert_eq!(function.max_nesting, 3);
    assert_eq!(function.parameters, 1);
    assert_eq!(function.sloc, 6);
    assert_eq!(analysis.code_lines.count(), 6);
}

#[test]
fn python_docstrings_depend_on_the_first_non_comment_sibling() {
    let source = "# header\n\"module docs\"\n\"module expression\"\nclass C:\n    # comment\n    \"class docs\"\n    \"class expression\"\n    def f(self):\n        # comment\n        \"function docs\"\n        \"function expression\"\n        if self:\n            \"ordinary block expression\"\n        return 1\n";
    let analysis = analysis(Python, source);
    assert_eq!(analysis.code_lines.count(), 8);
    assert_eq!(analysis.functions[0].sloc, 5);
    assert_eq!(analysis.functions[0].cyclomatic, 2);
}

#[test]
fn python_interpolated_and_byte_strings_are_executable_code() {
    let source = "def f(x):\n    f\"{1 if x else 0}\"\n    return x\n\ndef g():\n    b\"not a docstring\"\n    return 0\n";
    let analysis = analysis(Python, source);
    assert_eq!(analysis.functions[0].cyclomatic, 2);
    assert_eq!(analysis.code_lines.count(), 6);
}

#[test]
fn python_string_lines_between_escapes_are_code() {
    let source = "X = \"\"\"first\\n\nsecond\nthird\"\"\"\n";
    assert_eq!(analysis(Python, source).code_lines.count(), 3);
}

#[test]
fn nested_functions_own_their_decisions_and_lines() {
    let source = "def outer(x):\n    def inner(y):\n        if y:\n            return 1\n        return 0\n    if x:\n        return inner(x)\n    return 0\n";
    for function in analysis(Python, source).functions {
        assert_eq!(function.cyclomatic, 2, "{}", function.name);
        assert_eq!(function.max_nesting, 1, "{}", function.name);
        assert_eq!(function.sloc, 4, "{}", function.name);
    }
}

#[test]
fn every_language_has_function_metrics() {
    for (language, source) in [
        (
            Python,
            "def f(x):\n    if x:\n        return 1\n    return 0\n",
        ),
        (Javascript, "const f = x => { if (x) return 1; return 0; };"),
        (
            Typescript,
            "function f(x: boolean): number { if (x) return 1; return 0; }",
        ),
        (
            Tsx,
            "const F = ({x}: {x:boolean}) => { if (x) return <b/>; return <i/>; };",
        ),
        (Rust, "fn f(x: bool) -> i32 { if x { return 1; } 0 }"),
        (
            Go,
            "package p\nfunc f(x bool) int { if x { return 1 }; return 0 }",
        ),
    ] {
        let functions = analysis(language, source).functions;
        assert_eq!(functions.len(), 1, "{language}");
        assert_eq!(functions[0].cyclomatic, 2, "{language}");
    }
}

#[test]
fn logical_operators_count_in_every_language() {
    for (language, source) in [
        (
            Python,
            "def f(a, b, c):\n    if a and b or c:\n        return 1\n    return 0\n",
        ),
        (
            Javascript,
            "function f(a,b,c) { if (a && b || c) return 1; return 0; }",
        ),
        (
            Typescript,
            "function f(a:boolean,b:boolean,c:boolean) { if (a && b || c) return 1; return 0; }",
        ),
        (
            Rust,
            "fn f(a: bool, b: bool, c: bool) -> i32 { if a && b || c { 1 } else { 0 } }",
        ),
        (
            Go,
            "package p\nfunc f(a,b,c bool) int { if a && b || c { return 1 }; return 0 }",
        ),
    ] {
        assert_eq!(function(language, source).cyclomatic, 4, "{language}");
    }
}

#[test]
fn else_if_chains_have_consistent_cognitive_cost() {
    for (language, source) in [
        (
            Python,
            "def f(x):\n    if x > 1:\n        return 1\n    elif x < 0:\n        return 2\n    else:\n        return 3\n",
        ),
        (
            Javascript,
            "function f(x) { if (x > 1) return 1; else if (x < 0) return 2; else return 3; }",
        ),
        (
            Rust,
            "fn f(x: i32) -> i32 { if x > 1 { 1 } else if x < 0 { 2 } else { 3 } }",
        ),
        (
            Go,
            "package p\nfunc f(x int) int { if x > 1 { return 1 } else if x < 0 { return 2 } else { return 3 } }",
        ),
    ] {
        assert_eq!(complexity(language, source), (3, 3), "{language}");
    }
}

#[test]
fn conditional_expressions_cost_one_cognitive_point() {
    // Python's `else` keyword belongs to the conditional expression, not an else clause.
    assert_eq!(
        complexity(Python, "def f(x):\n    return 1 if x else 0\n"),
        (2, 1)
    );
    assert_eq!(
        complexity(Javascript, "function f(x) { return x ? 1 : 0; }"),
        (2, 1)
    );
}

#[test]
fn closures_and_lambdas_do_not_add_decisions_to_parent() {
    for (language, source) in [
        (Python, "def f():\n    return lambda x: 1 if x else 0\n"),
        (Javascript, "function f() { return x => x ? 1 : 0; }"),
        (
            Rust,
            "fn f() { let g = |x: bool| { if x { 1 } else { 0 } }; }",
        ),
        (
            Go,
            "package p\nfunc f() { g := func(x bool) int { if x { return 1 }; return 0 }; _ = g }",
        ),
    ] {
        let functions = analysis(language, source).functions;
        assert_eq!(functions.len(), 2, "{language}");
        assert_eq!(functions[0].cyclomatic, 1, "{language}");
        assert_eq!(functions[1].cyclomatic, 2, "{language}");
        assert!(functions[1].closure, "{language}");
    }
}

#[test]
fn irrefutable_match_captures_do_not_add_branches() {
    for (language, source) in [
        (
            Python,
            "def f(x):\n    match x:\n        case 1:\n            return 1\n        case other:\n            return 0\n",
        ),
        (
            Rust,
            "fn f(x: i32) -> i32 { match x { 1 => 1, other => 0 } }\n",
        ),
    ] {
        assert_eq!(complexity(language, source), (2, 1), "{language}");
    }
}

#[test]
fn guarded_irrefutable_match_arms_are_decisions() {
    let source = "def f(x, enabled):\n    match x:\n        case _ if enabled:\n            return 1\n        case _:\n            return 2\n";
    assert_eq!(complexity(Python, source), (2, 1));
    let source = "fn f(x: u8, enabled: bool) -> u8 { match x { value if enabled => 1, _ => 2 } }";
    assert_eq!(complexity(Rust, source), (2, 1));
}

#[test]
fn rust_constant_and_variant_arms_are_decisions() {
    for (source, expected) in [
        (
            "const RED: u8 = 0;\nconst GREEN: u8 = 1;\nfn f(c: u8) -> u8 { match c { RED => 10, GREEN => 20, _ => 0 } }",
            3,
        ),
        (
            "use std::cmp::Ordering::*;\nfn f(o: std::cmp::Ordering) -> u8 { match o { Less => 1, Equal => 2, Greater => 3 } }",
            4,
        ),
        (
            "fn f(x: Option<u8>) -> u8 { match x { Some(v) => v, None => 0 } }",
            3,
        ),
    ] {
        assert_eq!(complexity(Rust, source).0, expected, "{source}");
    }
}

#[test]
fn rust_propagation_does_not_inflate_complexity() {
    let source = "fn f() -> Result<(), ()> {\n    first()?;\n    second()?;\n    Ok(())\n}\n";
    assert_eq!(complexity(Rust, source), (1, 0));
}

#[test]
fn parameter_counts_handle_go_groups_and_python_separators() {
    for (language, source, expected) in [
        (
            Go,
            "package p\nfunc f(a, b int, values ...int) int { return a + b }",
            3,
        ),
        (Python, "def f(a, /, b, *, c):\n    return a + b + c\n", 3),
        (Javascript, "const f = x => x + 1;", 1),
        (Rust, "impl T { fn f(&self, a: i32) {} }", 2),
        (
            Rust,
            "fn f(#[allow(unused)] a: u8, #[cfg(test)] b: u8) {}",
            2,
        ),
    ] {
        assert_eq!(
            function(language, source).parameters,
            expected,
            "{language}"
        );
    }
}

#[test]
fn closures_take_the_name_that_holds_them() {
    let names = |language, source| -> Vec<String> {
        analysis(language, source)
            .functions
            .into_iter()
            .map(|function| function.name)
            .collect()
    };
    assert_eq!(
        names(
            Javascript,
            "const f = () => 1;\nobj.g = function () {};\nrun(() => 2);\n"
        ),
        ["f", "obj.g", "<anonymous@3:5>"]
    );
    assert_eq!(names(Python, "sorted(xs, key=lambda x: x)\n"), ["key"]);
    // An operand of `||` or `or` is not held by the name on the left.
    assert_eq!(
        names(Javascript, "const h = options.onError || function () {};\n"),
        ["<anonymous@1:30>"]
    );
    assert_eq!(
        names(Python, "fallback = cb or lambda: 0\n"),
        ["<anonymous@1:18>"]
    );
}

#[test]
fn deeply_nested_source_does_not_overflow_the_analyzer_stack() {
    let expression = format!("{}x{}", "(".repeat(4_000), ")".repeat(4_000));
    let nested = function(
        Javascript,
        &format!("function f(x) {{ return {expression}; }}"),
    );
    assert_eq!(nested.cyclomatic, 1);

    let chain = vec!["x"; 10_000].join(" && ");
    let chained = function(Javascript, &format!("function g(x) {{ return {chain}; }}"));
    assert_eq!((chained.cyclomatic, chained.cognitive), (10_000, 1));
}

#[test]
fn syntax_errors_name_their_first_line() {
    let error = Worker::default()
        .analyze("def ok():\n    return 1\n\ndef broken(\n", Python)
        .unwrap_err();
    assert!(
        matches!(error, AnalysisError::Syntax { line: 4 }),
        "{error:?}"
    );
}

#[test]
fn invalid_utf8_names_its_line() {
    let bytes = [b"x = 1\ny = '".as_slice(), &[0xff], b"'\n"].concat();
    let error = std::str::from_utf8(&bytes).unwrap_err();
    let error = AnalysisError::not_utf8(&bytes, error);
    assert!(
        matches!(error, AnalysisError::NotUtf8 { line: 2 }),
        "{error:?}"
    );
}
