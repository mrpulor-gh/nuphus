//! 断言验证步骤
use super::*;

impl Executor {
    /// 执行断言验证：condition 为 false → Err(message 或默认错误)
    pub(super) async fn execute_assert_step(
        &self,
        assert: &AssertDef,
        variables: &HashMap<String, serde_json::Value>,
    ) -> crate::Result<String> {
        let passed = super::variables::eval_condition(&assert.condition, variables);
        if passed {
            return Ok("assert_passed".to_string());
        }
        let msg = match &assert.message {
            Some(m) if !m.is_empty() => m.clone(),
            _ => format!("断言失败: '{:?}'", assert.condition),
        };
        Err(crate::NuphusError::agent(msg))
    }
}
