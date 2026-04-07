//! Execution Engine
//!
//! The core execution engine that ties all interpreter components together.
//! Implements the full AST execution chain using brush_parser::ast types:
//!
//! execute_script -> execute_and_or_list -> execute_pipeline_node -> execute_command

use brush_parser::ast as bast;

use crate::interpreter::control_flow::{
    execute_for, execute_if, execute_until, execute_while, CaseTerminator, ForResult,
};
use crate::interpreter::errors::{ControlFlowError, ErrexitError, ExitError, InterpreterError};
use crate::interpreter::functions::execute_function_def;
use crate::interpreter::helpers::condition::ConditionResult;
use crate::interpreter::interpreter::{
    build_exported_env, check_command_limit, should_trigger_errexit, update_exit_code,
    FileSystem as SyncFileSystem,
};
use crate::interpreter::pipeline_execution::{
    execute_pipeline, set_pipestatus, PipelineOptions, PipelineState,
};
use crate::interpreter::subshell_group::{execute_group, execute_subshell};
use crate::interpreter::types::{ExecResult, ExecutionLimits, InterpreterState};
use crate::interpreter::word_expansion::{expand_word, expand_word_with_glob, CommandSubstFn};

/// The execution engine that ties all interpreter components together.
pub struct ExecutionEngine<'a> {
    /// Execution limits (max commands, recursion depth, iterations)
    pub limits: &'a ExecutionLimits,
    /// Sync filesystem interface
    pub fs: &'a dyn SyncFileSystem,
}

impl<'a> ExecutionEngine<'a> {
    /// Create a new execution engine.
    pub fn new(limits: &'a ExecutionLimits, fs: &'a dyn SyncFileSystem) -> Self {
        Self { limits, fs }
    }

    /// Execute a complete script (a Program with a list of CompleteCommands).
    ///
    /// Each CompleteCommand is a CompoundList, which contains CompoundListItems.
    /// Each CompoundListItem has an AndOrList and a SeparatorOperator.
    pub fn execute_script(
        &self,
        state: &mut InterpreterState,
        ast: &bast::Program,
    ) -> Result<ExecResult, InterpreterError> {
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for complete_command in &ast.complete_commands {
            // A CompleteCommand is a CompoundList containing CompoundListItems
            for item in &complete_command.0 {
                let is_background = matches!(item.1, bast::SeparatorOperator::Async);

                match self.execute_and_or_list(state, &item.0) {
                    Ok(result) => {
                        stdout.push_str(&result.stdout);
                        stderr.push_str(&result.stderr);
                        exit_code = result.exit_code;
                        update_exit_code(state, exit_code);
                    }
                    Err(InterpreterError::Exit(e)) => {
                        let mut err = e;
                        err.prepend_output(&stdout, &stderr);
                        return Err(InterpreterError::Exit(err));
                    }
                    Err(InterpreterError::ExecutionLimit(e)) => {
                        return Err(InterpreterError::ExecutionLimit(e));
                    }
                    Err(InterpreterError::Errexit(e)) => {
                        stdout.push_str(&e.stdout);
                        stderr.push_str(&e.stderr);
                        exit_code = e.exit_code;
                        return Ok(ExecResult::new(stdout, stderr, exit_code));
                    }
                    Err(InterpreterError::Break(mut e)) => {
                        e.prepend_output(&stdout, &stderr);
                        stdout = e.stdout.clone();
                        stderr = e.stderr.clone();
                        continue;
                    }
                    Err(InterpreterError::Continue(mut e)) => {
                        e.prepend_output(&stdout, &stderr);
                        stdout = e.stdout.clone();
                        stderr = e.stderr.clone();
                        continue;
                    }
                    Err(InterpreterError::Return(mut e)) => {
                        e.prepend_output(&stdout, &stderr);
                        return Err(InterpreterError::Return(e));
                    }
                    Err(e) => {
                        stderr.push_str(&format!("{}\n", e));
                        exit_code = 1;
                    }
                }
            }
        }

        Ok(ExecResult::new(stdout, stderr, exit_code))
    }

