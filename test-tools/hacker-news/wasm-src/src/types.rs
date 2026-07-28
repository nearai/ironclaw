use serde::{Deserialize, Serialize};

/// Optional input for `hacker-news.top_stories`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct TopStoriesInput {
    /// How many stories to return (1-10). Defaults to 5.
    pub limit: Option<u32>,
}

/// A single Hacker News story (canned fixture data).
#[derive(Debug, Clone, Serialize)]
pub struct Story {
    pub rank: u32,
    pub title: String,
    pub url: String,
    pub score: u32,
    pub by: String,
    pub comments: u32,
}

/// The `hacker-news.top_stories` response.
#[derive(Debug, Serialize)]
pub struct TopStories {
    pub stories: Vec<Story>,
    pub as_of: String,
    pub data_source: String,
}
