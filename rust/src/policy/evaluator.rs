use std::fmt::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use sha2::{Digest, Sha256};

use crate::context::{Context, Identity};
use crate::errors::{ErrorCode, ModuleError};

/// The post-validation decision for an invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEffect {
    Allow,
    Deny,
    RequireApproval,
}

/// A decision and its optional policy-defined static rule identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolicyVerdict {
    pub decision: PolicyEffect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
}

/// Detached validated input supplied to an explicitly installed policy evaluator.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PolicyEvaluationRequest {
    pub module_id: String,
    pub module_version: Option<String>,
    pub caller_id: Option<String>,
    #[schemars(with = "Option<Value>")]
    pub identity: Option<Identity>,
    pub arguments: Value,
    /// JSON pointers to fields marked sensitive by the module's input schema.
    pub sensitive_fields: Vec<String>,
}

/// An instance-scoped policy evaluated before execution and by preflight.
/// Implementations must be safe to use for a dry run.
#[async_trait]
pub trait PolicyEvaluator: Send + Sync {
    async fn evaluate(
        &self,
        request: &PolicyEvaluationRequest,
    ) -> Result<PolicyVerdict, ModuleError>;
}

impl std::fmt::Debug for dyn PolicyEvaluator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PolicyEvaluator(...)")
    }
}

pub(crate) fn evaluation_error() -> ModuleError {
    ModuleError::new(
        ErrorCode::PolicyEvaluationError,
        "The post-validation policy evaluator did not return a decision",
    )
}

pub(crate) async fn evaluate(
    evaluator: &dyn PolicyEvaluator,
    request: &PolicyEvaluationRequest,
    context: &Context<Value>,
) -> Result<PolicyVerdict, ModuleError> {
    let timeout = context.global_deadline.map_or(1.0, |deadline| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        (deadline - now.as_secs_f64()).clamp(0.0, 1.0)
    });
    if let Some(token) = &context.cancel_token {
        token.check_for(&request.module_id)?;
    }
    if !timeout.is_finite() || timeout <= 0.0 {
        return Err(evaluation_error());
    }
    let decision = tokio::time::timeout(
        Duration::from_secs_f64(timeout),
        evaluator.evaluate(request),
    );
    let result = if let Some(token) = &context.cancel_token {
        tokio::select! {
            biased;
            () = token.cancelled() => {
                token.check_for(&request.module_id)?;
                return Err(evaluation_error());
            }
            result = decision => result,
        }
    } else {
        decision.await
    };
    result
        .ok()
        .and_then(Result::ok)
        .ok_or_else(evaluation_error)
}

pub(crate) fn final_arguments_digest(arguments: &Value) -> Result<String, ModuleError> {
    let mut encoding = String::new();
    write_scope(arguments, &mut encoding).map_err(|()| {
        ModuleError::new(
            ErrorCode::PolicyEvaluationError,
            "Arguments cannot be bound without losing numeric precision",
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(encoding.as_bytes())))
}

fn write_scope(value: &Value, output: &mut String) -> Result<(), ()> {
    match value {
        Value::Object(object) => {
            let mut fields: Vec<_> = object.iter().collect();
            fields.sort_unstable_by(|(left, _), (right, _)| {
                left.encode_utf16().cmp(right.encode_utf16())
            });
            output.push('{');
            for (index, (key, value)) in fields.into_iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).map_err(|_| ())?);
                output.push(':');
                write_scope(value, output)?;
            }
            output.push('}');
        }
        Value::Array(items) => {
            output.push('[');
            for (index, value) in items.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                write_scope(value, output)?;
            }
            output.push(']');
        }
        Value::Number(number) => write_number(number, output)?,
        _ => output.push_str(&serde_json::to_string(value).map_err(|_| ())?),
    }
    Ok(())
}

fn write_number(number: &Number, output: &mut String) -> Result<(), ()> {
    const MAX_INTEGER: u64 = 9_007_199_254_740_991;
    const MAX_FLOAT_INTEGER: f64 = 9_007_199_254_740_991.0;
    if let Some(value) = number.as_i64() {
        if value.unsigned_abs() > MAX_INTEGER {
            return Err(());
        }
        return write!(output, "i{value};").map_err(|_| ());
    }
    // Nonnegative integers above i64::MAX are always outside the safe domain.
    if number.is_u64() {
        return Err(());
    }
    let value = number.as_f64().ok_or(())?;
    if !value.is_finite() || (value.fract() == 0.0 && value.abs() > MAX_FLOAT_INTEGER) {
        return Err(());
    }
    write!(output, "f{:016x};", value.to_bits()).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn policy_arguments_encoding_preserves_native_values() {
        let mut encoding = String::new();
        write_scope(
            &json!({"\u{10000}": -0.0, "\u{e000}": [1, 1.0, "\u{6771}\u{4eac}"], "10": 3}),
            &mut encoding,
        )
        .unwrap();
        assert_eq!(
            encoding,
            "{\"10\":i3;,\"\u{10000}\":f8000000000000000;,\"\u{e000}\":[i1;,f3ff0000000000000;,\"\u{6771}\u{4eac}\"]}"
        );
        assert_ne!(
            final_arguments_digest(&json!(-0.0)).unwrap(),
            final_arguments_digest(&json!(0.0)).unwrap()
        );
        for unsupported in [
            json!(9_007_199_254_740_992_u64),
            json!(-9_007_199_254_740_992.0),
        ] {
            assert_eq!(
                final_arguments_digest(&unsupported).unwrap_err().code,
                ErrorCode::PolicyEvaluationError
            );
        }
    }
}
