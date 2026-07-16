pub mod args;
mod commands;
mod standalone;

use args::{parse_args, Command};

pub fn run() {
    // A standalone app executable (soli runtime + embedded bundle) boots the
    // app here and never reaches the soli CLI. Regular soli binaries have no
    // embedded payload and fall straight through.
    standalone::boot_if_standalone();

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
        Command::GenerateAuth { folder } => commands::run_generate_auth(folder),
        Command::GenerateMailer {
            name,
            actions,
            folder,
        } => commands::run_generate_mailer(name, actions, folder),
        Command::GenerateComponent { name, folder } => {
            commands::run_generate_component(name, folder)
        }
        Command::DbMigrate { action, folder } => commands::run_db_migrate(action, folder),
        Command::DbSeed { action, folder } => commands::run_db_seed(action, folder),
        Command::DbIndexes { folder } => commands::run_db_indexes(folder),
        Command::Routes { folder, grep, json } => {
            commands::run_routes(folder, grep.as_deref(), *json)
        }
        Command::Graph {
            folder,
            no_embed,
            database,
            dry_run,
            fresh,
            ext,
            exclude,
            config,
        } => commands::run_graph(
            folder,
            *no_embed,
            database.as_deref(),
            *dry_run,
            *fresh,
            ext.as_deref(),
            exclude.as_deref(),
            config.as_deref(),
        ),
        Command::GraphQuery {
            question,
            folder,
            database,
            limit,
            hops,
            path,
            json,
        } => commands::run_graph_query(
            question,
            folder,
            database.as_deref(),
            *limit,
            *hops,
            path.as_deref(),
            *json,
        ),
        Command::Serve {
            folder,
            port,
            dev_mode,
            workers,
            daemonize,
        } => commands::run_serve(folder, *port, *dev_mode, *workers, *daemonize),
        Command::Lint { paths } => commands::run_lint(paths),
        Command::Check { paths } => commands::run_check(paths),
        Command::Fmt {
            paths,
            check,
            stdin,
        } => commands::run_fmt(paths, *check, *stdin),
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
            paths,
            jobs,
            coverage_formats,
            coverage_min,
            no_coverage,
            show_uncovered,
            fail_on_n1,
        } => commands::run_test(
            paths,
            *jobs,
            coverage_formats,
            *coverage_min,
            *no_coverage,
            *show_uncovered,
            *fail_on_n1,
        ),
        Command::Engine { action } => commands::run_engine(action),
        Command::Lsp => commands::run_lsp(),
        Command::Build {
            folder,
            output,
            standalone,
            encrypt,
            protect,
            target,
        } => commands::run_build(
            folder,
            output.as_deref(),
            *standalone,
            *encrypt,
            *protect,
            target.as_deref(),
        ),
    }
}
