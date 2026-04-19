// Generate bindings from the WIT interface
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "wit/tool.wit",
});

use self::near::agent::host::{self, LogLevel};
use exports::near::agent::tool::{Guest, Request, Response};
use serde::{Deserialize, Serialize};

// Input structure for job prompt requests
#[derive(Deserialize)]
struct JobPromptInput {
    task_description: String,
    skill_level: Option<String>,
    domain: Option<String>,
    format: Option<String>,
    max_length: Option<usize>,
}

// Output structure for job prompt responses
#[derive(Serialize)]
struct JobPromptOutput {
    prompt: String,
    estimated_tokens: usize,
    confidence_score: f32,
    suggestions: Vec<String>,
}

struct JobPromptTool;

impl Guest for JobPromptTool {
    fn execute(req: Request) -> Response {
        // Parse input
        let input: JobPromptInput = match serde_json::from_str(&req.params) {
            Ok(i) => i,
            Err(e) => {
                return Response {
                    output: None,
                    error: Some(format!("Invalid input: {}", e)),
                }
            }
        };

        host::log(
            LogLevel::Info,
            &format!(
                "Processing job prompt request for: {}",
                input.task_description
            ),
        );

        // Check if authentication is available
        let has_auth = host::secret_exists("AUTH_TOKEN") || host::secret_exists("API_KEY");

        if !has_auth {
            return Response {
                output: None,
                error: Some(
                    "Authentication required: Missing AUTH_TOKEN or API_KEY secret".to_string(),
                ),
            };
        }

        // Generate the job prompt based on input parameters
        let prompt = generate_job_prompt(&input);
        let estimated_tokens = estimate_tokens(&prompt);
        let confidence_score = 0.85; // Default confidence score
        let suggestions = generate_suggestions(&input);

        let output = JobPromptOutput {
            prompt,
            estimated_tokens,
            confidence_score,
            suggestions,
        };

        // Return success
        Response {
            output: Some(serde_json::to_string(&output).unwrap()),
            error: None,
        }
    }

    fn schema() -> String {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_description": {
                    "type": "string",
                    "description": "Description of the job or task to create a prompt for"
                },
                "skill_level": {
                    "type": "string",
                    "enum": ["beginner", "intermediate", "advanced", "expert"],
                    "description": "Skill level required for the job"
                },
                "domain": {
                    "type": "string",
                    "description": "Domain or industry for the job"
                },
                "format": {
                    "type": "string",
                    "enum": ["detailed", "concise", "technical", "business"],
                    "description": "Format of the prompt"
                },
                "max_length": {
                    "type": "integer",
                    "minimum": 50,
                    "maximum": 5000,
                    "description": "Maximum length of the generated prompt"
                }
            },
            "required": ["task_description"]
        })
        .to_string()
    }

    fn description() -> String {
        "Generates optimized prompts for job descriptions and task specifications. Handles authentication and provides formatted prompts with confidence scoring.".to_string()
    }
}

// Helper function to generate job prompt
fn generate_job_prompt(input: &JobPromptInput) -> String {
    let skill_level = input.skill_level.as_deref().unwrap_or("intermediate");
    let domain = input.domain.as_deref().unwrap_or("general");
    let format = input.format.as_deref().unwrap_or("detailed");

    match format {
        "concise" => format!(
            "Create a concise job prompt for: {}. Skill level: {}. Domain: {}.",
            input.task_description, skill_level, domain
        ),
        "technical" => format!(
            "Technical job prompt: {}. Required skills: {}. Industry: {}. Include specific technical requirements and qualifications.",
            input.task_description, skill_level, domain
        ),
        "business" => format!(
            "Business-oriented job prompt: {}. Level: {}. Domain: {}. Focus on business objectives and ROI.",
            input.task_description, skill_level, domain
        ),
        _ => format!(
            "Create a detailed job prompt for the following task: {}\n\nSkill Level: {}\nDomain: {}\n\nPlease include:\n- Clear objectives\n- Required skills and qualifications\n- Expected deliverables\n- Success criteria",
            input.task_description, skill_level, domain
        ),
    }
}

// Helper function to estimate tokens (simplified)
fn estimate_tokens(text: &str) -> usize {
    // Rough estimate: ~4 characters per token
    text.chars().count() / 4
}

// Helper function to generate suggestions
fn generate_suggestions(input: &JobPromptInput) -> Vec<String> {
    let mut suggestions = Vec::new();

    if input.skill_level.is_none() {
        suggestions.push(
            "Consider specifying a skill level (beginner, intermediate, advanced, expert)"
                .to_string(),
        );
    }

    if input.domain.is_none() {
        suggestions.push("Specify a domain/industry for more targeted prompts".to_string());
    }

    if input.max_length.is_none() {
        suggestions.push("Set a max_length to control prompt size".to_string());
    }

    suggestions
}

export!(JobPromptTool);
