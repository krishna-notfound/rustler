//! Command description types and the `Command` trait.

use crate::ctx::Ctx;
use async_trait::async_trait;
use serde::Serialize;

/// Behavioural hints for agents/front-ends (MCP-style tool annotations).
#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Annotations {
    pub read_only: bool,
    pub destructive: bool,
    pub network: bool,
    pub costs_money: bool,
    pub long_running: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Example {
    pub description: &'static str,
    pub cli: &'static str,
}

#[derive(Serialize, Clone, Debug)]
pub struct CommandSpec {
    /// "group.action", e.g. "source.import".
    pub name: &'static str,
    pub summary: &'static str,
    /// Input field names filled from CLI positional arguments, in order.
    pub positional: &'static [&'static str],
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub annotations: Annotations,
    pub examples: Vec<Example>,
}

#[async_trait]
pub trait Command: Send + Sync {
    fn spec(&self) -> CommandSpec;
    async fn run(
        &self,
        ctx: &Ctx,
        input: serde_json::Value,
    ) -> rustler_core::Result<serde_json::Value>;
}
