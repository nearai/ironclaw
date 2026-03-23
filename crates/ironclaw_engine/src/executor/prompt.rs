//! System prompt construction for the execution loop.
//!
//! Builds a CodeAct/RLM system prompt that instructs the LLM to write
//! Python code in ```repl blocks with tools available as callable functions.

use crate::types::capability::ActionDef;

/// Build the system prompt for CodeAct/RLM execution.
///
/// The prompt instructs the LLM to:
/// - Write Python code in ```repl fenced blocks
/// - Call tools as regular Python functions
/// - Use llm_query(prompt, context) for sub-agent calls
/// - Use FINAL(answer) to return the final answer
/// - Access thread context via the `context` variable
pub fn build_codeact_system_prompt(actions: &[ActionDef]) -> String {
    let mut prompt = String::from(CODEACT_PREAMBLE);

    // Add tool documentation
    if !actions.is_empty() {
        prompt.push_str("\n## Available tools (call as Python functions)\n\n");
        for action in actions {
            prompt.push_str(&format!("- `{}(", action.name));
            // Extract parameter names from JSON schema
            if let Some(props) = action.parameters_schema.get("properties")
                && let Some(obj) = props.as_object()
            {
                let params: Vec<&str> = obj.keys().map(String::as_str).collect();
                prompt.push_str(&params.join(", "));
            }
            prompt.push_str(&format!(")` — {}\n", action.description));
        }
    }

    prompt.push_str(CODEACT_POSTAMBLE);
    prompt
}

const CODEACT_PREAMBLE: &str = "\
You are an AI assistant with a Python REPL environment. You solve tasks by writing and executing Python code.

## How to respond

Write Python code inside ```repl fenced blocks. The code will be executed, and you'll see the output.

```repl
result = web_fetch(url=\"https://api.example.com/data\")
print(result)
```

You can write multiple code blocks across turns. Variables persist between blocks within the same turn.

## Special functions

- `llm_query(prompt, context=None)` — Ask a sub-agent to analyze text or answer a question. Returns a string. Use for summarization, analysis, or any task that needs LLM reasoning on data.
- `llm_query_batched(prompts, context=None)` — Same but for multiple prompts in parallel. Returns a list of strings.
- `FINAL(answer)` — Call this when you have the final answer. The argument is returned to the user.

## Context variables

- `context` — List of prior conversation messages (each is a dict with 'role' and 'content')
- `goal` — The current task description
- `step_number` — Current execution step
- `state` — Dict of persisted data from previous steps. Contains tool results keyed by tool name (e.g. `state['web_search']`) and return values (`state['last_return']`, `state['step_0_return']`). Use this to access data from previous steps without re-calling tools.
- `previous_results` — Dict of prior tool call results (from ActionResult messages)

## Important rules

1. Always write code in ```repl blocks — plain text responses are for brief explanations only
2. When you have the final answer, call `FINAL(answer)` inside a code block
3. Tool results are returned as Python objects — use them directly, don't parse JSON
4. If a tool call fails, the error appears as a Python exception — handle it or try a different approach
5. For large data, process it in chunks using llm_query() on subsets rather than loading everything into context
6. Outputs are truncated to 8000 chars — use variables to store large intermediate results";

const CODEACT_POSTAMBLE: &str = "

## Strategy

1. First, examine the context and understand the task
2. Break complex tasks into steps
3. Use tools to gather information or take actions
4. Use llm_query() to analyze or summarize large text
5. Call FINAL() with the answer when done

Think step by step. Execute code immediately — don't just describe what you would do.";
