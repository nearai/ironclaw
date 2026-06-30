use crate::request::github_request;
use crate::validation::*;

const REVIEW_THREADS_QUERY: &str = r#"
query($owner: String!, $repo: String!, $number: Int!, $first: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      reviewThreads(first: $first, after: $after) {
        nodes {
          id
          isResolved
        }
        pageInfo {
          hasNextPage
          endCursor
        }
      }
    }
  }
}
"#;

const RESOLVE_REVIEW_THREAD_MUTATION: &str = r#"
mutation($threadId: ID!) {
  resolveReviewThread(input: { threadId: $threadId }) {
    thread {
      id
      isResolved
    }
  }
}
"#;

const UNRESOLVE_REVIEW_THREAD_MUTATION: &str = r#"
mutation($threadId: ID!) {
  unresolveReviewThread(input: { threadId: $threadId }) {
    thread {
      id
      isResolved
    }
  }
}
"#;

pub(crate) fn list_pull_request_review_threads(
    owner: &str,
    repo: &str,
    pr_number: u32,
    first: Option<u32>,
    after: Option<&str>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let first = first.unwrap_or(30);
    if !(1..=100).contains(&first) {
        return Err("invalid_limit".to_string());
    }
    if let Some(after) = after {
        validate_input_length(after, "after")?;
    }

    let variables = serde_json::json!({
        "owner": owner,
        "repo": repo,
        "number": pr_number,
        "first": first,
        "after": after,
    });
    graphql_request(REVIEW_THREADS_QUERY, variables)
}

pub(crate) fn resolve_review_thread(thread_id: &str) -> Result<String, String> {
    review_thread_mutation(RESOLVE_REVIEW_THREAD_MUTATION, thread_id)
}

pub(crate) fn unresolve_review_thread(thread_id: &str) -> Result<String, String> {
    review_thread_mutation(UNRESOLVE_REVIEW_THREAD_MUTATION, thread_id)
}

fn review_thread_mutation(query: &str, thread_id: &str) -> Result<String, String> {
    validate_node_id(thread_id)?;
    graphql_request(
        query,
        serde_json::json!({
            "threadId": thread_id,
        }),
    )
}

fn graphql_request(query: &str, variables: serde_json::Value) -> Result<String, String> {
    let req_body = serde_json::json!({
        "query": query,
        "variables": variables,
    });
    let response = github_request("POST", "/graphql", Some(req_body.to_string()))?;
    parse_graphql_response(&response)
}

fn parse_graphql_response(response: &str) -> Result<String, String> {
    let response_json: serde_json::Value = serde_json::from_str(response)
        .map_err(|err| format!("github_api_invalid_json: graphql response parse failed: {err}"))?;

    if let Some(errors) = response_json
        .get("errors")
        .and_then(|errors| errors.as_array())
    {
        if !errors.is_empty() {
            return Err(format!(
                "github_graphql_errors: {}",
                serde_json::to_string(errors).unwrap_or_else(|err| format!(
                    r#"[{{"message":"failed to serialize graphql errors: {err}"}}]"#
                ))
            ));
        }
    }

    Ok(response.to_string())
}

fn validate_node_id(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("invalid_thread_id".to_string());
    }
    validate_input_length(value, "thread_id")
}

#[cfg(test)]
mod tests {
    use super::list_pull_request_review_threads;
    use crate::request::test_support;

    #[test]
    fn list_pull_request_review_threads_returns_successful_graphql_body() {
        test_support::set_response(Ok(serde_json::json!({
            "data": {
                "repository": {
                    "pullRequest": {
                        "reviewThreads": {
                            "nodes": [],
                            "pageInfo": {
                                "hasNextPage": false,
                                "endCursor": null
                            }
                        }
                    }
                }
            }
        })
        .to_string()));

        let response = list_pull_request_review_threads("nearai", "ironclaw", 17, Some(30), None)
            .expect("graphQL response should succeed");

        assert!(response.contains("\"reviewThreads\""));
        let requests = test_support::requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/graphql");
    }

    #[test]
    fn list_pull_request_review_threads_surfaces_graphql_errors() {
        test_support::set_response(Ok(serde_json::json!({
            "data": {
                "repository": null
            },
            "errors": [
                {
                    "message": "review threads failed",
                    "path": ["repository", "pullRequest", "reviewThreads"]
                }
            ]
        })
        .to_string()));

        let error = list_pull_request_review_threads("nearai", "ironclaw", 17, Some(30), None)
            .expect_err("graphQL errors should fail the tool");

        assert!(error.starts_with("github_graphql_errors: "));
        assert!(error.contains("review threads failed"));
        assert!(error.contains("reviewThreads"));
    }
}
