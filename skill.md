---
name: dictionary-skill
version: 0.1.0
description: Defines any word with meaning, examples and synonyms
activation:
  keywords:
    - define
    - definition
    - meaning of
    - what does mean
    - synonym
    - word meaning
  tags:
    - dictionary
    - language
    - education
  max_context_tokens: 1000
---

## Dictionary Skill

When the user asks to define a word:

1. Give the word's definition clearly
2. Show part of speech (noun, verb, adjective)
3. Give 2 example sentences
4. List 3 synonyms
5. List 1 antonym
6. End with: "Want me to define another word?"
