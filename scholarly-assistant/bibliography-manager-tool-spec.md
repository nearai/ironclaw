# Bibliography Manager WASM Tool Specification

## Purpose

Manage academic citations and references with support for multiple citation styles (BibTeX, APA, MLA, Chicago).

## Directory Structure

```
tools-src/bibliography/
├── Cargo.toml
├── bibliography.capabilities.json
├── src/
│   ├── lib.rs
│   ├── bibtex.rs
│   ├── styles.rs
│   └── storage.rs
└── README.md
```

## Features

1. **Citation Storage** - Store and retrieve citations in workspace
2. **Format Conversion** - Convert between BibTeX, JSON, and formatted citations
3. **Style Formatting** - Format citations in APA, MLA, Chicago styles
4. **Deduplication** - Detect and merge duplicate entries
5. **Search/Filter** - Find citations by author, year, title, keywords
6. **Export** - Generate formatted bibliographies

## Cargo.toml

```toml
[package]
name = "bibliography-tool"
version = "0.1.0"
edition = "2021"
description = "Academic bibliography and citation manager"
license = "MIT OR Apache-2.0"
publish = false

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
wit-bindgen = "0.41.0"

[lib]
crate-type = ["cdylib"]

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1

[workspace]
```

## Capabilities File

```json
{
  "name": "bibliography",
  "version": "0.1.0",
  "description": "Manage academic citations and bibliographies",
  "http_endpoints": [],
  "secrets": [],
  "workspace_paths": {
    "read": ["bibliography/**"],
    "write": ["bibliography/**"]
  },
  "tools": [
    {
      "name": "bib_add",
      "description": "Add a citation to the bibliography",
      "parameters": {
        "type": "object",
        "properties": {
          "entry": {
            "type": "object",
            "description": "Citation entry (BibTeX or structured format)"
          },
          "collection": {
            "type": "string",
            "description": "Collection name (default: 'default')",
            "default": "default"
          }
        },
        "required": ["entry"]
      }
    },
    {
      "name": "bib_format",
      "description": "Format citations in a specific style",
      "parameters": {
        "type": "object",
        "properties": {
          "keys": {
            "type": "array",
            "items": {"type": "string"},
            "description": "Citation keys to format (empty = all)"
          },
          "style": {
            "type": "string",
            "enum": ["bibtex", "apa", "mla", "chicago"],
            "description": "Citation style",
            "default": "apa"
          },
          "collection": {
            "type": "string",
            "description": "Collection name",
            "default": "default"
          }
        }
      }
    },
    {
      "name": "bib_search",
      "description": "Search citations by author, title, year, etc.",
      "parameters": {
        "type": "object",
        "properties": {
          "query": {
            "type": "string",
            "description": "Search query"
          },
          "author": {
            "type": "string",
            "description": "Filter by author name"
          },
          "year": {
            "type": "string",
            "description": "Filter by year or year range (e.g., '2020', '2020-2024')"
          },
          "collection": {
            "type": "string",
            "description": "Collection name",
            "default": "default"
          }
        }
      }
    },
    {
      "name": "bib_export",
      "description": "Export bibliography to a file",
      "parameters": {
        "type": "object",
        "properties": {
          "format": {
            "type": "string",
            "enum": ["bibtex", "json", "apa", "mla", "chicago"],
            "description": "Export format",
            "default": "bibtex"
          },
          "output_path": {
            "type": "string",
            "description": "Output file path (relative to workspace)"
          },
          "collection": {
            "type": "string",
            "description": "Collection name",
            "default": "default"
          }
        },
        "required": ["output_path"]
      }
    },
    {
      "name": "bib_list",
      "description": "List all citations in the bibliography",
      "parameters": {
        "type": "object",
        "properties": {
          "collection": {
            "type": "string",
            "description": "Collection name",
            "default": "default"
          },
          "sort_by": {
            "type": "string",
            "enum": ["author", "year", "title", "added"],
            "description": "Sort order",
            "default": "author"
          }
        }
      }
    },
    {
      "name": "bib_remove",
      "description": "Remove a citation from the bibliography",
      "parameters": {
        "type": "object",
        "properties": {
          "key": {
            "type": "string",
            "description": "Citation key to remove"
          },
          "collection": {
            "type": "string",
            "description": "Collection name",
            "default": "default"
          }
        },
        "required": ["key"]
      }
    }
  ]
}
```

