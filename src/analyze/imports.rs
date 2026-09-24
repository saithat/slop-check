//! Statically visible imports, recorded for the dependency graph.

use super::Code;
use crate::{
    language::Language,
    model::{Import, PythonImport},
    syntax::{Field, NodeExt, Syntax},
};
use tree_sitter::Node;

pub(super) fn inspect(node: Node<'_>, syntax: Syntax, code: Code<'_>, out: &mut Vec<Import>) {
    match code.language {
        Language::Python => python(node, syntax, code, out),
        Language::Javascript | Language::Typescript | Language::Tsx => {
            javascript(node, syntax, code, out);
        }
        Language::Rust | Language::Go => {}
    }
}

/// Static `import` and `export ... from`, plus `require()` and `import()` with a literal
/// specifier.
fn javascript(node: Node<'_>, syntax: Syntax, code: Code<'_>, out: &mut Vec<Import>) {
    let specifier = match syntax {
        Syntax::Import | Syntax::Export => node.field(Field::Source),
        Syntax::Call => node
            .field(Field::Function)
            .filter(|callee| matches!(code.text(*callee), "require" | "import"))
            .and_then(|_| node.field(Field::Arguments)?.named_child(0))
            .filter(|argument| code.syntax(*argument) == Syntax::String),
        _ => None,
    };
    if let Some(specifier) = specifier {
        let text = code.text(specifier).trim_matches(['\'', '"']);
        out.push(Import::Specifier(text.to_owned()));
    }
}

fn python(node: Node<'_>, syntax: Syntax, code: Code<'_>, out: &mut Vec<Import>) {
    let mut cursor = node.walk();
    match syntax {
        // `import a.b, c as d`
        Syntax::Import => {
            for imported in node.named_children(&mut cursor) {
                if code.syntax(imported) != Syntax::Comment {
                    let module = imported.field(Field::Name).unwrap_or(imported);
                    out.push(Import::Python(PythonImport::parse(
                        code.text(module),
                        vec![],
                    )));
                }
            }
        }
        // `from ..a import b, c as d`
        Syntax::ImportFrom => {
            let Some(module) = node.field(Field::ModuleName) else {
                return;
            };
            let names = node
                .named_children(&mut cursor)
                .filter(|child| child.id() != module.id() && code.syntax(*child) != Syntax::Comment)
                .map(|child| code.text(child.field(Field::Name).unwrap_or(child)))
                .filter(|name| *name != "*")
                .map(str::to_owned)
                .collect();
            out.push(Import::Python(PythonImport::parse(
                code.text(module),
                names,
            )));
        }
        _ => {}
    }
}
