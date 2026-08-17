# Soli is built for AI

Public landing page: [/ai](/ai).

Soli gives coding agents the same thing it gives you: one way to do each thing, a short language, and a complete product stack.

## Four reasons

1. **Convention over configuration.** Standard names, folders, and `CLAUDE.md` so generated changes land closer to idiomatic Soli.
2. **Token efficiency.** A small language and no package/bundler tax. Agents spend context on the product.
3. **A contract, not a prompt.** `soli new` ships `AGENTS.md`, per-directory guides, `/soli-verify`, and a coverage gate.
4. **The one-person stack.** MVC, jobs, auth, LiveView, LLM, and retrieval in one binary.

## Agents on Soli

A model leaderboard (accuracy, median speed, mean tokens/cost, API recall)
will appear on `/ai` once a paid run is committed to `www/data/ai_evals.json`.
Until then the table is empty on purpose. Corpus: `evals/`. Methodology:
[Agents on Soli](/docs/development-tools/ai-evals).

## What to read next

- [Agents on Soli](/docs/development-tools/ai-evals) — corpus, grading, cost
- [AI agents in Soli projects](/docs/development-tools/ai-agents) — files that ship, verification loop, slash commands
- [AI builtins](/docs/builtins/ai) — `llm_generate`, embeddings
- [RAG / search](/docs/database/search#rag) — `.similar()`, `Model.rag`
- [Code graph](/docs/development-tools/graph) — `soli graph build` / `query`
- [Philosophy post](/docs/blog/ai-coding-agents)
