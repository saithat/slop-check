//! The single depth-first pass that gathers every per-file fact.

use super::{Code, Context, Parent, complexity, functions, imports, python, rules};
use crate::{
    lines::{CodeLines, LineSpan},
    model::{Analysis, Candidate, Finding, Fingerprint, Function, Import, POLICY},
    syntax::{Field, Syntax},
};
use hashbrown::HashMap;
use std::ops::RangeInclusive;
use tree_sitter::{Node, Tree, TreeCursor};
use xxhash_rust::xxh3::Xxh3;

pub(super) fn analyze(tree: &Tree, code: Code<'_>) -> Analysis {
    let mut traversal = Traversal::new(code);
    // The cursor is an explicit depth-first stack, so deeply nested, machine-generated
    // source cannot overflow the Rust call stack.
    let mut cursor = tree.walk();
    loop {
        let descend = traversal.enter(cursor.node(), cursor.field_name());
        if descend && cursor.goto_first_child() {
            continue;
        }
        if !advance(&mut cursor, &mut traversal) {
            return traversal.finish();
        }
    }
}

/// Exits finished nodes until the cursor reaches a next sibling. Returns `false` once
/// the whole tree has been exited.
fn advance<'tree>(cursor: &mut TreeCursor<'tree>, traversal: &mut Traversal<'tree, '_>) -> bool {
    loop {
        traversal.exit();
        if cursor.goto_next_sibling() {
            return true;
        }
        if !cursor.goto_parent() {
            return false;
        }
    }
}

struct Traversal<'tree, 'source> {
    code: Code<'source>,
    physical_lines: u32,
    /// One flag per 0-based row: whether the row holds code.
    code_rows: Vec<bool>,
    functions: Vec<Function>,
    findings: Vec<Finding>,
    candidates: Vec<Candidate>,
    imports: Vec<Import>,
    /// The functions enclosing the current node, innermost last.
    active: Vec<ActiveFunction<'source>>,
    /// Identifier ordinals for code outside every function.
    file_identifiers: IdentifierScope<'source>,
    /// One frame per entered node, innermost last.
    frames: Vec<Frame<'tree>>,
}

struct ActiveFunction<'source> {
    /// Position in `Traversal::functions`.
    index: usize,
    /// Control-flow nesting depth for cognitive complexity.
    depth: u32,
    /// Lines owned by this function itself. Tracked once it contains a nested function;
    /// until then its SLOC is simply the code lines in its span.
    own_lines: Option<LineCounter>,
    identifiers: IdentifierScope<'source>,
}

struct Frame<'tree> {
    node: Node<'tree>,
    syntax: Syntax,
    /// Fingerprint state; `None` for comments and docstrings, which are excluded from
    /// metrics and fingerprints and whose children are never entered.
    hash: Option<SyntaxHash>,
    tokens: u32,
    opens_function: bool,
    opens_nesting: bool,
    /// Whether a named child other than a comment has been entered.
    seen_statement: bool,
}

impl<'tree> Frame<'tree> {
    /// A comment or docstring: no metrics, no fingerprint, and no children visited.
    fn skipped(node: Node<'tree>, syntax: Syntax) -> Self {
        Self {
            node,
            syntax,
            hash: None,
            tokens: 0,
            opens_function: false,
            opens_nesting: false,
            seen_statement: false,
        }
    }
}

impl<'tree, 'source> Traversal<'tree, 'source> {
    fn new(code: Code<'source>) -> Self {
        let physical_lines = code.source.split_inclusive('\n').count();
        Self {
            code,
            physical_lines: u32::try_from(physical_lines).unwrap_or(u32::MAX),
            code_rows: vec![false; physical_lines],
            functions: vec![],
            findings: vec![],
            candidates: vec![],
            imports: vec![],
            active: vec![],
            file_identifiers: IdentifierScope::default(),
            frames: vec![],
        }
    }

