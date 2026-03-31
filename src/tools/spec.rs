use anyhow::Result;
use serde_json::Value;

pub trait ToolHandler: Send + Sync {
    fn name(&self) -> &'static str;
    #[allow(dead_code)]
    fn description(&self) -> &'static str;
    fn execute(&self, args: &Value) -> Result<String>;
}
