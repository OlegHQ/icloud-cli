//! Arithmetic Evaluation (brush-parser AST)
//!
//! Evaluates bash arithmetic expressions including:
//! - Basic operators (+, -, *, /, %)
//! - Comparison operators (<, <=, >, >=, ==, !=)
//! - Bitwise operators (&, |, ^, ~, <<, >>)
//! - Logical operators (&&, ||, !)
//! - Assignment operators (=, +=, -=, etc.)
//! - Ternary operator (? :)
//! - Pre/post increment/decrement (++, --)
//!
//! Uses brush_parser::arithmetic::parse to produce a bast::ArithmeticExpr AST,
//! then evaluates it recursively.

use brush_parser::ast as bast;
use std::collections::HashSet;

use crate::interpreter::errors::ArithmeticError;
use crate::interpreter::types::InterpreterContext;

// ============================================================================
// Callback Types
// ============================================================================

/// Callback type for executing command substitutions in arithmetic expressions.
/// Takes the command string and returns (stdout, stderr, exit_code).
pub type ArithExecFn = Box<dyn Fn(&str) -> (String, String, i32)>;

// ============================================================================
// Public Entry Points
// ============================================================================

/// Parse and evaluate an arithmetic expression string.
///
/// This is the main entry point. It parses the string with brush_parser,
/// then evaluates the resulting AST.
pub fn evaluate_arithmetic(
    ctx: &mut InterpreterContext,
    expr_str: &str,
    _is_expansion_context: bool,
    _exec_fn: Option<&ArithExecFn>,
) -> Result<i64, ArithmeticError> {
    let expr_str = expr_str.trim();
    if expr_str.is_empty() {
        return Ok(0);
    }

    // Fast path: simple integer literal
    if let Ok(num) = expr_str.parse::<i64>() {
        if expr_str
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-')
        {
            return Ok(num);
        }
    }

    let parsed = brush_parser::arithmetic::parse(expr_str).map_err(|e| {
        ArithmeticError::new(
            format!(
                "syntax error in expression (error token is \"{}\")",
                expr_str
            ),
            String::new(),
            format!("bash: {}\n", e),
            false,
        )
    })?;

    eval_expr(ctx, &parsed)
}

/// Evaluate a simple arithmetic expression for array index.
/// Used by declare/local builtins for array index evaluation.
pub fn evaluate_array_index(state: &mut InterpreterState, expr: &str) -> i64 {
    use crate::interpreter::types::ExecutionLimits;

    // Fast path: simple integer
    if let Ok(n) = expr.trim().parse::<i64>() {
        return n;
    }

    let limits = ExecutionLimits::default();
    let mut ctx = InterpreterContext::new(state, &limits);

    match evaluate_arithmetic(&mut ctx, expr, false, None) {
        Ok(n) => n,
        Err(_) => 0,
    }
}

use crate::interpreter::types::InterpreterState;

// ============================================================================
// Core AST Evaluator
// ============================================================================

