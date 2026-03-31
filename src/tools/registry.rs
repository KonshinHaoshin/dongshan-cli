use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::tools::spec::ToolHandler;

pub struct ToolRegistry {
    tools: BTreeMap<&'static str, Arc<dyn ToolHandler>>,
}

impl ToolRegistry {
    pub fn claude_style() -> Self {
        let mut registry = Self {
            tools: BTreeMap::new(),
        };
        registry.register(crate::tools::shell::ShellTool);
        registry.register(crate::tools::file_read::FileReadTool);
        registry.register(crate::tools::file_write::FileWriteTool);
        registry.register(crate::tools::file_edit::FileEditTool);
        registry.register(crate::tools::grep::GrepTool);
        registry.register(crate::tools::glob::GlobTool);
        registry.register(crate::tools::prompt::PromptTool);
        registry.register(crate::tools::config::ConfigTool);
        registry.register(crate::tools::model::ModelTool);
        registry.register(crate::tools::skills::SkillsTool);
        registry
    }

    pub fn register<T>(&mut self, tool: T)
    where
        T: ToolHandler + 'static,
    {
        self.tools.insert(tool.name(), Arc::new(tool));
    }

    pub fn execute(&self, name: &str, args: &Value) -> Result<String> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;
        tool.execute(args)
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.tools.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ToolRegistry;

    #[test]
    fn registry_exposes_core_tools() {
        let registry = ToolRegistry::claude_style();
        assert!(registry.names().contains(&"fs_read_file"));
        assert!(registry.names().contains(&"shell"));
    }

    #[test]
    fn unknown_tool_fails() {
        let registry = ToolRegistry::claude_style();
        assert!(registry.execute("nope", &json!({})).is_err());
    }
}
