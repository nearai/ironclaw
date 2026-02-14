//! Testing harness for built tools.
//!
//! Provides automated testing of generated tools before registration,
//! ensuring they work correctly with various inputs.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::context::JobContext;
use crate::tools::tool::Tool;
use crate::tools::wasm::{Capabilities, WasmError, WasmToolRuntime, WasmToolWrapper};

/// Errors during testing.
#[derive(Debug, Error)]
pub enum TestError {
    #[error("Failed to load WASM module: {0}")]
    LoadError(#[from] WasmError),

    #[error("Test execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Test timed out after {0:?}")]
    Timeout(Duration),

    #[error("Output mismatch: expected {expected}, got {actual}")]
    OutputMismatch { expected: String, actual: String },

    #[error("Test assertion failed: {0}")]
    AssertionFailed(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// A single test case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    /// Name of the test.
    pub name: String,
    /// Description of what this test verifies.
    pub description: Option<String>,
    /// Input JSON to pass to the tool.
    pub input: serde_json::Value,
    /// Expected output (if exact match required).
    pub expected_output: Option<serde_json::Value>,
    /// Expected fields in output (partial match).
    pub expected_fields: Option<Vec<ExpectedField>>,
    /// Whether the tool should return an error.
    pub expect_error: bool,
    /// Expected error message substring (if expect_error is true).
    pub error_contains: Option<String>,
    /// Timeout for this specific test.
    pub timeout_ms: Option<u64>,
}

/// An expected field in the output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedField {
    /// JSON path to the field (e.g., "result.value" or "data[0].name").
    pub path: String,
    /// Expected value at that path.
    pub value: Option<serde_json::Value>,
    /// Just check that the field exists (if value is None).
    pub exists: bool,
}

/// Result of running a single test.
#[derive(Debug, Clone)]
pub struct TestResult {
    /// Name of the test.
    pub name: String,
    /// Whether the test passed.
    pub passed: bool,
    /// Duration of the test.
    pub duration: Duration,
    /// Error message if failed.
    pub error: Option<String>,
    /// Actual output from the tool.
    pub actual_output: Option<serde_json::Value>,
}

/// A suite of tests for a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSuite {
    /// Name of the test suite.
    pub name: String,
    /// Description of the suite.
    pub description: Option<String>,
    /// Test cases in the suite.
    pub tests: Vec<TestCase>,
    /// Default timeout for tests in milliseconds.
    pub default_timeout_ms: u64,
}

impl Default for TestSuite {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            description: None,
            tests: Vec::new(),
            default_timeout_ms: 5000,
        }
    }
}

impl TestSuite {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Add a test case.
    pub fn add_test(&mut self, test: TestCase) -> &mut Self {
        self.tests.push(test);
        self
    }

    /// Add a simple input/output test.
    pub fn add_io_test(
        &mut self,
        name: impl Into<String>,
        input: serde_json::Value,
        expected: serde_json::Value,
    ) -> &mut Self {
        self.tests.push(TestCase {
            name: name.into(),
            description: None,
            input,
            expected_output: Some(expected),
            expected_fields: None,
            expect_error: false,
            error_contains: None,
            timeout_ms: None,
        });
        self
    }

    /// Add a test that expects an error.
    pub fn add_error_test(
        &mut self,
        name: impl Into<String>,
        input: serde_json::Value,
        error_contains: impl Into<String>,
    ) -> &mut Self {
        self.tests.push(TestCase {
            name: name.into(),
            description: None,
            input,
            expected_output: None,
            expected_fields: None,
            expect_error: true,
            error_contains: Some(error_contains.into()),
            timeout_ms: None,
        });
        self
    }
}

/// Harness for running tests against WASM tools.
pub struct TestHarness {
    runtime: Arc<WasmToolRuntime>,
    capabilities: Capabilities,
    default_timeout: Duration,
}

impl TestHarness {
    pub fn new(runtime: Arc<WasmToolRuntime>) -> Self {
        Self {
            runtime,
            capabilities: Capabilities::none(),
            default_timeout: Duration::from_secs(5),
        }
    }

    /// Set capabilities for test execution.
    pub fn with_capabilities(mut self, caps: Capabilities) -> Self {
        self.capabilities = caps;
        self
    }

