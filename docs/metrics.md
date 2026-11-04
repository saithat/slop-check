# Metric definitions (`metrics/6`)

All metrics are static structural measurements. A result is evidence for review,
not proof of poor design, semantic equivalence, or an AI author.

## Source lines and functions

Physical lines include comments and blanks. SLOC counts lines containing a
non-whitespace syntax leaf outside comments and Python module/class/function
docstrings. Blank lines within multiline strings are not SLOC. Closing braces
and other punctuation count as code. A line is counted once regardless of the
number of syntax leaves on it.

Function SLOC belongs to the closest enclosing function. A nested function's
lines therefore do not inflate its parent, while a physical line containing
syntax from both scopes can belong to both. Decisions follow the same closest-
function rule. Anonymous functions, arrows, closures, methods, and Python
lambdas are included and marked with `closure: true`; bodyless signatures are
not. Function names are inferred where possible, and anonymous names include
line and column.

Parameter count excludes Python `/` and bare `*` separators, includes Rust
`self` and Python `self`/`cls`, and counts Go grouped names individually. Go
receivers are not parameters. Destructured parameters count as one parameter.
Functions and closures with **more than 5** parameters are reported as
`parameter_count` observations; `summary.parameters` records the maximum and how
many functions exceed the threshold. Trait implementations and overrides are
counted too, although their parameter lists are fixed elsewhere.

## Cyclomatic complexity

Each function starts at 1. Decisions are counted within the closest enclosing
function, using the bundled language grammar:

| Language | Additional decision nodes |
| --- | --- |
| Python | `if`, `elif`, `for`, `while`, `except`, conditional expressions, comprehension `for`/`if`, non-fallback match cases, `and`/`or` |
| JavaScript / TypeScript / TSX | `if`, loops, `catch`, ternaries, non-default switch cases, `&&`/`||`/`??` |
| Rust | `if`, `for`, `while`, `loop`, non-fallback match arms, `&&`/`||` |
| Go | `if`, `for`, non-default expression/type switch cases, `&&`/`||` |

Irrefutable Python and Rust capture arms are treated as fallbacks when they have
no guard. This is a syntax-based McCabe-style approximation, not a full
control-flow graph. For example, a Rust loop counts even when it has no
conditional exit; optional
chaining, Go select cases, pattern alternatives within one arm, Python asserts,
and macro-expanded decisions are not counted separately. Statements at module
scope do not contribute to per-function complexity.