/// Recursively evaluate a parsed arithmetic expression AST node.
fn eval_expr(
    ctx: &mut InterpreterContext,
    expr: &bast::ArithmeticExpr,
) -> Result<i64, ArithmeticError> {
    match expr {
        bast::ArithmeticExpr::Literal(value) => Ok(*value),

        bast::ArithmeticExpr::Reference(target) => resolve_target(ctx, target),

        bast::ArithmeticExpr::UnaryOp(op, operand) => {
            let val = eval_expr(ctx, operand)?;
            Ok(apply_unary_op(val, op))
        }

        bast::ArithmeticExpr::BinaryOp(op, left, right) => {
            // Short-circuit evaluation for logical operators
            match op {
                bast::BinaryOperator::LogicalOr => {
                    let l = eval_expr(ctx, left)?;
                    if l != 0 {
                        return Ok(1);
                    }
                    let r = eval_expr(ctx, right)?;
                    Ok(if r != 0 { 1 } else { 0 })
                }
                bast::BinaryOperator::LogicalAnd => {
                    let l = eval_expr(ctx, left)?;
                    if l == 0 {
                        return Ok(0);
                    }
                    let r = eval_expr(ctx, right)?;
                    Ok(if r != 0 { 1 } else { 0 })
                }
                _ => {
                    let l = eval_expr(ctx, left)?;
                    let r = eval_expr(ctx, right)?;
                    apply_binary_op(l, r, op)
                }
            }
        }

        bast::ArithmeticExpr::Conditional(cond, consequent, alternate) => {
            let c = eval_expr(ctx, cond)?;
            if c != 0 {
                eval_expr(ctx, consequent)
            } else {
                eval_expr(ctx, alternate)
            }
        }

        bast::ArithmeticExpr::Assignment(target, value_expr) => {
            let value = eval_expr(ctx, value_expr)?;
            assign_to_target(ctx, target, value)?;
            Ok(value)
        }

        bast::ArithmeticExpr::BinaryAssignment(op, target, value_expr) => {
            let current = resolve_target(ctx, target)?;
            let rhs = eval_expr(ctx, value_expr)?;
            let new_value = apply_binary_op(current, rhs, op)?;
            assign_to_target(ctx, target, new_value)?;
            Ok(new_value)
        }

        bast::ArithmeticExpr::UnaryAssignment(op, target) => {
            let current = resolve_target(ctx, target)?;
            let (return_value, store_value) = match op {
                bast::UnaryAssignmentOperator::PrefixIncrement => {
                    (current + 1, current + 1)
                }
                bast::UnaryAssignmentOperator::PrefixDecrement => {
                    (current - 1, current - 1)
                }
                bast::UnaryAssignmentOperator::PostfixIncrement => {
                    (current, current + 1)
                }
                bast::UnaryAssignmentOperator::PostfixDecrement => {
                    (current, current - 1)
                }
            };
            assign_to_target(ctx, target, store_value)?;
            Ok(return_value)
        }
    }
}

// ============================================================================
// Operator Helpers
// ============================================================================

/// Evaluate a binary operator on two i64 values.
fn apply_binary_op(
    left: i64,
    right: i64,
    op: &bast::BinaryOperator,
) -> Result<i64, ArithmeticError> {
    match op {
        bast::BinaryOperator::Add => Ok(left + right),
        bast::BinaryOperator::Subtract => Ok(left - right),
        bast::BinaryOperator::Multiply => Ok(left * right),
        bast::BinaryOperator::Divide => {
            if right == 0 {
                Err(ArithmeticError::simple("division by 0"))
            } else {
                Ok(left / right)
            }
        }
        bast::BinaryOperator::Modulo => {
            if right == 0 {
                Err(ArithmeticError::simple("division by 0"))
            } else {
                Ok(left % right)
            }
        }
        bast::BinaryOperator::Power => {
            if right < 0 {
                Err(ArithmeticError::simple("exponent less than 0"))
            } else {
                Ok(left.saturating_pow(right as u32))
            }
        }
        bast::BinaryOperator::ShiftLeft => Ok(left << right),
        bast::BinaryOperator::ShiftRight => Ok(left >> right),
        bast::BinaryOperator::LessThan => Ok(if left < right { 1 } else { 0 }),
        bast::BinaryOperator::LessThanOrEqualTo => Ok(if left <= right { 1 } else { 0 }),
        bast::BinaryOperator::GreaterThan => Ok(if left > right { 1 } else { 0 }),
        bast::BinaryOperator::GreaterThanOrEqualTo => Ok(if left >= right { 1 } else { 0 }),
        bast::BinaryOperator::Equals => Ok(if left == right { 1 } else { 0 }),
        bast::BinaryOperator::NotEquals => Ok(if left != right { 1 } else { 0 }),
        bast::BinaryOperator::BitwiseAnd => Ok(left & right),
        bast::BinaryOperator::BitwiseOr => Ok(left | right),
        bast::BinaryOperator::BitwiseXor => Ok(left ^ right),
        bast::BinaryOperator::Comma => Ok(right),
        // LogicalAnd/LogicalOr handled at call site for short-circuit
        bast::BinaryOperator::LogicalAnd | bast::BinaryOperator::LogicalOr => Ok(right),
    }
}