    /// Set default timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Run a test suite against a WASM file.
    pub async fn run_suite_file(
        &self,
        wasm_path: &Path,
        suite: &TestSuite,
    ) -> Result<Vec<TestResult>, TestError> {
        let bytes = tokio::fs::read(wasm_path).await?;
        self.run_suite_bytes(&bytes, suite).await
    }

    /// Run a test suite against WASM bytes.
    pub async fn run_suite_bytes(
        &self,
        wasm_bytes: &[u8],
        suite: &TestSuite,
    ) -> Result<Vec<TestResult>, TestError> {
        // Prepare the module
        let prepared = self.runtime.prepare(&suite.name, wasm_bytes, None).await?;

        // Create a tool wrapper for execution
        let tool = WasmToolWrapper::new(
            Arc::clone(&self.runtime),
            prepared,
            self.capabilities.clone(),
        );

        let mut results = Vec::with_capacity(suite.tests.len());

        for test in &suite.tests {
            let result = self.run_test(&tool, test, suite.default_timeout_ms).await;
            results.push(result);
        }

        Ok(results)
    }

    /// Run a single test case.
    async fn run_test(
        &self,
        tool: &WasmToolWrapper,
        test: &TestCase,
        default_timeout_ms: u64,
    ) -> TestResult {
        let timeout = Duration::from_millis(test.timeout_ms.unwrap_or(default_timeout_ms));
        let start = Instant::now();
        let ctx = JobContext::default();

        // Execute with timeout
        let exec_result = tokio::time::timeout(timeout, async {
            tool.execute(test.input.clone(), &ctx).await
        })
        .await;

        let duration = start.elapsed();

        match exec_result {
            Err(_) => TestResult {
                name: test.name.clone(),
                passed: false,
                duration,
                error: Some(format!("Test timed out after {:?}", timeout)),
                actual_output: None,
            },
            Ok(Err(e)) => {
                // Execution error
                if test.expect_error {
                    let error_str = e.to_string();
                    let matches = test
                        .error_contains
                        .as_ref()
                        .is_none_or(|expected| error_str.contains(expected));

                    TestResult {
                        name: test.name.clone(),
                        passed: matches,
                        duration,
                        error: if matches {
                            None
                        } else {
                            Some(format!(
                                "Expected error containing '{}', got: {}",
                                test.error_contains.as_deref().unwrap_or(""),
                                error_str
                            ))
                        },
                        actual_output: None,
                    }
                } else {
                    TestResult {
                        name: test.name.clone(),
                        passed: false,
                        duration,
                        error: Some(format!("Unexpected error: {}", e)),
                        actual_output: None,
                    }
                }
            }
            Ok(Ok(output)) => {
                let actual = output.result;

                // Check if output contains an error field
                if let Some(error_val) = actual.get("error") {
                    if test.expect_error {
                        let error_str = error_val.as_str().unwrap_or("");
                        let matches = test
                            .error_contains
                            .as_ref()
                            .is_none_or(|expected| error_str.contains(expected));

                        return TestResult {
                            name: test.name.clone(),
                            passed: matches,
                            duration,
                            error: if matches {
                                None
                            } else {
                                Some(format!(
                                    "Expected error containing '{}', got: {}",
                                    test.error_contains.as_deref().unwrap_or(""),
                                    error_str
                                ))
                            },
                            actual_output: Some(actual),
                        };
                    } else {
                        return TestResult {
                            name: test.name.clone(),
                            passed: false,
                            duration,
                            error: Some(format!("Unexpected error in output: {}", error_val)),
                            actual_output: Some(actual),
                        };
                    }
                }

                // Verify expected output
                if let Some(ref expected) = test.expected_output
                    && &actual != expected
                {
                    return TestResult {
                        name: test.name.clone(),
                        passed: false,
                        duration,
                        error: Some(format!(
                            "Output mismatch:\nExpected: {}\nActual: {}",
                            serde_json::to_string_pretty(expected).unwrap_or_default(),
                            serde_json::to_string_pretty(&actual).unwrap_or_default()
                        )),
                        actual_output: Some(actual),
                    };
                }

                // Verify expected fields
                if let Some(ref fields) = test.expected_fields {
                    for field in fields {
                        let field_value = get_json_path(&actual, &field.path);

                        if field.exists && field_value.is_none() {
                            return TestResult {
                                name: test.name.clone(),
                                passed: false,
                                duration,
                                error: Some(format!("Missing expected field: {}", field.path)),
                                actual_output: Some(actual),
                            };
                        }

                        if let Some(ref expected_value) = field.value
                            && field_value != Some(expected_value)
                        {
                            return TestResult {
                                name: test.name.clone(),
                                passed: false,
                                duration,
                                error: Some(format!(
                                    "Field '{}' mismatch: expected {:?}, got {:?}",
                                    field.path, expected_value, field_value
                                )),
                                actual_output: Some(actual),
                            };
                        }
                    }
                }

                TestResult {
                    name: test.name.clone(),
                    passed: true,
                    duration,
                    error: None,
                    actual_output: Some(actual),
                }
            }
        }
    }
}

