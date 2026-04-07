# Brush-Parser Migration Plan

Replace the hand-rolled parser (~11,700 LOC) with `brush-parser` and adapt the
interpreter to consume `brush_parser::ast` types directly — no translation layer,
no adapter, zero-cost.

## Architecture Change

```
BEFORE:
  input --> hand-rolled lexer/parser --> just-bash AST --> interpreter

AFTER:
  input --> brush_parser::tokenize_str + parse_tokens --> brush_parser::ast --> interpreter
  (word expansion) --> brush_parser::word::parse() --> WordPiece --> expand directly
```

### Key Design Decisions

1. **Words become strings.** brush-parser stores words as `Word { value: String }`.
   Word-internal structure (`WordPiece`) is parsed on-demand during expansion via
   `brush_parser::word::parse()`. This eliminates our pre-parsed `WordNode { parts: Vec<WordPart> }`
   and all 11 `WordPart` variants from the AST.

2. **Arithmetic stays as strings until evaluation.** brush-parser uses
   `UnexpandedArithmeticExpr { value: String }` in the AST. Parsing into
   `ArithmeticExpr` happens at evaluation time via `brush_parser::arithmetic::parse()`.
   This eliminates our `ArithmeticExpressionNode` wrapper.

3. **Statements become `CompoundList`.** Our `ScriptNode { statements }` / `StatementNode`
   / `PipelineNode` triple maps to brush's `Program` / `CompoundList` / `AndOrList` /
   `Pipeline`. Different shape, same semantics.

4. **`[[ ]]` is a top-level command variant**, not nested inside `CompoundCommand`.
   brush has `Command::ExtendedTest(...)`.

5. **Functions store `FunctionDefinition`** (brush type) in `InterpreterState.functions`
   instead of our `FunctionDefNode`.

---

## What Gets Deleted (~11,700 LOC)

| Path | LOC | Description |
|------|-----|-------------|
| `src/ast/types.rs` | 1,216 | All hand-rolled AST types + factory |
| `src/ast/mod.rs` | ~10 | Module declaration |
| `src/parser/lexer/` (6 files) | ~1,750 | Tokenizer, tokens, operators, word, heredoc, lookahead |
| `src/parser/parser.rs` | 879 | Main parser |
| `src/parser/parser_compound.rs` | 877 | Compound command parsing |
| `src/parser/parser_word.rs` | ~250 | Word parsing |
| `src/parser/parser_redirection.rs` | ~200 | Redirection parsing |
| `src/parser/parser_substitution.rs` | ~200 | Substitution parsing |
| `src/parser/expansion_parser.rs` | 1,164 | Parameter expansion parsing |
| `src/parser/arithmetic_parser.rs` | 1,358 | Arithmetic expression parsing |
| `src/parser/arithmetic_primaries.rs` | ~250 | Number parsing |
| `src/parser/conditional_parser.rs` | ~200 | `[[ ]]` parsing |
| `src/parser/command_parser.rs` | ~300 | Simple command parsing |
| `src/parser/compound_parser.rs` | ~300 | Compound command parsing |
| `src/parser/word_parser.rs` | ~300 | Word tokenization |
| `src/parser/types.rs` | ~50 | ParseException |
| `src/parser/mod.rs` | 26 | Module declarations |
| **Total** | **~11,700** | |

## What Gets Added

| File | Est. LOC | Description |
|------|----------|-------------|
| `src/parser.rs` (new, thin) | ~80 | `pub fn parse(input: &str) -> Result<Program, ParseError>` wrapper around brush-parser |
| Interpreter edits | ~net negative | Replace AST type references across 19 files |

---

## Dependency

Add to `crates/just-bash/Cargo.toml`:
```toml
[dependencies]
brush-parser = "0.3"
```

---

## Type Mapping Reference

### Top-Level Structure

| just-bash | brush-parser | Notes |
|-----------|-------------|-------|
| `ScriptNode { statements: Vec<StatementNode> }` | `Program { complete_commands: Vec<CompleteCommand> }` | `CompleteCommand = CompoundList` |
| `StatementNode { pipelines, operators, background, ... }` | `CompoundListItem(AndOrList, SeparatorOperator)` | `SeparatorOperator::Async` = background |
| `StatementOperator::And/Or/Semi` | `AndOr::And(Pipeline) / Or(Pipeline)` | Folded into `AndOrList.additional` |
| `PipelineNode { commands, negated, timed, pipe_stderr }` | `Pipeline { seq: Vec<Command>, bang: bool, timed: Option<PipelineTimed> }` | `pipe_stderr` → handled via redirect |

### Commands

| just-bash | brush-parser | Notes |
|-----------|-------------|-------|
| `CommandNode::Simple(SimpleCommandNode)` | `Command::Simple(SimpleCommand)` | Fields differ (see below) |
| `CommandNode::Compound(CompoundCommandNode)` | `Command::Compound(CompoundCommand, Option<RedirectList>)` | Redirects attached to command, not inner node |
| `CommandNode::FunctionDef(FunctionDefNode)` | `Command::Function(FunctionDefinition)` | |
| *(none)* | `Command::ExtendedTest(ExtendedTestExprCommand, ...)` | `[[ ]]` promoted to top-level |

### SimpleCommand

| just-bash | brush-parser | Notes |
|-----------|-------------|-------|
| `SimpleCommandNode { name, args, assignments, redirections, line }` | `SimpleCommand { prefix, word_or_name, suffix }` | Prefix/suffix contain mixed items |
| `AssignmentNode { name, value, append, array }` | `Assignment { name: AssignmentName, value: AssignmentValue, append, loc }` | `AssignmentName` has `VariableName` and `ArrayElementName` |
| `name: Option<WordNode>` | `word_or_name: Option<Word>` | `Word` is just a string |
| `args: Vec<WordNode>` | Items in `CommandSuffix` with variant `Word(Word)` | Must iterate suffix |
| `redirections: Vec<RedirectionNode>` | Items in prefix/suffix with variant `IoRedirect(IoRedirect)` | Mixed in with args |

### CompoundCommand

| just-bash | brush-parser | Notes |
|-----------|-------------|-------|
| `CompoundCommandNode::If(IfNode)` | `CompoundCommand::IfClause(IfClauseCommand)` | |
| `CompoundCommandNode::For(ForNode)` | `CompoundCommand::ForClause(ForClauseCommand)` | |
| `CompoundCommandNode::CStyleFor(CStyleForNode)` | `CompoundCommand::ArithmeticForClause(ArithmeticForClauseCommand)` | |
| `CompoundCommandNode::While(WhileNode)` | `CompoundCommand::WhileClause(WhileOrUntilClauseCommand)` | Shared type for while/until |
| `CompoundCommandNode::Until(UntilNode)` | `CompoundCommand::UntilClause(WhileOrUntilClauseCommand)` | |
| `CompoundCommandNode::Case(CaseNode)` | `CompoundCommand::CaseClause(CaseClauseCommand)` | |
| `CompoundCommandNode::Subshell(SubshellNode)` | `CompoundCommand::Subshell(SubshellCommand)` | |
| `CompoundCommandNode::Group(GroupNode)` | `CompoundCommand::BraceGroup(BraceGroupCommand)` | |
| `CompoundCommandNode::ArithmeticCommand(...)` | `CompoundCommand::Arithmetic(ArithmeticCommand)` | Uses `UnexpandedArithmeticExpr` (string) |
| `CompoundCommandNode::ConditionalCommand(...)` | `Command::ExtendedTest(...)` (top-level!) | Moved out of CompoundCommand |
| *(none)* | `CompoundCommand::Coprocess(CoprocessCommand)` | New — not yet needed |

