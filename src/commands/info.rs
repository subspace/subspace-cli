use color_eyre::eyre::{eyre, Report, Result};
use single_instance::SingleInstance;

use crate::commands::farm::SINGLE_INSTANCE;
use crate::summary::Summary;

/// implementation of the `init` command.
///
/// informs the user about the current farming instance
pub(crate) async fn info() -> Result<()> {
    let instance = SingleInstance::new(SINGLE_INSTANCE).map_err(Report::msg)?;
    if !instance.is_single() {
        println!("A farmer instance is active!");
    } else {
        println!("There is no active farmer instance...");
    }

    let summary = Summary::new(None).await?;

    println!(
        "You have pledged to the network: {}",
        summary.get_user_space_pledged().await.map_err(|_| eyre!(
            "Couldn't read the summary file, are you sure you ran the farm command?"
        ))?
    );

    println!(
        "Total farmed blocks: {}",
        summary.get_farmed_block_count().await.map_err(|_| eyre!(
            "Couldn't read the summary file, are you sure you ran the farm command?"
        ))?
    );

    if summary.get_initial_plotting_progress().await.map_err(|_| {
        eyre!("Couldn't read the summary file, are you sure you ran the farm command?")
    })? {
        println!("Initial plotting is finished!");
    } else {
        println!("Initial plotting is not finished...");
    }

    Ok(())
}
