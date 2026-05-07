//! Agent-facing template strings — AGENTS.md, per-directory CLAUDE.md, .claude/

/// Tool-agnostic AGENTS.md stub at the project root.
pub const AGENTS_MD_TEMPLATE: &str = include_str!("AGENTS.md");

/// Per-directory CLAUDE.md files. Claude Code auto-loads these from cwd up to root.
pub const CLAUDE_CONTROLLERS_TEMPLATE: &str = include_str!("CLAUDE_controllers.md");
pub const CLAUDE_MODELS_TEMPLATE: &str = include_str!("CLAUDE_models.md");
pub const CLAUDE_VIEWS_TEMPLATE: &str = include_str!("CLAUDE_views.md");
pub const CLAUDE_MIDDLEWARE_TEMPLATE: &str = include_str!("CLAUDE_middleware.md");
pub const CLAUDE_TESTS_TEMPLATE: &str = include_str!("CLAUDE_tests.md");
pub const CLAUDE_MIGRATIONS_TEMPLATE: &str = include_str!("CLAUDE_migrations.md");

/// .claude/ directory: settings + project slash commands.
pub const CLAUDE_SETTINGS_TEMPLATE: &str = include_str!("claude_settings.json");
pub const CMD_SOLI_VERIFY_TEMPLATE: &str = include_str!("cmd_soli_verify.md");
pub const CMD_SOLI_TEST_TEMPLATE: &str = include_str!("cmd_soli_test.md");
pub const CMD_SOLI_RESOURCE_TEMPLATE: &str = include_str!("cmd_soli_resource.md");