### Control Flow Details

| just-bash | brush-parser | Notes |
|-----------|-------------|-------|
| `IfNode { clauses: Vec<IfClause>, else_body }` | `IfClauseCommand { condition, then, elses: Option<Vec<ElseClause>> }` | Only first if-clause inline; elif/else in `elses` |
| `IfClause { condition, body }` | `ElseClause { condition: Option<CompoundList>, body }` | `condition=None` means `else` |
| `ForNode { variable, words, body, redirections }` | `ForClauseCommand { variable_name, values, body: DoGroupCommand, loc }` | Redirects on parent `Command::Compound` |
| `CStyleForNode { init, condition, update, body }` | `ArithmeticForClauseCommand { initializer, condition, updater, body, loc }` | Arith exprs are `UnexpandedArithmeticExpr` (strings) |
| `WhileNode { condition, body, redirections }` | `WhileOrUntilClauseCommand(CompoundList, DoGroupCommand, SourceSpan)` | Tuple struct |
| `CaseNode { word, items, redirections }` | `CaseClauseCommand { value, cases: Vec<CaseItem>, loc }` | |
| `CaseItemNode { patterns, body, terminator }` | `CaseItem { patterns, cmd: Option<CompoundList>, post_action }` | |
| `CaseTerminator::DoubleSemi/SemiAnd/SemiSemiAnd` | `CaseItemPostAction::ExitCase/UnconditionallyExecuteNextCaseItem/ContinueEvaluatingCases` | |

### Redirections

| just-bash | brush-parser | Notes |
|-----------|-------------|-------|
| `RedirectionNode { fd, fd_variable, operator, target }` | `IoRedirect` enum (4 variants) | Completely different structure |
| `RedirectionOperator` (12 variants) | `IoFileRedirectKind` (7 variants) + `IoRedirect` variant selection | Split across type and variant |
| `RedirectionTarget::Word(WordNode)` | `IoFileRedirectTarget::Filename(Word)` | |
| `RedirectionTarget::HereDoc(HereDocNode)` | `IoRedirect::HereDocument(_, IoHereDocument)` | |
| `HereDocNode { delimiter, content, strip_tabs, quoted }` | `IoHereDocument { remove_tabs, requires_expansion, here_end, doc }` | `quoted` → `!requires_expansion` |

### Words (Architectural Shift)

| just-bash | brush-parser | Notes |
|-----------|-------------|-------|
| `WordNode { parts: Vec<WordPart> }` | `Word { value: String, loc }` | **Words are strings** |
| `WordPart::Literal(LiteralPart)` | `WordPiece::Text(String)` | Parsed on-demand |
| `WordPart::SingleQuoted(SingleQuotedPart)` | `WordPiece::SingleQuotedText(String)` | |
| `WordPart::DoubleQuoted(DoubleQuotedPart)` | `WordPiece::DoubleQuotedSequence(Vec<WordPieceWithSource>)` | |
| `WordPart::Escaped(EscapedPart)` | `WordPiece::EscapeSequence(String)` | |
| `WordPart::ParameterExpansion(ParameterExpansionPart)` | `WordPiece::ParameterExpansion(ParameterExpr)` | Very different inner structure |
| `WordPart::CommandSubstitution(CommandSubstitutionPart)` | `WordPiece::CommandSubstitution(String)` | Body is string, not parsed AST |
| `WordPart::ArithmeticExpansion(ArithmeticExpansionPart)` | `WordPiece::ArithmeticExpression(UnexpandedArithmeticExpr)` | String, not parsed expr |
| `WordPart::ProcessSubstitution(ProcessSubstitutionPart)` | `CommandPrefixOrSuffixItem::ProcessSubstitution(...)` | Moved to command level |
| `WordPart::BraceExpansion(BraceExpansionPart)` | Handled by `word::parse_brace_expansions()` | Separate API |
| `WordPart::TildeExpansion(TildeExpansionPart)` | `WordPiece::TildeExpansion(TildeExpr)` | Richer: Home/UserHome/WorkingDir/etc. |
| `WordPart::Glob(GlobPart)` | No AST node — globs are text, expanded by interpreter | |
| *(none)* | `WordPiece::AnsiCQuotedText(String)` | `$'...'` quoting |
| *(none)* | `WordPiece::GettextDoubleQuotedSequence(...)` | `$"..."` quoting |

### Parameter Expansion

| just-bash | brush-parser | Notes |
|-----------|-------------|-------|
| `ParameterExpansionPart { parameter: String, operation }` | `ParameterExpr` (19-variant enum) | Flat enum vs nested operation |
| `ParameterOperation::Inner(DefaultValue{...})` | `ParameterExpr::UseDefaultValues { parameter, indirect, test_type, default_value }` | Fields differ |
| `parameter: String` | `Parameter` enum: `Positional(u32)`, `Special(SpecialParameter)`, `Named(String)`, `NamedWithIndex{...}` | Much richer parameter representation |
| `ParameterOperation::Indirection(...)` | `indirect: bool` field on each `ParameterExpr` variant | Orthogonal, not separate variant |

### Arithmetic

| just-bash | brush-parser | Notes |
|-----------|-------------|-------|
| `ArithmeticExpressionNode { expression: ArithExpr, original_text }` | `UnexpandedArithmeticExpr { value: String }` | **String in AST, parsed on demand** |
| `ArithExpr` (21 variants) | `ArithmeticExpr` (8 variants) | Parsed via `brush_parser::arithmetic::parse()` |
| `ArithExpr::Number(ArithNumberNode { value: i64 })` | `ArithmeticExpr::Literal(i64)` | |
| `ArithExpr::Variable(ArithVariableNode { name, has_dollar_prefix })` | `ArithmeticExpr::Reference(ArithmeticTarget::Variable(String))` | No `has_dollar_prefix` |
| `ArithExpr::Binary(Box<ArithBinaryNode>)` | `ArithmeticExpr::BinaryOp(BinaryOperator, Box<Self>, Box<Self>)` | Inline tuple vs named struct |
| `ArithExpr::Unary(Box<ArithUnaryNode>)` | `ArithmeticExpr::UnaryOp(UnaryOperator, Box<Self>)` | No `prefix` field |
| `ArithExpr::Ternary(Box<ArithTernaryNode>)` | `ArithmeticExpr::Conditional(Box<Self>, Box<Self>, Box<Self>)` | |
| `ArithExpr::Assignment(Box<ArithAssignmentNode>)` | `ArithmeticExpr::Assignment(ArithmeticTarget, Box<Self>)` | No compound assign |
| `ArithExpr::DynamicAssignment(...)` | *(handled differently)* | |
| `ArithExpr::ArrayElement(ArithArrayElementNode)` | `ArithmeticTarget::ArrayElement(String, Box<ArithmeticExpr>)` | Target, not expr |
| `ArithBinaryOperator` (20 variants) | `BinaryOperator` (20 variants) | Names differ slightly |
| `ArithUnaryOperator` (6 variants) | `UnaryOperator` (4) + `UnaryAssignmentOperator` (4) | Split inc/dec out |
| `ArithAssignmentOperator` (11 variants) | `ArithmeticExpr::BinaryAssignment(BinaryOperator, ...)` | Reuses `BinaryOperator` |
| `ArithExpr::Concat/DynamicBase/DynamicNumber/...` (8 variants) | *(no equivalent — brush handles these during parsing)* | Eliminated |

