---
title: "Web Search"
description: "Let your agent search the web"
icon: web-search
---

The Web Search tool allows your agent to use the [Brave Search API]() search the web for up-to-date information, making it ideal for answering questions about current events, finding specific data, or gathering general information.

---

## Setup


<Steps>

<Step title="Get a Brave Search API Key">
To use the Web Search tool, you need to obtain an API key from Brave Search. You can get one by signing up at https://api-dashboard.search.brave.com 

<Info>

As of the time of writing, Brave Search API offers 5$ of free credits per month on their basic plan, which is more than enough for testing and small-scale use.

</Info>


</Step>

<Step title="Install the Web Search Extension">

To install the Web Search extension, run the following command in your terminal:

```bash
ironclaw registry install web-search
```

</Step>

<Step title="Configure the API Key">

After installing the extension, you need to configure your Brave Search API key in IronClaw. You can do this by running:

```bash
ironclaw tool auth web-search
```

Then follow the prompts to enter your API key.

</Step>

</Steps>