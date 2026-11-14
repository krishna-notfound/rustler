//! The registry map and the `typed` command builder.

use crate::ctx::Ctx;
use crate::spec::{Annotations, Command, CommandSpec, Example};
use async_trait::async_trait;
use rustler_core::{ErrorCode, RustlerError};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::future::Future;
use std::marker::PhantomData;

#[derive(Default)]
pub struct Registry {
    commands: BTreeMap<&'static str, Box<dyn Command>>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            commands: BTreeMap::new(),
        }
    }

    /// Panics on a duplicate command name (programming error).
    pub fn register(&mut self, c: Box<dyn Command>) {
        let name = c.spec().name;
        if self.commands.insert(name, c).is_some() {
            panic!("command `{name}` registered twice");
        }
    }

    /// All command specs, sorted by name.
    pub fn list(&self) -> Vec<CommandSpec> {
        self.commands.values().map(|c| c.spec()).collect()
    }

    pub fn describe(&self, name: &str) -> Option<CommandSpec> {
        self.commands.get(name).map(|c| c.spec())
    }

    /// `NotFound` (hint: `rustler commands list`) for unknown names.
    pub async fn dispatch(
        &self,
        name: &str,
        input: serde_json::Value,
        ctx: Ctx,
    ) -> rustler_core::Result<serde_json::Value> {
        let cmd = self.commands.get(name).ok_or_else(|| {
            RustlerError::new(
                ErrorCode::NotFound,
                "dispatch",
                format!("unknown command `{name}`"),
            )
            .hint("run `rustler commands list` to see available commands")
        })?;
        cmd.run(&ctx, input).await
    }
}

fn schema_value<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(serde_json::Value::Null)
}

struct TypedCommand<I, O, F, Fut> {
    name: &'static str,
    summary: &'static str,
    positional: &'static [&'static str],
    annotations: Annotations,
    examples: Vec<Example>,
    f: F,
    _types: PhantomData<fn(I) -> (O, Fut)>,
}

#[async_trait]
impl<I, O, F, Fut> Command for TypedCommand<I, O, F, Fut>
where
    I: DeserializeOwned + JsonSchema + Send + 'static,
    O: Serialize + JsonSchema + 'static,
    F: Fn(Ctx, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = rustler_core::Result<O>> + Send + 'static,
{
    fn spec(&self) -> CommandSpec {
        CommandSpec {
            name: self.name,
            summary: self.summary,
            positional: self.positional,
            input_schema: schema_value::<I>(),
            output_schema: schema_value::<O>(),
            annotations: self.annotations,
            examples: self.examples.clone(),
        }
    }

    async fn run(
        &self,
        ctx: &Ctx,
        input: serde_json::Value,
    ) -> rustler_core::Result<serde_json::Value> {
        // A missing input (null) means "no arguments".
        let input = if input.is_null() {
            serde_json::Value::Object(Default::default())
        } else {
            input
        };
        let parsed: I = serde_json::from_value(input).map_err(|e| {
            RustlerError::new(
                ErrorCode::Validation,
                self.name,
                format!("invalid input: {e}"),
            )
            .hint(format!(
                "run `rustler commands describe {}` for the input schema",
                self.name
            ))
        })?;
        let out = (self.f)(ctx.clone(), parsed).await?;
        serde_json::to_value(out).map_err(|e| {
            RustlerError::new(
                ErrorCode::Internal,
                self.name,
                format!("could not serialize output: {e}"),
            )
        })
    }
}

/// Builds a Command from typed input/output; deserialization errors become
/// `ErrorCode::Validation` with the serde message.
///
/// Note: `I`, `O` and `Fut` must be `'static` because the result is a
/// `Box<dyn Command>` (implicitly `'static`).
pub fn typed<I, O, F, Fut>(
    name: &'static str,
    summary: &'static str,
    positional: &'static [&'static str],
    ann: Annotations,
    examples: Vec<Example>,
    f: F,
) -> Box<dyn Command>
where
    I: DeserializeOwned + JsonSchema + Send + 'static,
    O: Serialize + JsonSchema + 'static,
    F: Fn(Ctx, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = rustler_core::Result<O>> + Send + 'static,
{
    Box::new(TypedCommand {
        name,
        summary,
        positional,
        annotations: ann,
        examples,
        f,
        _types: PhantomData,
    })
}
