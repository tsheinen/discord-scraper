use color_eyre::eyre::Result;

use serenity::http::Http;
use serenity::model::channel::{ChannelType, GuildChannel, Message};
use serenity::model::id::{ChannelId, MessageId};

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::fs::OpenOptions;
use tokio::sync::mpsc::*;

fn get_http() -> Http {
    Http::new_with_token(
        &std::env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN env var must be set"),
    )
}

async fn get_messages(
    channel: GuildChannel,
    after: u64,
    tx: Sender<Message>,
    pb: ProgressBar,
) -> Result<()> {
    let http = get_http();
    let mut after = after;
    loop {
        let mut messages: Vec<Message> = {
            loop {
                if let Ok(res) = channel
                    .messages(&http, |retriever| retriever.after(MessageId(after)))
                    .await
                {
                    break res;
                }
            }
        };
        if messages.len() == 0 {
            break;
        }
        pb.inc(messages.len() as u64);
        messages.reverse();
        after = *messages.last().unwrap().id.as_u64();
        for i in messages {
            tx.send(i).await.expect("channel send to work");
        }
    }
    pb.set_message(&format!("{} done!", channel.name));
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    pretty_env_logger::init();
    let server_id: u64 = std::env::args()
        .nth(1)
        .expect("arg[1] to be defined")
        .parse()
        .expect("arg[1] to be u64");
    let progress_bars = MultiProgress::new();
    let http = get_http();
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("messages.csv")?;
    let mut wtr = csv::Writer::from_writer(file);

    let (sx, mut rx) = channel(100);
    let channels = http
        .get_guild(server_id)
        .await?
        .channels(&http)
        .await?
        .into_iter()
        .filter(|(_, c)| c.kind == ChannelType::Text)
        .collect::<HashMap<ChannelId, GuildChannel>>();
    let global_progress_bar = progress_bars.add(ProgressBar::new(channels.len() as u64));
    global_progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
            .progress_chars("##-"),
    );
    let spinner_style =
        ProgressStyle::default_spinner().template("[{elapsed_precise}] {msg} ({pos}/?)");
    let messages = futures::future::join_all(channels.into_iter().enumerate().map(
        |(index, (_, guild_channel))| {
            let sx = sx.clone();
            let pb = global_progress_bar.clone();
            let channel_pb = progress_bars.add(ProgressBar::new(1000));
            channel_pb.set_style(spinner_style.clone());
            channel_pb.set_message(&guild_channel.name);
            channel_pb.set_position(1 + index as u64);
            tokio::spawn(async move {
                get_messages(guild_channel.clone(), 0, sx, channel_pb)
                    .await
                    .expect("get_messages failed");
                pb.inc(1);
            })
        },
    ));
    drop(sx);
    let finish = tokio::spawn(async move {
        while let Some(x) = rx.recv().await {
            wtr.write_record(&[
                x.id.0.to_string(),
                x.channel_id.0.to_string(),
                x.author.name,
                x.content.replace("\n", "\\n"),
            ])
            .expect("couldn't write message to csv");
        }
        messages.await;
        global_progress_bar.finish_with_message("done!");
    });
    progress_bars.join().unwrap();
    finish.await.expect("couldn't finish async tasks?");
    Ok(())
}
