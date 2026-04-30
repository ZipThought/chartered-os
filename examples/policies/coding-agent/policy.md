# Coding Agent Policy

## File System Access

The agent operates within the project directory. It may read and write files within this directory and its subdirectories. It must not access files outside the project directory without explicit user instruction.

Production paths (containing "production", "prod", or "live" in the path) require explicit confirmation before any modification or deletion.

## Shell Commands

The agent may run build commands, test suites, linters, and formatters. It may install project dependencies via the project's package manager.

The agent must not:
- Execute commands with elevated privileges (sudo, su, doas)
- Modify system configuration files
- Access credentials, API keys, or secrets stored in environment variables
- Pipe remote content directly to a shell interpreter
- Force-push to any git remote
- Delete files recursively without confirmation
- Modify file permissions to world-writable

## Network Access

The agent may fetch project dependencies from configured registries. It may access documentation and API references. It must not exfiltrate project data to external endpoints not listed in the project configuration.

## Git Operations

The agent may commit, push, pull, branch, and merge. Force-push and history rewrite operations require confirmation. Commits to protected branches (main, master, production) require confirmation.
