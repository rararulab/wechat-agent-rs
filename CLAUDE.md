# CLAUDE.md — wechat-agent-rs 开发指南

## Communication
- 用中文与用户交流

## Project Identity

wechat-agent-rs 是一个用 Rust 编写的微信企业号 Agent SDK 库（iLink Bot），从 [frostming/weixin-agent-sdk](https://github.com/frostming/weixin-agent-sdk) 移植而来。本项目是一个 library crate，提供微信 Agent 的消息收发、事件处理等核心功能。

## Development Workflow

所有变更——无论多小——都必须遵循 issue → worktree → PR → merge 流程，无一例外。

@docs/guides/workflow.md
@docs/guides/commit-style.md

## Code Quality

@docs/guides/rust-style.md
@docs/guides/code-comments.md

## Guardrails

@docs/guides/anti-patterns.md