### Conditionals (`[[ ]]`)

| just-bash | brush-parser | Notes |
|-----------|-------------|-------|
| `ConditionalExpressionNode` (7 variants) | `ExtendedTestExpr` (6 variants) | |
| `CondBinaryOperator` (15 variants) | `BinaryPredicate` (17 variants) | Descriptive names |
| `CondUnaryOperator` (26 single-char variants) | `UnaryPredicate` (23 descriptive variants) | e.g. `A` → `FileExists` |
| `CondWordNode { word: WordNode }` | *(no equivalent — leaf nodes use `Word`)* | |

---

## DAG: Parallelizable Work Streams

Tasks are organized as a dependency DAG. Steps at the same level can execute
in parallel. Each step lists affected files and estimated LOC delta.

```
                    ┌─────────────────────┐
                    │ P0: Add brush-parser │
                    │     dependency       │
                    └─────────┬───────────┘
                              │
              ┌───────────────┼───────────────┐
              │               │               │
     ┌────────▼──────┐ ┌─────▼──────┐ ┌──────▼───────┐
     │ P1a: New      │ │ P1b: Type  │ │ P1c: Delete  │
     │ parser.rs     │ │ aliases    │ │ src/ast/      │
     │ entry point   │ │ module     │ │              │
     └────────┬──────┘ └─────┬──────┘ └──────┬───────┘
              │               │               │
              └───────────────┼───────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          │                   │                   │
  ┌───────▼────────┐ ┌───────▼────────┐ ┌────────▼───────┐
  │ P2a: Execution │ │ P2b: Word      │ │ P2c: Redirect  │
  │ engine +       │ │ expansion      │ │ handling       │
  │ control flow   │ │ rewrite        │ │ rewrite        │
  └───────┬────────┘ └───────┬────────┘ └────────┬───────┘
          │                   │                   │
          └───────────────────┼───────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          │                   │                   │
  ┌───────▼────────┐ ┌───────▼────────┐ ┌────────▼───────┐
  │ P3a: Arithmetic│ │ P3b: Condition │ │ P3c: Functions │
  │ evaluator      │ │ evaluator      │ │ + state types  │
  └───────┬────────┘ └───────┬────────┘ └────────┬───────┘
          │                   │                   │
          └───────────────────┼───────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          │                   │                   │
  ┌───────▼────────┐ ┌───────▼────────┐ ┌────────▼───────┐
  │ P4a: Builtins  │ │ P4b: Pipeline  │ │ P4c: Helpers   │
  │ + assignments  │ │ + subshell     │ │ + remaining    │
  └───────┬────────┘ └───────┬────────┘ └────────┬───────┘
          │                   │                   │
          └───────────────────┼───────────────────┘
                              │
                    ┌─────────▼───────────┐
                    │ P5: Delete old      │
                    │ parser/, fix tests, │
                    │ update lib.rs       │
                    └─────────────────────┘
```

---

## P0: Add brush-parser dependency

**Parallel: none (root)**
**Files:** `crates/just-bash/Cargo.toml`

1. Add `brush-parser = "0.3"` to `[dependencies]`.
2. Run `cargo check` to ensure it compiles.
3. Verify `brush_parser::ast`, `brush_parser::word`, `brush_parser::arithmetic` are importable.

---

## P1a: New parser entry point

**Parallel with: P1b, P1c**
**Files:** `src/parser.rs` (new, replaces `src/parser/` directory)
**Depends on: P0**

Create a thin `parser.rs` module:

```rust
//! Parser — thin wrapper around brush-parser.

pub use brush_parser::ast;
pub use brush_parser::{ParseError, Parser, ParserOptions};

/// Parse a bash script string into a brush-parser AST.
pub fn parse(input: &str) -> Result<ast::Program, ParseError> {
    let tokens = brush_parser::tokenize_str(input)?;
    let program = brush_parser::parse_tokens(
        &tokens,
        &brush_parser::ParserOptions::default(),
        &brush_parser::SourceInfo::default(),
    )?;
    Ok(program)
}
```

This replaces the entire `src/parser/` directory (20 files, ~10,500 LOC) with ~15 lines.

---

## P1b: Type re-exports / aliases module

**Parallel with: P1a, P1c**
**Files:** `src/types.rs` (new, temporary bridge)
**Depends on: P0**

Create a temporary module that re-exports brush-parser types under short aliases
to minimize churn in the interpreter files. This is deleted once all interpreter
files are updated.

```rust
//! Temporary type aliases during migration.
//! Delete this module once all interpreter files use brush_parser::ast directly.

pub use brush_parser::ast::{
    AndOr, AndOrList, Assignment, AssignmentName, AssignmentValue,
    BinaryPredicate, CaseClauseCommand, CaseItem, CaseItemPostAction,
    Command, CommandPrefixOrSuffixItem, CompoundCommand, CompoundList,
    CompoundListItem, DoGroupCommand, ExtendedTestExpr, ExtendedTestExprCommand,
    ForClauseCommand, FunctionBody, FunctionDefinition,
    IfClauseCommand, ElseClause, IoRedirect, IoFileRedirectKind,
    IoFileRedirectTarget, IoHereDocument, Pipeline, Program, RedirectList,
    SeparatorOperator, SimpleCommand, SubshellCommand, UnaryPredicate,
    WhileOrUntilClauseCommand, Word, ArithmeticCommand,
    ArithmeticForClauseCommand, BraceGroupCommand,
};

pub use brush_parser::ast::{
    ArithmeticExpr, ArithmeticTarget, BinaryOperator, UnaryOperator,
    UnaryAssignmentOperator, UnexpandedArithmeticExpr,
};

pub use brush_parser::word::{
    Parameter, ParameterExpr, ParameterTestType, ParameterTransformOp,
    SpecialParameter, SubstringMatchKind, TildeExpr, WordPiece, WordPieceWithSource,
};
```

---

## P1c: Delete `src/ast/`

**Parallel with: P1a, P1b**
**Files:** Delete `src/ast/types.rs`, `src/ast/mod.rs`
**Depends on: P0**

Delete the entire `src/ast/` directory (1,216 LOC). This will cause compile
errors in the interpreter — those are resolved in P2-P4.

---

## P2a: Execution engine + control flow

**Parallel with: P2b, P2c**
**Files:**
- `src/interpreter/execution_engine.rs` (848 LOC)
- `src/interpreter/control_flow.rs` (548 LOC)
**Depends on: P1a, P1b, P1c**

### execution_engine.rs changes

