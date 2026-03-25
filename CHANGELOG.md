# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-03-25

### Added
- Real-time TUI dashboard for AI coding sessions
- Copilot CLI provider (watches `~/.copilot/session-state/`)
- Claude Code provider (watches `~/.claude/projects/`)
- Codex CLI provider (watches `~/.codex/sessions/`)
- Live event feed with chronological tool calls, messages, and warnings
- Token tracking with per-model cost estimation
- Tool call breakdown (name, count, success rate)
- File change audit (paths touched, read/write counts)
- Aggregate stats bar (tokens, cache rate, cost)
- Session list with active/inactive indicators
- CLI mode: `--list`, `--stats`, `--provider` flags
- File system watching via notify crate for real-time updates
