//! CEL (POL-002). Platzhalter.
pub mod ext;

use crate::Phase;
use crate::context::Activation;
use crate::types::Ty;
use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{line}:{column}: {message}")]
pub struct CompileError { pub line: u32, pub column: u32, pub message: String }

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EvalError {
    #[error("{0}")]
    Error(String),
    #[error("Kostenlimit überschritten")]
    CostExceeded,
    #[error("Zeitbudget überschritten")]
    Deadline,
}

pub trait Clock: Send + Sync { fn now_us(&self) -> u64; }

#[derive(Debug, Clone)]
pub struct Limits { pub max_expr_bytes: usize, pub max_cost: u64, pub eval_budget_us: u64 }
impl Default for Limits { fn default() -> Self { Self { max_expr_bytes: 4096, max_cost: 10_000, eval_budget_us: 10_000 } } }

pub struct Meter<'a> { _c: Option<&'a dyn Clock> }
impl<'a> Meter<'a> {
    pub fn new(_l: &Limits, c: &'a dyn Clock) -> Self { Self { _c: Some(c) } }
    pub fn unlimited() -> Self { Self { _c: None } }
}

#[derive(Debug, Clone)]
pub struct Program;
impl Program {
    pub fn eval(&self, _a: &Activation, _m: &mut Meter) -> Result<Value, EvalError> { Err(EvalError::Error("todo".into())) }
    pub fn eval_bool(&self, _a: &Activation, _m: &mut Meter) -> Result<bool, EvalError> { Err(EvalError::Error("todo".into())) }
}
pub fn compile_expr(_s: &str, _p: Phase, _t: Ty) -> Result<Program, CompileError> { Err(CompileError { line: 0, column: 0, message: "todo".into() }) }
pub fn compile_bool(s: &str, p: Phase) -> Result<Program, CompileError> { compile_expr(s, p, Ty::Bool) }
