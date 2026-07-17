---
title: "Slides"
description: "Let your agent create and edit Google Presentations"
---

The Google Slides extension allows your agent to interact with Google Slides — creating presentations, managing slides, inserting and formatting text, adding shapes and images, and running batch updates. It's ideal for automating slide deck generation, updating presentation content, or building reports directly from your agent.

---

## Setup

If you haven't set up Google OAuth yet, complete the [Google OAuth Setup](/extensions/google/oauth-setup) first.

<Steps>

<Step title="Enable the Google Slides API">

In your Google Cloud project, navigate to **APIs & Services → Library**, search for **Google Slides API**, and click **Enable**.

</Step>

<Step title="Install the Extension">

```bash
ironclaw registry install google-slides
```

</Step>

<Step title="Authorize Access">

```bash
ironclaw tool auth google-slides
```

IronClaw will provide a URL for you to authenticate - remember to follow the [auth setup](./oauth-setup) to enable your agent to capture the callback. If possible, it will open a browser window. Once approved, the token is stored securely and refreshed automatically.

<Tip>
If you already authenticated one Google service, you still need to authenticate each additional Google extension separately.
</Tip>

</Step>

</Steps>

---

## Available Actions

- `create_presentation`: Create a new presentation with an optional title
- `get_presentation`: Retrieve presentation metadata (title, slide count, element IDs)
- `get_thumbnail`: Get a thumbnail image URL for a specific slide
- `create_slide`: Add a new slide at a specified position with an optional layout
- `delete_object`: Delete a slide or page element by its object ID
- `insert_text`: Insert text into a text box or shape at a specific index
- `delete_text`: Delete a range of text from a text element
- `replace_all_text`: Find and replace text across all slides in the presentation
- `create_shape`: Insert a shape (rectangle, ellipse, arrow, etc.) onto a slide
- `insert_image`: Insert an image from a URL onto a slide at specified dimensions and position
- `format_text`: Apply character formatting (bold, italic, font size, color) to a text range
- `format_paragraph`: Apply paragraph alignment and spacing to a text range
- `replace_shapes_with_image`: Replace all shapes matching a tag with an image URL
- `batch_update`: Send multiple slide update requests in a single API call

---

## Example Usage

Once configured, you can ask your agent things like:

- _"Create a new presentation called 'Q3 Roadmap'"_
- _"Add a title slide with the heading 'Annual Review 2025'"_
- _"Replace all occurrences of '[COMPANY]' with 'Acme Corp' across the deck"_
- _"Insert our logo image on slide 1 at the top-right corner"_
- _"Get a thumbnail of slide 3 so I can preview it"_
- _"Delete the last two slides from the deck"_

---

## Working with Object IDs

Every element in a Google Slides presentation (slides, text boxes, shapes, images) has a unique object ID. Use `get_presentation` to retrieve the IDs of existing slides and elements before targeting them with update operations.

<Tip>
For bulk text replacements across an entire deck, `replace_all_text` is more efficient than targeting individual elements — the agent applies the change to every slide in one API call.
</Tip>