/// Get a value from a JSON object by path (e.g., "foo.bar[0].baz").
fn get_json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;

    for segment in path.split('.') {
        // Handle array indexing like "items[0]"
        if let Some(bracket_pos) = segment.find('[') {
            let key = &segment[..bracket_pos];
            let index_str = &segment[bracket_pos + 1..segment.len() - 1];

            if !key.is_empty() {
                current = current.get(key)?;
            }

            let index: usize = index_str.parse().ok()?;
            current = current.get(index)?;
        } else {
            current = current.get(segment)?;
        }
    }

    Some(current)
}

/// Generate basic test cases for a tool based on its schema.
#[allow(dead_code)] // Public API for auto-generating test cases
pub fn generate_basic_tests(name: &str, input_schema: &serde_json::Value) -> TestSuite {
    let mut suite = TestSuite::new(format!("{}_basic_tests", name));
    suite.description = Some("Auto-generated basic tests".to_string());

    // Test with empty input
    suite.add_error_test("empty_input", serde_json::json!({}), "");

    // Test with null values for required fields
    if let Some(required) = input_schema.get("required").and_then(|r| r.as_array()) {
        let mut null_input = serde_json::Map::new();
        for req in required {
            if let Some(field_name) = req.as_str() {
                null_input.insert(field_name.to_string(), serde_json::Value::Null);
            }
        }
        suite.add_error_test(
            "null_required_fields",
            serde_json::Value::Object(null_input),
            "",
        );
    }

    // Test with valid minimal input (if we can construct it)
    if let Some(properties) = input_schema.get("properties").and_then(|p| p.as_object()) {
        let mut minimal_input = serde_json::Map::new();

        for (name, prop) in properties {
            if let Some(prop_type) = prop.get("type").and_then(|t| t.as_str()) {
                let value = match prop_type {
                    "string" => serde_json::Value::String("test".to_string()),
                    "integer" | "number" => serde_json::Value::Number(0.into()),
                    "boolean" => serde_json::Value::Bool(false),
                    "array" => serde_json::Value::Array(vec![]),
                    "object" => serde_json::Value::Object(serde_json::Map::new()),
                    _ => continue,
                };
                minimal_input.insert(name.clone(), value);
            }
        }

        suite.tests.push(TestCase {
            name: "minimal_valid_input".to_string(),
            description: Some("Test with minimal valid input".to_string()),
            input: serde_json::Value::Object(minimal_input),
            expected_output: None,
            expected_fields: None,
            expect_error: false,
            error_contains: None,
            timeout_ms: None,
        });
    }

    suite
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_json_path() {
        let json = serde_json::json!({
            "foo": {
                "bar": [1, 2, 3],
                "baz": "hello"
            }
        });

        assert_eq!(
            get_json_path(&json, "foo.baz"),
            Some(&serde_json::json!("hello"))
        );
        assert_eq!(
            get_json_path(&json, "foo.bar[0]"),
            Some(&serde_json::json!(1))
        );
        assert_eq!(
            get_json_path(&json, "foo.bar[2]"),
            Some(&serde_json::json!(3))
        );
        assert_eq!(get_json_path(&json, "foo.missing"), None);
    }

    #[test]
    fn test_test_suite_builder() {
        let mut suite = TestSuite::new("my_tests");
        suite
            .add_io_test(
                "basic",
                serde_json::json!({"x": 1}),
                serde_json::json!({"y": 2}),
            )
            .add_error_test("invalid", serde_json::json!({}), "required");

        assert_eq!(suite.tests.len(), 2);
        assert!(!suite.tests[0].expect_error);
        assert!(suite.tests[1].expect_error);
    }

    #[test]
    fn test_generate_basic_tests() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "count": {"type": "integer"}
            },
            "required": ["name"]
        });

        let suite = generate_basic_tests("my_tool", &schema);
        assert!(!suite.tests.is_empty());
    }
}
