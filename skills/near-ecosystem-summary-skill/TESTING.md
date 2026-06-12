# Testing Guide: NEAR Ecosystem Summary Skill

## Skill Name
near-ecosystem-summary-skill

## Test Purpose
This document explains how to test the NEAR Ecosystem Summary Skill inside IronClaw.

The goal is to confirm that the skill can:
- Accept NEAR ecosystem update text as input
- Generate a short structured summary
- Highlight IronClaw, NEAR AI, NEAR Protocol, and NEAR Intents mentions
- Create one human-style community comment

## Sample Test Input
NEAR AI released updates around IronClaw, encrypted agent execution, NEAR Intents, and ecosystem developer activity. The community is discussing how secure AI agents can help automate Web3 workflows.

## Expected Output
The skill should return a Markdown summary similar to:

## NEAR Ecosystem Daily Summary

### Key Updates
- NEAR AI released updates around IronClaw, encrypted agent execution, and NEAR Intents.
- Ecosystem developer activity is increasing.
- The community is discussing secure AI agents for Web3 workflow automation.

### Highlights
- IronClaw supports secure AI agent workflows.
- NEAR AI is advancing secure AI agent infrastructure.
- NEAR Intents can support cross-chain AI agent operations.

### Community Comment
NEAR's AI ecosystem is becoming more useful for real Web3 automation. IronClaw, encrypted execution, and NEAR Intents together make the agent stack feel more practical and secure.

## Actual Test Result
The skill was tested inside IronClaw using the sample input.

The output successfully included:
- Key updates
- Highlights
- A community-style comment
- Mentions of IronClaw, NEAR AI, and NEAR Intents

## Status
Test passed.