**`execute_script`** — Change signature:
```rust
// BEFORE
pub fn execute_script(&self, state: &mut InterpreterState, ast: &ScriptNode) -> ...
// AFTER
pub fn execute_script(&self, state: &mut InterpreterState, program: &ast::Program) -> ...
```

Iteration changes:
```rust
// BEFORE: for statement in &ast.statements { self.execute_statement(state, statement) }
// AFTER:
for complete_command in &program.complete_commands {
    // CompoundList = Vec<CompoundListItem>
    // CompoundListItem = (AndOrList, SeparatorOperator)
    for item in &complete_command.0 {
        let background = matches!(item.1, SeparatorOperator::Async);
        let result = self.execute_and_or_list(state, &item.0, background)?;
        // ... same error handling ...
    }
}
```

**New `execute_and_or_list`** — replaces `execute_statement`:
```rust
pub fn execute_and_or_list(
    &self, state: &mut InterpreterState,
    list: &ast::AndOrList, background: bool,
) -> Result<ExecResult, InterpreterError> {
    // Execute first pipeline
    let mut result = self.execute_pipeline(state, &list.first)?;
    // Then process && / || chain
    for and_or in &list.additional {
        match and_or {
            ast::AndOr::And(pipeline) => {
                if result.exit_code == 0 {
                    result = self.execute_pipeline(state, pipeline)?;
                }
            }
            ast::AndOr::Or(pipeline) => {
                if result.exit_code != 0 {
                    result = self.execute_pipeline(state, pipeline)?;
                }
            }
        }
    }
    Ok(result)
}
```

**`execute_pipeline_node`** → `execute_pipeline`:
```rust
// BEFORE: pipeline: &PipelineNode
// AFTER:  pipeline: &ast::Pipeline
// pipeline.commands → pipeline.seq
// pipeline.negated  → pipeline.bang
// pipeline.timed    → pipeline.timed.is_some()
// pipeline.time_posix → matches!(pipeline.timed, Some(PipelineTimed::TimedWithPosixOutput(_)))
// pipeline.pipe_stderr → not in brush AST; handled via IoRedirect
```

**`execute_command`**:
```rust
// BEFORE: match cmd { CommandNode::Simple(..) | Compound(..) | FunctionDef(..) }
// AFTER:
match cmd {
    ast::Command::Simple(simple) => self.execute_simple_command(state, simple, stdin),
    ast::Command::Compound(compound, redirects) => {
        self.execute_compound_command(state, compound, redirects.as_ref(), stdin)
    }
    ast::Command::Function(func_def) => {
        execute_function_def(state, func_def, current_source.as_deref())
    }
    ast::Command::ExtendedTest(test_cmd, redirects) => {
        self.execute_extended_test(state, test_cmd, redirects.as_ref())
    }
}
```

**`execute_simple_command`**:
```rust
// BEFORE: cmd.name, cmd.args, cmd.assignments, cmd.redirections
// AFTER: iterate cmd.prefix and cmd.suffix
//
// Extract from prefix: assignments (AssignmentWord), redirects (IoRedirect)
// Extract from suffix: args (Word), redirects (IoRedirect)
// word_or_name: command name
//
// Word expansion: expand_word(state, &word) where word is &ast::Word
// Since Word is a string, expansion calls brush_parser::word::parse()
// internally to get WordPieces, then expands each piece.
```

**`execute_compound_command`** — 10 match arms:
```rust
match compound {
    ast::CompoundCommand::IfClause(cmd) => { ... }
    ast::CompoundCommand::ForClause(cmd) => { ... }
    ast::CompoundCommand::ArithmeticForClause(cmd) => { ... }
    ast::CompoundCommand::WhileClause(cmd) => { ... }
    ast::CompoundCommand::UntilClause(cmd) => { ... }
    ast::CompoundCommand::CaseClause(cmd) => { ... }
    ast::CompoundCommand::Subshell(cmd) => { ... }
    ast::CompoundCommand::BraceGroup(cmd) => { ... }
    ast::CompoundCommand::Arithmetic(cmd) => { ... }
    ast::CompoundCommand::Coprocess(_) => {
        // Not implemented — return error
        Ok(ExecResult::new(String::new(), "coproc: not supported\n".into(), 1))
    }
}
```

Specific field mappings per arm:

**IfClause:**
```rust
// brush: IfClauseCommand { condition: CompoundList, then: CompoundList, elses: Option<Vec<ElseClause>> }
// ElseClause { condition: Option<CompoundList>, body: CompoundList }
// condition=Some → elif, condition=None → else
//
// Map to execute_if by building clauses from:
//   first clause: (cmd.condition, cmd.then)
//   elif clauses: elses where condition.is_some()
//   else body: first else where condition.is_none()
```

**ForClause:**
```rust
// brush: ForClauseCommand { variable_name: String, values: Option<Vec<Word>>, body: DoGroupCommand }
// body.list is CompoundList
// variable_name replaces for_node.variable
// values replaces for_node.words — but values are Word (strings), not WordNode
```

**WhileClause / UntilClause:**
```rust
// brush: WhileOrUntilClauseCommand(condition: CompoundList, body: DoGroupCommand, loc)
// Tuple struct — access via .0, .1, .2
// condition is CompoundList, body is DoGroupCommand { list: CompoundList }
```

**CaseClause:**
```rust
// brush: CaseClauseCommand { value: Word, cases: Vec<CaseItem> }
// CaseItem { patterns: Vec<Word>, cmd: Option<CompoundList>, post_action: CaseItemPostAction }
// CaseItemPostAction::ExitCase = ;;
// CaseItemPostAction::UnconditionallyExecuteNextCaseItem = ;&
// CaseItemPostAction::ContinueEvaluatingCases = ;;&
```

**Arithmetic:**
```rust
// brush: ArithmeticCommand { expr: UnexpandedArithmeticExpr { value: String } }
// Parse on demand: brush_parser::arithmetic::parse(&cmd.expr.value)?
// Then evaluate_arithmetic(ctx, &parsed_expr, ...)
```

### control_flow.rs changes

Control flow functions are generic over statement types (they take closures).
The main change is that `execute_if`, `execute_for`, `execute_while`, `execute_until`
need to accept `CompoundList` slices instead of `Vec<&StatementNode>`.

Option A (simpler): Change these functions to accept `&[CompoundListItem]`.
Option B (no change): Keep the closure-based design — the execution engine
already wraps each call. The closures just change their inner type.

**Recommended: Option A** — change signatures to work with `CompoundList` directly:
```rust
pub fn execute_for(
    state: &mut InterpreterState,
    variable: &str,
    words: &[String],
    body: &ast::CompoundList,       // was &[&StatementNode]
    max_iterations: u64,
    exec_fn: impl FnMut(&mut InterpreterState, &ast::CompoundList) -> Result<ExecResult, InterpreterError>,
) -> Result<ForResult, InterpreterError>
```

---

## P2b: Word expansion rewrite

