use crate::request::github_request;
use crate::validation::*;

pub(crate) fn search_repositories(
    query: &str,
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<String, String> {
    validate_input_length(query, "query")?;
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/search/repositories?q={}&per_page={}",
        url_encode_query(query),
        limit
    );
    append_search_params(&mut path, page, sort, order)?;
    github_request("GET", &path, None)
}

pub(crate) fn search_code(
    query: &str,
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<String, String> {
    validate_input_length(query, "query")?;
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/search/code?q={}&per_page={}",
        url_encode_query(query),
        limit
    );
    append_search_params(&mut path, page, sort, order)?;
    github_request("GET", &path, None)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn search_issues_pull_requests(
    query: Option<&str>,
    repository: Option<&str>,
    owner: Option<&str>,
    repo: Option<&str>,
    author: Option<&str>,
    assignee: Option<&str>,
    involves: Option<&str>,
    state: Option<&str>,
    issue_type: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<String, String> {
    let query = build_issue_search_query(
        query, repository, owner, repo, author, assignee, involves, state, issue_type,
    )?;
    validate_page(page)?;
    validate_limit(limit)?;
    validate_search_sort(sort)?;
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/search/issues?q={}&per_page={}",
        url_encode_query(&query),
        limit
    );
    append_search_params(&mut path, page, sort, order)?;
    github_request("GET", &path, None)
}
