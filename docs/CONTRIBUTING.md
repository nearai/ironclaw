# Contributing to IronClaw Docs

## Preview Locally

Install dependencies and start the development server:

```bash
npm install          # Install mintlify CLI (pinned version)
npx mintlify dev     # Starts local server at http://localhost:3000
```

## Write Content

- Pages are `.mdx` files in `docs/`
- Add new pages to the `navigation` groups in `docs.json`
- Use Mintlify components: `<Card>`, `<Steps>`, `<Tabs>`, `<Accordion>`, `<Warning>`, `<Note>`
- Code blocks: Use `bash` (not `sh`), `toml` (not `TOML`), `json` for correct syntax highlighting

## Deploy

Mintlify deploys automatically when changes to `docs/` or `docs.json` are merged to `main`. No manual deploy step required.

To force a redeploy: Go to mintlify.com/dashboard → your project → Redeploy.

## Update the Domain

CNAME: `docs` → Configured in Mintlify dashboard under Domains.