**Parallel with: P2a, P2c**
**Files:**
- `src/interpreter/word_expansion.rs` (1,063 LOC)
- `src/interpreter/expansion/variable.rs` (690 LOC)
- `src/interpreter/expansion/unquoted_expansion.rs` (841 LOC)
- `src/interpreter/expansion/parameter_ops.rs`
- `src/interpreter/expansion/tilde.rs`
- `src/interpreter/expansion/command_substitution.rs`
- `src/interpreter/expansion/arith_text_expansion.rs`
- `src/interpreter/expansion/brace_range.rs`
- `src/interpreter/expansion/quoting.rs`
- `src/interpreter/expansion/word_split.rs`
- `src/interpreter/expansion/pattern_expansion.rs`
- `src/interpreter/expansion/analysis.rs`
- `src/interpreter/expansion/array_word_expansion.rs`
- `src/interpreter/expansion/indirect_expansion.rs`
- `src/interpreter/expansion/pattern_removal.rs`
- `src/interpreter/expansion/array_slice_transform.rs`
- `src/interpreter/expansion/array_pattern_ops.rs`
- `src/interpreter/expansion/array_prefix_suffix.rs`
- `src/interpreter/expansion/element_ops.rs`
- `src/interpreter/expansion/regex_ops.rs`
- `src/interpreter/expansion/positional_params.rs`
- `src/interpreter/expansion/variable_attrs.rs`
- `src/interpreter/expansion/prompt.rs`
- `src/interpreter/expansion/glob_escape.rs`
- `src/interpreter/expansion/pattern.rs`
- `src/interpreter/helpers/word_parts.rs`
- `src/interpreter/helpers/word_matching.rs`
**Depends on: P1a, P1b, P1c**

### Architectural shift

The core change: `expand_word` no longer takes `&WordNode` (pre-parsed parts).
It takes `&ast::Word` (a string) and parses it on-demand.

**New `expand_word` signature:**
```rust
pub fn expand_word(
    state: &mut InterpreterState,
    word: &brush_parser::ast::Word,  // just a string wrapper
    cmd_subst: Option<CommandSubstFn>,
) -> WordExpansionResult {
    let options = brush_parser::ParserOptions::default();
    let pieces = brush_parser::word::parse(&word.value, &options)
        .unwrap_or_default();
    expand_word_pieces(state, &pieces, cmd_subst)
}
```

**New `expand_word_pieces`:**
```rust
fn expand_word_pieces(
    state: &mut InterpreterState,
    pieces: &[WordPieceWithSource],
    cmd_subst: Option<CommandSubstFn>,
) -> WordExpansionResult {
    let mut result = String::new();
    for piece_with_source in pieces {
        result.push_str(&expand_piece(state, &piece_with_source.piece, cmd_subst));
    }
    WordExpansionResult::simple(result)
}
```

**`expand_piece` — maps WordPiece variants:**
```rust
fn expand_piece(state: &mut InterpreterState, piece: &WordPiece, ...) -> String {
    match piece {
        WordPiece::Text(s) => s.clone(),
        WordPiece::SingleQuotedText(s) => s.clone(),
        WordPiece::AnsiCQuotedText(s) => s.clone(), // NEW: $'...' support for free
        WordPiece::EscapeSequence(s) => s.clone(),
        WordPiece::DoubleQuotedSequence(inner_pieces) => {
            // Recursively expand inner pieces
            expand_word_pieces(state, inner_pieces, cmd_subst).value
        }
        WordPiece::TildeExpansion(tilde_expr) => {
            expand_tilde(state, tilde_expr) // adapt to TildeExpr enum
        }
        WordPiece::ParameterExpansion(param_expr) => {
            expand_parameter(state, param_expr) // adapt to ParameterExpr enum
        }
        WordPiece::CommandSubstitution(cmd_string) => {
            // cmd_string is already a string — no need to re-serialize AST
            if let Some(f) = cmd_subst {
                let (output, code) = f(cmd_string, state);
                output
            } else {
                String::new()
            }
        }
        WordPiece::BackquotedCommandSubstitution(cmd_string) => {
            // Same as above
        }
        WordPiece::ArithmeticExpression(unexpanded) => {
            // Parse and evaluate: brush_parser::arithmetic::parse(&unexpanded.value)
            // then evaluate_arithmetic(...)
        }
        WordPiece::GettextDoubleQuotedSequence(pieces) => {
            // Treat like double-quoted for now
            expand_word_pieces(state, pieces, cmd_subst).value
        }
    }
}
```

### Parameter expansion rewrite

The biggest sub-task. Our current code matches on `ParameterOperation` (nested enum).
brush-parser uses a flat `ParameterExpr` with 19 variants. Each variant includes
the `parameter: Parameter` and `indirect: bool` fields.

**`expand_parameter` must match all 19 `ParameterExpr` variants:**

```rust
fn expand_parameter(state: &mut InterpreterState, expr: &ParameterExpr) -> String {
    match expr {
        ParameterExpr::Parameter { parameter, indirect } => {
            let value = resolve_parameter(state, parameter);
            if *indirect { resolve_indirect(state, &value) } else { value }
        }
        ParameterExpr::UseDefaultValues { parameter, indirect, test_type, default_value } => {
            // ${param:-default} or ${param-default}
            // test_type: UnsetOrNull vs Unset
        }
        ParameterExpr::AssignDefaultValues { ... } => { /* ${param:=default} */ }
        ParameterExpr::IndicateErrorIfNullOrUnset { ... } => { /* ${param:?error} */ }
        ParameterExpr::UseAlternativeValue { ... } => { /* ${param:+alt} */ }
        ParameterExpr::ParameterLength { parameter, indirect } => { /* ${#param} */ }
        ParameterExpr::RemoveSmallestSuffixPattern { ... } => { /* ${param%pat} */ }
        ParameterExpr::RemoveLargestSuffixPattern { ... } => { /* ${param%%pat} */ }
        ParameterExpr::RemoveSmallestPrefixPattern { ... } => { /* ${param#pat} */ }
        ParameterExpr::RemoveLargestPrefixPattern { ... } => { /* ${param##pat} */ }
        ParameterExpr::Substring { parameter, indirect, offset, length } => {
            // ${param:offset:length} — offset/length are UnexpandedArithmeticExpr (strings)
            // Parse and evaluate them at expansion time
        }
        ParameterExpr::Transform { parameter, indirect, op } => { /* ${param@Q} etc. */ }
        ParameterExpr::UppercaseFirstChar { ... } => { /* ${param^pat} */ }
        ParameterExpr::UppercasePattern { ... } => { /* ${param^^pat} */ }
        ParameterExpr::LowercaseFirstChar { ... } => { /* ${param,pat} */ }
        ParameterExpr::LowercasePattern { ... } => { /* ${param,,pat} */ }
        ParameterExpr::ReplaceSubstring { ... } => { /* ${param/pat/repl} */ }
        ParameterExpr::VariableNames { prefix, concatenate } => { /* ${!prefix*} */ }
        ParameterExpr::MemberKeys { variable_name, concatenate } => { /* ${!arr[@]} */ }
    }
}
```

