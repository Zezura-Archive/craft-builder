use anyhow::Result;
use craft_builder::build::{self, deps};
use craft_builder::cli::{Mode, Options};
use craft_builder::config::Config;
use craft_builder::context::Ctx;
use craft_builder::gitenv::GitEnv;
use craft_builder::github::{self, access, release, stars};
use craft_builder::repo::{self, RepoItem};
use craft_builder::report::{self, Failures, Log};
use craft_builder::{clean, sync, token, tools};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<ExitCode> {
    let opts = Options::parse();
    let mut cfg = Config::from_env(&opts)?;
    if opts.mode == Mode::Clean {
        return Ok(if clean::run(&cfg) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }

    tools::require_commands(&cfg)?;
    let token = token::resolve()?;
    let gh = github::client(&token)?;
    let gh_user = access::check_access(&gh, &cfg.fork_owner).await?;
    cfg.prepare_dirs()?;

    let all_items = stars::read_star_lists(&gh, &cfg).await?;
    let items = if cfg.only.is_empty() {
        all_items.clone()
    } else {
        let selected = repo::select_only(&all_items, &cfg.only)?;
        println!("Selected {} of them with --only.", selected.len());
        selected
    };
    repo::ensure_unique_names(&items)?;
    println!(
        "Syncing {} repo(s) into {}",
        items.len(),
        cfg.dest_dir.display()
    );
    if items.is_empty() {
        println!("Nothing to do.");
        return Ok(ExitCode::SUCCESS);
    }
    access::check_fork_write_access(&items, &gh_user, &cfg.fork_owner, cfg.release)?;

    let ctx = Arc::new(Ctx {
        cfg,
        gh,
        token,
        gh_user,
    });
    let mut failures = Failures::new();

    let synced = sync_phase(&ctx, &items, &mut failures).await;

    let mut env = GitEnv::new(&ctx.cfg, &ctx.token);
    let built = if ctx.cfg.build {
        build_phase(
            &ctx,
            &mut env,
            &all_items,
            items.len(),
            &synced,
            &mut failures,
        )
    } else {
        Vec::new()
    };
    if ctx.cfg.release {
        release_phase(&ctx, &env, &built, &mut failures).await;
    }

    Ok(report::summary(&ctx.cfg, items.len(), &failures))
}

/// Phase 1: sync every repo (`-j` at a time); returns the synced ones in list order.
async fn sync_phase(ctx: &Arc<Ctx>, items: &[RepoItem], failures: &mut Failures) -> Vec<RepoItem> {
    let mut results: Vec<Result<(), String>> = Vec::with_capacity(items.len());
    if ctx.cfg.jobs == 1 {
        for item in items {
            let mut log = Log::live();
            log.header(&item.repo);
            results.push(sync::sync_repo(ctx, item, &mut log).await);
        }
    } else {
        println!(
            "Syncing up to {} repos at a time; each repo's output appears when it finishes.",
            ctx.cfg.jobs
        );
        let permits = Arc::new(Semaphore::new(ctx.cfg.jobs));
        let mut tasks = JoinSet::new();
        for (i, item) in items.iter().cloned().enumerate() {
            let (ctx, permits) = (Arc::clone(ctx), Arc::clone(&permits));
            tasks.spawn(async move {
                let _permit = permits
                    .acquire_owned()
                    .await
                    .expect("semaphore is never closed");
                let mut log = Log::buffered();
                log.header(&item.repo);
                let result = sync::sync_repo(&ctx, &item, &mut log).await;
                report::print_block(&log.into_buffer());
                (i, result)
            });
        }
        let mut slots: Vec<Option<Result<(), String>>> = vec![None; items.len()];
        while let Some(joined) = tasks.join_next().await {
            if let Ok((i, result)) = joined {
                slots[i] = Some(result);
            }
        }
        results.extend(slots.into_iter().zip(items).map(|(slot, item)| {
            slot.unwrap_or_else(|| Err(format!("{} (sync: aborted)", item.repo)))
        }));
    }

    let mut synced = Vec::new();
    for (item, result) in items.iter().zip(results) {
        match result {
            Ok(()) => synced.push(item.clone()),
            Err(failure) => failures.push(failure),
        }
    }
    synced
}

/// Phase 2: git rewrites over all repos, then build each synced repo serially.
/// Returns the fully built binary repos.
fn build_phase(
    ctx: &Ctx,
    env: &mut GitEnv,
    all_items: &[RepoItem],
    selected: usize,
    synced: &[RepoItem],
    failures: &mut Failures,
) -> Vec<RepoItem> {
    let mut log = Log::live();
    if !synced.is_empty() {
        log.out("");
        log.out(format!(
            "Synced {} of {selected} repo(s); building...",
            synced.len()
        ));
    }
    let mut sources = deps::setup_rewrites(all_items, &ctx.cfg.dest_dir, env, &mut log);

    let mut built = Vec::new();
    for item in synced {
        log.header(format_args!("build {}", item.repo));
        let outcome = build::build_repo(&ctx.cfg, env, &mut sources, item, &mut log);
        failures.extend(outcome.failures);
        if outcome.releasable {
            built.push(item.clone());
        }
    }
    built
}

/// Phase 3: release the built repos owned by FORK_OWNER.
async fn release_phase(ctx: &Ctx, env: &GitEnv, built: &[RepoItem], failures: &mut Failures) {
    let mut log = Log::live();
    for item in built {
        if !item.is_owned_by(&ctx.cfg.fork_owner) {
            log.out("");
            log.out(format!(
                "Not releasing {}: owned by {}, not {}.",
                item.repo,
                item.owner(),
                ctx.cfg.fork_owner
            ));
            continue;
        }
        log.header(format_args!("release {}", item.repo));
        if let Err(err) = release::release_repo(ctx, env, item, &mut log).await {
            log.err(format!("Error: {err:#}"));
            log.err(format!("WARNING: failed to release {}", item.repo));
            failures.push(format!("{} (release)", item.repo));
        }
    }
}
