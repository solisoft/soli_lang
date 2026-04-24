pub mod args;
mod commands;

use args::{parse_args, Command};

pub fn run() {
    let options = parse_args();

    match &options.command {
        Command::Repl => commands::run_repl(),
        Command::Run { file } => commands::run_file(file, &options),
        Command::Eval { code } => commands::run_eval(code, &options),
        Command::New { name, template } => commands::run_new(name, template.as_deref()),
        Command::Generate {
            scaffold_name,
            fields,
            folder,
        } => commands::run_generate(scaffold_name, fields, folder),
        Command::DbMigrate { action, folder } => commands::run_db_migrate(action, folder),
        Command::Serve {
            folder,
            port,
            dev_mode,
            workers,
            daemonize,
        } => commands::run_serve(folder, *port, *dev_mode, *workers, *daemonize),
        Command::Lint { path } => commands::run_lint(path.as_deref()),
        Command::Deploy { folder } => commands::run_deploy(folder.as_deref()),
        Command::Init => commands::run_init(),
        Command::Add {
            name,
            git,
            path,
            tag,
            branch,
            rev,
            version,
        } => commands::run_add(name, git, path, tag, branch, rev, version),
        Command::Remove { name } => commands::run_remove(name),
        Command::Install => commands::run_install(),
        Command::Update { name } => commands::run_update(name.as_deref()),
        Command::SelfUpdate => commands::run_self_update()
            .map_err(|e| e.to_string())
            .expect("Update failed"),
        Command::Login { registry, token } => {
            commands::run_login(registry.as_deref(), token.as_deref())
        }
        Command::Publish { registry } => commands::run_publish(registry.as_deref()),
        Command::Test {
            path,
            jobs,
            coverage_formats,
            coverage_min,
            no_coverage,
        } => commands::run_test(
            path.as_deref(),
            *jobs,
            coverage_formats,
            *coverage_min,
            *no_coverage,
        ),
        Command::Engine { action } => commands::run_engine(action),
    }
}