**`resolve_parameter` — maps brush `Parameter` enum to variable lookup:**
```rust
fn resolve_parameter(state: &InterpreterState, param: &Parameter) -> String {
    match param {
        Parameter::Named(name) => state.env.get(name).cloned().unwrap_or_default(),
        Parameter::Positional(n) => state.env.get(&n.to_string()).cloned().unwrap_or_default(),
        Parameter::Special(special) => match special {
            SpecialParameter::AllPositionalParameters { concatenate } => { ... }
            SpecialParameter::PositionalParameterCount => { ... }
            SpecialParameter::LastExitStatus => state.last_exit_code.to_string(),
            SpecialParameter::ProcessId => { ... }
            SpecialParameter::ShellName => { ... }
            // etc.
        },
        Parameter::NamedWithIndex { name, index } => { /* array[index] */ }
        Parameter::NamedWithAllIndices { name, concatenate } => { /* arr[@] or arr[*] */ }
    }
}
```

### Tilde expansion adaptation

Our `TildeExpansionPart { user: Option<String> }` becomes brush's richer `TildeExpr`:
```rust
fn expand_tilde(state: &InterpreterState, expr: &TildeExpr) -> String {
    match expr {
        TildeExpr::Home => state.env.get("HOME").cloned().unwrap_or_default(),
        TildeExpr::UserHome(user) => { /* lookup user home */ }
        TildeExpr::WorkingDir => state.env.get("PWD").cloned().unwrap_or_default(),
        TildeExpr::OldWorkingDir => state.env.get("OLDPWD").cloned().unwrap_or_default(),
        TildeExpr::NthDirFromTopOfDirStack { n, plus_used } => { ... }
        TildeExpr::NthDirFromBottomOfDirStack { n } => { ... }
    }
}
```

### Brace expansion

Handled via `brush_parser::word::parse_brace_expansions()` — returns
`Vec<BraceExpressionOrText>`. Adapt our `brace_range.rs` to match on
`BraceExpressionMember::NumberSequence/CharSequence/Child`.

### Files that can be mostly preserved

The following expansion modules don't directly reference AST types and need
minimal or no changes:
- `pattern.rs` — glob pattern → regex conversion (no AST dependency)
- `pattern_expansion.rs` — pattern matching logic
- `word_split.rs` — IFS splitting (operates on strings)
- `quoting.rs` — quote removal (operates on strings)
- `glob_escape.rs` — glob escaping utilities
- `prompt.rs` — PS1 expansion

### Files that need moderate changes

These reference `WordNode`/`WordPart` and need to switch to string-based word handling:
- `variable.rs` — change `get_variable(&str)` calls (parameter name stays string)
- `parameter_ops.rs` — rewrite to match `ParameterExpr` variants
- `indirect_expansion.rs` — adapt to `indirect: bool` on each variant
- `array_word_expansion.rs` — word is now a string
- `command_substitution.rs` — body is now a string, not `ScriptNode`
- `arith_text_expansion.rs` — arithmetic is now `UnexpandedArithmeticExpr` (string)
- `tilde.rs` — adapt to `TildeExpr` enum
- `brace_range.rs` — adapt to `BraceExpressionMember`

### helpers to update
- `word_parts.rs` — delete or rewrite (helpers like `get_literal_value(&WordPart)` become
  `get_text_value(&WordPiece)`)
- `word_matching.rs` — likely deletable or simplified

---

## P2c: Redirection handling rewrite

**Parallel with: P2a, P2b**
**Files:**
- `src/interpreter/redirections.rs` (694 LOC)
**Depends on: P1a, P1b, P1c**

Redirections change from a flat struct with operator enum to a 4-variant enum.
All functions taking `&RedirectionNode` change to `&ast::IoRedirect`.

**Match on IoRedirect:**
```rust
match redirect {
    IoRedirect::File(fd, kind, target) => {
        // kind: IoFileRedirectKind (Read/Write/Append/ReadAndWrite/Clobber/DuplicateInput/DuplicateOutput)
        // target: IoFileRedirectTarget (Filename(Word)/Fd(IoFd)/ProcessSubstitution/Duplicate(Word))
        // fd: Option<IoFd> — same as our fd: Option<i32>
    }
    IoRedirect::HereDocument(fd, here_doc) => {
        // here_doc.doc: Word — the content (expand if requires_expansion)
        // here_doc.remove_tabs: bool
        // here_doc.requires_expansion: bool (inverse of our `quoted`)
    }
    IoRedirect::HereString(fd, word) => {
        // <<< word — expand the word
    }
    IoRedirect::OutputAndError(word, append) => {
        // &> or &>> — redirect both stdout and stderr
    }
}
```

The `fd_variable` feature (dynamic FD allocation with `{varname}>file`) needs
investigation — check if brush-parser represents this in the AST. If not, it may
need upstream contribution or workaround.

---

## P3a: Arithmetic evaluator

**Parallel with: P3b, P3c**
**Files:**
- `src/interpreter/arithmetic.rs` (1,420 LOC)
**Depends on: P2b (for arith expansion in words)**

### Fundamental change

Our evaluator currently takes `&ArithExpr` (pre-parsed, 21 variants).
After migration, arithmetic expressions arrive as `UnexpandedArithmeticExpr { value: String }`.

**Two-step process:**
1. Parse: `brush_parser::arithmetic::parse(&expr.value)` → `ArithmeticExpr`
2. Evaluate: `evaluate_arithmetic(ctx, &parsed)` → `i64`

### evaluate_arithmetic rewrite

Rewrite to match brush's 8 `ArithmeticExpr` variants instead of our 21:

```rust
pub fn evaluate_arithmetic(
    ctx: &mut InterpreterContext,
    expr: &ast::ArithmeticExpr,
    // Remove: in_assignment, arith_exec_fn params
) -> Result<i64, ArithmeticError> {
    match expr {
        ast::ArithmeticExpr::Literal(n) => Ok(*n),

        ast::ArithmeticExpr::Reference(target) => {
            resolve_arith_target(ctx, target)
        }

        ast::ArithmeticExpr::UnaryOp(op, operand) => {
            let val = evaluate_arithmetic(ctx, operand)?;
            match op {
                UnaryOperator::UnaryPlus => Ok(val),
                UnaryOperator::UnaryMinus => Ok(-val),
                UnaryOperator::LogicalNot => Ok(if val == 0 { 1 } else { 0 }),
                UnaryOperator::BitwiseNot => Ok(!val),
            }
        }

        ast::ArithmeticExpr::BinaryOp(op, left, right) => {
            // Short-circuit for LogicalAnd/LogicalOr
            apply_binary_op(
                evaluate_arithmetic(ctx, left)?,
                evaluate_arithmetic(ctx, right)?,
                op,
            )
        }

        ast::ArithmeticExpr::Conditional(cond, then_expr, else_expr) => {
            let c = evaluate_arithmetic(ctx, cond)?;
            if c != 0 {
                evaluate_arithmetic(ctx, then_expr)
            } else {
                evaluate_arithmetic(ctx, else_expr)
            }
        }

        ast::ArithmeticExpr::Assignment(target, value) => {
            let val = evaluate_arithmetic(ctx, value)?;
            assign_to_target(ctx, target, val)?;
            Ok(val)
        }

        ast::ArithmeticExpr::BinaryAssignment(op, target, value) => {
            // +=, -=, etc.
            let current = resolve_arith_target(ctx, target)?;
            let rhs = evaluate_arithmetic(ctx, value)?;
            let result = apply_binary_op(current, rhs, op)?;
            assign_to_target(ctx, target, result)?;
            Ok(result)
        }

        ast::ArithmeticExpr::UnaryAssignment(op, target) => {
            let current = resolve_arith_target(ctx, target)?;
            match op {
                UnaryAssignmentOperator::PrefixIncrement => {
                    assign_to_target(ctx, target, current + 1)?;
                    Ok(current + 1)
                }
                UnaryAssignmentOperator::PostfixIncrement => {
                    assign_to_target(ctx, target, current + 1)?;
                    Ok(current) // return old value
                }
                UnaryAssignmentOperator::PrefixDecrement => {
                    assign_to_target(ctx, target, current - 1)?;
                    Ok(current - 1)
                }
                UnaryAssignmentOperator::PostfixDecrement => {
                    assign_to_target(ctx, target, current - 1)?;
                    Ok(current)
                }
            }
        }
    }
}
```

