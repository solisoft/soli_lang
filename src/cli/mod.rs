pub mod args;
mod commands;
mod macho;
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
        Command::GenerateOidcProvider { folder } => commands::run_generate_oidc_provider(folder),
        Command::GenerateOauth { provider, folder } => {
            commands::run_generate_oauth(provider, folder)
        }
        Command::GenerateMailer {
            name,
            actions,
            folder,
        } => commands::run_generate_mailer(name, actions, folder),
        Command::GenerateComponent { name, folder } => {
            commands::run_generate_component(name, folder)
        }
        Command::GenerateDevices { folder } => commands::run_generate_devices(folder),
        Command::GenerateClient {
            platform,
            url,
            package_id,
            scheme,
            app_name,
            team_id,
            fcm,
            folder,
        } => commands::run_generate_client(&solilang::scaffold::ClientOptions {
            platform: platform.to_string(),
            url: url.to_string(),
            package_id: package_id.to_string(),
            scheme: scheme.to_string(),
            app_name: app_name.to_string(),
            team_id: team_id.to_string(),
            fcm: *fcm,
            folder: folder.to_string(),
        }),
        Command::GenerateAppLinks {
            android_package,
            android_sha256,
            apple_app_id,
            paths,
            folder,
        } => commands::run_generate_app_links(
            android_package,
            android_sha256,
            apple_app_id,
            paths,
            folder,
        ),
        Command::GenerateOffline { folder } => commands::run_generate_offline(folder),
        Command::DbMigrate {
            action,
            folder,
            connection,
        } => commands::run_db_migrate(action, folder, connection.as_deref()),
        Command::DbSeed { action, folder } => commands::run_db_seed(action, folder),
        Command::DbImport { collections } => commands::run_db_import(collections),
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
            kind,
            json,
        } => commands::run_graph_query(
            question,
            folder,
            database.as_deref(),
            *limit,
            *hops,
            path.as_deref(),
            kind.as_deref(),
            *json,
        ),
        Command::Serve(serve) => commands::run_serve(serve),
        Command::Lint { paths } => commands::run_lint(paths),
        Command::Check { paths } => commands::run_check(paths),
        Command::Fmt {
            paths,
            check,
            stdin,
        } => commands::run_fmt(paths, *check, *stdin),
        Command::Cloud {
            action,
            folder,
            app,
            server,
            domains,
            dry_run,
        } => commands::run_cloud(
            action,
            folder,
            app.as_deref(),
            server.as_deref(),
            domains,
            *dry_run,
        ),
        Command::Deploy { folder } => commands::run_deploy(folder.as_deref()),
        Command::Env {
            action,
            folder,
            server,
            proxy_url,
        } => commands::run_env(action, folder, server.as_deref(), proxy_url.as_deref()),
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
        Command::UpdateDocs { folder } => commands::run_update_docs(folder),
        Command::UpdateKeygen => {
            if let Err(e) = run_update_keygen() {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Command::SignUpdate { manifest, key_path } => {
            if let Err(e) = run_sign_update(manifest, key_path) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
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
            browser,
            headed,
        } => commands::run_test(
            paths,
            *jobs,
            coverage_formats,
            *coverage_min,
            *no_coverage,
            *show_uncovered,
            *fail_on_n1,
            *browser,
            *headed,
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
            update_url,
            update_key,
        } => commands::run_build(
            folder,
            output.as_deref(),
            *standalone,
            *encrypt,
            *protect,
            target.as_deref(),
            update_url.as_deref(),
            update_key.as_deref(),
        ),
        Command::DesktopBuild {
            folder,
            app_id,
            app_name,
            output,
            db_binary,
            db_version,
            seed,
            protect,
            target,
            update_url,
            update_key,
        } => commands::desktop::run(commands::desktop::DesktopBuildArgs {
            folder,
            app_id,
            app_name: app_name.as_deref(),
            output: output.as_deref(),
            db_binary: db_binary.as_deref(),
            db_version: db_version.as_deref(),
            seed: seed.as_deref(),
            protect: *protect,
            target: target.as_deref(),
            update_url: update_url.as_deref(),
            update_key: update_key.as_deref(),
        }),
        Command::DesktopRegisterProtocol {
            exe,
            scheme,
            app_name,
        } => commands::desktop::run_register_protocol(exe, scheme, app_name),
    }
}

/// `soli update-keygen` — a P-256 keypair for signing update manifests.
fn run_update_keygen() -> Result<(), Box<dyn std::error::Error>> {
    let (private_pem, public_b64) = solilang::update::generate_keypair()?;
    println!("{}", private_pem.trim());
    println!();
    println!("# Public key — pass to `soli build --update-key`:");
    println!("{}", public_b64);
    Ok(())
}

/// `soli sign-update <latest.json> --key <private.pem>`.
fn run_sign_update(manifest: &str, key_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let key_pem = std::fs::read_to_string(key_path)
        .map_err(|e| format!("cannot read key {}: {}", key_path, e))?;
    solilang::update::sign_manifest_file(std::path::Path::new(manifest), &key_pem)?;
    println!("Signed {}", manifest);
    Ok(())
}
