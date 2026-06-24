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
    github_request("POST", "/graphql", Some(req_body.to_string()))
}

fn validate_node_id(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("invalid_thread_id".to_string());
    }
    validate_input_length(value, "thread_id")
}