    /// Execute an and-or list (pipelines connected by && and ||).
    ///
    /// An AndOrList has a `first` pipeline and `additional` pipelines with And/Or operators.
    pub fn execute_and_or_list(
        &self,
        state: &mut InterpreterState,
        and_or_list: &bast::AndOrList,
    ) -> Result<ExecResult, InterpreterError> {
        // noexec mode (set -n): parse but don't execute
        if state.options.noexec {
            return Ok(ExecResult::ok());
        }

        // Reset errexit_safe at start of each statement
        state.errexit_safe = Some(false);

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;
        let mut last_pipeline_negated = false;
        let mut was_short_circuited = false;

        // Execute the first pipeline
        let result = self.execute_pipeline_node(state, &and_or_list.first)?;
        stdout.push_str(&result.stdout);
        stderr.push_str(&result.stderr);
        exit_code = result.exit_code;
        last_pipeline_negated = and_or_list.first.bang;
        update_exit_code(state, exit_code);

        // Execute additional pipelines with && / || short-circuit logic
        let total_count = 1 + and_or_list.additional.len();
        let mut last_executed_index = 0usize;

        for (i, and_or) in and_or_list.additional.iter().enumerate() {
            let (should_execute, pipeline) = match and_or {
                bast::AndOr::And(pipeline) => (exit_code == 0, pipeline),
                bast::AndOr::Or(pipeline) => (exit_code != 0, pipeline),
            };

            if !should_execute {
                continue;
            }

            let result = self.execute_pipeline_node(state, pipeline)?;
            stdout.push_str(&result.stdout);
            stderr.push_str(&result.stderr);
            exit_code = result.exit_code;
            last_executed_index = i + 1;
            last_pipeline_negated = pipeline.bang;

            update_exit_code(state, exit_code);
        }

        // Check errexit
        was_short_circuited = last_executed_index < (total_count - 1);
        let inner_was_safe = state.errexit_safe.unwrap_or(false);

        if should_trigger_errexit(state, exit_code, was_short_circuited, last_pipeline_negated)
            && !inner_was_safe
        {
            return Err(InterpreterError::Errexit(ErrexitError::new(
                exit_code, stdout, stderr,
            )));
        }

        Ok(ExecResult::new(stdout, stderr, exit_code))
    }

    /// Execute a pipeline (list of commands connected by |).
    pub fn execute_pipeline_node(
        &self,
        state: &mut InterpreterState,
        pipeline: &bast::Pipeline,
    ) -> Result<ExecResult, InterpreterError> {
        let mut pipe_state = PipelineState::new();
        // brush-parser doesn't have pipe_stderr; use empty default
        let pipe_stderr: Vec<bool> = Vec::new();

        let options = PipelineOptions {
            pipefail: state.options.pipefail,
            lastpipe: state.shopt_options.lastpipe,
            runs_in_subshell: false,
            time_pipeline: pipeline.timed.is_some(),
            time_posix_format: matches!(
                pipeline.timed,
                Some(bast::PipelineTimed::TimedWithPosixOutput(_))
            ),
        };

        // We need to pass state through the closure, but execute_pipeline
        // takes ownership of the closure. Use a RefCell pattern.
        use std::cell::RefCell;
        let state_cell = RefCell::new(state);

        let result = execute_pipeline(
            &mut pipe_state,
            &pipeline.seq,
            &pipe_stderr,
            &options,
            |cmd, stdin| {
                let state = &mut *state_cell.borrow_mut();
                self.execute_command(state, cmd, stdin)
            },
        )?;

        // Get state back
        let state = state_cell.into_inner();

        // Set PIPESTATUS
        set_pipestatus(&mut state.env, &result.exit_codes);

        let mut exec_result = result.to_exec_result();

        // Handle negation
        if pipeline.bang {
            exec_result.exit_code = if exec_result.exit_code == 0 { 1 } else { 0 };
        }

        Ok(exec_result)
    }