    /// Records a node and returns whether to visit its children.
    fn enter(&mut self, node: Node<'tree>, field: Option<&'static str>) -> bool {
        let syntax = self.code.syntax(node);
        let context = self.context(field);
        if syntax == Syntax::Comment {
            self.frames.push(Frame::skipped(node, syntax));
            return false;
        }
        self.note_statement(node);
        if python::is_docstring(node, &context, self.code) {
            self.frames.push(Frame::skipped(node, syntax));
            return false;
        }

        let opens_function = node.is_named() && syntax.is_function_like();
        if opens_function {
            self.begin_function(node, syntax, &context);
        }
        self.reset_body_identifiers(syntax, &context);
        let opens_nesting = self.count_decision(node, syntax, &context);
        let first_new = self.findings.len();
        rules::inspect(node, syntax, &context, self.code, &mut self.findings);
        if let Some(function) = self.active.last() {
            for finding in &mut self.findings[first_new..] {
                finding.function = Some(self.functions[function.index].name.clone());
            }
        }
        imports::inspect(node, syntax, self.code, &mut self.imports);

        let mut hash = SyntaxHash::new(node.kind());
        // Python's `string_content` keeps escape sequences as children; treating it as
        // one leaf retains the literal text around them, as other grammars do.
        let leaf = node.child_count() == 0 || node.kind() == "string_content";
        let tokens = if leaf {
            self.record_leaf(node, syntax, &context, &mut hash)
        } else {
            0
        };
        self.frames.push(Frame {
            node,
            syntax,
            hash: Some(hash),
            tokens,
            opens_function,
            opens_nesting,
            seen_statement: false,
        });
        !leaf
    }

    /// Named children other than comments are statements for docstring detection. A
    /// docstring is one too, so a later string expression is executable code.
    fn note_statement(&mut self, node: Node<'tree>) {
        if !node.is_named() {
            return;
        }
        if let Some(parent) = self.frames.last_mut() {
            parent.seen_statement = true;
        }
    }

    /// A body's fingerprint depends on how it reuses names, not on the parameter names
    /// read before it. The function's own fingerprint still covers those.
    fn reset_body_identifiers(&mut self, syntax: Syntax, context: &Context<'tree>) {
        let is_body = syntax == Syntax::Block
            && context
                .parent
                .is_some_and(|parent| parent.syntax.is_function_like());
        if !is_body {
            return;
        }
        if let Some(function) = self.active.last_mut() {
            function.identifiers = IdentifierScope::default();
        }
    }

