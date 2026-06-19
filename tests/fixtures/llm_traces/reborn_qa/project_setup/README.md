Project-setup Reborn QA fixtures are recorded manually with live keys.

Do not fabricate fixtures in this directory. Record them with:

```bash
ANTHROPIC_API_KEY=... GITHUB_TOKEN=... \
  cargo test --test reborn_qa_project_setup record_ \
    -- --ignored --test-threads=1 --nocapture
```