## Data Structure

Citations stored in `workspace/bibliography/{collection}.json`:

```json
{
  "version": "0.1.0",
  "collection": "default",
  "entries": {
    "smith2023transformer": {
      "key": "smith2023transformer",
      "type": "article",
      "title": "Attention Is All You Need",
      "authors": ["Smith, John", "Doe, Jane"],
      "year": 2023,
      "journal": "Nature Machine Intelligence",
      "volume": "5",
      "pages": "123-145",
      "doi": "10.1038/s42256-023-00123-4",
      "url": "https://example.com/paper.pdf",
      "abstract": "...",
      "keywords": ["transformers", "attention", "deep learning"],
      "added_date": "2024-01-15T10:30:00Z",
      "read_status": "read",
      "notes": "Important foundational paper",
      "bibtex": "@article{smith2023transformer,\n  title={...},\n  author={...},\n  ...\n}"
    }
  }
}
```

## Implementation Sketch

```rust
// src/lib.rs
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
struct Bibliography {
    version: String,
    collection: String,
    entries: HashMap<String, CitationEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CitationEntry {
    key: String,
    #[serde(rename = "type")]
    entry_type: String,  // article, book, inproceedings, etc.
    title: String,
    authors: Vec<String>,
    year: u32,
    journal: Option<String>,
    booktitle: Option<String>,
    volume: Option<String>,
    pages: Option<String>,
    doi: Option<String>,
    url: Option<String>,
    abstract_text: Option<String>,
    keywords: Vec<String>,
    added_date: String,
    read_status: String,  // to-read, reading, read
    notes: Option<String>,
    bibtex: Option<String>,
}

// Format citation in APA style
fn format_apa(entry: &CitationEntry) -> String {
    let authors = entry.authors.join(", ");
    let title = &entry.title;
    let year = entry.year;

    match entry.entry_type.as_str() {
        "article" => {
            let journal = entry.journal.as_deref().unwrap_or("Unknown");
            let volume = entry.volume.as_deref().unwrap_or("");
            let pages = entry.pages.as_deref().unwrap_or("");

            format!(
                "{} ({}). {}. *{}*, *{}*, {}.",
                authors, year, title, journal, volume, pages
            )
        }
        "book" => {
            format!("{} ({}). *{}*.", authors, year, title)
        }
        _ => {
            format!("{} ({}). {}.", authors, year, title)
        }
    }
}

// Format citation in MLA style
fn format_mla(entry: &CitationEntry) -> String {
    let authors = if entry.authors.is_empty() {
        "Unknown".to_string()
    } else {
        let first = &entry.authors[0];
        if entry.authors.len() > 1 {
            format!("{}, et al.", first)
        } else {
            first.clone()
        }
    };

    format!(
        "{}. \"{}.\" {} ({})",
        authors, entry.title, entry.journal.as_deref().unwrap_or("Unknown"), entry.year
    )
}

// Format citation in Chicago style
fn format_chicago(entry: &CitationEntry) -> String {
    let authors = entry.authors.join(", ");
    format!(
        "{}. \"{}.\" {} {} ({}): {}.",
        authors,
        entry.title,
        entry.journal.as_deref().unwrap_or(""),
        entry.volume.as_deref().unwrap_or(""),
        entry.year,
        entry.pages.as_deref().unwrap_or("")
    )
}

// Search entries by query
fn search_entries(
    bib: &Bibliography,
    query: Option<&str>,
    author: Option<&str>,
    year: Option<&str>,
) -> Vec<&CitationEntry> {
    let mut results: Vec<&CitationEntry> = bib.entries.values().collect();

    if let Some(q) = query {
        let q_lower = q.to_lowercase();
        results.retain(|entry| {
            entry.title.to_lowercase().contains(&q_lower)
                || entry.authors.iter().any(|a| a.to_lowercase().contains(&q_lower))
                || entry.keywords.iter().any(|k| k.to_lowercase().contains(&q_lower))
        });
    }

    if let Some(a) = author {
        let a_lower = a.to_lowercase();
        results.retain(|entry| {
            entry.authors.iter().any(|author| author.to_lowercase().contains(&a_lower))
        });
    }

    if let Some(y) = year {
        if y.contains('-') {
            // Year range
            let parts: Vec<&str> = y.split('-').collect();
            if parts.len() == 2 {
                if let (Ok(start), Ok(end)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                    results.retain(|entry| entry.year >= start && entry.year <= end);
                }
            }
        } else {
            // Single year
            if let Ok(target_year) = y.parse::<u32>() {
                results.retain(|entry| entry.year == target_year);
            }
        }
    }

    results
}

// Load bibliography from workspace
fn load_bibliography(collection: &str) -> Result<Bibliography, String> {
    let path = format!("bibliography/{}.json", collection);
    // Use workspace read/write APIs from WIT
    // For now, return empty
    Ok(Bibliography {
        version: "0.1.0".to_string(),
        collection: collection.to_string(),
        entries: HashMap::new(),
    })
}

// Save bibliography to workspace
fn save_bibliography(bib: &Bibliography) -> Result<(), String> {
    let path = format!("bibliography/{}.json", bib.collection);
    // Use workspace write API
    Ok(())
}

// Main tool execution dispatch
fn execute_tool(tool_name: &str, params: serde_json::Value) -> Result<String, String> {
    match tool_name {
        "bib_add" => {
            // Add citation implementation
            Ok("Citation added successfully".to_string())
        }
        "bib_format" => {
            // Format citations implementation
            Ok("Citations formatted".to_string())
        }
        "bib_search" => {
            // Search implementation
            Ok("Search results".to_string())
        }
        "bib_export" => {
            // Export implementation
            Ok("Bibliography exported".to_string())
        }
        "bib_list" => {
            // List implementation
            Ok("Citations listed".to_string())
        }
        "bib_remove" => {
            // Remove implementation
            Ok("Citation removed".to_string())
        }
        _ => Err(format!("Unknown tool: {}", tool_name)),
    }
}
```

## Usage Examples

### Add a Citation

```
> Add this paper to my bibliography:
  Title: Attention Is All You Need
  Authors: Vaswani et al.
  Year: 2017
  Journal: NeurIPS
```

### Format Citations

```
> Format all citations in APA style
> Generate APA bibliography for papers from 2020-2024
```

### Search

```
> Search bibliography for papers by "Smith"
> Find citations about "transformers" from 2020-2024
```

### Export

```
> Export bibliography to bibliography/references.bib in BibTeX format
> Generate APA-formatted bibliography for thesis
```

## Integration with Literature Review Workflow

1. **Paper Discovery** → Add to bibliography automatically
2. **Paper Reading** → Update read status and notes
3. **Writing** → Generate formatted citations
4. **Export** → Create bibliography for thesis/papers

## Future Enhancements

1. **Automatic BibTeX generation** from DOI
2. **Citation style templates** - Custom styles
3. **Duplicate detection** - Fuzzy matching on title/authors
4. **Import from BibTeX files**
5. **Integration with Semantic Scholar** - Fetch metadata
6. **Citation graph visualization**
7. **Shared collections** - Collaborate on bibliographies

---

*Specification by Andy*
*For Joaquín's PhD project*
