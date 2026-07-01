# hacker-news.top_stories

Returns the current top Hacker News stories: rank, title, url, score, author,
and comment count. Optional `limit` (1-10, default 5).

Use this whenever the user asks what's on Hacker News, "HN", the tech front
page, or the top stories. The data is fixture/canned (no live Hacker News
feed), but the tool declares `news.ycombinator.com` as its egress host.