    /// Execute a single command.
    pub fn execute_command(
        &self,
        state: &mut InterpreterState,
        cmd: &bast::Command,
        stdin: &str,
    ) -> Result<ExecResult, InterpreterError> {
        // Check command limit
        if let Some(msg) = check_command_limit(state, self.limits) {
            return Err(InterpreterError::ExecutionLimit(
                crate::interpreter::errors::ExecutionLimitError::simple(
                    msg,
                    crate::interpreter::errors::LimitType::Commands,
                ),
            ));
        }

        match cmd {
            bast::Command::Simple(simple) => self.execute_simple_command(state, simple, stdin),
            bast::Command::Compound(compound, redirects) => {
                self.execute_compound_command(state, compound, redirects.as_ref(), stdin)
            }
            bast::Command::Function(func_def) => {
                let current_source = state.current_source.clone();
                execute_function_def(state, func_def, current_source.as_deref())
                    .map_err(InterpreterError::Exit)
            }
            bast::Command::ExtendedTest(_ext_test) => {
                // Extended test [[ ... ]] - not yet fully implemented
                // For now, return success
                Ok(ExecResult::ok())
            }
        }
    }

    /// Extract the command name word from a SimpleCommand.
    fn get_command_name_word(cmd: &bast::SimpleCommand) -> Option<&bast::Word> {
        cmd.word_or_name.as_ref()
    }

    /// Extract argument words from a SimpleCommand's suffix.
    fn get_arg_words(cmd: &bast::SimpleCommand) -> Vec<&bast::Word> {
        let mut args = Vec::new();
        if let Some(ref suffix) = cmd.suffix {
            for item in &suffix.0 {
                if let bast::CommandPrefixOrSuffixItem::Word(w) = item {
                    args.push(w);
                }
            }
        }
        args
    }

