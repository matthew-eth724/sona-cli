//! splice — a video editing CLI for agents.
mod asr;
mod ffmpeg_install;
mod audio;
mod cache;
mod config;
mod caption;
mod cli;
mod ffmpeg;
mod frames;
mod graph;
mod history;
mod index;
mod media;
mod motion;
mod paths;
mod project;
mod render;
mod report;
mod text;
mod time;
mod transcript;

fn main() {
    cli::run();
}
