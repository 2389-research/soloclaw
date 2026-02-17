// ABOUTME: AskUser tool — lets the LLM ask the user a free-text question.
// ABOUTME: The tool is registered so the LLM sees it, but execution is intercepted by the agent loop.

use async_trait::async_trait;
use mux::prelude::*;

/// The tool name used for both registration and interception in the agent loop.
pub const ASK_USER_TOOL_NAME: &str = "ask_user";

/// Tool that allows the LLM to ask the user a question and receive a free-text response.
pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        ASK_USER_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Ask the user a question. Prefer providing multiple-choice options when possible. Use free-text only when the answer is truly open-ended."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Multiple-choice options for the user to select from. Preferred over free-text when the answer is one of a known set."
                }
            },
            "required": ["question"]
        })
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> bool {
        false
    }

    async fn execute(&self, _params: serde_json::Value) -> Result<ToolResult, anyhow::Error> {
        Ok(ToolResult::text(
            "[ask_user tool: should be intercepted by agent loop]",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_is_ask_user() {
        let tool = AskUserTool;
        assert_eq!(tool.name(), ASK_USER_TOOL_NAME);
        assert_eq!(ASK_USER_TOOL_NAME, "ask_user");
    }

    #[test]
    fn tool_description_is_nonempty() {
        let tool = AskUserTool;
        assert!(!tool.description().is_empty());
        assert!(tool.description().contains("question"));
    }

    #[test]
    fn schema_has_question_property() {
        let tool = AskUserTool;
        let schema = tool.schema();
        let props = schema.get("properties").expect("should have properties");
        let question = props.get("question").expect("should have question property");
        assert_eq!(question.get("type").unwrap(), "string");
    }

    #[test]
    fn schema_has_options_property() {
        let tool = AskUserTool;
        let schema = tool.schema();
        let props = schema.get("properties").expect("should have properties");
        let options = props.get("options").expect("should have options property");
        assert_eq!(options.get("type").unwrap(), "array");
    }

    #[test]
    fn schema_requires_question() {
        let tool = AskUserTool;
        let schema = tool.schema();
        let required = schema.get("required").expect("should have required");
        let required_arr = required.as_array().unwrap();
        assert!(required_arr.iter().any(|v| v == "question"));
        // options is NOT required
        assert!(!required_arr.iter().any(|v| v == "options"));
    }

    #[test]
    fn requires_approval_always_false() {
        let tool = AskUserTool;
        let params = serde_json::json!({"question": "what color?"});
        assert!(!tool.requires_approval(&params));
    }

    #[tokio::test]
    async fn execute_returns_fallback_text() {
        let tool = AskUserTool;
        let params = serde_json::json!({"question": "what?"});
        let result = tool.execute(params).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("intercepted by agent loop"));
    }
}
