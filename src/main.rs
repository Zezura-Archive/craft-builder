use anyhow::Result;
use craft_builder::build::{self, deps};
use craft_builder::cli::{Mode, Options};
use craft_builder::config::Config;
use craft_builder::context::Ctx;
use craft_builder::gitenv::GitEnv;
use craft_builder::github::{self, access, release, repos};
use craft_builder::repo::{self, RepoItem, RepoKind};
use craft_builder::report::{self, Failures, Log};
use craft_builder::{clean, sync, token, tools};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::{JoinError, JoinSet};

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

    cfg.ensure_repos_listed()?;
    tools::require_commands(&cfg)?;
    let token = token::resolve()?;
    let gh = github::client(&token)?;
    let gh_user = access::check_access(&gh, &cfg.fork_owner).await?;
    cfg.prepare_dirs()?;

    let all_items = repos::read_repos(&gh, &cfg).await?;
    let libraries = of_kind(&all_items, RepoKind::Library);
    let apps = if cfg.only.is_empty() {
        of_kind(&all_items, RepoKind::App)
    } else {
        let selected = repo::select_only(&all_items, &cfg.only)?;
        println!("Selected {} of them with --only.", selected.len());
        selected
    };
    let items = [libraries.clone(), apps.clone()].concat();
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

    let mut synced = sync_phase(&ctx, &libraries, &mut failures).await;
    synced.extend(sync_phase(&ctx, &apps, &mut failures).await);

    let mut env = GitEnv::new(&ctx.cfg, &ctx.token);
    if ctx.cfg.build {
        build_phase(
            &ctx,
            &mut env,
            &all_items,
            items.len(),
            &synced,
            &mut failures,
        )
        .await;
    }

    Ok(report::summary(&ctx.cfg, items.len(), &failures))
}

fn of_kind(items: &[RepoItem], kind: RepoKind) -> Vec<RepoItem> {
    items.iter().filter(|i| i.kind == kind).cloned().collect()
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

/// Buffered output and success of one release, keyed by the repo's index in the synced list.
type ReleaseDone = (usize, String, bool);

/// Phase 2: git rewrites over all repos, then build each synced app serially.
/// With release enabled, each built app owned by `fork_owner` is released in the
/// background (one release at a time) while the next app builds. Release output
/// is printed between builds, never during one.
async fn build_phase(
    ctx: &Arc<Ctx>,
    env: &mut GitEnv,
    all_items: &[RepoItem],
    selected: usize,
    synced: &[RepoItem],
    failures: &mut Failures,
) {
    let mut log = Log::live();
    if !synced.is_empty() {
        log.out("");
        log.out(format!(
            "Synced {} of {selected} repo(s); building...",
            synced.len()
        ));
    }
    let mut sources = deps::setup_rewrites(all_items, &ctx.cfg.dest_dir, env, &mut log);

    let release_env = env.clone();
    let one_at_a_time = Arc::new(Semaphore::new(1));
    let mut releases = JoinSet::new();
    let mut released: Vec<Option<bool>> = vec![None; synced.len()];
    for (i, item) in synced.iter().enumerate() {
        if item.kind != RepoKind::App {
            continue;
        }
        print_finished_releases(&mut releases, &mut released);
        log.header(format_args!("build {}", item.repo));
        let outcome = tokio::task::block_in_place(|| {
            build::build_repo(&ctx.cfg, env, &mut sources, item, &mut log)
        });
        failures.extend(outcome.failures);
        if !ctx.cfg.release || !outcome.releasable {
            continue;
        }
        if item.is_owned_by(&ctx.cfg.fork_owner) {
            released[i] = Some(false);
            releases.spawn(release_task(
                (i, item.clone()),
                Arc::clone(ctx),
                release_env.clone(),
                Arc::clone(&one_at_a_time),
            ));
        } else {
            log.out("");
            log.out(format!(
                "Not releasing {}: owned by {}, not {}.",
                item.repo,
                item.owner(),
                ctx.cfg.fork_owner
            ));
        }
    }

    while let Some(joined) = releases.join_next().await {
        record_release(joined, &mut released);
    }
    for (item, state) in synced.iter().zip(released) {
        if state == Some(false) {
            failures.push(format!("{} (release)", item.repo));
        }
    }
}

/// Prints the output of releases that have already finished.
fn print_finished_releases(releases: &mut JoinSet<ReleaseDone>, released: &mut [Option<bool>]) {
    while let Some(joined) = releases.try_join_next() {
        record_release(joined, released);
    }
}

/// Prints a finished release's output and stores its success; a panicked task stays failed.
fn record_release(joined: Result<ReleaseDone, JoinError>, released: &mut [Option<bool>]) {
    if let Ok((i, block, ok)) = joined {
        report::print_block(&block);
        released[i] = Some(ok);
    }
}

/// Releases one built repo once no other release is running, buffering its output.
async fn release_task(
    (i, item): (usize, RepoItem),
    ctx: Arc<Ctx>,
    env: GitEnv,
    one_at_a_time: Arc<Semaphore>,
) -> ReleaseDone {
    let _permit = one_at_a_time
        .acquire_owned()
        .await
        .expect("semaphore is never closed");
    let mut log = Log::buffered();
    log.header(format_args!("release {}", item.repo));
    let ok = match release::release_repo(&ctx, &env, &item, &mut log).await {
        Ok(()) => true,
        Err(err) => {
            log.err(format!("Error: {err:#}"));
            log.err(format!("WARNING: failed to release {}", item.repo));
            false
        }
    };
    (i, log.into_buffer(), ok)
}