**Helper: `resolve_arith_target`:**
```rust
fn resolve_arith_target(ctx: &InterpreterContext, target: &ArithmeticTarget) -> Result<i64, ArithmeticError> {
    match target {
        ArithmeticTarget::Variable(name) => {
            let val = ctx.state.env.get(name).cloned().unwrap_or_default();
            parse_as_arith_value(ctx, &val) // recursive variable expansion
        }
        ArithmeticTarget::ArrayElement(name, index_expr) => {
            let index = evaluate_arithmetic(ctx, index_expr)?;
            // array lookup...
        }
    }
}
```

### What gets deleted

Our 21-variant match and all the extra node types:
- `ArithExpr::Concat` — brush handles during parsing
- `ArithExpr::DynamicAssignment` — handled by parser
- `ArithExpr::DynamicElement` — handled by parser
- `ArithExpr::DynamicBase` — handled by parser
- `ArithExpr::DynamicNumber` — handled by parser
- `ArithExpr::Group` — handled by parser
- `ArithExpr::Nested` — handled by parser
- `ArithExpr::CommandSubst` — handled during word expansion
- `ArithExpr::BracedExpansion` — handled during word expansion
- `ArithExpr::DoubleSubscript` — handled by parser
- `ArithExpr::NumberSubscript` — error case handled by parser
- `ArithExpr::SyntaxError` — error case handled by parser
- `ArithExpr::SingleQuote` — handled by parser

**Net effect: 1,420 LOC → ~200-300 LOC.** Massive simplification.

### apply_binary_op

Rename variants to match brush's `BinaryOperator`:
```
ArithBinaryOperator::Add → BinaryOperator::Add
ArithBinaryOperator::Pow → BinaryOperator::Power
// etc. — mostly 1:1 renames
```

---

## P3b: Conditional evaluator (`[[ ]]` and `[ ]`)

**Parallel with: P3a, P3c**
**Files:**
- `src/interpreter/conditionals.rs` (872 LOC)
- `src/interpreter/helpers/condition.rs`
- `src/interpreter/helpers/file_tests.rs`
- `src/interpreter/helpers/string_tests.rs`
- `src/interpreter/helpers/variable_tests.rs`
**Depends on: P2b (word expansion for test operands)**

### `[[ ]]` evaluation

Our `ConditionalExpressionNode` (7 variants) → brush's `ExtendedTestExpr` (6 variants):

```rust
fn evaluate_extended_test(
    state: &mut InterpreterState,
    expr: &ast::ExtendedTestExpr,
) -> Result<bool, InterpreterError> {
    match expr {
        ast::ExtendedTestExpr::UnaryTest(predicate, word) => {
            let expanded = expand_word(state, word, None).value;
            evaluate_unary_predicate(state, predicate, &expanded)
        }
        ast::ExtendedTestExpr::BinaryTest(predicate, left, right) => {
            let l = expand_word(state, left, None).value;
            let r = expand_word(state, right, None).value;
            evaluate_binary_predicate(state, predicate, &l, &r)
        }
        ast::ExtendedTestExpr::And(left, right) => {
            Ok(evaluate_extended_test(state, left)? && evaluate_extended_test(state, right)?)
        }
        ast::ExtendedTestExpr::Or(left, right) => {
            Ok(evaluate_extended_test(state, left)? || evaluate_extended_test(state, right)?)
        }
        ast::ExtendedTestExpr::Not(inner) => {
            Ok(!evaluate_extended_test(state, inner)?)
        }
        ast::ExtendedTestExpr::Parenthesized(inner) => {
            evaluate_extended_test(state, inner)
        }
    }
}
```

**Predicate mapping — `UnaryPredicate`:**
```
CondUnaryOperator::E → UnaryPredicate::FileExists
CondUnaryOperator::F → UnaryPredicate::FileExistsAndIsRegularFile
CondUnaryOperator::D → UnaryPredicate::FileExistsAndIsDir
CondUnaryOperator::Z → UnaryPredicate::StringHasZeroLength
CondUnaryOperator::LowerN → UnaryPredicate::StringHasNonZeroLength
// etc. — 23 variants, all self-documenting names
```

**Predicate mapping — `BinaryPredicate`:**
```
CondBinaryOperator::Eq/EqEq → BinaryPredicate::StringExactlyMatchesPattern
CondBinaryOperator::Ne → BinaryPredicate::StringDoesNotExactlyMatchPattern
CondBinaryOperator::Match → BinaryPredicate::StringMatchesRegex
CondBinaryOperator::NumEq → BinaryPredicate::ArithmeticEqualTo
CondBinaryOperator::Nt → BinaryPredicate::LeftFileIsNewerOrExistsWhenRightDoesNot
// etc.
```

### `[ ]` / `test` evaluation

brush has `TestExpr` (separate from `ExtendedTestExpr`) with the same structure
but string operands instead of `Word`. Our `test`/`[` builtin can use this or
continue with its existing implementation that operates on expanded strings.

---

## P3c: Functions + InterpreterState

**Parallel with: P3a, P3b**
**Files:**
- `src/interpreter/functions.rs` (200+ LOC)
- `src/interpreter/types.rs` (300+ LOC)
**Depends on: P1b**

### InterpreterState.functions

```rust
// BEFORE
pub functions: HashMap<String, FunctionDefNode>,
// AFTER
pub functions: HashMap<String, ast::FunctionDefinition>,
```

`FunctionDefinition` has:
```rust
pub struct FunctionDefinition {
    pub fname: Word,           // function name (was String)
    pub body: FunctionBody,    // FunctionBody(CompoundCommand, Option<RedirectList>)
}
```

**Note:** Our `FunctionDefNode` had `source_file: Option<String>` for `BASH_SOURCE`.
brush's `FunctionDefinition` doesn't have this. Options:
- Store source file separately: `HashMap<String, (ast::FunctionDefinition, Option<String>)>`
- Or wrap: `struct StoredFunction { def: ast::FunctionDefinition, source_file: Option<String> }`

**Recommended:** Use a wrapper struct to keep it clean:
```rust
pub struct StoredFunction {
    pub def: ast::FunctionDefinition,
    pub source_file: Option<String>,
}
pub functions: HashMap<String, StoredFunction>,
```

### execute_function_def