/// Evaluate a unary operator on an i64 value.
fn apply_unary_op(val: i64, op: &bast::UnaryOperator) -> i64 {
    match op {
        bast::UnaryOperator::UnaryPlus => val,
        bast::UnaryOperator::UnaryMinus => -val,
        bast::UnaryOperator::BitwiseNot => !val,
        bast::UnaryOperator::LogicalNot => {
            if val == 0 { 1 } else { 0 }
        }
    }
}

// ============================================================================
// Variable / Target Resolution
// ============================================================================

/// Resolve an ArithmeticTarget to its current i64 value.
fn resolve_target(
    ctx: &mut InterpreterContext,
    target: &bast::ArithmeticTarget,
) -> Result<i64, ArithmeticError> {
    match target {
        bast::ArithmeticTarget::Variable(name) => {
            resolve_variable(ctx, name, &mut HashSet::new())
        }
        bast::ArithmeticTarget::ArrayElement(name, index_expr) => {
            let index = eval_expr(ctx, index_expr)?;
            let env_key = format!("{}_{}", name, index);
            let value = ctx.state.env.get(&env_key).cloned().unwrap_or_default();
            if value.is_empty() {
                // Scalar decay: arr[0] falls back to scalar value
                if index == 0 {
                    let scalar = ctx.state.env.get(name.as_str()).cloned().unwrap_or_default();
                    if !scalar.is_empty() {
                        return parse_value_as_arith(ctx, &scalar);
                    }
                }
                Ok(0)
            } else {
                parse_value_as_arith(ctx, &value)
            }
        }
    }
}

/// Assign a value to an ArithmeticTarget.
fn assign_to_target(
    ctx: &mut InterpreterContext,
    target: &bast::ArithmeticTarget,
    value: i64,
) -> Result<(), ArithmeticError> {
    let env_key = match target {
        bast::ArithmeticTarget::Variable(name) => name.clone(),
        bast::ArithmeticTarget::ArrayElement(name, index_expr) => {
            let index = eval_expr(ctx, index_expr)?;
            format!("{}_{}", name, index)
        }
    };
    ctx.state.env.insert(env_key, value.to_string());
    Ok(())
}

/// Recursively resolve a variable name to its numeric value.
///
/// In bash arithmetic, if a variable contains another variable name or an
/// arithmetic expression, it is recursively evaluated:
///   foo=5; bar=foo; $((bar)) => 5
///   e=1+2; $((e + 3)) => 6
fn resolve_variable(
    ctx: &mut InterpreterContext,
    name: &str,
    visited: &mut HashSet<String>,
) -> Result<i64, ArithmeticError> {
    if visited.contains(name) {
        return Ok(0);
    }
    visited.insert(name.to_string());

    let value = get_arith_variable(ctx, name);

    if value.is_empty() {
        return Ok(0);
    }

    let trimmed = value.trim();

    // Try as simple integer
    if let Ok(num) = trimmed.parse::<i64>() {
        if trimmed.chars().all(|c| c.is_ascii_digit() || c == '-') {
            return Ok(num);
        }
    }

    // If it's a valid identifier, recursively resolve
    if is_valid_identifier(trimmed) {
        return resolve_variable(ctx, trimmed, visited);
    }

    // Parse and evaluate as expression
    parse_value_as_arith(ctx, trimmed)
}

/// Parse a string value and evaluate it as arithmetic.
fn parse_value_as_arith(
    ctx: &mut InterpreterContext,
    value: &str,
) -> Result<i64, ArithmeticError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(0);
    }
    if let Ok(num) = value.parse::<i64>() {
        if value.chars().all(|c| c.is_ascii_digit() || c == '-') {
            return Ok(num);
        }
    }
    let parsed = brush_parser::arithmetic::parse(value).map_err(|e| {
        ArithmeticError::new(
            format!(
                "syntax error in expression (error token is \"{}\")",
                value
            ),
            String::new(),
            format!("bash: {}\n", e),
            false,
        )
    })?;
    eval_expr(ctx, &parsed)
}