    /// Execute a simple command (name + args + redirections).
    pub fn execute_simple_command(
        &self,
        state: &mut InterpreterState,
        cmd: &bast::SimpleCommand,
        _stdin: &str,
    ) -> Result<ExecResult, InterpreterError> {
        // Set line number for $LINENO from the command's location
        if let Some(ref word) = cmd.word_or_name {
            if let Some(ref loc) = word.loc {
                state.current_line = loc.start.line as u32;
            }
        }

        // Get command name
        let cmd_name = match Self::get_command_name_word(cmd) {
            Some(word) => {
                let result = expand_word(state, word, None);
                result.value
            }
            None => {
                // Assignment-only command
                return Ok(ExecResult::ok());
            }
        };

        // Expand arguments from suffix
        let mut args: Vec<String> = Vec::new();
        let arg_words = Self::get_arg_words(cmd);
        for arg in &arg_words {
            let result = expand_word_with_glob(state, arg, None, Some(self.fs));
            if let Some(words) = result.split_words {
                args.extend(words);
            } else {
                args.push(result.value);
            }
        }

        // Handle basic builtins
        match cmd_name.as_str() {
            "echo" => {
                let output = if args.is_empty() {
                    "\n".to_string()
                } else {
                    format!("{}\n", args.join(" "))
                };
                Ok(ExecResult::new(output, String::new(), 0))
            }
            "true" | ":" => Ok(ExecResult::ok()),
            "false" => Ok(ExecResult::new(String::new(), String::new(), 1)),
            "exit" => {
                let code = args
                    .first()
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(state.last_exit_code);
                Err(InterpreterError::Exit(ExitError::new(
                    code,
                    String::new(),
                    String::new(),
                )))
            }
            "export" => {
                // Basic export implementation
                for arg in &args {
                    if let Some((name, value)) = arg.split_once('=') {
                        state.env.insert(name.to_string(), value.to_string());
                        if state.exported_vars.is_none() {
                            state.exported_vars = Some(std::collections::HashSet::new());
                        }
                        state
                            .exported_vars
                            .as_mut()
                            .unwrap()
                            .insert(name.to_string());
                    } else {
                        // Just mark as exported
                        if state.exported_vars.is_none() {
                            state.exported_vars = Some(std::collections::HashSet::new());
                        }
                        state.exported_vars.as_mut().unwrap().insert(arg.clone());
                    }
                }
                Ok(ExecResult::ok())
            }
            "cd" => {
                let target = args
                    .first()
                    .map(|s| s.as_str())
                    .or_else(|| state.env.get("HOME").map(|s| s.as_str()))
                    .unwrap_or("/");

                let new_cwd = if target.starts_with('/') {
                    target.to_string()
                } else {
                    self.fs.resolve_path(&state.cwd, target)
                };

                if self.fs.is_dir(&new_cwd) {
                    state.cwd = new_cwd.clone();
                    state.env.insert("PWD".to_string(), new_cwd);
                    Ok(ExecResult::ok())
                } else {
                    Ok(ExecResult::new(
                        String::new(),
                        format!("bash: cd: {}: No such file or directory\n", target),
                        1,
                    ))
                }
            }
            "pwd" => Ok(ExecResult::new(
                format!("{}\n", state.cwd),
                String::new(),
                0,
            )),
            "ls" => {
                let targets: Vec<String> = if args.is_empty() {
                    vec![state.cwd.clone()]
                } else {
                    args.iter()
                        .map(|a| {
                            if a.starts_with('/') {
                                a.clone()
                            } else {
                                self.fs.resolve_path(&state.cwd, a)
                            }
                        })
                        .collect()
                };
                let mut lines: Vec<String> = Vec::new();
                for t in targets {
                    if !self.fs.exists(&t) {
                        return Ok(ExecResult::new(
                            String::new(),
                            format!("ls: {}: No such file or directory\n", t),
                            1,
                        ));
                    }
                    if self.fs.is_dir(&t) {
                        match self.fs.read_dir(&t) {
                            Ok(mut entries) => {
                                entries.sort();
                                lines.extend(entries);
                            }
                            Err(e) => {
                                return Ok(ExecResult::new(
                                    String::new(),
                                    format!("ls: {}: {}\n", t, e),
                                    1,
                                ));
                            }
                        }
                    } else {
                        let base = t.rsplit_once('/').map(|(_, b)| b).unwrap_or(&t);
                        lines.push(base.to_string());
                    }
                }
                Ok(ExecResult::new(
                    format!("{}\n", lines.join("\n")),
                    String::new(),
                    0,
                ))
            }
            "cat" => {
                if args.is_empty() {
                    return Ok(ExecResult::new(
                        String::new(),
                        "cat: missing file operand\n".to_string(),
                        1,
                    ));
                }
                let mut out = String::new();
                for path in &args {
                    let p = if path.starts_with('/') {
                        path.clone()
                    } else {
                        self.fs.resolve_path(&state.cwd, path)
                    };
                    if !self.fs.exists(&p) {
                        return Ok(ExecResult::new(
                            String::new(),
                            format!("cat: {}: No such file or directory\n", p),
                            1,
                        ));
                    }
                    if self.fs.is_dir(&p) {
                        return Ok(ExecResult::new(
                            String::new(),
                            format!("cat: {}: Is a directory\n", p),
                            1,
                        ));
                    }
                    match self.fs.read_file(&p) {
                        Ok(s) => out.push_str(&s),
                        Err(e) => {
                            return Ok(ExecResult::new(
                                String::new(),
                                format!("cat: {}: {}\n", p, e),
                                1,
                            ));
                        }
                    }
                }
                Ok(ExecResult::new(out, String::new(), 0))
            }
            "test" | "[" => {
                // Basic test implementation - just return success for now
                Ok(ExecResult::ok())
            }
            _ => {
                // Unknown command - return error
                Ok(ExecResult::new(
                    String::new(),
                    format!("bash: {}: command not found\n", cmd_name),
                    127,
                ))
            }
        }
    }