```rust
pub fn execute_function_def(
    state: &mut InterpreterState,
    node: &ast::FunctionDefinition,
    current_source: Option<&str>,
) -> Result<ExecResult, ExitError> {
    let name = &node.fname.value; // Word.value is the string
    // POSIX check...
    state.functions.insert(name.clone(), StoredFunction {
        def: node.clone(),
        source_file: current_source.map(String::from),
    });
    Ok(ExecResult::ok())
}
```

### call_function

When calling a function, extract the body:
```rust
let func = state.functions.get(name).unwrap();
let ast::FunctionBody(compound_cmd, redirects) = &func.def.body;
// Execute compound_cmd with redirects applied
self.execute_compound_command(state, compound_cmd, redirects.as_ref(), stdin)
```

---

## P4a: Builtins + assignments

**Parallel with: P4b, P4c**
**Files:**
- `src/interpreter/builtin_dispatch.rs` (1,053 LOC)
- `src/interpreter/builtins/*.rs` (30+ files, ~8,000 LOC)
- `src/interpreter/simple_command_assignments.rs` (150 LOC)
- `src/interpreter/assignment_expansion.rs` (150 LOC)
**Depends on: P2b, P3c**

### Builtins

Most builtins operate on expanded strings, not AST types. Changes are minimal:
- `eval_cmd.rs`: calls `parse()` → now returns `Program` instead of `ScriptNode`
- `source_cmd.rs`: same — calls `parse()`
- `declare_cmd.rs`: may reference `AssignmentNode` → change to `Assignment`
- `declare_array_parsing.rs`: adapt to `AssignmentValue::Array(Vec<(Option<Word>, Word)>)`

### Assignments

```rust
// BEFORE: process AssignmentNode { name, value, append, array }
// AFTER:  process ast::Assignment { name: AssignmentName, value: AssignmentValue, append, loc }

match &assignment.name {
    AssignmentName::VariableName(name) => {
        match &assignment.value {
            AssignmentValue::Scalar(word) => {
                let expanded = expand_word(state, word, cmd_subst).value;
                if assignment.append { /* append */ } else { /* set */ }
            }
            AssignmentValue::Array(pairs) => {
                // pairs: Vec<(Option<Word>, Word)> — key-value pairs
                // Option<Word> = array index, Word = value
            }
        }
    }
    AssignmentName::ArrayElementName(name, index) => {
        // arr[index]=value
    }
}
```

---

## P4b: Pipeline + subshell

**Parallel with: P4a, P4c**
**Files:**
- `src/interpreter/pipeline_execution.rs` (791 LOC)
- `src/interpreter/subshell_group.rs` (639 LOC)
**Depends on: P2a**

### pipeline_execution.rs

Change `execute_pipeline` to accept `&[ast::Command]` instead of `&[CommandNode]`.
The `pipe_stderr` handling changes — brush doesn't have a `pipe_stderr` field on
`Pipeline`. Instead, `|&` is represented as a redirect in the AST. Check how
brush represents this and adapt accordingly.

### subshell_group.rs

Change `execute_subshell` and `execute_group` to accept `&ast::CompoundList`
instead of `&[StatementNode]`. The subshell body is `SubshellCommand { list: CompoundList }`,
and the group body is `BraceGroupCommand { list: CompoundList }`.

---

## P4c: Remaining helpers

**Parallel with: P4a, P4b**
**Files:**
- `src/interpreter/alias_expansion.rs`
- `src/interpreter/command_resolution.rs`
- `src/interpreter/type_command.rs`
- `src/interpreter/helpers/statements.rs`
- `src/interpreter/helpers/array.rs`
- `src/interpreter/helpers/xtrace.rs`
- `src/interpreter/expansion/analysis.rs`
**Depends on: P2a, P2b**

Most of these work with strings and need minimal changes. The main updates:
- Replace `&WordNode` params with `&ast::Word`
- Replace `&StatementNode` with `&ast::CompoundListItem`
- Update any pattern matching on AST types

---

## P5: Final cleanup

**Depends on: P4a, P4b, P4c (all complete)**
**Files:**
- Delete `src/parser/` directory (20 files)
- Delete `src/ast/` directory (2 files)
- Delete `src/types.rs` (temporary bridge from P1b)
- Update `src/lib.rs` — remove `pub mod ast`, `pub mod parser` (old), add new `pub mod parser`
- Update all `pub use` re-exports
- Run full test suite: `cargo nextest run -p just-bash`
- Fix any remaining compile errors

### lib.rs final state

```rust
pub mod bash;
pub mod commands;
pub mod fs;
pub mod interpreter;
pub mod network;
pub mod parser;  // new thin module
#[cfg(feature = "sandbox")]
pub mod sandbox;
pub mod shell;

pub use bash::Bash;
pub use commands::{Command, CommandContext, CommandResult};
pub use fs::{FileSystem, InMemoryFs};
pub use parser::parse;

// Re-export brush-parser types that downstream consumers need
pub use brush_parser::ast;
pub use brush_parser::ParseError;
```

### Test updates

Tests that construct AST nodes directly (using the `AST` factory) need rewriting:
- `execution_engine.rs` tests: Currently call `crate::parser::parse("...")` which
  returns `ScriptNode`. After migration, it returns `Program`. The tests themselves
  don't construct AST nodes manually — they parse strings — so they mostly just
  need the return type updated.
- `functions.rs` tests: Has `make_function()` that constructs `FunctionDefNode`.
  Rewrite to construct `ast::FunctionDefinition`.
- Any test that builds `WordNode` / `WordPart` values: rewrite to use
  `ast::Word { value: "..." }` (just strings).

---

## Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| brush-parser parses differently than our parser for edge cases | Medium | Run full test suite; add regression tests for tricky cases |
| `pipe_stderr` (`\|&`) representation differs | Low | Check brush AST; may need to detect `2>&1` redirect |
| `fd_variable` (`{var}>file`) not in brush AST | Low | Check brush AST; may need upstream issue |
| `source_file` tracking for functions | Low | Use `StoredFunction` wrapper |
| `deferred_error` / `source_text` fields lost | Low | These are parser-internal; brush has its own error reporting |
| brush-parser version stability | Low | Pin to 0.3.x; brush is actively maintained |
| `$LINENO` tracking | Medium | brush AST has `SourceSpan` with positions; extract line numbers |

---

## LOC Impact Summary

| Category | Before | After | Delta |
|----------|--------|-------|-------|
| Parser + AST | 11,700 | ~80 | **-11,620** |
| Interpreter (arithmetic) | 1,420 | ~300 | **-1,120** |
| Interpreter (word expansion) | ~8,860 | ~6,000 | **-2,860** |
| Interpreter (other) | ~15,000 | ~14,500 | **-500** |
| New dependency | 0 | brush-parser | (external) |
| **Total** | **~37,000** | **~21,000** | **~-16,000** |

**Net reduction: ~16,000 lines of hand-rolled code eliminated.**

---

## Execution Order for Sequential Work

If running sequentially (not parallel):

1. P0 → P1a → P1c → P2a → P2b → P2c → P3a → P3b → P3c → P4a → P4b → P4c → P5

Critical path through the DAG:
**P0 → P1a → P2b (word expansion) → P3a (arithmetic) → P4a (builtins) → P5**

This is the longest chain because word expansion is the foundation for
everything else.