// ============================================================================
// Variable Access Helpers
// ============================================================================

/// Get an arithmetic variable value with array[0] decay support.
fn get_arith_variable(ctx: &InterpreterContext, name: &str) -> String {
    // Handle special variables
    match name {
        "?" => return ctx.state.last_exit_code.to_string(),
        "$" => return ctx.state.bash_pid.to_string(),
        "!" => {
            return if ctx.state.last_background_pid == 0 {
                String::new()
            } else {
                ctx.state.last_background_pid.to_string()
            };
        }
        "#" => {
            return ctx
                .state
                .env
                .get("#")
                .cloned()
                .unwrap_or_else(|| "0".to_string());
        }
        "@" | "*" => return ctx.state.env.get(name).cloned().unwrap_or_default(),
        _ => {}
    }

    // Direct variable lookup
    if let Some(val) = ctx.state.env.get(name) {
        return val.clone();
    }
    // Array decay: varName_0
    let array_zero_key = format!("{}_0", name);
    if let Some(val) = ctx.state.env.get(&array_zero_key) {
        return val.clone();
    }
    String::new()
}

/// Check if a variable name is a valid identifier.
fn is_valid_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let bytes = name.as_bytes();
    let first = bytes[0];
    if !matches!(first, b'a'..=b'z' | b'A'..=b'Z' | b'_') {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|&b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_'))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::types::{ExecutionLimits, InterpreterContext, InterpreterState};

    /// Helper macro to create a context with proper lifetimes.
    /// We can't return a context borrowing a local, so tests use this inline.
    macro_rules! with_ctx {
        ($state:expr, |$ctx:ident| $body:expr) => {{
            let limits = ExecutionLimits::default();
            let mut $ctx = InterpreterContext::new($state, &limits);
            $body
        }};
    }

    #[test]
    fn test_apply_binary_op() {
        assert_eq!(apply_binary_op(5, 3, &bast::BinaryOperator::Add).unwrap(), 8);
        assert_eq!(apply_binary_op(5, 3, &bast::BinaryOperator::Subtract).unwrap(), 2);
        assert_eq!(apply_binary_op(5, 3, &bast::BinaryOperator::Multiply).unwrap(), 15);
        assert_eq!(apply_binary_op(6, 3, &bast::BinaryOperator::Divide).unwrap(), 2);
        assert_eq!(apply_binary_op(6, 3, &bast::BinaryOperator::Modulo).unwrap(), 0);
        assert_eq!(apply_binary_op(5, 3, &bast::BinaryOperator::LessThan).unwrap(), 0);
        assert_eq!(apply_binary_op(3, 5, &bast::BinaryOperator::LessThan).unwrap(), 1);
        assert_eq!(apply_binary_op(5, 5, &bast::BinaryOperator::Equals).unwrap(), 1);
    }

    #[test]
    fn test_apply_binary_op_division_by_zero() {
        assert!(apply_binary_op(5, 0, &bast::BinaryOperator::Divide).is_err());
        assert!(apply_binary_op(5, 0, &bast::BinaryOperator::Modulo).is_err());
    }

    #[test]
    fn test_apply_unary_op() {
        assert_eq!(apply_unary_op(5, &bast::UnaryOperator::UnaryMinus), -5);
        assert_eq!(apply_unary_op(-5, &bast::UnaryOperator::UnaryMinus), 5);
        assert_eq!(apply_unary_op(0, &bast::UnaryOperator::LogicalNot), 1);
        assert_eq!(apply_unary_op(5, &bast::UnaryOperator::LogicalNot), 0);
    }

    #[test]
    fn test_bitwise_ops() {
        assert_eq!(
            apply_binary_op(0b1010, 0b1100, &bast::BinaryOperator::BitwiseAnd).unwrap(),
            0b1000
        );
        assert_eq!(
            apply_binary_op(0b1010, 0b1100, &bast::BinaryOperator::BitwiseOr).unwrap(),
            0b1110
        );
        assert_eq!(
            apply_binary_op(0b1010, 0b1100, &bast::BinaryOperator::BitwiseXor).unwrap(),
            0b0110
        );
        assert_eq!(
            apply_binary_op(5, 2, &bast::BinaryOperator::ShiftLeft).unwrap(),
            20
        );
        assert_eq!(
            apply_binary_op(20, 2, &bast::BinaryOperator::ShiftRight).unwrap(),
            5
        );
    }

    #[test]
    fn test_is_valid_identifier() {
        assert!(is_valid_identifier("foo"));
        assert!(is_valid_identifier("_bar"));
        assert!(is_valid_identifier("foo123"));
        assert!(is_valid_identifier("foo_bar"));
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("123foo"));
        assert!(!is_valid_identifier("foo-bar"));
    }

    #[test]
    fn test_evaluate_simple_expressions() {
        let mut state = InterpreterState::default();
        with_ctx!(&mut state, |ctx| {
            assert_eq!(evaluate_arithmetic(&mut ctx, "5", false, None).unwrap(), 5);
            assert_eq!(evaluate_arithmetic(&mut ctx, "1+2", false, None).unwrap(), 3);
            assert_eq!(evaluate_arithmetic(&mut ctx, "10-3", false, None).unwrap(), 7);
            assert_eq!(evaluate_arithmetic(&mut ctx, "2*3", false, None).unwrap(), 6);
            assert_eq!(evaluate_arithmetic(&mut ctx, "10/3", false, None).unwrap(), 3);
            assert_eq!(evaluate_arithmetic(&mut ctx, "10%3", false, None).unwrap(), 1);
        });
    }

    #[test]
    fn test_evaluate_variable() {
        let mut state = InterpreterState::default();
        state.env.insert("x".to_string(), "42".to_string());
        with_ctx!(&mut state, |ctx| {
            assert_eq!(evaluate_arithmetic(&mut ctx, "x", false, None).unwrap(), 42);
            assert_eq!(evaluate_arithmetic(&mut ctx, "x+1", false, None).unwrap(), 43);
        });
    }

    #[test]
    fn test_evaluate_assignment() {
        let mut state = InterpreterState::default();
        with_ctx!(&mut state, |ctx| {
            assert_eq!(evaluate_arithmetic(&mut ctx, "x=5", false, None).unwrap(), 5);
            assert_eq!(ctx.state.env.get("x").unwrap(), "5");
        });
    }

    #[test]
    fn test_evaluate_ternary() {
        let mut state = InterpreterState::default();
        with_ctx!(&mut state, |ctx| {
            assert_eq!(evaluate_arithmetic(&mut ctx, "1 ? 10 : 20", false, None).unwrap(), 10);
            assert_eq!(evaluate_arithmetic(&mut ctx, "0 ? 10 : 20", false, None).unwrap(), 20);
        });
    }

    #[test]
    fn test_evaluate_increment_decrement() {
        let mut state = InterpreterState::default();
        state.env.insert("x".to_string(), "5".to_string());
        with_ctx!(&mut state, |ctx| {
            // Prefix increment: returns new value
            assert_eq!(evaluate_arithmetic(&mut ctx, "++x", false, None).unwrap(), 6);
            assert_eq!(ctx.state.env.get("x").unwrap(), "6");
        });
    }

    #[test]
    fn test_evaluate_postfix_increment() {
        let mut state = InterpreterState::default();
        state.env.insert("x".to_string(), "5".to_string());
        with_ctx!(&mut state, |ctx| {
            // Postfix increment: returns old value
            assert_eq!(evaluate_arithmetic(&mut ctx, "x++", false, None).unwrap(), 5);
            assert_eq!(ctx.state.env.get("x").unwrap(), "6");
        });
    }

    #[test]
    fn test_evaluate_array_index_simple() {
        let mut state = InterpreterState::default();

        assert_eq!(evaluate_array_index(&mut state, "5"), 5);
        assert_eq!(evaluate_array_index(&mut state, "0"), 0);
        assert_eq!(evaluate_array_index(&mut state, "-1"), -1);
    }

    #[test]
    fn test_evaluate_array_index_arithmetic() {
        let mut state = InterpreterState::default();

        assert_eq!(evaluate_array_index(&mut state, "1+2"), 3);
        assert_eq!(evaluate_array_index(&mut state, "10-3"), 7);
        assert_eq!(evaluate_array_index(&mut state, "2*3"), 6);
    }

    #[test]
    fn test_evaluate_array_index_with_variable() {
        let mut state = InterpreterState::default();
        state.env.insert("i".to_string(), "5".to_string());

        assert_eq!(evaluate_array_index(&mut state, "i"), 5);
        assert_eq!(evaluate_array_index(&mut state, "i+1"), 6);
    }

    #[test]
    fn test_special_vars() {
        let mut state = InterpreterState::default();
        state.last_exit_code = 42;
        state.bash_pid = 12345;
        state.last_background_pid = 9999;
        state.env.insert("#".to_string(), "3".to_string());

        let limits = ExecutionLimits::default();
        let ctx = InterpreterContext::new(&mut state, &limits);

        assert_eq!(get_arith_variable(&ctx, "?"), "42");
        assert_eq!(get_arith_variable(&ctx, "$"), "12345");
        assert_eq!(get_arith_variable(&ctx, "!"), "9999");
        assert_eq!(get_arith_variable(&ctx, "#"), "3");
    }

    #[test]
    fn test_special_vars_empty_background_pid() {
        let mut state = InterpreterState::default();
        state.last_background_pid = 0;

        let limits = ExecutionLimits::default();
        let ctx = InterpreterContext::new(&mut state, &limits);
        assert_eq!(get_arith_variable(&ctx, "!"), "");
    }

    #[test]
    fn test_logical_short_circuit() {
        let mut state = InterpreterState::default();
        with_ctx!(&mut state, |ctx| {
            // LogicalOr short-circuits when left is true
            assert_eq!(evaluate_arithmetic(&mut ctx, "1 || 0", false, None).unwrap(), 1);
            assert_eq!(evaluate_arithmetic(&mut ctx, "0 || 0", false, None).unwrap(), 0);

            // LogicalAnd short-circuits when left is false
            assert_eq!(evaluate_arithmetic(&mut ctx, "0 && 1", false, None).unwrap(), 0);
            assert_eq!(evaluate_arithmetic(&mut ctx, "1 && 1", false, None).unwrap(), 1);
        });
    }

    #[test]
    fn test_division_by_zero() {
        let mut state = InterpreterState::default();
        with_ctx!(&mut state, |ctx| {
            assert!(evaluate_arithmetic(&mut ctx, "1/0", false, None).is_err());
            assert!(evaluate_arithmetic(&mut ctx, "1%0", false, None).is_err());
        });
    }

    #[test]
    fn test_recursive_variable_resolution() {
        let mut state = InterpreterState::default();
        state.env.insert("a".to_string(), "b".to_string());
        state.env.insert("b".to_string(), "42".to_string());
        with_ctx!(&mut state, |ctx| {
            assert_eq!(evaluate_arithmetic(&mut ctx, "a", false, None).unwrap(), 42);
        });
    }

    #[test]
    fn test_empty_expression() {
        let mut state = InterpreterState::default();
        with_ctx!(&mut state, |ctx| {
            assert_eq!(evaluate_arithmetic(&mut ctx, "", false, None).unwrap(), 0);
            assert_eq!(evaluate_arithmetic(&mut ctx, "  ", false, None).unwrap(), 0);
        });
    }
}
