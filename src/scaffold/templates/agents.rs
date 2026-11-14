//! Agent-facing template strings — AGENTS.md, per-directory CLAUDE.md, .claude/

/// Tool-agnostic AGENTS.md stub at the project root.
pub const AGENTS_MD_TEMPLATE: &str = include_str!("AGENTS.md");

/// Per-directory CLAUDE.md files. Claude Code auto-loads these from cwd up to root.
pub const CLAUDE_CONTROLLERS_TEMPLATE: &str = include_str!("CLAUDE_controllers.md");
pub const CLAUDE_MODELS_TEMPLATE: &str = include_str!("CLAUDE_models.md");
pub const CLAUDE_CONCERNS_TEMPLATE: &str = include_str!("CLAUDE_concerns.md");
pub const CLAUDE_VIEWS_TEMPLATE: &str = include_str!("CLAUDE_views.md");
pub const CLAUDE_MIDDLEWARE_TEMPLATE: &str = include_str!("CLAUDE_middleware.md");
pub const CLAUDE_PUBLIC_JS_TEMPLATE: &str = include_str!("CLAUDE_public_js.md");
pub const CLAUDE_TESTS_TEMPLATE: &str = include_str!("CLAUDE_tests.md");
pub const CLAUDE_MIGRATIONS_TEMPLATE: &str = include_str!("CLAUDE_migrations.md");

/// .claude/ directory: settings + project slash commands.
pub const CLAUDE_SETTINGS_TEMPLATE: &str = include_str!("claude_settings.json");
pub const CMD_SOLI_VERIFY_TEMPLATE: &str = include_str!("cmd_soli_verify.md");
pub const CMD_SOLI_TEST_TEMPLATE: &str = include_str!("cmd_soli_test.md");
pub const CMD_SOLI_RESOURCE_TEMPLATE: &str = include_str!("cmd_soli_resource.md");

/// The solidb skill, written to `.claude/skills/solidb/`: SDBQL and the
/// SoliDB HTTP API as observed on a running server. A copy of
/// `~/.claude/skills/solidb/`, written only by
/// `scripts/sync-solidb-skill.sh`.
pub static SOLIDB_SKILL: include_dir::Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/scaffold/templates/skills/solidb");

/// Where an app keeps it.
pub const SOLIDB_SKILL_PATH: &str = ".claude/skills/solidb";