    fn context(&self, field: Option<&'static str>) -> Context<'tree> {
        let mut ancestors = self.frames.iter().rev().map(|frame| Parent {
            node: frame.node,
            syntax: frame.syntax,
        });
        Context {
            parent: ancestors.next(),
            grandparent: ancestors.next(),
            field,
            first_statement: self.frames.last().is_none_or(|frame| !frame.seen_statement),
        }
    }

    fn begin_function(&mut self, node: Node<'tree>, syntax: Syntax, context: &Context<'tree>) {
        if let Some(parent) = self.active.last_mut() {
            if parent.own_lines.is_none() {
                // From here on each leaf counts toward its innermost function only, so
                // backfill the lines the parent covered before its first nested function.
                let first_row = *self.functions[parent.index].lines.rows().start();
                let rows = first_row..=node.start_position().row;
                parent.own_lines = Some(LineCounter::from_rows(&self.code_rows, rows));
            }
        }
        self.active.push(ActiveFunction {
            index: self.functions.len(),
            depth: 0,
            own_lines: None,
            identifiers: IdentifierScope::default(),
        });
        self.functions.push(Function {
            name: functions::name(node, context, self.code),
            lines: LineSpan::of(node),
            sloc: 0,
            cyclomatic: 1,
            cognitive: 0,
            max_nesting: 0,
            parameters: functions::parameter_count(node, self.code),
            closure: syntax == Syntax::Closure,
        });
    }

    /// Adds a node's decisions to the innermost function; returns whether it nests.
    /// Decisions belong to the closest function, so nested functions never inflate parents.
    fn count_decision(
        &mut self,
        node: Node<'tree>,
        syntax: Syntax,
        context: &Context<'tree>,
    ) -> bool {
        let Some(function) = self.active.last_mut() else {
            return false;
        };
        let decision = complexity::decision(node, syntax, context, self.code);
        let metrics = &mut self.functions[function.index];
        metrics.cyclomatic += decision.cyclomatic;
        metrics.cognitive += decision.cognitive;
        if decision.nests {
            // A nesting construct also costs the depth it sits at.
            metrics.cognitive += function.depth;
            function.depth += 1;
            metrics.max_nesting = metrics.max_nesting.max(function.depth);
        }
        decision.nests
    }

    /// Hashes a leaf's text, marks its code lines, and returns its token count.
    fn record_leaf(
        &mut self,
        node: Node<'tree>,
        syntax: Syntax,
        context: &Context<'tree>,
        hash: &mut SyntaxHash,
    ) -> u32 {
        let text = self.code.text(node);
        if text.is_empty() {
            return 0;
        }
        if is_local_name(syntax, context) {
            // Alpha-rename local names to first-use ordinals: `x + x` matches `y + y`
            // but not `x + y`.
            let scope = match self.active.last_mut() {
                Some(function) => &mut function.identifiers,
                None => &mut self.file_identifiers,
            };
            hash.update(&scope.ordinal(text).to_le_bytes());
        } else {
            hash.update(text.as_bytes());
        }
        let owner = self
            .active
            .last_mut()
            .and_then(|function| function.own_lines.as_mut());
        mark_code_rows(node, text, &mut self.code_rows, owner);
        1
    }

    fn exit(&mut self) {
        let frame = self
            .frames
            .pop()
            .expect("the cursor exits only nodes it entered");
        let Some(hash) = &frame.hash else {
            return;
        };
        let fingerprint = hash.finish();
        self.record_clone_candidate(&frame, fingerprint);
        if let Some(parent) = self.frames.last_mut() {
            parent.tokens += frame.tokens;
            // Folding each child's fingerprint into its parent encodes the tree's shape
            // without keeping a serialized copy of the tree.
            if let Some(parent_hash) = &mut parent.hash {
                parent_hash.update(&fingerprint.0);
            }
        }
        if frame.opens_nesting {
            if let Some(function) = self.active.last_mut() {
                function.depth -= 1;
            }
        }
        if frame.opens_function {
            self.end_function();
        }
    }

    fn end_function(&mut self) {
        let Some(function) = self.active.pop() else {
            return;
        };
        let metrics = &mut self.functions[function.index];
        metrics.sloc = match function.own_lines {
            Some(own_lines) => own_lines.count,
            None => count_marked(&self.code_rows, metrics.lines.rows()),
        };
    }

    fn record_clone_candidate(&mut self, frame: &Frame<'tree>, fingerprint: Fingerprint) {
        if frame.syntax != Syntax::Block || frame.tokens < POLICY.min_clone_tokens {
            return;
        }
        let parent = self.frames.last().map(|parent| Parent {
            node: parent.node,
            syntax: parent.syntax,
        });
        if has_two_statements(frame.node, parent, self.code) {
            // The SLOC threshold waits for `finish`, when every code line is known.
            self.candidates.push(Candidate {
                fingerprint,
                lines: LineSpan::of(frame.node),
                tokens: frame.tokens,
            });
        }
    }

    fn finish(mut self) -> Analysis {
        let code_lines = CodeLines::from_rows(&self.code_rows);
        self.candidates
            .retain(|candidate| code_lines.count_in(candidate.lines) >= POLICY.min_clone_lines);
        self.findings.sort_unstable_by_key(|finding| {
            (finding.lines.start(), finding.rule, finding.lines.end())
        });
        self.findings.dedup();
        Analysis {
            physical_lines: self.physical_lines,
            code_lines,
            functions: self.functions,
            findings: self.findings,
            candidates: self.candidates,
            imports: self.imports,
        }
    }
}

/// Identifiers are alpha-normalized unless they name a member reached through another
/// name (`a.b`, `a::b`) or a keyword argument. Those spellings are part of the structure.
fn is_local_name(syntax: Syntax, context: &Context<'_>) -> bool {
    matches!(syntax, Syntax::Identifier | Syntax::ShorthandIdentifier) && !is_member_name(context)
}

fn is_member_name(context: &Context<'_>) -> bool {
    let Some(field) = context.field else {
        return false;
    };
    match context.parent.map(|parent| parent.syntax) {
        Some(Syntax::QualifiedName) => [
            Field::Attribute,
            Field::Name,
            Field::Property,
            Field::Member,
        ]
        .iter()
        .any(|member| member.name() == field),
        Some(Syntax::KeywordArgument) => field == Field::Name.name(),
        _ => false,
    }
}

/// Clone candidates need two executable statements, so a block that only wraps one call
/// or a docstring is not reported as duplicated structure.
fn has_two_statements(block: Node<'_>, parent: Option<Parent<'_>>, code: Code<'_>) -> bool {
    let block = Parent {
        node: block,
        syntax: Syntax::Block,
    };
    // Go wraps a block's statements in a statement list.
    let list = (block.node.named_child_count() == 1)
        .then(|| block.node.named_child(0))
        .flatten()
        .filter(|child| code.syntax(*child) == Syntax::StatementList);
    let (container, grandparent) = match list {
        Some(list) => (
            Parent {
                node: list,
                syntax: Syntax::StatementList,
            },
            Some(block),
        ),
        None => (block, parent),
    };
    let mut first_statement = true;
    let mut cursor = container.node.walk();
    container
        .node
        .named_children(&mut cursor)
        .filter(|statement| {
            let syntax = code.syntax(*statement);
            if syntax == Syntax::Comment {
                return false;
            }
            let context = Context {
                parent: Some(container),
                grandparent,
                field: None,
                first_statement,
            };
            first_statement = false;
            syntax != Syntax::Attribute && !python::is_docstring(*statement, &context, code)
        })
        .take(2)
        .count()
        >= 2
}

/// Marks the rows where a leaf has non-whitespace text. Whitespace-only lines inside a
/// leaf, such as blank lines in a multiline string, are not code.
fn mark_code_rows(
    node: Node<'_>,
    text: &str,
    rows: &mut [bool],
    mut owner: Option<&mut LineCounter>,
) {
    let first = node.start_position().row;
    for (offset, line) in text.split('\n').enumerate() {
        if line.bytes().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let row = first + offset;
        if let Some(flag) = rows.get_mut(row) {
            *flag = true;
        }
        if let Some(owner) = owner.as_deref_mut() {
            owner.mark(row);
        }
    }
}

fn count_marked(marks: &[bool], rows: RangeInclusive<usize>) -> u32 {
    let end = (*rows.end()).min(marks.len().saturating_sub(1));
    let count = marks
        .get(*rows.start()..=end)
        .map_or(0, |marks| marks.iter().filter(|code| **code).count());
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Counts distinct rows. Leaves arrive in source order, so comparing with the previous
/// row is enough to count a row once however many leaves it holds.
#[derive(Default)]
struct LineCounter {
    last: Option<usize>,
    count: u32,
}

impl LineCounter {
    fn from_rows(marks: &[bool], rows: RangeInclusive<usize>) -> Self {
        let mut counter = Self::default();
        for row in rows.filter(|row| marks.get(*row).copied().unwrap_or(false)) {
            counter.mark(row);
        }
        counter
    }

    fn mark(&mut self, row: usize) {
        if self.last != Some(row) {
            self.last = Some(row);
            self.count += 1;
        }
    }
}

/// Incremental XXH3-128 state for one node's structural fingerprint.
struct SyntaxHash(Xxh3);

impl SyntaxHash {
    fn new(kind: &str) -> Self {
        let mut state = Xxh3::new();
        state.update(kind.as_bytes());
        state.update(&[0]);
        Self(state)
    }

    fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> Fingerprint {
        Fingerprint(self.0.digest128().to_le_bytes())
    }
}

const INLINE_NAMES: usize = 16;

/// Assigns each distinct name in a scope the ordinal of its first use.
#[expect(
    clippy::large_enum_variant,
    reason = "the inline table exists to avoid allocating for small scopes"
)]
enum IdentifierScope<'source> {
    /// Most scopes hold few names, and a linear search avoids allocating.
    Inline {
        names: [&'source str; INLINE_NAMES],
        len: usize,
    },
    Map(HashMap<&'source str, u32>),
}

impl Default for IdentifierScope<'_> {
    fn default() -> Self {
        Self::Inline {
            names: [""; INLINE_NAMES],
            len: 0,
        }
    }
}

impl<'source> IdentifierScope<'source> {
    fn ordinal(&mut self, name: &'source str) -> u32 {
        let (names, len) = match self {
            Self::Map(ordinals) => {
                let next = u32::try_from(ordinals.len()).unwrap_or(u32::MAX);
                return *ordinals.entry(name).or_insert(next);
            }
            Self::Inline { names, len } => (names, len),
        };
        let known = &names[..*len];
        if let Some(index) = known.iter().position(|known| *known == name) {
            return index as u32;
        }
        if *len < INLINE_NAMES {
            names[*len] = name;
            *len += 1;
            return (*len - 1) as u32;
        }
        // The inline table is full: move to a map that keeps the ordinals assigned so far.
        let mut ordinals: HashMap<_, _> = names
            .iter()
            .enumerate()
            .map(|(index, known)| (*known, index as u32))
            .collect();
        let ordinal = INLINE_NAMES as u32;
        ordinals.insert(name, ordinal);
        *self = Self::Map(ordinals);
        ordinal
    }
}
