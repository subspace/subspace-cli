use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use color_eyre::eyre::{Report, Result};
use futures::prelude::*;
use indicatif::{ProgressBar, ProgressStyle};
use single_instance::SingleInstance;
use tracing::instrument;

use subspace_sdk::Farmer;
use subspace_sdk::{chain_spec, Node, PlotDescription, PublicKey};

use crate::config::parse_config;
use crate::summary::{create_summary_file, get_farmed_block_count, update_summary};
use crate::utils::{install_tracing, node_directory_getter};

#[derive(Debug)]
pub(crate) struct FarmingArgs {
    reward_address: PublicKey,
    node: Node,
    plot: PlotDescription,
}

#[instrument]
pub(crate) async fn farm(is_verbose: bool) -> Result<()> {
    install_tracing(is_verbose);
    color_eyre::install()?;

    let instance = SingleInstance::new("subspaceFarmer")
        .map_err(|_| Report::msg("Cannot take the instance lock from the OS! Aborting..."))?;
    if !instance.is_single() {
        return Err(Report::msg(
            "It seems like there is already a farming instance running. Aborting...",
        ));
    }

    println!("Starting node ... (this might take up to couple of minutes)");
    let args = prepare_farming().await?;
    println!("Node started successfully!");

    create_summary_file().await?;

    println!("Starting farmer ...");
    let (farmer, _node) = start_farming(args).await?;
    println!("Farmer started successfully!");

    if !is_verbose {
        let is_initial_progress_finished = Arc::new(AtomicBool::new(false));
        let sector_size_bytes = farmer.get_info().await.map_err(Report::msg)?.sector_size;
        let farmer_clone = farmer.clone();
        let finished_flag = is_initial_progress_finished.clone();

        // initial plotting progress subscriber
        tokio::spawn(async move {
            for (plot_id, plot) in farmer_clone.iter_plots().await.enumerate() {
                println!(
                    "Initial plotting for plot: #{plot_id} ({})",
                    plot.directory().display()
                );
                let progress_bar = plotting_progress_bar(plot.allocated_space().as_u64());
                plot.subscribe_initial_plotting_progress()
                    .await
                    .for_each(|progress| {
                        let pb_clone = progress_bar.clone();
                        async move {
                            let current_bytes = progress.current_sector * sector_size_bytes;
                            pb_clone.set_position(current_bytes);
                        }
                    })
                    .await;
                progress_bar.set_style(
                    ProgressStyle::with_template(
                        "{spinner} [{elapsed_precise}] {percent}% [{bar:40.cyan/blue}]
                  ({bytes}/{total_bytes}) {msg}",
                    )
                    .unwrap()
                    .progress_chars("=> "),
                );
                progress_bar.finish_with_message("Initial plotting finished!");
                finished_flag.store(true, Ordering::Relaxed);
                update_summary(Some(true), None)
                    .await
                    .expect("couldn't update summary");
            }
        });

        // solution subscriber
        tokio::spawn({
            let farmer_clone = farmer.clone();

            let farmed_blocks = get_farmed_block_count()
                .await
                .expect("couldn't read farmed blocks count from summary");
            let farmed_block_count = Arc::new(AtomicU64::new(farmed_blocks));
            async move {
                for plot in farmer_clone.iter_plots().await {
                    plot.subscribe_new_solutions()
                        .await
                        .for_each(|_solution| async {
                            let total_farmed = farmed_block_count.fetch_add(1, Ordering::Relaxed);
                            update_summary(None, Some(total_farmed))
                                .await
                                .expect("couldn't update summary");
                            if is_initial_progress_finished.load(Ordering::Relaxed) {
                                println!("You have farmed {total_farmed} block(s) in total!");
                            }
                        })
                        .await
                }
            }
        });
    }

    Ok(())
}

#[instrument]
async fn start_farming(farming_args: FarmingArgs) -> Result<(Farmer, Node)> {
    let FarmingArgs {
        reward_address,
        node,
        plot,
    } = farming_args;

    Ok((
        Farmer::builder()
            .build(reward_address, node.clone(), &[plot])
            .await?,
        node,
    ))
}

#[instrument]
async fn prepare_farming() -> Result<FarmingArgs> {
    let config_args = parse_config()?;

    let node_name = config_args.node_config_args.name;
    let chain = match config_args.node_config_args.chain.as_str() {
        "gemini-2a" => chain_spec::gemini_2a().unwrap(),
        "dev" => chain_spec::dev_config().unwrap(),
        _ => unreachable!("there are no other valid chain-specs at the moment"),
    };
    let is_validator = config_args.node_config_args.validator;
    let role = match is_validator {
        true => subspace_sdk::node::Role::Authority,
        false => subspace_sdk::node::Role::Full,
    };
    let node_directory = node_directory_getter();

    let node: Node = Node::builder()
        .name(node_name)
        .force_authoring(is_validator)
        .role(role)
        .build(node_directory, chain)
        .await
        .expect("error building the node");

    Ok(FarmingArgs {
        reward_address: config_args.farmer_config_args.reward_address,
        plot: config_args.farmer_config_args.plot,
        node,
    })
}

fn plotting_progress_bar(total_size: u64) -> ProgressBar {
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] {percent}% [{bar:40.cyan/blue}]
      ({bytes}/{total_bytes}) {bytes_per_sec}, {msg}, ETA: {eta}",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    pb.set_message("plotting");

    pb
}