References: [Lizard](https://github.com/terryyin/lizard),
[Radon metric definitions](https://radon.readthedocs.io/en/latest/intro.html).
Their language conventions differ from this implementation.

## Cognitive complexity and nesting

This tool defines a transparent **cognitive-complexity heuristic**, inspired by
the control-flow/nesting distinction in
[Sonar's metric documentation](https://docs.sonarsource.com/sonarqube-server/user-guide/code-metrics/metrics-definition).
It is not an implementation of the complete Sonar algorithm.

- Each branch/loop/catch/conditional expression counted as a nested decision
  adds `1 + current nesting depth`, then increases depth within that construct.
- An `elif` or `else if` adds 1 without adding another nesting level.
- A plain `else`, including Rust `let ... else`, adds 1. The `else` in an
  `else if` and the `else` keyword inside a Python conditional expression
  (`a if c else b`) are not counted separately.
- A logical operator adds 1 per same-operator AST chain; a changed operator or
  a parenthesized separate chain starts another group.
- Switch/match containers increase nesting and add their current depth; their
  non-fallback cases/arms add 1 each without a further case-level nesting
  increment. Unlike Sonar, which counts a whole switch once, a long `match` that
  maps each variant to a value therefore scores one point per arm.
- Recursion and ordinary returns do not add cognitive points.
- Entering a nested function resets its nesting context.

`max_nesting` is the maximum of this control-flow depth, not indentation or brace
depth. Simple functions may have cognitive complexity 0.

## Erosion

For each function, `mass = cyclomatic × sqrt(function SLOC)`. Structural erosion
is the share of total mass in functions **strictly above** the fixed reporting
threshold of 15. This is the formula from
[SlopCodeBench §2.3](https://arxiv.org/pdf/2603.24755v2).

Cognitive erosion substitutes cognitive complexity and uses its separate
threshold of 10.
Zero denominators yield 0. `summary.cyclomatic` and `summary.cognitive` each
report `max`, `high_functions`, `mass`, `high_mass`, and `erosion`, so the ratio
cannot hide the underlying changes.

The paper's function extraction and counting conventions need not match ours.
An implementation fingerprint identifies the exact implementation used.

## Clones

The detector hashes complete syntax blocks (`block` / `statement_block`) with
at least two executable body statements, 50 leaf tokens, and 5 SLOC.

Each node's fingerprint includes its node kind and ordered child fingerprints.
Whitespace and comments are ignored. Leaf literal spellings are retained, and so
are names reached through another name (`b` in `a.b` or `a::b`: attributes,
properties, fields, and path members) and Python keyword-argument names. Other
identifiers are alpha-normalized by order of first use within each function body,
preserving whether two uses have the same spelling. `run(x)` and `go(x)` can
therefore match, while `client.read()` and `client.delete()`, or
`Kind::First` and `Kind::Second`, cannot. Blocks match only within the same grammar language, including separate
TS/TSX groups. Function names lie outside body blocks, so changing only a
function's name does not change its body's fingerprint.

This detects identical normalized syntax, **not semantic equivalence**. It does
not resolve bindings, so shadowed identifiers with the same spelling can still
be conflated. Function-scoped normalization may also miss equivalent nested
blocks whose earlier identifier-use order differs. XXH3-128 fingerprints are
not collision-proof and are not suitable for adversarial proof of equality.
The detector does not find arbitrary token windows, slightly edited clones, or
arbitrary statement sequences like PMD CPD does.

Groups are processed largest-first. A nested group already fully covered by
reported clones is suppressed. A nested group that adds coverage is retained.
Duplicated SLOC includes **all occurrences**, not only excess copies. Overlapping
and nested ranges are unioned per file and intersected with actual code lines.

Reference: [PMD CPD](https://pmd.github.io/pmd/pmd_userdocs_cpd.html).

## Structural rules and flagged coverage

| Rule | Languages | Detection |
| --- | --- | --- |
| `self-assignment` | Python, JS/TS, Rust | Plain identifier assignment to the same identifier |
| `empty-catch` | Python, JS/TS | An `except` or `catch` containing only comments or `pass` |
| `unreachable-statement` | All | Direct sibling statements after an unconditional return/throw/raise/break/continue in a block; Rust explicit return/break/continue expression statements are included |
| `wide-tuple-return` | Rust, TS/TSX, Python | A declared return type that is a tuple of 3 or more unlabeled values, such as `-> (u32, u32, bool)` |
| `chained-tuple-index` | Rust | A tuple position read through another tuple position, such as `pair.1[0].1` or `nested.0.1` |
| `unreachable-wildcard-arm` | Rust | A `_ =>` arm, without a guard, whose body is only `unreachable!()` or `panic!()` |
| `equality-dispatch-chain` | All | An `if`/`else if` chain of 3 or more conditions, each comparing the same expression with `==`/`===` against a literal or a qualified name such as `Kind.A` or `Kind::A` |

The unreachable rule does not prove general reachability. It does not expand
macros or analyze calls such as `panic()`/`sys.exit()`. Function declarations are
excluded from its findings because they may be hoisted; JS and Go labels reset the
local terminal-state check, and Go case bodies are checked like blocks. Rust
returns/breaks/continues preceded by `cfg` or `cfg_attr` are conservatively
treated as conditional because compilation may remove them. Intentional exception
swallowing and framework behavior can make a finding appropriate code.

The last four rules point at places where a type or construct could state
intent that the code currently leaves implicit:

- `wide-tuple-return` reports the return type. Callers of such a function read
  results by position; a named struct, record, or interface would label them.
  TypeScript tuples whose elements are labeled (`[start: number, end: number,
  ok: boolean]`) are exempt. So are Rust methods in `impl Trait for Type`,
  whose signature the trait declaration fixes; the declaration itself is
  reported. Go's multiple results are idiomatic and not checked.
- `chained-tuple-index` reports each access path once per line. Single positions,
  including newtype access such as `self.0`, are not reported.
- `unreachable-wildcard-arm` suggests the matched type admits values the match
  cannot handle. A narrower type would let the compiler check exhaustiveness.
- `equality-dispatch-chain` reports the head of the chain. A `match` or
  `switch` states the cases directly and, for enums, lets the compiler check
  that every variant is handled. Chains with fewer than 3 comparisons, a
  condition that is not an equality test, a comparison against a plain variable
  (which a Rust or Python pattern would bind rather than compare), or a Go
  statement before the condition are not reported.

`flagged_lines` is the union of clone and rule ranges intersected with SLOC;
`flagged_coverage = flagged_lines / total SLOC`. `rule_findings` is a count of
distinct rule/location findings. These rules are the versioned `structural/2`
pack, not SlopCodeBench's 137-rule set. A zero therefore
means only that these rules and clone thresholds found no covered lines. The
current CLI has no rule-pack loading or per-rule suppression. Inspect the source
and tests before changing code.

## Dependencies

The dependency graph resolves only scanned files:

- Python absolute imports are resolved under the scan root and its `src/`
  directory; dotted names, relative imports, module files, and `__init__.py`
  are supported. Names in `from ... import ...` are also tried as submodules.
- JS/TS relative static imports, exports-from, literal `require()` calls, and
  literal `import()` calls are resolved against files and `index` files.
  `.js` → `.ts`/`.tsx`, `.mjs` → `.mts`, and `.cjs` → `.cts` fallbacks are supported.
  A specifier without a script extension, including dotted names such as
  `./user.service`, has each script extension and `/index` tried in turn.
- Package manifests, TS path aliases, Python custom source roots, dynamic imports,
  Rust modules, and Go package graphs are not resolved.

`unresolved_imports` includes external dependencies as well as unsupported local
resolution. It is not an error count. Conditional/type-only imports still form
structural edges; a reported component need not be a runtime import failure.

`cycles` contains sorted strongly connected components with more than one file,
not every simple cycle or a traversal order. Review the edge and component
detail rather than treating the component count as a complete measure.

## Report (schema 6)

The JSON report is the tool's only output. [`report.schema.json`](report.schema.json)
defines it completely, and the test suite validates real reports against that
schema. In outline:

- Every object always has all of its keys. A value that does not apply is `null`,
  for example `symbol` on a duplicate block or the lines of a dependency-cycle file.
- `observations` holds every observation in the order of `observation_codes`, and
  within a metric, strongest first. Each one has a `kind`, `code`, `message`,
  `symbol` (the measured function, or the innermost function containing a finding),
  `value`, `threshold`, and `locations`.
- `observation_codes` lists every code the tool can emit, even when this report has
  none, with its description, the languages that can produce it, and its `count`.
- Each diagnostic has a `code`, `path` (null for the scan as a whole), `line` (the
  first offending line of a syntax or encoding error, otherwise null), and
  `message`. Codes are `no-supported-files`, `discovery-error`, `non-utf8-path`,
  `read-error`, `file-too-large`, `allocation-error`, `not-utf8`, `syntax-error`,
  and `parse-aborted`.
- Each finding in `files[].findings` names its `rule`, lines, and enclosing
  `function`. Dependency edges are `{"from", "to"}` objects, and `scan_scope` is
  `{"kind": "file" | "directory", "path"}`.

## Comparing repositories

Prefer changes within the same repository, language, scope, and analyzer
fingerprint. Language grammars have different counting conventions and support.
Use `by_language` for heterogeneous repositories; TSX is reported separately.
Generated files, tests, deliberate duplication, and inherent domain complexity
can dominate aggregate results. Pair structural metrics with correctness tests
and review rather than optimizing the metric alone.
