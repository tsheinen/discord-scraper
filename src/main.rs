#[macro_use]
extern crate log;

use color_eyre::eyre::Result;
use serenity::async_trait;
use serenity::client::{Client, Context, EventHandler};
use serenity::framework::standard::{
    macros::{command, group},
    CommandResult, StandardFramework,
};
use serenity::http::Http;
use serenity::model::channel::{ChannelType, GuildChannel, Message};
use serenity::model::id::{ChannelId, MessageId};

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serenity::http::GuildPagination::After;
use std::collections::HashMap;
use std::fs::OpenOptions;
use tokio::sync::mpsc::*;



async fn get_messages(
    channel: GuildChannel,
    after: u64,
    mut tx: Sender<Message>,
    pb: ProgressBar,
) -> Result<()> {
    let http = Http::new_with_token(
        &std::env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN env var must be set"),
    );
    info!("entering get_messages");
    let mut after = after;
    loop {
        info!("retrieving 50 messages");
        let mut messages: Vec<Message> = channel
            .messages(&http, |retriever| retriever.after(MessageId(after)))
            .await?;

        if messages.len() == 0 {
            break;
        }
        pb.inc(messages.len() as u64);
        messages.reverse();
        after = *messages.last().unwrap().id.as_u64();
        info!("sending messages over channel");
        for i in messages {
            tx.send(i).await;
        }
    }
    pb.set_message(&format!("{} done!", channel.name));
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    pretty_env_logger::init();
    let DISCORD_TOKEN = std::env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN env var must be set");
    let SERVER_ID: u64 = std::env::args().nth(1).expect("arg[1] to be defined").parse().expect("arg[1] to be u64");
    let m = MultiProgress::new();
    let mut http = Http::new_with_token(&DISCORD_TOKEN.clone());
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("messages.csv")?;
    let mut wtr = csv::Writer::from_writer(file);
    let (mut sx, mut rx) = channel(100);
    let channels = http
        .get_guild(SERVER_ID)
        .await?
        .channels(&http)
        .await?
        .into_iter()
        .filter(|(id, c)| c.kind == ChannelType::Text)
        .collect::<HashMap<ChannelId, GuildChannel>>();
    let channels_progress = m.add(ProgressBar::new(channels.len() as u64));
    channels_progress.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
            .progress_chars("##-"),
    );
    let spinner_style =
        ProgressStyle::default_spinner().template("[{elapsed_precise}] {msg} ({pos}/?)");
    let messages = futures::future::join_all(
        channels
            .into_iter()
            .enumerate()
            .map(|(index, (_, guild_channel))| {
                let mut sx = sx.clone();
                let pb = channels_progress.clone();
                let channel_pb = m.add(ProgressBar::new(1000));
                channel_pb.set_style(spinner_style.clone());
                channel_pb.set_message(&guild_channel.name);
                channel_pb.set_position(1 + index as u64);
                tokio::spawn(async move {
                    get_messages(guild_channel.clone(), 0, sx, channel_pb).await;
                    pb.inc(1);
                })
            }),
    );
    drop(sx);
    let finish = tokio::spawn(async move {
        while let Some(x) = rx.recv().await {
            wtr.write_record(&[
                x.id.0.to_string(),
                x.channel_id.0.to_string(),
                x.author.name,
                x.content.replace("\n", "\\n"),
            ]);
        }
        messages.await;
        channels_progress.finish_with_message("done!");
    });
    m.join().unwrap();
    finish.await;
    Ok(())
}