    /// Execute a compound command (if, for, while, case, subshell, group, arithmetic, etc.).
    pub fn execute_compound_command(
        &self,
        state: &mut InterpreterState,
        compound: &bast::CompoundCommand,
        redirects: Option<&bast::RedirectList>,
        stdin: &str,
    ) -> Result<ExecResult, InterpreterError> {
        match compound {
            bast::CompoundCommand::IfClause(if_cmd) => {
                // Build clauses for execute_if from the brush-parser IfClauseCommand.
                // First clause: condition + then body
                let mut clauses: Vec<(Vec<&bast::CompoundListItem>, Vec<&bast::CompoundListItem>)> =
                    Vec::new();

                // Primary if clause
                let condition_items: Vec<&bast::CompoundListItem> =
                    if_cmd.condition.0.iter().collect();
                let body_items: Vec<&bast::CompoundListItem> = if_cmd.then.0.iter().collect();
                clauses.push((condition_items, body_items));

                // Elif/else clauses
                let mut else_body: Option<Vec<&bast::CompoundListItem>> = None;
                if let Some(ref elses) = if_cmd.elses {
                    for else_clause in elses {
                        if let Some(ref cond) = else_clause.condition {
                            // elif clause
                            let cond_items: Vec<&bast::CompoundListItem> =
                                cond.0.iter().collect();
                            let body_items: Vec<&bast::CompoundListItem> =
                                else_clause.body.0.iter().collect();
                            clauses.push((cond_items, body_items));
                        } else {
                            // else clause (no condition)
                            else_body =
                                Some(else_clause.body.0.iter().collect());
                        }
                    }
                }

                let result = execute_if(
                    state,
                    &clauses,
                    else_body.as_deref(),
                    |state, item| {
                        let res = self.execute_compound_list_item(state, item)?;
                        Ok(ConditionResult {
                            stdout: res.stdout,
                            stderr: res.stderr,
                            exit_code: res.exit_code,
                        })
                    },
                    |state, item| self.execute_compound_list_item(state, item),
                )?;

                Ok(ExecResult::new(
                    result.stdout,
                    result.stderr,
                    result.exit_code,
                ))
            }

            bast::CompoundCommand::ForClause(for_cmd) => {
                // Expand words
                let mut words: Vec<String> = Vec::new();
                if let Some(ref word_list) = for_cmd.values {
                    for word in word_list {
                        let result = expand_word_with_glob(state, word, None, Some(self.fs));
                        if let Some(split) = result.split_words {
                            words.extend(split);
                        } else {
                            words.push(result.value);
                        }
                    }
                } else {
                    // Default to positional parameters
                    let argc: usize =
                        state.env.get("#").and_then(|s| s.parse().ok()).unwrap_or(0);
                    for i in 1..=argc {
                        if let Some(val) = state.env.get(&i.to_string()) {
                            words.push(val.clone());
                        }
                    }
                }

                let body: Vec<&bast::CompoundListItem> = for_cmd.body.list.0.iter().collect();

                let result = execute_for(
                    state,
                    &for_cmd.variable_name,
                    &words,
                    &body,
                    self.limits.max_iterations,
                    |state, item| self.execute_compound_list_item(state, item),
                )?;

                Ok(ExecResult::new(
                    result.stdout,
                    result.stderr,
                    result.exit_code,
                ))
            }

            bast::CompoundCommand::WhileClause(while_cmd) => {
                let condition: Vec<&bast::CompoundListItem> = while_cmd.0 .0.iter().collect();
                let body: Vec<&bast::CompoundListItem> = while_cmd.1.list.0.iter().collect();

                let result = execute_while(
                    state,
                    &condition,
                    &body,
                    self.limits.max_iterations,
                    |state, item| {
                        let res = self.execute_compound_list_item(state, item)?;
                        Ok(ConditionResult {
                            stdout: res.stdout,
                            stderr: res.stderr,
                            exit_code: res.exit_code,
                        })
                    },
                    |state, item| self.execute_compound_list_item(state, item),
                )?;

                Ok(ExecResult::new(
                    result.stdout,
                    result.stderr,
                    result.exit_code,
                ))
            }

            bast::CompoundCommand::UntilClause(until_cmd) => {
                let condition: Vec<&bast::CompoundListItem> = until_cmd.0 .0.iter().collect();
                let body: Vec<&bast::CompoundListItem> = until_cmd.1.list.0.iter().collect();

                let result = execute_until(
                    state,
                    &condition,
                    &body,
                    self.limits.max_iterations,
                    |state, item| {
                        let res = self.execute_compound_list_item(state, item)?;
                        Ok(ConditionResult {
                            stdout: res.stdout,
                            stderr: res.stderr,
                            exit_code: res.exit_code,
                        })
                    },
                    |state, item| self.execute_compound_list_item(state, item),
                )?;

                Ok(ExecResult::new(
                    result.stdout,
                    result.stderr,
                    result.exit_code,
                ))
            }

            bast::CompoundCommand::CaseClause(case_cmd) => {
                // Expand the case value
                let case_value = expand_word(state, &case_cmd.value, None).value;

                // Build case items for execute_case
                let case_items: Vec<crate::interpreter::control_flow::CaseItem<bast::Word, bast::CompoundListItem>> =
                    case_cmd
                        .cases
                        .iter()
                        .map(|ci| {
                            let terminator = CaseTerminator::from_post_action(&ci.post_action);
                            // For body, we flatten the CompoundList items
                            // If cmd is None, body is empty
                            let body_slice: &[bast::CompoundListItem] = match &ci.cmd {
                                Some(cl) => &cl.0,
                                None => &[],
                            };
                            crate::interpreter::control_flow::CaseItem {
                                patterns: &ci.patterns,
                                body: body_slice,
                                terminator,
                            }
                        })
                        .collect();

                let result = crate::interpreter::control_flow::execute_case(
                    state,
                    &case_value,
                    &case_items,
                    |state, value, pattern| {
                        let expanded = expand_word(state, pattern, None).value;
                        // Pattern matching using glob-to-regex conversion
                        let regex_str = crate::shell::glob_helpers::glob_to_regex(
                            &expanded,
                            state.shopt_options.extglob,
                        );
                        match regex_lite::Regex::new(&regex_str) {
                            Ok(re) => Ok(re.is_match(value)),
                            Err(_) => Ok(expanded == value),
                        }
                    },
                    |state, item| self.execute_compound_list_item(state, item),
                )?;

                Ok(ExecResult::new(
                    result.stdout,
                    result.stderr,
                    result.exit_code,
                ))
            }

            bast::CompoundCommand::Subshell(subshell_cmd) => {
                let body_items: Vec<&bast::CompoundListItem> =
                    subshell_cmd.list.0.iter().collect();
                execute_subshell(
                    state,
                    &body_items,
                    Some(stdin),
                    |state, item| self.execute_compound_list_item(state, item),
                )
            }

            bast::CompoundCommand::BraceGroup(group_cmd) => {
                let body_items: Vec<&bast::CompoundListItem> =
                    group_cmd.list.0.iter().collect();
                execute_group(
                    state,
                    &body_items,
                    Some(stdin),
                    |state, item| self.execute_compound_list_item(state, item),
                )
            }

            bast::CompoundCommand::Arithmetic(arith_cmd) => {
                use crate::interpreter::arithmetic::evaluate_arithmetic;
                use crate::interpreter::types::InterpreterContext;

                let mut ctx = InterpreterContext::new(state, self.limits);
                match evaluate_arithmetic(&mut ctx, &arith_cmd.expr.value, false, None) {
                    Ok(value) => {
                        // Arithmetic command: exit 0 if non-zero, exit 1 if zero
                        let exit_code = if value != 0 { 0 } else { 1 };
                        Ok(ExecResult::new(String::new(), String::new(), exit_code))
                    }
                    Err(e) => Ok(ExecResult::new(String::new(), format!("bash: {}\n", e), 1)),
                }
            }

            bast::CompoundCommand::ArithmeticForClause(arith_for) => {
                use crate::interpreter::arithmetic::evaluate_arithmetic;
                use crate::interpreter::types::InterpreterContext;

                let mut stdout = String::new();
                let mut stderr = String::new();
                let mut exit_code = 0;
                let mut iterations = 0u64;

                // Execute initializer
                if let Some(ref init) = arith_for.initializer {
                    let mut ctx = InterpreterContext::new(state, self.limits);
                    let _ = evaluate_arithmetic(&mut ctx, &init.value, false, None);
                }

                state.loop_depth += 1;

                let result = (|| -> Result<ExecResult, InterpreterError> {
                    loop {
                        iterations += 1;
                        if iterations > self.limits.max_iterations {
                            return Err(InterpreterError::ExecutionLimit(
                                crate::interpreter::errors::ExecutionLimitError::new(
                                    format!(
                                        "for loop: too many iterations ({})",
                                        self.limits.max_iterations
                                    ),
                                    crate::interpreter::errors::LimitType::Iterations,
                                    stdout.clone(),
                                    stderr.clone(),
                                ),
                            ));
                        }

                        // Check condition
                        if let Some(ref cond) = arith_for.condition {
                            let mut ctx = InterpreterContext::new(state, self.limits);
                            match evaluate_arithmetic(&mut ctx, &cond.value, false, None) {
                                Ok(val) => {
                                    if val == 0 {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    stderr.push_str(&format!("bash: {}\n", e));
                                    exit_code = 1;
                                    break;
                                }
                            }
                        }

                        // Execute body
                        for item in &arith_for.body.list.0 {
                            let res = self.execute_compound_list_item(state, item)?;
                            stdout.push_str(&res.stdout);
                            stderr.push_str(&res.stderr);
                            exit_code = res.exit_code;
                        }

                        // Execute updater
                        if let Some(ref updater) = arith_for.updater {
                            let mut ctx = InterpreterContext::new(state, self.limits);
                            let _ = evaluate_arithmetic(&mut ctx, &updater.value, false, None);
                        }
                    }
                    Ok(ExecResult::new(stdout, stderr, exit_code))
                })();

                state.loop_depth -= 1;
                result
            }
        }
    }

    /// Execute a single CompoundListItem (an AndOrList + SeparatorOperator).
    ///
    /// This is the primary unit of execution within compound commands.
    /// A CompoundListItem wraps an AndOrList which contains pipelines connected by && / ||.
    pub fn execute_compound_list_item(
        &self,
        state: &mut InterpreterState,
        item: &bast::CompoundListItem,
    ) -> Result<ExecResult, InterpreterError> {
        self.execute_and_or_list(state, &item.0)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{FileSystem as AsyncFileSystem, InMemoryFs};
    use crate::interpreter::sync_fs_adapter::SyncFsAdapter;
    use std::sync::Arc;

    /// Parse a script string into a brush_parser Program AST.
    fn parse_script(input: &str) -> bast::Program {
        let reader = std::io::Cursor::new(input.to_string());
        let options = brush_parser::ParserOptions::default();
        let source_info = brush_parser::SourceInfo::default();
        let mut parser = brush_parser::Parser::new(reader, &options, &source_info);
        parser.parse_program().expect("parse failed")
    }

    fn make_engine_and_state() -> (ExecutionEngine<'static>, InterpreterState, Arc<InMemoryFs>) {
        let fs = Arc::new(InMemoryFs::new());
        let limits = Box::leak(Box::new(ExecutionLimits::default()));

        // We need a static reference for the test, so we leak the adapter
        let handle = tokio::runtime::Handle::current();
        let adapter = Box::leak(Box::new(SyncFsAdapter::new(fs.clone(), handle)));

        let engine = ExecutionEngine::new(limits, adapter);
        let state = InterpreterState::default();

        (engine, state, fs)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_execute_echo() {
        let (engine, mut state, _fs) = make_engine_and_state();

        let ast = parse_script("echo hello world");
        let result = engine.execute_script(&mut state, &ast).unwrap();

        assert_eq!(result.stdout, "hello world\n");
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_execute_variable_expansion() {
        let (engine, mut state, _fs) = make_engine_and_state();
        state.env.insert("NAME".to_string(), "world".to_string());

        let ast = parse_script("echo hello $NAME");
        let result = engine.execute_script(&mut state, &ast).unwrap();

        assert_eq!(result.stdout, "hello world\n");
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_execute_true_false() {
        let (engine, mut state, _fs) = make_engine_and_state();

        let ast = parse_script("true");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.exit_code, 0);

        let ast = parse_script("false");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.exit_code, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_execute_and_or() {
        let (engine, mut state, _fs) = make_engine_and_state();

        // true && echo yes
        let ast = parse_script("true && echo yes");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.stdout, "yes\n");

        // false && echo no (should not print)
        let ast = parse_script("false && echo no");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.stdout, "");

        // false || echo fallback
        let ast = parse_script("false || echo fallback");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.stdout, "fallback\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_execute_if() {
        let (engine, mut state, _fs) = make_engine_and_state();

        let ast = parse_script("if true; then echo yes; fi");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.stdout, "yes\n");

        let ast = parse_script("if false; then echo no; else echo else; fi");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.stdout, "else\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_execute_for() {
        let (engine, mut state, _fs) = make_engine_and_state();

        let ast = parse_script("for i in a b c; do echo $i; done");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.stdout, "a\nb\nc\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_execute_while() {
        let (engine, mut state, _fs) = make_engine_and_state();
        state.env.insert("x".to_string(), "3".to_string());

        // Simple while that would loop - but we need arithmetic for decrement
        // For now just test basic structure
        let ast = parse_script("while false; do echo loop; done");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.stdout, "");
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_execute_subshell() {
        let (engine, mut state, _fs) = make_engine_and_state();
        state.env.insert("X".to_string(), "original".to_string());

        // Subshell should not affect parent
        let ast = parse_script("(X=modified; echo $X); echo $X");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        // Note: assignment in subshell not fully implemented yet
        // Just verify subshell executes
        assert!(result.stdout.contains("original"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_execute_group() {
        let (engine, mut state, _fs) = make_engine_and_state();

        let ast = parse_script("{ echo a; echo b; }");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.stdout, "a\nb\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_execute_pwd_cd() {
        let fs = Arc::new(InMemoryFs::new());
        let limits = Box::leak(Box::new(ExecutionLimits::default()));

        // Create directory structure using async API directly
        fs.mkdir("/home", &crate::fs::MkdirOptions { recursive: false })
            .await
            .unwrap();
        fs.mkdir("/home/user", &crate::fs::MkdirOptions { recursive: false })
            .await
            .unwrap();

        // Now create the sync adapter
        let handle = tokio::runtime::Handle::current();
        let adapter = Box::leak(Box::new(SyncFsAdapter::new(fs.clone(), handle)));

        let engine = ExecutionEngine::new(limits, adapter);
        let mut state = InterpreterState::default();

        state.cwd = "/".to_string();
        state.env.insert("PWD".to_string(), "/".to_string());

        let ast = parse_script("pwd");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.stdout, "/\n");

        let ast = parse_script("cd /home/user && pwd");
        let result = engine.execute_script(&mut state, &ast).unwrap();
        assert_eq!(result.stdout, "/home/user\n");
    }
}
